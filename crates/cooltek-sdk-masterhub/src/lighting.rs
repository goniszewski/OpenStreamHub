//! Per-LED lighting commands for the base, knob and fader modules.
//!
//! Recovered from the official Windows SDK (`Adjust_LED_Color_CM_M0`,
//! `Adjust_LED_Brightness_CM_M0`, `Adjust_LED_Mode_CM_M0`) and hardware-unverified.
//! Each command is an output report with ID 2 whose second byte selects the
//! command, followed by the LED module type and the LED index. The report length
//! comes from the selected interface's HID descriptor; nothing is guessed.
//!
//! The official application only ever addresses every LED of a module with the
//! same value, one call per index. Which LED an index lights is not yet known.

use hidapi::{HidError, HidResult};

use crate::ReportTransport;

/// Output report ID shared by the lighting commands.
pub const OUTPUT_REPORT_ID: u8 = 2;
/// Command byte of the colour command.
pub const COLOR_COMMAND: u8 = 0x2d;
/// Command byte of the brightness command.
pub const BRIGHTNESS_COMMAND: u8 = 0x2e;
/// Command byte of the mode (effect) command.
pub const MODE_COMMAND: u8 = 0x2c;
/// Index byte the SDK accepts for "every LED of the module" (knobs and faders only).
pub const ALL_LEDS: u8 = 0xff;
/// Highest effect number the SDK accepts.
pub const MAX_MODE: u8 = 9;
/// Shortest output report that holds any lighting command (ID, command and six bytes).
pub const MIN_REPORT_LENGTH: usize = 8;

/// Which module's LEDs a command addresses (the SDK's type byte).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedModule {
    /// The MasterHUB base's edge strip (type 0, a single LED index).
    Base,
    /// The three-knob module (type 1, indexes 0 through 4).
    Knob,
    /// The five-fader module (type 2, indexes 0 through 2).
    Slider,
}

impl LedModule {
    /// The SDK's type byte.
    pub const fn code(self) -> u8 {
        match self {
            Self::Base => 0,
            Self::Knob => 1,
            Self::Slider => 2,
        }
    }

    /// How many LED indexes the SDK accepts for this module.
    pub const fn led_count(self) -> u8 {
        match self {
            Self::Base => 1,
            Self::Knob => 5,
            Self::Slider => 3,
        }
    }
}

/// One LED of a module, or every LED at once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedTarget {
    /// A zero-based LED index below [`LedModule::led_count`].
    Index(u8),
    /// Every LED (`0xff`); the SDK refuses it for the base.
    All,
}

fn invalid(message: impl Into<String>) -> HidError {
    HidError::HidApiError {
        message: message.into(),
    }
}

fn header(command: u8, module: LedModule, target: LedTarget, length: usize) -> HidResult<Vec<u8>> {
    if length < MIN_REPORT_LENGTH {
        return Err(invalid(format!(
            "Output report length {length} is too short for a lighting command"
        )));
    }
    let index = match target {
        LedTarget::Index(index) if index < module.led_count() => index,
        LedTarget::Index(index) => {
            return Err(invalid(format!(
                "LED index {index} is out of range: this module has {} (0 through {})",
                module.led_count(),
                module.led_count() - 1
            )));
        }
        LedTarget::All if module == LedModule::Base => {
            return Err(invalid("The base has one LED; address it with index 0"));
        }
        LedTarget::All => ALL_LEDS,
    };
    let mut report = vec![0; length];
    report[..4].copy_from_slice(&[OUTPUT_REPORT_ID, command, module.code(), index]);
    Ok(report)
}

/// Builds the colour command. The three channels follow the official
/// application's argument order (red, green, blue); the order on the knob and
/// fader hardware is not yet verified. `save` asks the firmware to keep it.
///
/// # Errors
/// An out-of-range index, `All` for the base, or a report length below 8, before
/// any transport operation.
pub fn color_report(
    module: LedModule,
    target: LedTarget,
    channels: [u8; 3],
    save: bool,
    length: usize,
) -> HidResult<Vec<u8>> {
    let mut report = header(COLOR_COMMAND, module, target, length)?;
    report[4..7].copy_from_slice(&channels);
    report[7] = u8::from(save);
    Ok(report)
}

