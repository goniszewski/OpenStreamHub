use crate::{Action, Binding, InputEvent, InputValue, Profile, ServiceModule, TimerAlert};
use std::sync::{Arc, atomic::AtomicBool};

/// Independently identified device attributes used when matching a binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceContext {
    /// Supported module model established from the selected device.
    pub module: ServiceModule,
    /// Serial reported by the selected device, if available.
    pub serial_number: Option<String>,
}

impl DeviceContext {
    /// Builds a context from a reported serial, treating an empty or blank serial as absent.
    ///
    /// Live inventories and recorded captures can report `""` for devices
    /// without a serial. Normalizing here keeps both paths identical; any other
    /// serial must satisfy the same rules as a profile selector.
    pub fn new(module: ServiceModule, serial_number: Option<String>) -> Result<Self, String> {
        let serial_number = serial_number.filter(|serial| !serial.trim().is_empty());
        crate::profile::validate_serial_number(serial_number.as_deref())?;
        Ok(Self {
            module,
            serial_number,
        })
    }

    /// Applies [`DeviceContext::new`] normalization to a possibly struct-literal context.
    pub(crate) fn normalized(&self) -> Result<Self, String> {
        Self::new(self.module, self.serial_number.clone())
    }
}

/// A validated profile with deterministic serial-specific binding precedence.
#[derive(Clone, Debug)]
pub struct BindingEngine {
    profile: Profile,
}

impl BindingEngine {
    /// Validates a profile before it can select any host actions.
    pub fn new(profile: Profile) -> Result<Self, String> {
        profile.validate()?;
        if profile.keypad.is_some() {
            return Err("keypad page profiles require a stateful ProfileSession".into());
        }
        if profile.dial_functions.is_some() {
            return Err("dial function profiles require a stateful ProfileSession".into());
        }
        Ok(Self { profile })
    }

    pub(crate) fn from_validated(profile: Profile) -> Self {
        Self { profile }
    }

    /// Looks up one binding, preferring an exact serial match over a wildcard.
    ///
    /// A missing device serial never matches a serial-specific binding. The
    /// timestamp is not a selector: gesture timing belongs to the input session.
    pub fn binding_for(&self, device: &DeviceContext, event: &InputEvent) -> Option<&Binding> {
        let matches = |binding: &&Binding| {
            binding.module == device.module
                && binding.control == event.control
                && binding.gesture == event.gesture
        };
        let mut wildcard = None;
        for binding in self.profile.bindings.iter().filter(matches) {
            match binding.serial_number.as_deref() {
                Some(serial) if device.serial_number.as_deref() == Some(serial) => {
                    return Some(binding);
                }
                None => wildcard = Some(binding),
                Some(_) => {}
            }
        }
        wildcard
    }

    /// Selects a binding and resolves its input-dependent action to a host operation.
    ///
    /// Fader positions map to the nearest whole output percentage. Missing,
    /// unsupported, or out-of-range values fail before dispatch; an unmatched
    /// valid event returns `None`.
    pub fn action_for(
        &self,
        device: &DeviceContext,
        event: &InputEvent,
    ) -> Result<Option<Action>, String> {
        event.validate_for(device.module)?;
        let device = &device.normalized()?;
        if device.module == ServiceModule::M4315 && self.profile.keypad.is_some() {
            return Err("keypad pages require stateful ProfileSession routing".into());
        }
        if device.module == ServiceModule::M131E && self.profile.dial_functions.is_some() {
            return Err("dial functions require stateful ProfileSession routing".into());
        }
        let Some(binding) = self.binding_for(device, event) else {
            return Ok(None);
        };
        let action = match &binding.action {
            template @ (Action::VolumeFromPosition
            | Action::MicInputVolumeFromPosition
            | Action::ObsInputVolumeFromPosition { .. }) => {
                let Some(InputValue::Position { raw, maximum }) = event.value else {
                    return Err(format!(
                        "{} requires an absolute fader value",
                        template.type_name()
                    ));
                };
                // validate_for has rejected zero scale or out-of-range input.
                let percent =
                    ((u32::from(raw) * 100 + u32::from(maximum) / 2) / u32::from(maximum)) as u8;
                match template {
                    Action::ObsInputVolumeFromPosition { input } => Action::ObsSetInputVolume {
                        input: input.clone(),
                        percent,
                    },
                    Action::MicInputVolumeFromPosition => Action::SetMicInputVolume { percent },
                    _ => Action::SetVolume { percent },
                }
            }
            other => other.clone(),
        };
        action.validate_executable()?;
        Ok(Some(action))
    }

