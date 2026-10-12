//! The base's own orientation reading.
//!
//! Recovered from the base firmware (M0064 2.0.4.0, studied offline) and checked once on hardware: the base has an
//! accelerometer and answers **feature report 0x35** with one state byte, `35 00 <state> 00 …`, where the state is
//! 0 or 1. It also sends the same byte as an input report `01 2b 01 00 <state>` when it changes. The sensor tells two
//! positions apart, not four rotations. Which value is which position is read from the module, not known from the
//! firmware; see the hardware results.

use crate::ReportTransport;

/// Feature report ID the base answers with its orientation state.
pub const ORIENTATION_REPORT: u8 = 0x35;

/// Reads the base's orientation state (0 or 1) with one GET FEATURE and nothing else.
///
/// # Errors
/// Fails when the read fails or the reply is not report 0x35 holding a 0 or 1.
pub fn read_orientation_state<T: ReportTransport + ?Sized>(transport: &T) -> Result<u8, String> {
    let mut buffer = [0u8; 32];
    buffer[0] = ORIENTATION_REPORT;
    let read = transport
        .get_feature_report(&mut buffer)
        .map_err(|error| format!("could not read the base's orientation: {error}"))?;
    if read < 3 || buffer[0] != ORIENTATION_REPORT || buffer[2] > 1 {
        return Err("the base's orientation came back in an unknown shape".into());
    }
    Ok(buffer[2])
}

#[cfg(test)]
mod tests {
    use hidapi::{HidError, HidResult};

    use super::*;

    struct Fake(Result<Vec<u8>, ()>);

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("the orientation read never reads input")
        }

        fn get_feature_report(&self, buffer: &mut [u8]) -> HidResult<usize> {
            assert_eq!(buffer[0], ORIENTATION_REPORT);
            match &self.0 {
                Ok(bytes) => {
                    buffer[..bytes.len()].copy_from_slice(bytes);
                    Ok(bytes.len())
                }
                Err(()) => Err(HidError::HidApiError {
                    message: "unplugged".into(),
                }),
            }
        }

        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("the orientation read never sets a feature")
        }

        fn write_output(&self, _: &[u8]) -> HidResult<usize> {
            panic!("the orientation read never writes")
        }

        fn get_report_descriptor(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("the orientation read needs no descriptor")
        }
    }

    #[test]
    fn reads_the_state_byte_the_base_sent_on_hardware() {
        let mut captured = vec![0x35, 0x00, 0x01];
        captured.resize(32, 0);
        assert_eq!(read_orientation_state(&Fake(Ok(captured))), Ok(1));
        let mut other = vec![0x35, 0x00, 0x00];
        other.resize(32, 0);
        assert_eq!(read_orientation_state(&Fake(Ok(other))), Ok(0));
    }

    #[test]
    fn refuses_a_reply_of_another_shape_or_a_failed_read() {
        assert!(read_orientation_state(&Fake(Ok(vec![0x35, 0x00]))).is_err());
        assert!(read_orientation_state(&Fake(Ok(vec![0x35, 0x00, 0x02]))).is_err());
        assert!(read_orientation_state(&Fake(Ok(vec![0x2d, 0x00, 0x01]))).is_err());
        assert!(read_orientation_state(&Fake(Err(()))).is_err());
    }
}