/// Builds the brightness command for a value from 0 through 100.
///
/// # Errors
/// A value above 100 (the dial's range; the SDK's own limit was not recovered),
/// an out-of-range index, `All` for the base, or a short report length.
pub fn brightness_report(
    module: LedModule,
    target: LedTarget,
    value: u8,
    length: usize,
) -> HidResult<Vec<u8>> {
    if value > 100 {
        return Err(invalid("LED brightness must be between 0 and 100"));
    }
    let mut report = header(BRIGHTNESS_COMMAND, module, target, length)?;
    report[4] = value;
    Ok(report)
}

/// Builds the mode command for an effect from 0 (off) through 9.
///
/// # Errors
/// A mode above 9, an out-of-range index, `All` for the base, or a short length.
pub fn mode_report(
    module: LedModule,
    target: LedTarget,
    mode: u8,
    save: bool,
    length: usize,
) -> HidResult<Vec<u8>> {
    if mode > MAX_MODE {
        return Err(invalid(format!(
            "LED mode must be between 0 and {MAX_MODE}"
        )));
    }
    let mut report = header(MODE_COMMAND, module, target, length)?;
    report[4] = mode;
    report[5] = u8::from(save);
    Ok(report)
}

/// Reads the output length for report ID 2 from the descriptor and writes one
/// report, failing on a short write. The target is validated before any write.
fn send<T: ReportTransport + ?Sized>(
    transport: &T,
    build: impl FnOnce(usize) -> HidResult<Vec<u8>>,
) -> HidResult<()> {
    let length = crate::output_report_length(transport, OUTPUT_REPORT_ID)?;
    let report = build(length)?;
    crate::transport::write_reports(transport, std::iter::once(report), |_| {})?;
    Ok(())
}

/// Sets one LED's colour (or all of a module's) through a physical or test transport.
///
/// # Errors
/// An invalid target, a descriptor without output report 2, or a transport failure.
pub fn set_color<T: ReportTransport + ?Sized>(
    transport: &T,
    module: LedModule,
    target: LedTarget,
    channels: [u8; 3],
    save: bool,
) -> HidResult<()> {
    send(transport, |length| {
        color_report(module, target, channels, save, length)
    })
}

/// Sets one LED's brightness (or all of a module's).
///
/// # Errors
/// As for [`set_color`], plus a value above 100.
pub fn set_brightness<T: ReportTransport + ?Sized>(
    transport: &T,
    module: LedModule,
    target: LedTarget,
    value: u8,
) -> HidResult<()> {
    send(transport, |length| {
        brightness_report(module, target, value, length)
    })
}

