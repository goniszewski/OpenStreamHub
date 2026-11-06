//! The current values of a run's variables, shared by the dispatcher and the control channel.

use crate::profile::{Variable, VariableValue};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// Current values of the variables the running profile declares, shared by the
/// dispatcher (key presses) and the control channel (processes outside the
/// run). Clones share one store. Every change bumps [`SharedVariables::version`].
#[derive(Clone, Debug, Default)]
pub struct SharedVariables(Arc<Mutex<State>>);

#[derive(Debug, Default)]
struct State {
    /// In declaration order.
    values: Vec<(String, VariableValue)>,
    version: u64,
    /// The [`crate::ProfileSession::id`] of the profile last handed over by
    /// [`SharedVariables::swap`], when a swap has happened.
    profile: Option<u64>,
}

impl SharedVariables {
    /// An empty store: nothing is declared until [`SharedVariables::declare`].
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Follows a profile's declarations: a variable keeps its value while its
    /// name and kind stay, a new one starts at its declared value, and one no
    /// longer declared is dropped.
    pub fn declare(&self, declared: &[Variable]) {
        let mut state = self.lock();
        let kept: Vec<(String, VariableValue)> = declared
            .iter()
            .map(|variable| {
                let value = match state.values.iter().find(|(name, _)| *name == variable.name) {
                    Some((_, current)) if current.kind() == variable.value.kind() => {
                        current.clone()
                    }
                    _ => variable.value.clone(),
                };
                (variable.name.clone(), value)
            })
            .collect();
        if kept != state.values {
            state.values = kept;
            state.version += 1;
        }
    }

    /// Declares the variables of a profile swapped in while the run goes on (see
    /// [`SharedVariables::declare`]) and records its session id, so a press
    /// planned by the profile before can be told apart
    /// ([`SharedVariables::swapped_away_from`]).
    pub fn swap(&self, declared: &[Variable], profile: u64) {
        self.declare(declared);
        self.lock().profile = Some(profile);
    }

    /// Whether another profile than `profile` (a [`crate::ProfileSession::id`])
    /// was swapped in after it, so work it planned is from a profile no
    /// longer running. False until the first swap.
    pub fn swapped_away_from(&self, profile: u64) -> bool {
        self.lock()
            .profile
            .is_some_and(|current| current != profile)
    }

    /// Runs `read` with a lookup of the current values under one lock, so
    /// every value it reads is from the same moment and only those it asks
    /// for are copied.
    pub fn read<R>(&self, read: impl FnOnce(&dyn Fn(&str) -> Option<VariableValue>) -> R) -> R {
        let state = self.lock();
        read(&|name: &str| {
            state
                .values
                .iter()
                .find(|(declared, _)| declared == name)
                .map(|(_, value)| value.clone())
        })
    }

    /// Applies `change` to the value of `name`; the version moves only when the value does.
    fn change(
        &self,
        name: &str,
        change: impl FnOnce(&mut VariableValue) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut state = self.lock();
        let slot = state
            .values
            .iter_mut()
            .find(|(declared, _)| declared == name)
            .map(|(_, value)| value)
            .ok_or_else(|| format!("variable {name} is not declared; no action was taken"))?;
        let before = slot.clone();
        change(slot)?;
        let changed = *slot != before;
        if changed {
            state.version += 1;
        }
        Ok(())
    }

    /// Sets a declared variable to a value of its kind. Numbers are clamped to
    /// the variable range; text is checked as a profile's text is.
    pub fn set(&self, name: &str, value: VariableValue) -> Result<(), String> {
        let value = match value {
            VariableValue::Number(number) => VariableValue::Number(
                number.clamp(VariableValue::MIN_NUMBER, VariableValue::MAX_NUMBER),
            ),
            other => other,
        };
        value.validate()?;
        self.change(name, |slot| {
            if slot.kind() != value.kind() {
                return Err(format!(
                    "variable {name} is {}, not {}; no action was taken",
                    slot.kind().name(),
                    value.kind().name()
                ));
            }
            *slot = value;
            Ok(())
        })
    }

    /// Flips a declared bool variable.
    pub fn toggle(&self, name: &str) -> Result<(), String> {
        self.change(name, |slot| match slot {
            VariableValue::Bool(flag) => {
                *flag = !*flag;
                Ok(())
            }
            other => Err(format!(
                "variable {name} is {}, not bool; no action was taken",
                other.kind().name()
            )),
        })
    }

    /// Adds `amount` to a declared number variable, stopping at the limits.
    pub fn add(&self, name: &str, amount: i32) -> Result<(), String> {
        self.change(name, |slot| match slot {
            VariableValue::Number(number) => {
                *number = number
                    .saturating_add(amount)
                    .clamp(VariableValue::MIN_NUMBER, VariableValue::MAX_NUMBER);
                Ok(())
            }
            other => Err(format!(
                "variable {name} is {}, not number; no action was taken",
                other.kind().name()
            )),
        })
    }

    /// The current value of `name`, when it is declared.
    pub fn get(&self, name: &str) -> Option<VariableValue> {
        self.lock()
            .values
            .iter()
            .find(|(declared, _)| declared == name)
            .map(|(_, value)| value.clone())
    }

    /// Every declared variable with its current value, in declaration order.
    pub fn values(&self) -> Vec<(String, VariableValue)> {
        self.lock().values.clone()
    }

    /// A counter that grows with every change of any value or declaration.
    pub fn version(&self) -> u64 {
        self.lock().version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared() -> Vec<Variable> {
        vec![
            Variable {
                name: "agent_state".into(),
                value: VariableValue::Text("idle".into()),
            },
            Variable {
                name: "count".into(),
                value: VariableValue::Number(0),
            },
        ]
    }

    #[test]
    fn a_set_checks_the_declaration_and_kind_and_moves_the_version() {
        let store = SharedVariables::new();
        store.declare(&declared());
        let start = store.version();
        store
            .set("agent_state", VariableValue::Text("waiting".into()))
            .unwrap();
        assert_eq!(
            store.get("agent_state"),
            Some(VariableValue::Text("waiting".into()))
        );
        assert_eq!(store.version(), start + 1);
        // The same value again is no change.
        store
            .set("agent_state", VariableValue::Text("waiting".into()))
            .unwrap();
        assert_eq!(store.version(), start + 1);
        let error = store
            .set("agent_state", VariableValue::Number(1))
            .unwrap_err();
        assert!(error.contains("is text, not number"), "{error}");
        let error = store.set("nope", VariableValue::Bool(true)).unwrap_err();
        assert!(error.contains("not declared"), "{error}");
        let error = store
            .set("agent_state", VariableValue::Text("a".repeat(65)))
            .unwrap_err();
        assert!(error.contains("64 bytes"), "{error}");
        assert_eq!(store.version(), start + 1);
    }

    #[test]
    fn numbers_are_clamped_and_declarations_keep_matching_values() {
        let store = SharedVariables::new();
        store.declare(&declared());
        store.set("count", VariableValue::Number(i32::MAX)).unwrap();
        assert_eq!(
            store.get("count"),
            Some(VariableValue::Number(VariableValue::MAX_NUMBER))
        );
        let version = store.version();
        store.declare(&declared());
        assert_eq!(store.version(), version);
        assert_eq!(
            store.get("count"),
            Some(VariableValue::Number(VariableValue::MAX_NUMBER))
        );
        store.declare(&declared()[..1]);
        assert_eq!(store.get("count"), None);
        assert_eq!(store.values().len(), 1);
        assert!(store.version() > version);
    }
}
