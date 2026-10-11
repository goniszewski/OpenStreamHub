//! The Lighting page's dial writes: backlight (0–100) and one solid colour,
//! applied to M131E dials only.
//!
//! The reports are the SDK's recovered dial feature reports
//! ([`dial`]). The write order is the sequence verified on hardware
//! (docs/masterhub-hardware-results.md): lighting takes effect in software
//! mode, so an apply puts the dial in software mode first — the built-in
//! animation stops and the screen stays dark until a run draws on it — and
//! the reset command leaves software mode, which returns the animation.

use cooltek_sdk_cli::{
    capture::encode_hex,
    device::{parse_path, select_device},
};
use cooltek_sdk_masterhub::lighting::{self as led, LedModule, LedTarget};
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

/// What one LED module is told: a colour, an effect number and a brightness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedLook {
    pub colour: [u8; 3],
    pub mode: u8,
    pub brightness: u8,
}

/// The LED module a module kind is (knobs, faders and the base), or `None` for
/// kinds the app cannot light per LED (the dial has its own feature-report lighting).
pub fn led_module(kind: Option<ModuleKind>) -> Option<LedModule> {
    match kind? {
        ModuleKind::M003K => Some(LedModule::Knob),
        ModuleKind::M005S => Some(LedModule::Slider),
        ModuleKind::M0064 => Some(LedModule::Base),
        _ => None,
    }
}

/// Writes one module's lighting the way the official app does, per LED:
/// colour, then effect, then brightness, for one LED or each LED in turn
/// (the app never sends the SDK's `0xff` index). Everything is validated
/// before the first write, so a bad value or LED writes nothing; a transport
/// failure stops at the failing write, and the next apply rewrites everything.
pub fn write_led_lighting<T: ReportTransport + ?Sized>(
    transport: &T,
    module: LedModule,
    led_index: Option<u8>,
    look: LedLook,
) -> Result<(), String> {
    if look.mode > led::MAX_MODE {
        return Err(format!(
            "the effect must be between 0 and {}",
            led::MAX_MODE
        ));
    }
    if look.brightness > 100 {
        return Err("the brightness must be between 0 and 100".into());
    }
    let leds: Vec<u8> = match led_index {
        Some(index) if index < module.led_count() => vec![index],
        Some(index) => {
            return Err(format!(
                "LED {} does not exist: this module has {}",
                u16::from(index) + 1,
                module.led_count()
            ));
        }
        None => (0..module.led_count()).collect(),
    };
    for index in leds {
        let target = LedTarget::Index(index);
        let label = u16::from(index) + 1;
        led::set_color(transport, module, target, look.colour, false)
            .map_err(|error| format!("could not set the colour of LED {label}: {error}"))?;
        led::set_mode(transport, module, target, look.mode, false)
            .map_err(|error| format!("could not set the effect of LED {label}: {error}"))?;
        led::set_brightness(transport, module, target, look.brightness)
            .map_err(|error| format!("could not set the brightness of LED {label}: {error}"))?;
    }
    Ok(())
}

/// The one inventory entry an LED target must name: a knob or fader module, or the base.
pub fn select_led_identity<'a>(
    devices: &'a [ModuleIdentity],
    path: &[u8],
) -> Result<(&'a ModuleIdentity, LedModule), String> {
    let selected = select_device(devices, path, false).map_err(|error| match error.as_str() {
        cooltek_sdk_cli::device::NOT_IN_INVENTORY => {
            "the module went away or is still connecting; try again".to_string()
        }
        other => format!("could not select the module: {other}"),
    })?;
    let module = led_module(ModuleKind::from_ids(
        selected.vendor_id,
        selected.product_id,
    ))
    .ok_or("per-LED lighting works on knob and fader modules and the base only")?;
    Ok((selected, module))
}

/// A connected MasterHUB base: it takes lighting but no input, so it is not in
/// the input feed's inventory and is listed separately for the Lighting page.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct BaseModule {
    pub path_hex: String,
    pub module: &'static str,
    pub serial_number: Option<String>,
}

/// Every connected base, one entry per HID path, in path order.
pub fn base_modules(devices: &[ModuleIdentity]) -> Vec<BaseModule> {
    let mut listed: Vec<BaseModule> = Vec::new();
    for device in devices {
        if ModuleKind::from_ids(device.vendor_id, device.product_id) != Some(ModuleKind::M0064) {
            continue;
        }
        let path_hex = encode_hex(device.path.as_bytes());
        if listed.iter().any(|entry| entry.path_hex == path_hex) {
            continue;
        }
        listed.push(BaseModule {
            path_hex,
            module: "m0064",
            serial_number: device.serial_number.clone(),
        });
    }
    listed.sort_by(|a, b| a.path_hex.cmp(&b.path_hex));
    listed
}

