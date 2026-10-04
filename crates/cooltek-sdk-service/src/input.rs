use crate::{Gesture, ServiceModule};
use cooltek_sdk_masterhub::{
    DialGesture, DialGestureTracker,
    input::{
        DecodedM003K, DecodedM005S, DecodedM032LE, DecodedM131E, DecodedM4315, DialDirection,
        FADER_POSITION_MAXIMUM, KeyAction, KnobEventKind, M003KDecoder, M005SDecoder,
        M032LEDecoder, M131EDecoder, M131EEvent, M4315Decoder, RollerDirection, RotationDirection,
    },
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// An event normalized for binding lookup within one device session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputEvent {
    /// Caller-supplied monotonic timestamp in microseconds.
    pub timestamp_micros: u64,
    /// Zero-based wire control index, independent of physical placement.
    pub control: u8,
    /// Normalized input or host gesture.
    pub gesture: Gesture,
    /// Numeric input carried by continuous controls; omitted for discrete events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<InputValue>,
}

/// Numeric input retained alongside a binding's selector gesture.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum InputValue {
    /// An absolute position in the source module's verified integer range.
    Position {
        /// Received absolute integer value.
        raw: u16,
        /// Verified inclusive full-scale value for this source module.
        maximum: u16,
    },
}

impl InputEvent {
    /// Validates a caller-created event before selector lookup or action resolution.
    pub fn validate_for(&self, module: ServiceModule) -> Result<(), String> {
        module.validate_selector(self.control, self.gesture)?;
        match (module, self.value) {
            (ServiceModule::M005S, Some(InputValue::Position { raw, maximum }))
                if maximum == u16::from(FADER_POSITION_MAXIMUM) && raw <= maximum =>
            {
                Ok(())
            }
            (ServiceModule::M005S, _) => {
                Err("fader events require a valid absolute position value".into())
            }
            (_, None) => Ok(()),
            _ => Err("numeric value is unsupported for this module and gesture".into()),
        }
    }
}

/// The interpretation of one raw report; raw bytes remain owned by the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputUpdate {
    /// Known input, possibly with no events because a button snapshot repeated.
    Events(Vec<InputEvent>),
    /// Unsupported report; callers can retain or log the original bytes.
    Unknown,
}

/// Stateful input normalization for one independently identified device.
///
/// Create a separate session per physical device. Reset on disconnect,
/// suspension, or transport failure. Malformed reports leave all decoder and
/// gesture state unchanged. Unknown reports advance timestamp ordering without
/// changing button state. A dial release emits its wire transition first, then
/// a short/long gesture only if the session observed a matching press.
#[derive(Clone, Debug)]
pub struct InputSession {
    module: ServiceModule,
    knobs: M003KDecoder,
    dial: M131EDecoder,
    keys: M4315Decoder,
    faders: M005SDecoder,
    rollers: M032LEDecoder,
    gestures: DialGestureTracker,
    last_timestamp: Option<u64>,
}

impl InputSession {
    /// Maximum complete input report length accepted by the service.
    pub const MAX_REPORT_BYTES: usize = 4096;

    /// Starts with all controls assumed released and no pending host gesture.
    pub fn new(module: ServiceModule) -> Self {
        Self {
            module,
            knobs: M003KDecoder::new(),
            dial: M131EDecoder::new(),
            keys: M4315Decoder::new(),
            faders: M005SDecoder::new(),
            rollers: M032LEDecoder::new(),
            gestures: DialGestureTracker::new(),
            last_timestamp: None,
        }
    }

    /// Clears decoder state and timing without synthesizing releases or actions.
    pub fn reset(&mut self) {
        self.knobs.reset();
        self.dial.reset();
        self.keys.reset();
        self.faders.reset();
        self.rollers.reset();
        self.gestures.reset();
        self.last_timestamp = None;
    }

