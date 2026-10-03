use crate::{Action, Binding, InputEvent, Profile, ServiceModule};

/// Independently identified device attributes used when matching a binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceContext {
    /// Supported module model established from the selected device.
    pub module: ServiceModule,
    /// Serial reported by the selected device, if available.
    pub serial_number: Option<String>,
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
        Ok(Self { profile })
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

    /// Returns the validated profile for status and preview output.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}

/// The host boundary for explicitly dispatching a matched action.
pub trait ActionDispatcher {
    /// Executes one action or reports failure without an implicit retry.
    fn dispatch(&mut self, action: &Action) -> Result<(), String>;
}

/// A dispatcher that validates and records actions without operating-system I/O.
#[derive(Clone, Debug, Default)]
pub struct DryRunDispatcher {
    /// Validated actions in the order received.
    pub actions: Vec<Action>,
}

impl ActionDispatcher for DryRunDispatcher {
    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        action.validate()?;
        self.actions.push(action.clone());
        Ok(())
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
        })
        .unwrap()
    }

    fn event() -> InputEvent {
        InputEvent {
            timestamp_micros: 0,
            control: 1,
            gesture: Gesture::Clockwise,
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
    fn constructed_invalid_profiles_cannot_reach_binding_lookup() {
        let invalid = Profile {
            schema_version: 1,
            name: "Desk".into(),
            bindings: vec![binding(None, 0)],
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
}
