//! The Lighting page's dial writes: backlight (0–100) and one solid colour,
//! applied to M131E dials only.
//!
//! The reports are the SDK's recovered dial feature reports
//! ([`dial`]). The write order is the sequence verified on hardware
//! (docs/masterhub-hardware-results.md): lighting takes effect in software
//! mode, so an apply puts the dial in software mode first — the built-in
//! animation stops and the screen stays dark until a run draws on it — and
//! the reset command leaves software mode, which returns the animation.

use cooltek_sdk_cli::device::{parse_path, select_device};
use cooltek_sdk_masterhub::{DeviceRegistry, ModuleIdentity, ModuleKind, ReportTransport, dial};

/// Writes one dial's lighting in the order hardware verified: software mode,
/// then the backlight, then the solid colour. The backlight is validated
/// before any write, so an out-of-range value writes nothing.
///
/// A failure stops at the first failing write; earlier reports may have
/// reached the device. The next apply rewrites all three.
pub fn write_dial_lighting<T: ReportTransport + ?Sized>(
    transport: &T,
    backlight: u8,
    colour: [u8; 3],
) -> Result<(), String> {
    dial::backlight_report(backlight)
        .map_err(|error| format!("the backlight must be between 0 and 100: {error}"))?;
    dial::set_software_mode(transport, true)
        .map_err(|error| format!("could not put the dial in software mode: {error}"))?;
    dial::set_backlight(transport, backlight)
        .map_err(|error| format!("could not set the dial backlight: {error}"))?;
    dial::set_color(transport, colour)
        .map_err(|error| format!("could not set the dial colour: {error}"))
}

/// Hands one dial back to its built-in mode, which returns its own animation.
pub fn restore_dial_mode<T: ReportTransport + ?Sized>(transport: &T) -> Result<(), String> {
    dial::set_software_mode(transport, false)
        .map_err(|error| format!("could not return the dial to its built-in mode: {error}"))
}

/// The one inventory entry a lighting target must name: exactly one interface,
/// and an M131E dial. Other modules' lighting is not available yet, and a
/// wrong target must be refused before anything is opened.
pub fn select_dial_identity<'a>(
    devices: &'a [ModuleIdentity],
    path: &[u8],
) -> Result<&'a ModuleIdentity, String> {
    let selected = select_device(devices, path, false).map_err(|error| match error.as_str() {
        // The shared selector's message names the CLI; the app says what
        // the situation means for its own inventory instead.
        cooltek_sdk_cli::device::NOT_IN_INVENTORY => {
            "the dial went away or is still connecting; try again".to_string()
        }
        other => format!("could not select the dial: {other}"),
    })?;
    if ModuleKind::from_ids(selected.vendor_id, selected.product_id) != Some(ModuleKind::M131E) {
        return Err(
            "dial lighting works on an M131E dial only; other modules are not supported yet".into(),
        );
    }
    Ok(selected)
}

/// Names a dial for error messages: the serial when the inventory has one.
fn dial_label(selected: &ModuleIdentity) -> String {
    match selected.serial_number.as_deref() {
        Some(serial) => format!("the dial with serial {serial}"),
        None => "the dial".into(),
    }
}

/// Opens every target dial up front, failing on the first bad one so a wrong
/// target never leaves an earlier one half-written. The labels travel with the
/// open devices for the write errors.
fn open_dials(
    registry: &DeviceRegistry,
    targets: &[String],
) -> Result<Vec<(hidapi::HidDevice, String)>, String> {
    let inventory = registry.devices();
    let mut opened = Vec::new();
    for target in targets {
        let path = parse_path(target)?;
        let selected = select_dial_identity(&inventory, &path)?;
        let label = dial_label(selected);
        let device = registry
            .open(selected)
            .map_err(|error| format!("{label}: could not open it: {error}"))?;
        opened.push((device, label));
    }
    Ok(opened)
}

/// Applies the backlight and colour to every target dial, in order, stopping
/// at the first failure. Returns the number of dials written.
pub fn apply_dial_lighting(
    registry: &DeviceRegistry,
    targets: &[String],
    backlight: u8,
    colour: [u8; 3],
) -> Result<usize, String> {
    let dials = open_dials(registry, targets)?;
    for (device, label) in &dials {
        write_dial_lighting(device, backlight, colour)
            .map_err(|error| format!("{label}: {error}"))?;
    }
    Ok(dials.len())
}