    /// Returns the validated profile for status and preview output.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}

/// The host boundary for explicitly dispatching a matched action.
pub trait ActionDispatcher {
    /// Executes one action or reports failure without an implicit retry.
    fn dispatch(&mut self, action: &Action) -> Result<(), String>;

    /// Why this dispatcher can never run `action` in this run (for example an
    /// `obs_scene` action without an OBS endpoint), or `None`. Callers check it
    /// before dispatching and log an unavailable action instead of a failure;
    /// a `multi_action` with an unavailable step is unavailable as a whole, so
    /// none of its steps run.
    fn unavailable(&self, _action: &Action) -> Option<&'static str> {
        None
    }

    /// Shows the end of an expired `timer` countdown (notification and sound).
    ///
    /// `Ok(Some(warning))` reports an alert whose tools exited successfully
    /// but printed a diagnostic, such as a notification macOS refused to show.
    /// The countdown itself is owned by the service worker, never by a
    /// dispatcher. The default refuses, so a dispatcher that does not opt in
    /// never silently drops an alert.
    fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        alert.validate()?;
        Err("timer alerts are not supported by this dispatcher; nothing was shown or played".into())
    }

    /// A separate alerter that a service may move to its own thread, so slow
    /// alerts (a sound plays for up to about 3 s) never delay later actions.
    ///
    /// The alerter must stop starting processes, and kill a running one, once
    /// `cancel` is set; the caller sets it on a stop and at the end of the
    /// session. `None`, the default, means alerts run inline through
    /// [`ActionDispatcher::timer_alert`].
    fn timer_alerter(&mut self, cancel: Arc<AtomicBool>) -> Option<Box<dyn TimerAlerter>> {
        let _ = cancel;
        None
    }
}

/// Runs timer alerts away from the action dispatcher; see
/// [`ActionDispatcher::timer_alerter`].
pub trait TimerAlerter: Send {
    /// Same contract as [`ActionDispatcher::timer_alert`].
    fn alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String>;
}

/// A dispatcher that validates and records actions without operating-system I/O.
#[derive(Clone, Debug, Default)]
pub struct DryRunDispatcher {
    /// Validated actions in the order received.
    pub actions: Vec<Action>,
    /// Validated timer alerts in the order received; nothing is shown or played.
    pub timer_alerts: Vec<TimerAlert>,
}