/// Applies one look to every target knob or fader module, in order, stopping at
/// the first failure. All targets are opened first, so a wrong target never
/// leaves an earlier one half-written. Returns the number of modules written.
pub fn apply_led_lighting(
    registry: &DeviceRegistry,
    targets: &[String],
    led_index: Option<u8>,
    look: LedLook,
) -> Result<usize, String> {
    let inventory = registry.devices();
    let mut opened = Vec::new();
    for target in targets {
        let path = parse_path(target)?;
        let (selected, module) = select_led_identity(&inventory, &path)?;
        let label = match selected.serial_number.as_deref() {
            Some(serial) => format!("the module with serial {serial}"),
            None => "the module".into(),
        };
        let device = registry
            .open(selected)
            .map_err(|error| format!("{label}: could not open it: {error}"))?;
        opened.push((device, module, label));
    }
    for (device, module, label) in &opened {
        write_led_lighting(device, *module, led_index, look)
            .map_err(|error| format!("{label}: {error}"))?;
    }
    Ok(opened.len())
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

    /// Records output reports; the descriptor declares report 2 with 63 payload bytes.
    #[derive(Default)]
    struct OutputFake {
        outputs: RefCell<Vec<Vec<u8>>>,
        fail_at: Option<usize>,
    }

    impl ReportTransport for OutputFake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("lighting writes never poll input")
        }

        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("lighting writes never query feature reports")
        }

        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("LED lighting uses output reports")
        }

        fn write_output(&self, report: &[u8]) -> HidResult<usize> {
            let index = self.outputs.borrow().len();
            self.outputs.borrow_mut().push(report.to_vec());
            if self.fail_at == Some(index) {
                return Err(HidError::HidApiError {
                    message: "disconnected".into(),
                });
            }
            Ok(report.len())
        }

        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            const DESCRIPTOR: [u8; 17] = [
                0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x75, 0x08, 0x96, 0x3f, 0x00, 0x85, 0x02, 0x91,
                0x02, 0xc0, 0x00,
            ];
            buffer[..DESCRIPTOR.len()].copy_from_slice(&DESCRIPTOR);
            Ok(DESCRIPTOR.len())
        }
    }

    const LOOK: LedLook = LedLook {
        colour: [0x12, 0x34, 0x56],
        mode: 5,
        brightness: 80,
    };

    fn heads(fake: &OutputFake) -> Vec<Vec<u8>> {
        fake.outputs
            .borrow()
            .iter()
            .map(|r| r[..7].to_vec())
            .collect()
    }

    #[test]
    fn one_led_gets_colour_effect_brightness_in_the_official_apps_order() {
        let fake = OutputFake::default();
        write_led_lighting(&fake, LedModule::Knob, Some(1), LOOK).unwrap();
        assert_eq!(
            heads(&fake),
            [
                vec![2, 0x2d, 2, 1, 0x12, 0x34, 0x56],
                vec![2, 0x2c, 2, 1, 5, 0, 0],
                vec![2, 0x2e, 2, 1, 80, 0, 0],
            ]
        );
        assert!(
            fake.outputs
                .borrow()
                .iter()
                .all(|report| report.len() == 64)
        );
    }

    #[test]
    fn all_leds_are_written_one_index_at_a_time_never_with_the_all_index() {
        let knobs = OutputFake::default();
        write_led_lighting(&knobs, LedModule::Knob, None, LOOK).unwrap();
        let faders = OutputFake::default();
        write_led_lighting(&faders, LedModule::Slider, None, LOOK).unwrap();
        assert_eq!(knobs.outputs.borrow().len(), 9);
        assert_eq!(faders.outputs.borrow().len(), 15);
        for (fake, type_byte, count) in [(&knobs, 2, 3), (&faders, 1, 5)] {
            let indexes: Vec<u8> = fake.outputs.borrow().iter().map(|r| r[3]).collect();
            assert!(indexes.iter().all(|&index| index < count), "{indexes:?}");
            assert!(fake.outputs.borrow().iter().all(|r| r[2] == type_byte));
        }
    }

    #[test]
    fn bad_values_and_leds_write_nothing() {
        let fake = OutputFake::default();
        for (module, index, look) in [
            (LedModule::Knob, Some(3), LOOK),
            (LedModule::Slider, Some(5), LOOK),
            (LedModule::Knob, None, LedLook { mode: 10, ..LOOK }),
            (
                LedModule::Knob,
                None,
                LedLook {
                    brightness: 101,
                    ..LOOK
                },
            ),
        ] {
            assert!(write_led_lighting(&fake, module, index, look).is_err());
        }
        assert!(fake.outputs.borrow().is_empty());
    }

    #[test]
    fn an_led_transport_failure_names_the_step_and_stops() {
        let fake = OutputFake {
            fail_at: Some(4),
            ..OutputFake::default()
        };
        let error = write_led_lighting(&fake, LedModule::Knob, None, LOOK).unwrap_err();
        assert!(error.contains("effect of LED 2"), "{error}");
        assert_eq!(fake.outputs.borrow().len(), 5);
    }

    #[test]
    fn bases_are_listed_once_per_path_and_nothing_else() {
        let devices = [
            identity("kn", 0x0202, Some("MHKB03")),
            identity("bs", 0x01f6, Some("MHBS01")),
            identity("bs", 0x01f6, Some("MHBS01")),
            identity("b2", 0x9999, None),
        ];
        let bases = base_modules(&devices);
        assert_eq!(bases.len(), 2);
        assert!(bases.iter().all(|base| base.module == "m0064"));
        assert_eq!(bases[0].path_hex, encode_hex(b"b2"));
        assert_eq!(bases[1].serial_number.as_deref(), Some("MHBS01"));
        assert!(base_modules(&devices[..1]).is_empty());
    }

    #[test]
    fn led_selection_accepts_knobs_faders_and_the_base_only() {
        let devices = [
            identity("kn", 0x0202, Some("MHKB03")),
            identity("fd", 0x0200, None),
            identity("dl", 0x0206, None),
        ];
        assert_eq!(
            select_led_identity(&devices, b"kn").unwrap().1,
            LedModule::Knob
        );
        assert_eq!(
            select_led_identity(&devices, b"fd").unwrap().1,
            LedModule::Slider
        );
        let error = select_led_identity(&devices, b"dl").unwrap_err();
        assert!(error.contains("knob and fader"), "{error}");
        let error = select_led_identity(&devices, b"zz").unwrap_err();
        assert!(error.contains("went away"), "{error}");
    }
}
