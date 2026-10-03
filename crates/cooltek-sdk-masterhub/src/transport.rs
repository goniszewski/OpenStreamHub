use hidapi::{HidDevice, HidError, HidResult};

/// HID report operations used by the SDK, independent of a physical device.
///
/// Implement this trait for a fake transport to test commands, short reads, and
/// failures offline. Buffers follow HIDAPI's report-ID conventions. The returned
/// length must describe bytes actually filled, never the buffer capacity.
pub trait ReportTransport {
    /// Reads an input report; a zero length means the timeout expired.
    fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize>;

    /// Fetches a feature report whose ID is initially in `buffer[0]`.
    fn get_feature_report(&self, buffer: &mut [u8]) -> HidResult<usize>;

    /// Sends a feature report, including its ID in `data[0]`.
    fn send_feature_report(&self, data: &[u8]) -> HidResult<()>;

    /// Writes an output report including its report ID, returning bytes written.
    ///
    /// The default fails explicitly so existing read-only test transports remain
    /// compatible. A short write must not be treated as a successful transfer.
    fn write_output(&self, _data: &[u8]) -> HidResult<usize> {
        Err(HidError::HidApiError {
            message: "Transport does not support output-report writes".into(),
        })
    }

    /// Reads the HID report descriptor without initializing the device.
    ///
    /// Returns its actual length. Backends without descriptor access must fail;
    /// display code must not fall back to a guessed output report size.
    fn get_report_descriptor(&self, _buffer: &mut [u8]) -> HidResult<usize> {
        Err(HidError::HidApiError {
            message: "Transport does not support report descriptor queries".into(),
        })
    }
}

impl ReportTransport for HidDevice {
    fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
        HidDevice::read_timeout(self, buffer, timeout_ms)
    }

    fn get_feature_report(&self, buffer: &mut [u8]) -> HidResult<usize> {
        HidDevice::get_feature_report(self, buffer)
    }

    fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
        HidDevice::send_feature_report(self, data)
    }

    fn write_output(&self, data: &[u8]) -> HidResult<usize> {
        HidDevice::write(self, data)
    }

    fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
        HidDevice::get_report_descriptor(self, buffer)
    }
}

/// Queries and parses a descriptor to obtain an output length including its ID.
///
/// Errors propagate rather than substituting a default packet size.
pub fn output_report_length<T: ReportTransport + ?Sized>(
    transport: &T,
    report_id: u8,
) -> HidResult<usize> {
    let mut descriptor = [0; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let length = transport.get_report_descriptor(&mut descriptor)?;
    if length == 0 || length > descriptor.len() {
        return Err(HidError::HidApiError {
            message: "Transport returned an invalid report descriptor length".into(),
        });
    }
    crate::descriptor::output_report_length(&descriptor[..length], report_id)
}

/// A single-owner input reader with bounded read timeouts and a reusable buffer.
///
/// Construction and reading never initialize a module or send configuration.
/// This is a diagnostic foundation, not an automatic reconnect manager.
pub struct InputReader<T: ReportTransport> {
    transport: T,
    buffer: Vec<u8>,
}

impl<T: ReportTransport> InputReader<T> {
    /// Maximum accepted diagnostic input report length.
    pub const MAX_REPORT_SIZE: usize = 4096;

    /// Creates a reader without issuing transport operations.
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            buffer: vec![0; Self::MAX_REPORT_SIZE],
        }
    }

    /// Returns exactly the received bytes, or `None` after an idle timeout.
    ///
    /// Rejects zero/negative timeouts and impossible transport lengths. Errors
    /// propagate to the caller; an unplug is never mistaken for an idle timeout.
    pub fn read_input(&mut self, timeout_ms: i32) -> HidResult<Option<&[u8]>> {
        if timeout_ms <= 0 {
            return Err(HidError::HidApiError {
                message: "Input timeout must be a positive number of milliseconds".into(),
            });
        }
        let length = self.transport.read_timeout(&mut self.buffer, timeout_ms)?;
        if length > self.buffer.len() {
            return Err(HidError::HidApiError {
                message: "Transport returned an input length exceeding the buffer".into(),
            });
        }
        Ok((length != 0).then_some(&self.buffer[..length]))
    }

    /// Borrows the transport for explicit descriptor/feature operations.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Releases ownership of the transport without sending a command.
    pub fn into_transport(self) -> T {
        self.transport
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::*;

    enum Read {
        Data(Vec<u8>),
        Idle,
        Disconnected,
        InvalidLength,
    }

    struct Fake(RefCell<VecDeque<Read>>);

    impl ReportTransport for Fake {
        fn read_timeout(&self, buffer: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
            assert!(timeout_ms > 0);
            match self.0.borrow_mut().pop_front().unwrap() {
                Read::Data(data) => {
                    buffer[..data.len()].copy_from_slice(&data);
                    Ok(data.len())
                }
                Read::Idle => Ok(0),
                Read::Disconnected => Err(HidError::HidApiError {
                    message: "disconnected".into(),
                }),
                Read::InvalidLength => Ok(buffer.len() + 1),
            }
        }

        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("input reader must not issue feature queries")
        }

        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("input reader must not initialize or write")
        }
    }

    #[test]
    fn preserves_short_reads_and_distinguishes_idle_from_disconnect() {
        let fake = Fake(RefCell::new(VecDeque::from([
            Read::Data(vec![1, 2, 3, 4]),
            Read::Data(vec![5]),
            Read::Idle,
            Read::Disconnected,
        ])));
        let mut reader = InputReader::new(fake);
        assert_eq!(
            reader.read_input(10).unwrap(),
            Some([1, 2, 3, 4].as_slice())
        );
        assert_eq!(reader.read_input(10).unwrap(), Some([5].as_slice()));
        assert_eq!(reader.read_input(10).unwrap(), None);
        assert!(reader.read_input(10).is_err());
    }

    #[test]
    fn rejects_invalid_timeouts_without_calling_transport() {
        let mut reader = InputReader::new(Fake(RefCell::new(VecDeque::new())));
        assert!(reader.read_input(0).is_err());
        assert!(reader.read_input(-1).is_err());
    }

    #[test]
    fn rejects_impossible_read_lengths() {
        let mut reader =
            InputReader::new(Fake(RefCell::new(VecDeque::from([Read::InvalidLength]))));
        assert!(reader.read_input(10).is_err());
    }
}