    /// Decodes bounded bytes at a nondecreasing timestamp without hardware I/O.
    ///
    /// Errors leave the session unchanged, including when a caller supplies a
    /// backward timestamp. Duplicate button reports produce an empty event list;
    /// identical consecutive rotations each produce a new rotation event.
    pub fn decode(&mut self, raw: &[u8], timestamp_micros: u64) -> Result<InputUpdate, String> {
        if raw.len() > Self::MAX_REPORT_BYTES {
            return Err(format!(
                "input report exceeds {} bytes",
                Self::MAX_REPORT_BYTES
            ));
        }
        if self
            .last_timestamp
            .is_some_and(|last| timestamp_micros < last)
        {
            return Err("input timestamps must be nondecreasing".into());
        }
        let mut next = self.clone();
        let event = |control, gesture| InputEvent {
            timestamp_micros,
            control,
            gesture,
            value: None,
        };
        let update = match self.module {
            ServiceModule::M003K => match next
                .knobs
                .decode_report(raw)
                .map_err(|error| error.to_string())?
            {
                DecodedM003K::Unknown(_) => InputUpdate::Unknown,
                DecodedM003K::Events(events) => InputUpdate::Events(
                    events
                        .into_iter()
                        .map(|knob| {
                            let gesture = match knob.kind {
                                KnobEventKind::Rotated(RotationDirection::Clockwise) => {
                                    Gesture::Clockwise
                                }
                                KnobEventKind::Rotated(RotationDirection::CounterClockwise) => {
                                    Gesture::CounterClockwise
                                }
                                KnobEventKind::Pressed => Gesture::Pressed,
                                KnobEventKind::Released => Gesture::Released,
                            };
                            event(knob.knob_index, gesture)
                        })
                        .collect(),
                ),
            },
            ServiceModule::M131E => match next
                .dial
                .decode_report(raw)
                .map_err(|error| error.to_string())?
            {
                DecodedM131E::Unknown(_) => InputUpdate::Unknown,
                DecodedM131E::Events(events) => {
                    let mut normalized = Vec::with_capacity(2);
                    for input in events {
                        let gesture = match input {
                            M131EEvent::Rotated(DialDirection::Right) => Gesture::Right,
                            M131EEvent::Rotated(DialDirection::Left) => Gesture::Left,
                            M131EEvent::Pressed => Gesture::Pressed,
                            M131EEvent::Released => Gesture::Released,
                        };
                        normalized.push(event(0, gesture));
                        if let Some(gesture) = next
                            .gestures
                            .observe(input, Duration::from_micros(timestamp_micros))
                            .map_err(|error| error.to_string())?
                        {
                            normalized.push(event(
                                0,
                                match gesture {
                                    DialGesture::ShortPress { .. } => Gesture::ShortPress,
                                    DialGesture::LongPress { .. } => Gesture::LongPress,
                                },
                            ));
                        }
                    }
                    InputUpdate::Events(normalized)
                }
            },
            ServiceModule::M4315 => match next
                .keys
                .decode_report(raw)
                .map_err(|error| error.to_string())?
            {
                DecodedM4315::Unknown(_) => InputUpdate::Unknown,
                DecodedM4315::Events(events) => InputUpdate::Events(
                    events
                        .into_iter()
                        .map(|key| {
                            event(
                                key.key_index,
                                match key.action {
                                    KeyAction::Pressed => Gesture::Pressed,
                                    KeyAction::Released => Gesture::Released,
                                },
                            )
                        })
                        .collect(),
                ),
            },
            ServiceModule::M005S => match next
                .faders
                .decode_report(raw)
                .map_err(|error| error.to_string())?
            {
                DecodedM005S::Unknown(_) => InputUpdate::Unknown,
                DecodedM005S::Events(events) => InputUpdate::Events(
                    events
                        .into_iter()
                        .map(|fader| InputEvent {
                            timestamp_micros,
                            control: fader.fader_index,
                            gesture: Gesture::PositionChanged,
                            value: Some(InputValue::Position {
                                raw: u16::from(fader.position),
                                maximum: u16::from(FADER_POSITION_MAXIMUM),
                            }),
                        })
                        .collect(),
                ),
            },
            ServiceModule::M032LE => match next
                .rollers
                .decode_report(raw)
                .map_err(|error| error.to_string())?
            {
                DecodedM032LE::Unknown(_) => InputUpdate::Unknown,
                DecodedM032LE::Events(events) => InputUpdate::Events(
                    events
                        .into_iter()
                        .map(|roller| {
                            event(
                                roller.roller_index,
                                match roller.direction {
                                    RollerDirection::Up => Gesture::Up,
                                    RollerDirection::Down => Gesture::Down,
                                },
                            )
                        })
                        .collect(),
                ),
            },
        };
        next.last_timestamp = Some(timestamp_micros);
        *self = next;
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len()).unwrap().to_le_bytes();
        let mut raw = vec![1, tag, length[0], length[1]];
        raw.extend_from_slice(payload);
        raw
    }

    fn events(update: InputUpdate) -> Vec<InputEvent> {
        match update {
            InputUpdate::Events(events) => events,
            InputUpdate::Unknown => panic!("expected recognized input"),
        }
    }

    fn gestures(update: InputUpdate) -> Vec<Gesture> {
        events(update)
            .into_iter()
            .map(|event| event.gesture)
            .collect()
    }

    #[test]
    fn knobs_keep_wire_indexes_and_repeat_rotation_events() {
        let mut session = InputSession::new(ServiceModule::M003K);
        let raw = frame(0x2a, &[0, 3, 1, 0, 0xff]);
        for time in [0, 1] {
            let decoded = events(session.decode(&raw, time).unwrap());
            assert_eq!(decoded.len(), 2);
            assert_eq!(decoded[0].control, 0);
            assert_eq!(decoded[0].gesture, Gesture::Clockwise);
            assert_eq!(decoded[1].control, 1);
            assert_eq!(decoded[1].gesture, Gesture::CounterClockwise);
            assert_eq!(decoded[1].timestamp_micros, time);
        }
    }

    #[test]
    fn knob_press_snapshots_deduplicate_and_malformed_input_is_transactional() {
        let mut session = InputSession::new(ServiceModule::M003K);
        let press = frame(0x2a, &[3, 3, 1, 0, 1]);
        assert_eq!(
            gestures(session.decode(&press, 1).unwrap()),
            [Gesture::Pressed, Gesture::Pressed]
        );
        assert_eq!(
            session.decode(&press, 2).unwrap(),
            InputUpdate::Events(vec![])
        );
        assert!(session.decode(&frame(0x2a, &[3, 3, 0, 1, 2]), 3).is_err());
        assert_eq!(
            session.decode(&frame(0x2b, &[3]), 3).unwrap(),
            InputUpdate::Unknown
        );
        let release = events(session.decode(&frame(0x2a, &[3, 3, 0, 0, 0]), 4).unwrap());
        assert_eq!(
            release
                .iter()
                .map(|event| event.control)
                .collect::<Vec<_>>(),
            [0, 2]
        );
        assert!(
            release
                .iter()
                .all(|event| event.gesture == Gesture::Released)
        );
    }

    #[test]
    fn dial_retains_app_labels_and_duplicates_rotations() {
        let mut session = InputSession::new(ServiceModule::M131E);
        for (value, direction) in [(0, Gesture::Right), (1, Gesture::Left)] {
            let raw = frame(0x44, &[3, 0, value]);
            for _ in 0..2 {
                assert_eq!(gestures(session.decode(&raw, 0).unwrap()), [direction]);
            }
        }
    }

    #[test]
    fn dial_threshold_is_inclusive_and_duplicate_presses_keep_original_time() {
        for (duration, expected) in [
            (599_999, Gesture::ShortPress),
            (600_000, Gesture::LongPress),
        ] {
            let mut session = InputSession::new(ServiceModule::M131E);
            let press = frame(0x44, &[4, 0, 1]);
            assert_eq!(
                gestures(session.decode(&press, 100).unwrap()),
                [Gesture::Pressed]
            );
            assert_eq!(
                session.decode(&press, 200).unwrap(),
                InputUpdate::Events(vec![])
            );
            assert_eq!(
                gestures(
                    session
                        .decode(&frame(0x44, &[4, 0, 0]), duration + 100)
                        .unwrap()
                ),
                [Gesture::Released, expected]
            );
        }
    }

    #[test]
    fn unknown_and_malformed_dial_input_preserve_pending_gesture() {
        let mut session = InputSession::new(ServiceModule::M131E);
        session.decode(&frame(0x44, &[4, 0, 1]), 0).unwrap();
        assert_eq!(
            session.decode(&frame(0x44, &[4, 0, 2]), 1).unwrap(),
            InputUpdate::Unknown
        );
        assert!(session.decode(&frame(0x44, &[4, 0]), 2).is_err());
        assert_eq!(
            gestures(session.decode(&frame(0x44, &[1, 0, 0]), 600_000).unwrap()),
            [Gesture::Released, Gesture::LongPress]
        );
    }

    #[test]
    fn unmatched_release_and_reset_do_not_synthesize_gestures() {
        let mut session = InputSession::new(ServiceModule::M131E);
        let release = frame(0x44, &[4, 0, 0]);
        assert_eq!(
            session.decode(&release, 1).unwrap(),
            InputUpdate::Events(vec![])
        );
        session.decode(&frame(0x44, &[4, 0, 1]), 2).unwrap();
        session.reset();
        assert_eq!(
            session.decode(&release, 0).unwrap(),
            InputUpdate::Events(vec![])
        );
        assert_eq!(
            gestures(session.decode(&frame(0x44, &[4, 0, 1]), 1).unwrap()),
            [Gesture::Pressed]
        );
    }

    #[test]
    fn backward_timestamps_and_oversized_reports_do_not_mutate_state() {
        let mut session = InputSession::new(ServiceModule::M131E);
        session.decode(&frame(0x44, &[4, 0, 1]), 100).unwrap();
        assert!(session.decode(&frame(0x44, &[4, 0, 0]), 99).is_err());
        assert!(
            session
                .decode(&vec![0; InputSession::MAX_REPORT_BYTES + 1], 101)
                .is_err()
        );
        assert_eq!(
            gestures(
                session
                    .decode(&frame(0x44, &[4, 0, 0]), 100 + 600_000)
                    .unwrap()
            ),
            [Gesture::Released, Gesture::LongPress]
        );
    }

    #[test]
    fn malformed_report_does_not_advance_timestamp_but_unknown_does() {
        let mut session = InputSession::new(ServiceModule::M131E);
        session.decode(&frame(0x44, &[3, 0, 0]), 100).unwrap();
        assert!(session.decode(&[1, 0x44], 200).is_err());
        assert!(session.decode(&frame(0x44, &[3, 0, 0]), 150).is_ok());
        assert_eq!(session.decode(&[2], 300).unwrap(), InputUpdate::Unknown);
        assert!(session.decode(&frame(0x44, &[3, 0, 0]), 250).is_err());
    }

    #[test]
    fn key_transitions_use_wire_indexes_and_reset_without_releases() {
        let mut session = InputSession::new(ServiceModule::M4315);
        let mut payload = vec![2, 15];
        payload.extend_from_slice(&[0; 15]);
        payload[2] = 1;
        payload[16] = 1;
        let press = frame(0x34, &payload);
        let first = events(session.decode(&press, 1).unwrap());
        assert_eq!(
            first.iter().map(|event| event.control).collect::<Vec<_>>(),
            [0, 14]
        );
        assert!(
            first
                .iter()
                .all(|event| event.gesture == Gesture::Pressed && event.value.is_none())
        );
        assert_eq!(
            session.decode(&press, 2).unwrap(),
            InputUpdate::Events(vec![])
        );
        assert_eq!(session.decode(&[2], 3).unwrap(), InputUpdate::Unknown);
        let mut malformed = payload.clone();
        malformed[2] = 0;
        malformed[16] = 2;
        assert!(session.decode(&frame(0x34, &malformed), 4).is_err());
        assert_eq!(
            session.decode(&press, 4).unwrap(),
            InputUpdate::Events(vec![])
        );
        payload[2] = 0;
        payload[16] = 0;
        assert_eq!(
            gestures(session.decode(&frame(0x34, &payload), 5).unwrap()),
            [Gesture::Released, Gesture::Released]
        );
        session.reset();
        assert_eq!(events(session.decode(&press, 0).unwrap()).len(), 2);
    }

    #[test]
    fn faders_preserve_absolute_percentages_emit_changes_and_reset_initial_state() {
        let mut session = InputSession::new(ServiceModule::M005S);
        let first = frame(0x2a, &[2, 5, 0, 25, 50, 75, 100]);
        let initial = events(session.decode(&first, 1).unwrap());
        assert_eq!(initial.len(), 5);
        for (event, expected) in initial.iter().zip([0, 25, 50, 75, 100]) {
            assert_eq!(event.gesture, Gesture::PositionChanged);
            assert_eq!(
                event.value,
                Some(InputValue::Position {
                    raw: expected,
                    maximum: 100
                })
            );
            event.validate_for(ServiceModule::M005S).unwrap();
        }
        assert_eq!(
            session.decode(&first, 2).unwrap(),
            InputUpdate::Events(vec![])
        );
        let changed = frame(0x2a, &[2, 5, 0, 25, 51, 75, 100]);
        let updates = events(session.decode(&changed, 3).unwrap());
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].control, 2);
        assert_eq!(
            updates[0].value,
            Some(InputValue::Position {
                raw: 51,
                maximum: 100
            })
        );
        assert!(
            session
                .decode(&frame(0x2a, &[2, 5, 0, 26, 101, 75, 100]), 4)
                .is_err()
        );
        assert_eq!(session.decode(&[2], 4).unwrap(), InputUpdate::Unknown);
        assert_eq!(
            session.decode(&changed, 5).unwrap(),
            InputUpdate::Events(vec![])
        );
        session.reset();
        assert_eq!(events(session.decode(&changed, 0).unwrap()).len(), 5);
    }

    #[test]
    fn rollers_emit_repeatable_discrete_directions_without_inventing_delta() {
        let mut session = InputSession::new(ServiceModule::M032LE);
        let raw = frame(0x2a, &[1, 2, 0, 1]);
        for time in [0, 1] {
            let decoded = events(session.decode(&raw, time).unwrap());
            assert_eq!(
                decoded
                    .iter()
                    .map(|event| event.gesture)
                    .collect::<Vec<_>>(),
                [Gesture::Up, Gesture::Down]
            );
            assert!(decoded.iter().all(|event| event.value.is_none()));
        }
        assert_eq!(
            session
                .decode(&frame(0x2a, &[1, 2, 0xff, 0xff]), 2)
                .unwrap(),
            InputUpdate::Events(vec![])
        );
        assert_eq!(session.decode(&[2], 3).unwrap(), InputUpdate::Unknown);
        assert!(session.decode(&frame(0x2a, &[1, 2, 0, 2]), 4).is_err());
        session.reset();
        assert_eq!(events(session.decode(&raw, 0).unwrap()).len(), 2);
    }

    #[test]
    fn numeric_event_validation_rejects_missing_values_and_changed_scale() {
        let mut input = InputEvent {
            timestamp_micros: 0,
            control: 0,
            gesture: Gesture::PositionChanged,
            value: Some(InputValue::Position {
                raw: 50,
                maximum: 100,
            }),
        };
        input.validate_for(ServiceModule::M005S).unwrap();
        for value in [
            None,
            Some(InputValue::Position {
                raw: 50,
                maximum: 255,
            }),
            Some(InputValue::Position {
                raw: 101,
                maximum: 100,
            }),
            Some(InputValue::Position { raw: 0, maximum: 0 }),
        ] {
            input.value = value;
            assert!(input.validate_for(ServiceModule::M005S).is_err());
        }
        input.gesture = Gesture::Pressed;
        input.value = Some(InputValue::Position {
            raw: 50,
            maximum: 100,
        });
        assert!(input.validate_for(ServiceModule::M4315).is_err());
        assert!(input.validate_for(ServiceModule::M003K).is_err());
        input.value = None;
        input.validate_for(ServiceModule::M4315).unwrap();
    }

    #[test]
    fn optional_position_is_backward_compatible_and_strict() {
        let old = br#"{"timestamp_micros":0,"control":0,"gesture":"pressed"}"#;
        let input: InputEvent = serde_json::from_slice(old).unwrap();
        assert!(input.value.is_none());
        assert_eq!(
            serde_json::to_value(input).unwrap(),
            serde_json::from_slice::<serde_json::Value>(old).unwrap()
        );
        let json = br#"{"timestamp_micros":0,"control":0,"gesture":"position_changed","value":{"type":"position","raw":50,"maximum":100}}"#;
        let input: InputEvent = serde_json::from_slice(json).unwrap();
        input.validate_for(ServiceModule::M005S).unwrap();
        for value in [
            br#"{"type":"position","raw":50,"maximum":100,"extra":1}"#.as_slice(),
            br#"{"type":"delta","delta":1}"#.as_slice(),
        ] {
            assert!(serde_json::from_slice::<InputValue>(value).is_err());
        }
    }
}
