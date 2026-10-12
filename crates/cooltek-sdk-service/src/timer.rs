//! Countdown timers started by `timer` actions, independent of any clock or host.
//!
//! [`TimerTable`] holds at most [`TimerTable::MAX_RUNNING`] countdowns, one per
//! binding ([`TimerKey`]). Every method takes the current time explicitly, in
//! microseconds on a caller-chosen monotonic timeline, so tests drive it with a
//! fake clock. The table lives only in memory: it is dropped with the process
//! and never persisted.

use crate::{Action, Gesture, TimerSound};
use serde::Serialize;

/// The binding that owns a timer. A service process reads one interface, so
/// control, gesture, keypad page and dial function identify one binding.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct TimerKey {
    /// Wire control index.
    pub control: u8,
    /// Gesture of the binding.
    pub gesture: Gesture,
    /// Keypad page of a schema-2 key tile, if any.
    pub page_id: Option<String>,
    /// Dial function of an M131E binding routed through `dial_functions`, if any.
    pub dial_function: Option<String>,
}

/// What a dispatcher does when a timer ends: a notification and a sound.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimerAlert {
    /// The countdown length, for a notification without a label.
    pub seconds: u32,
    /// Notification text, already validated as a timer label.
    pub label: Option<String>,
    /// Sound to play; [`TimerSound::None`] plays nothing.
    pub sound: TimerSound,
}

impl TimerAlert {
    /// The alert of a `timer` action, or `None` for any other action.
    pub fn from_action(action: &Action) -> Option<Self> {
        match action {
            Action::Timer {
                seconds,
                label,
                sound,
            } => Some(Self {
                seconds: *seconds,
                label: label.clone(),
                sound: sound.unwrap_or(TimerSound::Glass),
            }),
            _ => None,
        }
    }

    /// Checks the same bounds as the `timer` action before any host operation.
    pub fn validate(&self) -> Result<(), String> {
        Action::Timer {
            seconds: self.seconds,
            label: self.label.clone(),
            sound: Some(self.sound),
        }
        .validate()
    }

    /// The notification body: the label, or the countdown length.
    pub fn message(&self) -> String {
        match &self.label {
            Some(label) => label.clone(),
            None => format!("{} countdown ended", format_duration(self.seconds)),
        }
    }
}

/// `H:MM:SS` for an hour or more, `M:SS` otherwise.
pub fn format_duration(seconds: u32) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// A running countdown.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunningTimer {
    /// The binding that started it.
    pub key: TimerKey,
    /// The validated `timer` action that started it.
    pub action: Action,
    /// Expiry time on the table's timeline, in microseconds.
    pub deadline_micros: u64,
}

/// The outcome of pressing a `timer` binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TimerToggle {
    /// A new countdown started.
    Started {
        /// Expiry time in microseconds.
        deadline_micros: u64,
        /// Running timers including this one.
        running: usize,
    },
    /// The binding's running countdown was cancelled.
    Cancelled(RunningTimer),
    /// The table is full; nothing started.
    Full,
}

/// Running countdowns, at most one per binding.
#[derive(Clone, Debug, Default)]
pub struct TimerTable {
    running: Vec<RunningTimer>,
}

impl TimerTable {
    /// Most countdowns that can run at once in one service process.
    pub const MAX_RUNNING: usize = 8;

    /// Starts a countdown for `key`, or cancels the one it already runs.
    ///
    /// Rejects anything but a valid `timer` action before changing state.
    pub fn toggle(
        &mut self,
        key: TimerKey,
        action: &Action,
        now_micros: u64,
    ) -> Result<TimerToggle, String> {
        action.validate_executable()?;
        let Action::Timer { seconds, .. } = action else {
            return Err(format!("{} is not a timer action", action.type_name()));
        };
        if let Some(index) = self.running.iter().position(|timer| timer.key == key) {
            return Ok(TimerToggle::Cancelled(self.running.remove(index)));
        }
        if self.running.len() >= Self::MAX_RUNNING {
            return Ok(TimerToggle::Full);
        }
        let deadline_micros = now_micros.saturating_add(u64::from(*seconds) * 1_000_000);
        self.running.push(RunningTimer {
            key,
            action: action.clone(),
            deadline_micros,
        });
        Ok(TimerToggle::Started {
            deadline_micros,
            running: self.running.len(),
        })
    }

    /// Removes and returns every countdown due at `now_micros`, earliest first.
    pub fn take_due(&mut self, now_micros: u64) -> Vec<RunningTimer> {
        let (mut due, running): (Vec<_>, Vec<_>) = std::mem::take(&mut self.running)
            .into_iter()
            .partition(|timer| timer.deadline_micros <= now_micros);
        self.running = running;
        due.sort_by_key(|timer| timer.deadline_micros);
        due
    }

    /// Removes and returns the earliest countdown due at `now_micros`, if any.
    ///
    /// Taking one at a time lets a caller that fails part-way leave the rest
    /// in the table.
    pub fn pop_due(&mut self, now_micros: u64) -> Option<RunningTimer> {
        let index = self
            .running
            .iter()
            .enumerate()
            .filter(|(_, timer)| timer.deadline_micros <= now_micros)
            .min_by_key(|(_, timer)| timer.deadline_micros)
            .map(|(index, _)| index)?;
        Some(self.running.remove(index))
    }