/// Sets one LED's effect (or all of a module's).
///
/// # Errors
/// As for [`set_color`], plus a mode above 9.
pub fn set_mode<T: ReportTransport + ?Sized>(
    transport: &T,
    module: LedModule,
    target: LedTarget,
    mode: u8,
    save: bool,
) -> HidResult<()> {
    send(transport, |length| {
        mode_report(module, target, mode, save, length)
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    // Output report 2 declared with 63 payload bytes, as on a captured M003K.
    const DESCRIPTOR: [u8; 17] = [
        0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x75, 0x08, 0x96, 0x3f, 0x00, 0x85, 0x02, 0x91, 0x02,
        0xc0, 0x00,
    ];

    #[derive(Default)]
    struct Fake {
        writes: RefCell<Vec<Vec<u8>>>,
        short: bool,
        no_descriptor: bool,
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            panic!("lighting commands never read")
        }

        fn get_feature_report(&self, _: &mut [u8]) -> HidResult<usize> {
            panic!("lighting commands never query")
        }

        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            panic!("lighting commands are output reports")
        }

        fn write_output(&self, report: &[u8]) -> HidResult<usize> {
            self.writes.borrow_mut().push(report.to_vec());
            Ok(if self.short {
                report.len() - 1
            } else {
                report.len()
            })
        }

        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            if self.no_descriptor {
                return Err(invalid("no descriptor"));
            }
            buffer[..DESCRIPTOR.len()].copy_from_slice(&DESCRIPTOR);
            Ok(DESCRIPTOR.len())
        }
    }

    #[test]
    fn color_report_matches_the_recovered_layout_and_pads_with_zeros() {
        let report = color_report(
            LedModule::Knob,
            LedTarget::Index(3),
            [0x11, 0x22, 0x33],
            false,
            64,
        )
        .unwrap();
        assert_eq!(report.len(), 64);
        assert_eq!(report[..8], [2, 0x2d, 1, 3, 0x11, 0x22, 0x33, 0]);
        assert!(report[8..].iter().all(|byte| *byte == 0));
        let saved = color_report(LedModule::Slider, LedTarget::All, [1, 2, 3], true, 8).unwrap();
        assert_eq!(saved, [2, 0x2d, 2, 0xff, 1, 2, 3, 1]);
    }

    #[test]
    fn brightness_and_mode_reports_match_the_recovered_layout() {
        let brightness = brightness_report(LedModule::Base, LedTarget::Index(0), 100, 16).unwrap();
        assert_eq!(brightness[..5], [2, 0x2e, 0, 0, 100]);
        let mode = mode_report(LedModule::Knob, LedTarget::All, 9, true, 16).unwrap();
        assert_eq!(mode[..6], [2, 0x2c, 1, 0xff, 9, 1]);
    }

    #[test]
    fn indexes_follow_each_modules_range() {
        let color = |module, target| color_report(module, target, [0; 3], false, 64);
        assert!(color(LedModule::Knob, LedTarget::Index(4)).is_ok());
        assert!(color(LedModule::Knob, LedTarget::Index(5)).is_err());
        assert!(color(LedModule::Slider, LedTarget::Index(2)).is_ok());
        assert!(color(LedModule::Slider, LedTarget::Index(3)).is_err());
        assert!(color(LedModule::Base, LedTarget::Index(0)).is_ok());
        assert!(color(LedModule::Base, LedTarget::Index(1)).is_err());
        assert!(color(LedModule::Base, LedTarget::All).is_err());
        assert!(color(LedModule::Knob, LedTarget::All).is_ok());
    }

    #[test]
    fn out_of_range_values_and_short_reports_are_refused() {
        assert!(brightness_report(LedModule::Knob, LedTarget::All, 101, 64).is_err());
        assert!(mode_report(LedModule::Knob, LedTarget::All, 10, false, 64).is_err());
        assert!(color_report(LedModule::Knob, LedTarget::All, [0; 3], false, 7).is_err());
    }

    #[test]
    fn setters_use_the_descriptor_length_and_write_once() {
        let fake = Fake::default();
        set_color(
            &fake,
            LedModule::Knob,
            LedTarget::Index(1),
            [9, 8, 7],
            false,
        )
        .unwrap();
        let writes = fake.writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].len(), 64);
        assert_eq!(writes[0][..8], [2, 0x2d, 1, 1, 9, 8, 7, 0]);
    }

    #[test]
    fn nothing_is_written_for_a_bad_target_or_a_missing_descriptor() {
        let fake = Fake::default();
        assert!(set_brightness(&fake, LedModule::Slider, LedTarget::Index(3), 50).is_err());
        assert!(set_mode(&fake, LedModule::Base, LedTarget::All, 1, false).is_err());
        let blind = Fake {
            no_descriptor: true,
            ..Fake::default()
        };
        assert!(set_color(&blind, LedModule::Knob, LedTarget::All, [0; 3], false).is_err());
        assert!(fake.writes.borrow().is_empty() && blind.writes.borrow().is_empty());
    }

    #[test]
    fn a_short_write_is_an_error() {
        let fake = Fake {
            short: true,
            ..Fake::default()
        };
        assert!(set_color(&fake, LedModule::Knob, LedTarget::All, [0; 3], false).is_err());
    }
}