/// Returns every target dial to its built-in mode, in order, stopping at the
/// first failure. Returns the number of dials reset.
pub fn reset_dial_lighting(registry: &DeviceRegistry, targets: &[String]) -> Result<usize, String> {
    let dials = open_dials(registry, targets)?;
    for (device, label) in &dials {
        restore_dial_mode(device).map_err(|error| format!("{label}: {error}"))?;
    }
    Ok(dials.len())
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use cooltek_sdk_masterhub::hidapi::{HidError, HidResult};
    use cooltek_sdk_masterhub::{ModuleIdentity, ReportTransport};

    use super::*;

    #[derive(Default)]
    struct Fake {
        feature_writes: RefCell<Vec<Vec<u8>>>,
        /// Zero-based index of the feature write that fails, or a value past
        /// the end when every write succeeds.
        fail_at: AtomicUsize,
    }

    impl Fake {
        /// Every write succeeds.
        fn succeeding() -> Self {
            Self {
                fail_at: AtomicUsize::new(usize::MAX),
                ..Self::default()
            }
        }

        /// The zero-based feature write that fails (and every write after it).
        fn failing(at: usize) -> Self {
            Self {
                fail_at: AtomicUsize::new(at),
                ..Self::default()
            }
        }
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("lighting writes never poll input")
        }

        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("lighting writes never query feature reports")
        }

        fn send_feature_report(&self, report: &[u8]) -> HidResult<()> {
            let index = self.feature_writes.borrow().len();
            self.feature_writes.borrow_mut().push(report.to_vec());
            if index >= self.fail_at.load(Ordering::SeqCst) {
                return Err(HidError::HidApiError {
                    message: "disconnected".into(),
                });
            }
            Ok(())
        }

        fn write_output(&self, _: &[u8]) -> HidResult<usize> {
            panic!("lighting writes never send output reports")
        }

        fn get_report_descriptor(&self, _: &mut [u8]) -> HidResult<usize> {
            // An unreadable descriptor must not block the command.
            Err(HidError::HidApiError {
                message: "no descriptor".into(),
            })
        }
    }

    fn written(fake: &Fake) -> Vec<Vec<u8>> {
        fake.feature_writes.borrow().clone()
    }

    #[test]
    fn lighting_write_sends_mode_backlight_colour_in_order() {
        for backlight in [0u8, 1, 72, 100] {
            let fake = Fake::succeeding();
            write_dial_lighting(&fake, backlight, [0x12, 0x34, 0x56]).unwrap();
            let writes = written(&fake);
            let expected: [&[u8]; 3] = [
                &[0x03, 0x49, 1],
                &[0x03, 0x44, backlight],
                &[0x03, 0x46, 0x12, 0x34, 0x56],
            ];
            assert_eq!(writes.len(), expected.len(), "backlight {backlight}");
            for (report, prefix) in writes.iter().zip(expected) {
                assert_eq!(report.len(), 32);
                assert_eq!(&report[..prefix.len()], prefix);
                assert!(report[prefix.len()..].iter().all(|&byte| byte == 0));
            }
        }
    }

    #[test]
    fn invalid_backlight_writes_nothing() {
        for value in 101..=u8::MAX {
            let fake = Fake::succeeding();
            let error = write_dial_lighting(&fake, value, [0; 3]).unwrap_err();
            assert!(error.contains("between 0 and 100"), "{error}");
            assert!(written(&fake).is_empty(), "value {value} must not write");
        }
    }

    #[test]
    fn transport_failure_stops_at_the_failing_write() {
        let fake = Fake::failing(0);
        assert!(write_dial_lighting(&fake, 50, [0; 3]).is_err());
        assert_eq!(written(&fake).len(), 1);

        // The backlight write may have reached the device before the colour
        // write failed; the error is the caller's signal to stop.
        let fake = Fake::failing(2);
        let error = write_dial_lighting(&fake, 50, [0; 3]).unwrap_err();
        assert!(error.contains("colour"), "{error}");
        assert_eq!(written(&fake).len(), 3);
    }

    #[test]
    fn reset_sends_only_the_leave_software_mode_write() {
        let fake = Fake::succeeding();
        restore_dial_mode(&fake).unwrap();
        let writes = written(&fake);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].len(), 32);
        assert_eq!(&writes[0][..3], &[0x03, 0x49, 0]);
        assert!(writes[0][3..].iter().all(|&byte| byte == 0));

        let fake = Fake::failing(0);
        assert!(restore_dial_mode(&fake).is_err());
        assert_eq!(written(&fake).len(), 1);
    }

    fn identity(path: &str, product_id: u16, serial: Option<&str>) -> ModuleIdentity {
        ModuleIdentity {
            path: std::ffi::CString::new(path).unwrap(),
            vendor_id: 0x2516,
            product_id,
            serial_number: serial.map(str::to_owned),
            manufacturer_string: None,
            product_string: None,
            release_number: 0,
            interface_number: -1,
            usage_page: 0xff00,
            usage: 1,
            module: ModuleKind::from_ids(0x2516, product_id),
        }
    }

    #[test]
    fn dial_selection_accepts_one_dial_path_only() {
        let devices = [
            identity("aa", 0x0206, Some("MHDD01")),
            identity("bb", 0x9994, None),
            // The keypad shares the vendor; its lighting is not available yet.
            identity("cc", 0x0207, Some("MHKE01")),
        ];
        assert_eq!(
            select_dial_identity(&devices, b"aa")
                .unwrap()
                .serial_number
                .as_deref(),
            Some("MHDD01")
        );
        assert_eq!(
            select_dial_identity(&devices, b"bb").unwrap().serial_number,
            None
        );
        for (path, fragment) in [
            (&b"cc"[..], "M131E dial only"),
            (&b"zz"[..], "went away or is still connecting"),
        ] {
            let error = select_dial_identity(&devices, path).unwrap_err();
            assert!(error.contains(fragment), "{error}");
        }
        // Two interfaces on one path stay refused, like every exact selection.
        let ambiguous = [identity("dd", 0x0206, None), identity("dd", 0x0206, None)];
        let error = select_dial_identity(&ambiguous, b"dd").unwrap_err();
        assert!(error.contains("ambiguous"), "{error}");
    }
}