impl ActionDispatcher for DryRunDispatcher {
    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        action.validate_executable()?;
        self.actions.push(action.clone());
        Ok(())
    }

    fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        alert.validate()?;
        self.timer_alerts.push(alert.clone());
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Gesture;

    fn binding(serial: Option<&str>, delta: i16) -> Binding {
        Binding {
            module: ServiceModule::M003K,
            control: 1,
            gesture: Gesture::Clockwise,
            serial_number: serial.map(str::to_owned),
            action: Action::Volume { delta },
        }
    }

    fn engine(bindings: Vec<Binding>) -> BindingEngine {
        BindingEngine::new(Profile {
            schema_version: 1,
            name: "Desk".into(),
            bindings,
            keypad: None,
            dial_functions: None,
            dial_display: None,
        })
        .unwrap()
    }

    fn event() -> InputEvent {
        InputEvent {
            timestamp_micros: 0,
            control: 1,
            gesture: Gesture::Clockwise,
            value: None,
        }
    }

    #[test]
    fn exact_serial_wins_independent_of_profile_order() {
        for bindings in [
            vec![binding(None, 5), binding(Some("unit-1"), 10)],
            vec![binding(Some("unit-1"), 10), binding(None, 5)],
        ] {
            let engine = engine(bindings);
            let device = DeviceContext {
                module: ServiceModule::M003K,
                serial_number: Some("unit-1".into()),
            };
            assert_eq!(
                engine.binding_for(&device, &event()).unwrap().action,
                Action::Volume { delta: 10 }
            );
        }
    }

    #[test]
    fn unrelated_or_missing_serial_falls_back_to_wildcard() {
        let engine = engine(vec![binding(Some("unit-1"), 10), binding(None, 5)]);
        for serial_number in [None, Some("unit-2".into())] {
            let device = DeviceContext {
                module: ServiceModule::M003K,
                serial_number,
            };
            assert_eq!(
                engine.binding_for(&device, &event()).unwrap().action,
                Action::Volume { delta: 5 }
            );
        }
    }

    #[test]
    fn every_selector_dimension_must_match() {
        let engine = engine(vec![binding(Some("unit-1"), 10)]);
        let mut device = DeviceContext {
            module: ServiceModule::M003K,
            serial_number: None,
        };
        assert!(engine.binding_for(&device, &event()).is_none());
        device.serial_number = Some("unit-1".into());
        let mut input = event();
        input.control = 0;
        assert!(engine.binding_for(&device, &input).is_none());
        input = event();
        input.gesture = Gesture::CounterClockwise;
        assert!(engine.binding_for(&device, &input).is_none());
        device.module = ServiceModule::M131E;
        assert!(engine.binding_for(&device, &event()).is_none());
    }

    #[test]
    fn blank_device_serials_are_absent_and_invalid_serials_fail_early() {
        for serial in ["", " ", "\t"] {
            let device = DeviceContext::new(ServiceModule::M003K, Some(serial.into())).unwrap();
            assert_eq!(device.serial_number, None);
        }
        let device = DeviceContext::new(ServiceModule::M003K, Some("unit-1".into())).unwrap();
        assert_eq!(device.serial_number.as_deref(), Some("unit-1"));
        assert!(DeviceContext::new(ServiceModule::M003K, Some("a\u{1}b".into())).is_err());
    }

    #[test]
    fn action_for_normalizes_struct_literal_blank_serials() {
        let engine = engine(vec![binding(None, 5)]);
        let blank = DeviceContext {
            module: ServiceModule::M003K,
            serial_number: Some(String::new()),
        };
        assert_eq!(
            engine.action_for(&blank, &event()).unwrap(),
            Some(Action::Volume { delta: 5 })
        );
        let invalid = DeviceContext {
            module: ServiceModule::M003K,
            serial_number: Some("a\u{1}b".into()),
        };
        assert!(engine.action_for(&invalid, &event()).is_err());
    }

    #[test]
    fn constructed_invalid_profiles_cannot_reach_binding_lookup() {
        let invalid = Profile {
            schema_version: 1,
            name: "Desk".into(),
            bindings: vec![binding(None, 0)],
            keypad: None,
            dial_functions: None,
            dial_display: None,
        };
        assert!(BindingEngine::new(invalid).is_err());
    }

    #[test]
    fn dry_dispatch_records_order_and_rejects_invalid_without_recording() {
        let mut dispatcher = DryRunDispatcher::default();
        dispatcher.dispatch(&Action::Volume { delta: 5 }).unwrap();
        assert!(dispatcher.dispatch(&Action::Volume { delta: 0 }).is_err());
        dispatcher.dispatch(&Action::ToggleMute).unwrap();
        assert_eq!(
            dispatcher.actions,
            [Action::Volume { delta: 5 }, Action::ToggleMute]
        );
    }

    #[test]
    fn fader_binding_resolves_actual_absolute_position_instead_of_fixed_delta() {
        let engine = engine(vec![Binding {
            module: ServiceModule::M005S,
            control: 2,
            gesture: Gesture::PositionChanged,
            serial_number: None,
            action: Action::VolumeFromPosition,
        }]);
        let device = DeviceContext {
            module: ServiceModule::M005S,
            serial_number: None,
        };
        for raw in [0, 1, 25, 50, 99, 100] {
            let input = InputEvent {
                timestamp_micros: 0,
                control: 2,
                gesture: Gesture::PositionChanged,
                value: Some(InputValue::Position { raw, maximum: 100 }),
            };
            assert_eq!(
                engine.action_for(&device, &input).unwrap(),
                Some(Action::SetVolume { percent: raw as u8 })
            );
            assert_eq!(
                engine.binding_for(&device, &input).unwrap().action,
                Action::VolumeFromPosition
            );
        }
    }

    #[test]
    fn microphone_fader_binding_resolves_to_an_absolute_input_volume() {
        let engine = engine(vec![Binding {
            module: ServiceModule::M005S,
            control: 1,
            gesture: Gesture::PositionChanged,
            serial_number: None,
            action: Action::MicInputVolumeFromPosition,
        }]);
        let device = DeviceContext {
            module: ServiceModule::M005S,
            serial_number: None,
        };
        for raw in [0, 33, 100] {
            let input = InputEvent {
                timestamp_micros: 0,
                control: 1,
                gesture: Gesture::PositionChanged,
                value: Some(InputValue::Position { raw, maximum: 100 }),
            };
            assert_eq!(
                engine.action_for(&device, &input).unwrap(),
                Some(Action::SetMicInputVolume { percent: raw as u8 })
            );
        }
    }

    #[test]
    fn obs_fader_binding_resolves_to_an_absolute_input_volume() {
        let engine = engine(vec![Binding {
            module: ServiceModule::M005S,
            control: 0,
            gesture: Gesture::PositionChanged,
            serial_number: None,
            action: Action::ObsInputVolumeFromPosition {
                input: "Mic/Aux".into(),
            },
        }]);
        let device = DeviceContext {
            module: ServiceModule::M005S,
            serial_number: None,
        };
        for raw in [0, 1, 50, 100] {
            let input = InputEvent {
                timestamp_micros: 0,
                control: 0,
                gesture: Gesture::PositionChanged,
                value: Some(InputValue::Position { raw, maximum: 100 }),
            };
            assert_eq!(
                engine.action_for(&device, &input).unwrap(),
                Some(Action::ObsSetInputVolume {
                    input: "Mic/Aux".into(),
                    percent: raw as u8
                })
            );
        }
        let mut dispatcher = DryRunDispatcher::default();
        assert!(
            dispatcher
                .dispatch(&Action::ObsInputVolumeFromPosition {
                    input: "Mic/Aux".into()
                })
                .is_err()
        );
        assert!(dispatcher.actions.is_empty());
    }

    #[test]
    fn resolution_validates_values_even_for_unmatched_events() {
        let engine = engine(vec![]);
        let device = DeviceContext {
            module: ServiceModule::M005S,
            serial_number: None,
        };
        let mut input = InputEvent {
            timestamp_micros: 0,
            control: 0,
            gesture: Gesture::PositionChanged,
            value: None,
        };
        assert!(engine.action_for(&device, &input).is_err());
        input.value = Some(InputValue::Position {
            raw: 50,
            maximum: 100,
        });
        assert_eq!(engine.action_for(&device, &input).unwrap(), None);
        input.value = Some(InputValue::Position {
            raw: 50,
            maximum: 255,
        });
        assert!(engine.action_for(&device, &input).is_err());
    }

    #[test]
    fn dry_dispatch_rejects_unresolved_marker_without_recording() {
        let mut dispatcher = DryRunDispatcher::default();
        assert!(dispatcher.dispatch(&Action::VolumeFromPosition).is_err());
        assert!(dispatcher.actions.is_empty());
        dispatcher
            .dispatch(&Action::SetVolume { percent: 50 })
            .unwrap();
        assert_eq!(dispatcher.actions, [Action::SetVolume { percent: 50 }]);
    }
}