    /// Puts back a countdown taken by [`TimerTable::take_due`] that was not
    /// handled, so a later [`TimerTable::cancel_all`] still reports it.
    pub fn restore(&mut self, timer: RunningTimer) {
        self.running.push(timer);
    }

    /// Removes and returns every running countdown, earliest first.
    pub fn cancel_all(&mut self) -> Vec<RunningTimer> {
        self.take_due(u64::MAX)
    }

    /// The earliest deadline, if any countdown runs.
    pub fn next_deadline(&self) -> Option<u64> {
        self.running.iter().map(|timer| timer.deadline_micros).min()
    }

    /// The running countdowns, in no particular order.
    pub fn running(&self) -> &[RunningTimer] {
        &self.running
    }

    /// Number of running countdowns.
    pub fn len(&self) -> usize {
        self.running.len()
    }

    /// Whether no countdown runs.
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(control: u8) -> TimerKey {
        TimerKey {
            control,
            gesture: Gesture::Pressed,
            page_id: Some("root".into()),
            dial_function: None,
        }
    }

    fn timer(seconds: u32) -> Action {
        Action::Timer {
            seconds,
            label: None,
            sound: None,
        }
    }

    #[test]
    fn a_fake_clock_drives_start_cancel_and_expiry() {
        let mut table = TimerTable::default();
        assert_eq!(
            table.toggle(key(0), &timer(5), 1_000).unwrap(),
            TimerToggle::Started {
                deadline_micros: 5_001_000,
                running: 1
            }
        );
        assert_eq!(table.next_deadline(), Some(5_001_000));
        assert!(table.take_due(5_000_999).is_empty());
        // Pressing the same binding again cancels it; nothing expires later.
        let TimerToggle::Cancelled(cancelled) = table.toggle(key(0), &timer(5), 2_000).unwrap()
        else {
            panic!("expected a cancellation");
        };
        assert_eq!(cancelled.key, key(0));
        assert!(table.is_empty());
        assert!(table.take_due(u64::MAX - 1).is_empty());
        // A third press starts a fresh countdown from that press.
        table.toggle(key(0), &timer(5), 3_000).unwrap();
        table.toggle(key(1), &timer(1), 3_000).unwrap();
        let due = table.take_due(10_000_000);
        assert_eq!(
            due.iter()
                .map(|timer| timer.key.control)
                .collect::<Vec<_>>(),
            [1, 0],
            "earliest deadline first"
        );
        assert!(table.is_empty() && table.next_deadline().is_none());
    }

    #[test]
    fn the_same_control_on_another_page_or_function_is_another_binding() {
        let mut table = TimerTable::default();
        let mut other_page = key(0);
        other_page.page_id = Some("folder".into());
        let mut released = key(0);
        released.gesture = Gesture::Released;
        let mut dial = key(0);
        dial.page_id = None;
        dial.dial_function = Some("Volume".into());
        for key in [key(0), other_page, released, dial] {
            assert!(matches!(
                table.toggle(key, &timer(60), 0).unwrap(),
                TimerToggle::Started { .. }
            ));
        }
        assert_eq!(table.len(), 4);
    }

    #[test]
    fn at_most_eight_run_and_a_full_table_still_cancels() {
        let mut table = TimerTable::default();
        for control in 0..8 {
            assert!(matches!(
                table.toggle(key(control), &timer(60), 0).unwrap(),
                TimerToggle::Started { running, .. } if running == usize::from(control) + 1
            ));
        }
        assert_eq!(
            table.toggle(key(8), &timer(60), 0).unwrap(),
            TimerToggle::Full
        );
        assert_eq!(table.len(), TimerTable::MAX_RUNNING);
        assert!(matches!(
            table.toggle(key(3), &timer(60), 0).unwrap(),
            TimerToggle::Cancelled(_)
        ));
        assert!(matches!(
            table.toggle(key(8), &timer(60), 0).unwrap(),
            TimerToggle::Started { running: 8, .. }
        ));
        assert_eq!(table.cancel_all().len(), 8);
        assert!(table.is_empty());
    }

    #[test]
    fn invalid_or_other_actions_never_change_the_table() {
        let mut table = TimerTable::default();
        for action in [timer(0), timer(86_401), Action::ToggleMute] {
            assert!(table.toggle(key(0), &action, 0).is_err());
        }
        assert!(table.is_empty());
        // The largest countdown cannot overflow the timeline.
        table.toggle(key(0), &timer(86_400), u64::MAX - 1).unwrap();
        assert_eq!(table.next_deadline(), Some(u64::MAX));
    }

    #[test]
    fn alerts_default_to_glass_and_describe_unlabelled_timers() {
        let alert = TimerAlert::from_action(&timer(90)).unwrap();
        assert_eq!(alert.sound, TimerSound::Glass);
        assert_eq!(alert.message(), "1:30 countdown ended");
        assert_eq!(format_duration(86_400), "24:00:00");
        assert_eq!(format_duration(3_661), "1:01:01");
        assert_eq!(format_duration(1), "0:01");
        let labelled = TimerAlert::from_action(&Action::Timer {
            seconds: 5,
            label: Some("Tea".into()),
            sound: Some(TimerSound::None),
        })
        .unwrap();
        assert_eq!(labelled.message(), "Tea");
        assert_eq!(labelled.sound, TimerSound::None);
        assert!(TimerAlert::from_action(&Action::ToggleMute).is_none());
        let mut invalid = labelled;
        invalid.label = Some("a\nb".into());
        assert!(invalid.validate().is_err());
    }
}
