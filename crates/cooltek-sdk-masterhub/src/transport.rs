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
    crate::descriptor::output_report_length(read_descriptor(transport, &mut descriptor)?, report_id)
}

/// Queries and parses a descriptor to obtain a feature length including its ID.
pub fn feature_report_length<T: ReportTransport + ?Sized>(
    transport: &T,
    report_id: u8,
) -> HidResult<usize> {
    let mut descriptor = [0; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    crate::descriptor::feature_report_length(
        read_descriptor(transport, &mut descriptor)?,
        report_id,
    )
}

/// Rejects a fixed-size feature command whose descriptor declares another size.
///
/// The SDK builds feature commands with a fixed length (32 bytes, including the
/// report ID). Sending them to an interface declaring a different length would
/// be truncated or padded by the OS, so a positive mismatch is an error. When
/// the descriptor cannot be read or does not declare `report_id` as a feature
/// report, the check is inconclusive and passes: these declarations are not yet
/// verified on hardware, and a guess must not block otherwise working devices.
pub(crate) fn check_feature_report_length<T: ReportTransport + ?Sized>(
    transport: &T,
    report_id: u8,
    expected: usize,
) -> HidResult<()> {
    match feature_report_length(transport, report_id) {
        Ok(declared) if declared != expected => Err(HidError::HidApiError {
            message: format!(
                "Feature report {report_id:#04x} is declared as {declared} bytes by the HID \
                 descriptor, but the SDK sends fixed {expected}-byte feature reports"
            ),
        }),
        _ => Ok(()),
    }
}

fn read_descriptor<'a, T: ReportTransport + ?Sized>(
    transport: &T,
    buffer: &'a mut [u8],
) -> HidResult<&'a [u8]> {
    let length = transport.get_report_descriptor(buffer)?;
    if length == 0 || length > buffer.len() {
        return Err(HidError::HidApiError {
            message: "Transport returned an invalid report descriptor length".into(),
        });
    }
    Ok(&buffer[..length])
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

    struct DescriptorFake(Result<Vec<u8>, usize>);

    impl ReportTransport for DescriptorFake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("descriptor queries must not read input")
        }
        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("descriptor queries must not fetch feature reports")
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("descriptor queries must not write")
        }
        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            match &self.0 {
                Ok(bytes) => {
                    buffer[..bytes.len()].copy_from_slice(bytes);
                    Ok(bytes.len())
                }
                // Simulates a backend claiming an impossible length.
                Err(extra) => Ok(buffer.len() + extra),
            }
        }
    }

    const DESCRIPTOR: [u8; 14] = [
        0x75, 0x08, // Report Size (8)
        0x85, 0x02, 0x95, 0x3f, 0x91, 0x02, // ID 2: 63-byte output
        0x85, 0x03, 0x95, 0x1f, 0xb1, 0x02, // ID 3: 31-byte feature
    ];

    #[test]
    fn descriptor_lengths_reject_zero_and_overlong_transport_results() {
        for fake in [
            DescriptorFake(Ok(Vec::new())),
            DescriptorFake(Err(1)),
            DescriptorFake(Err(1000)),
        ] {
            let output = output_report_length(&fake, 2).unwrap_err().to_string();
            assert!(output.contains("invalid report descriptor length"));
            let feature = feature_report_length(&fake, 3).unwrap_err().to_string();
            assert!(feature.contains("invalid report descriptor length"));
            // Inconclusive descriptors do not block fixed-size commands.
            check_feature_report_length(&fake, 3, 32).unwrap();
        }
        let exact = DescriptorFake(Ok(vec![0x75, 0x08, 0x95, 0x01, 0x91, 0x02]));
        assert_eq!(output_report_length(&exact, 0).unwrap(), 2);
    }

    #[test]
    fn descriptor_lengths_are_derived_per_report_type() {
        let fake = DescriptorFake(Ok(DESCRIPTOR.to_vec()));
        assert_eq!(output_report_length(&fake, 2).unwrap(), 64);
        assert_eq!(feature_report_length(&fake, 3).unwrap(), 32);
        assert!(feature_report_length(&fake, 2).is_err());
        check_feature_report_length(&fake, 3, 32).unwrap();
        let mismatch = check_feature_report_length(&fake, 3, 16)
            .unwrap_err()
            .to_string();
        assert!(mismatch.contains("declared as 32 bytes"));
        assert!(mismatch.contains("fixed 16-byte"));
        // An undeclared feature ID is inconclusive rather than fatal.
        check_feature_report_length(&fake, 4, 32).unwrap();
    }

    #[test]
    fn rejects_impossible_read_lengths() {
        let mut reader =
            InputReader::new(Fake(RefCell::new(VecDeque::from([Read::InvalidLength]))));
        assert!(reader.read_input(10).is_err());
    }
}
