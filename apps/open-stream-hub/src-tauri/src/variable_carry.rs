//! Keeps a profile's variable values across the restart a save or a profile switch causes.
//!
//! A run that draws on its module takes a new profile by restarting, which would reset every variable (an agent's
//! `agent_state` among them). Just before a service is stopped its values are read and kept for a short while; the
//! next run of the same module gets them back once it answers. The service itself decides what applies: a
//! variable the new profile no longer declares, or declares as another kind, refuses the value and is skipped.
//! `agent_decision` is never carried, so a stale Accept can not answer a prompt in the next run, and neither is a
//! waiting prompt (see `carried`).

use crate::service::{ControlPipe, Services};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

/// How long a kept snapshot stays usable: a restart takes seconds; a start much later is a fresh run.
const KEEP: Duration = Duration::from_secs(20);
/// How long the next run gets to answer before the values are dropped.
const WAIT_READY: Duration = Duration::from_secs(10);
const ASK: Duration = Duration::from_millis(500);

type Values = Vec<(String, Value)>;

#[derive(Default)]
pub struct VariableCarry {
    kept: Mutex<HashMap<String, (Instant, Values)>>,
}

/// Whether a variable's value may be carried into the next run. Not the decision (an old Accept must not answer a
/// prompt in the next run), and not a waiting prompt: the ask that showed it cannot survive the restart, so a
/// carried `waiting` or count would stay on the deck with nothing behind it.
fn carried(name: &str, value: &Value) -> bool {
    name != osh::AGENT_DECISION
        && name != osh::AGENT_WAITING
        && !(name == osh::AGENT_STATE && value == "waiting")
}

impl VariableCarry {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (Instant, Values)>> {
        self.kept
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Keeps `values` (minus the ones that never carry) for `service_id`; nothing to carry forgets the old snapshot.
    pub fn keep(&self, service_id: &str, values: Values, now: Instant) {
        let values: Values = values
            .into_iter()
            .filter(|(name, value)| carried(name, value))
            .collect();
        let mut kept = self.lock();
        if values.is_empty() {
            kept.remove(service_id);
        } else {
            kept.insert(service_id.to_string(), (now, values));
        }
    }

    /// The snapshot of `service_id` if it is recent enough; it is handed out once.
    pub fn take(&self, service_id: &str, now: Instant) -> Option<Values> {
        let (at, values) = self.lock().remove(service_id)?;
        (now.saturating_duration_since(at) <= KEEP).then_some(values)
    }

    /// Reads the values of a running service and keeps them. Any failure keeps nothing.
    pub fn snapshot(&self, services: &Services, service_id: &str) {
        let Some(pipe) = services.control(service_id) else {
            return;
        };
        match read_values(&pipe) {
            Some(values) => self.keep(service_id, values, Instant::now()),
            None => {
                self.lock().remove(service_id);
            }
        }
    }

    /// Gives a restarted service its values back, on a short-lived thread that waits for the control channel.
    pub fn restore(&self, app: tauri::AppHandle, service_id: &str) {
        let Some(values) = self.take(service_id, Instant::now()) else {
            return;
        };
        let id = service_id.to_string();
        thread::spawn(move || {
            let deadline = Instant::now() + WAIT_READY;
            while Instant::now() < deadline {
                let pipe = tauri::Manager::state::<crate::AppState>(&app)
                    .services
                    .control(&id);
                if let Some(pipe) = pipe {
                    for (name, value) in &values {
                        // A refusal means the new profile does not declare it (as that kind): skipped.
                        let _ = pipe.request(
                            json!({"type":"set_variable","name":name,"value":value}),
                            ASK,
                        );
                    }
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
    }
}

fn read_values(pipe: &ControlPipe) -> Option<Values> {
    let answer = pipe.request(json!({"type":"get_variables"}), ASK).ok()?;
    let list = answer["variables"].as_array()?;
    Some(
        list.iter()
            .filter_map(|variable| {
                Some((
                    variable["name"].as_str()?.to_string(),
                    variable["value"].clone(),
                ))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        vec![
            ("agent_state".into(), json!("waiting")),
            ("agent_decision".into(), json!("allow")),
            ("agent_waiting".into(), json!(1)),
            ("count".into(), json!(3)),
        ]
    }

    #[test]
    fn a_decision_is_never_carried() {
        let carry = VariableCarry::default();
        let now = Instant::now();
        carry.keep("m", values(), now);
        let names: Vec<String> = carry
            .take("m", now)
            .unwrap()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        // `agent_state` is `waiting` here, which does not carry either.
        assert_eq!(names, ["count"]);
        carry.keep("m", vec![("agent_state".into(), json!("working"))], now);
        assert_eq!(carry.take("m", now).unwrap().len(), 1);
    }

    #[test]
    fn a_snapshot_is_handed_out_once() {
        let carry = VariableCarry::default();
        let now = Instant::now();
        carry.keep("m", values(), now);
        assert!(carry.take("m", now).is_some());
        assert!(carry.take("m", now).is_none());
    }

    #[test]
    fn an_old_snapshot_is_dropped() {
        let carry = VariableCarry::default();
        let now = Instant::now();
        carry.keep("m", values(), now);
        assert!(
            carry
                .take("m", now + KEEP + Duration::from_secs(1))
                .is_none()
        );
    }

    #[test]
    fn nothing_to_carry_forgets_the_older_snapshot() {
        let carry = VariableCarry::default();
        let now = Instant::now();
        carry.keep("m", values(), now);
        carry.keep("m", vec![("agent_decision".into(), json!("deny"))], now);
        assert!(carry.take("m", now).is_none());
    }
}
