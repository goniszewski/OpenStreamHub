use super::*;

// Synthetic protocol examples; these are not captured hardware reports.
fn frame(tag: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(payload.len()).unwrap().to_le_bytes();
    let mut raw = vec![1, tag, length[0], length[1]];
    raw.extend_from_slice(payload);
    raw
}

fn keys(selector: u8, pressed: &[usize]) -> Vec<u8> {
    let mut payload = vec![selector, 15];
    payload.extend_from_slice(&[0; 15]);
    for &index in pressed {
        payload[index + 2] = 1;
    }
    frame(0x34, &payload)
}

#[test]
fn key_snapshots_support_probe_and_renderer_example_selectors() {
    let mut decoder = M4315Decoder::new();
    let expected = vec![
        KeyEvent {
            key_index: 0,
            action: KeyAction::Pressed,
        },
        KeyEvent {
            key_index: 7,
            action: KeyAction::Pressed,
        },
        KeyEvent {
            key_index: 14,
            action: KeyAction::Pressed,
        },
    ];
    assert_eq!(
        decoder.decode_report(&keys(2, &[0, 7, 14])).unwrap(),
        DecodedM4315::Events(expected.clone())
    );
    assert_eq!(
        decoder.decode_report(&keys(0, &[0, 7, 14])).unwrap(),
        DecodedM4315::Events(vec![])
    );
    assert_eq!(
        decoder.decode_report(&keys(2, &[])).unwrap(),
        DecodedM4315::Events(vec![
            KeyEvent {
                key_index: 0,
                action: KeyAction::Released
            },
            KeyEvent {
                key_index: 7,
                action: KeyAction::Released
            },
            KeyEvent {
                key_index: 14,
                action: KeyAction::Released
            },
        ])
    );
    decoder.reset();
    assert_eq!(decoder.pressed_states(), [false; 15]);
    assert_eq!(
        decoder.decode_report(&keys(2, &[0, 7, 14])).unwrap(),
        DecodedM4315::Events(expected)
    );
}

#[test]
fn invalid_late_key_state_and_unknown_reports_preserve_held_keys() {
    let mut decoder = M4315Decoder::new();
    decoder.decode_report(&keys(2, &[0])).unwrap();
    let previous = decoder.pressed_states();
    let mut invalid = keys(2, &[1]);
    invalid[20] = 2;
    assert!(matches!(
        decoder.decode_report(&invalid),
        Err(InputError::InvalidControlValue {
            control_index: 14,
            ..
        })
    ));
    assert_eq!(decoder.pressed_states(), previous);
    let unknown = keys(0x7f, &[]);
    assert_eq!(
        decoder.decode_report(&unknown),
        Ok(DecodedM4315::Unknown(&unknown))
    );
    assert_eq!(decoder.pressed_states(), previous);
    assert_eq!(
        decoder.decode_report(&keys(2, &[])).unwrap(),
        DecodedM4315::Events(vec![KeyEvent {
            key_index: 0,
            action: KeyAction::Released
        }])
    );
}

#[test]
fn fader_positions_begin_unknown_then_emit_only_changes_and_reset() {
    let mut decoder = M005SDecoder::new();
    assert_eq!(decoder.positions(), None);
    let raw = frame(0x2a, &[2, 5, 0, 25, 50, 75, 100]);
    let expected: Vec<_> = [0, 25, 50, 75, 100]
        .into_iter()
        .enumerate()
        .map(|(index, position)| FaderEvent {
            fader_index: index as u8,
            position,
        })
        .collect();
    assert_eq!(
        decoder.decode_report(&raw),
        Ok(DecodedM005S::Events(expected.clone()))
    );
    assert_eq!(decoder.positions(), Some([0, 25, 50, 75, 100]));
    assert_eq!(
        decoder.decode_report(&raw),
        Ok(DecodedM005S::Events(vec![]))
    );
    let changed = frame(0x2a, &[2, 5, 100, 25, 50, 75, 0]);
    assert_eq!(
        decoder.decode_report(&changed),
        Ok(DecodedM005S::Events(vec![
            FaderEvent {
                fader_index: 0,
                position: 100
            },
            FaderEvent {
                fader_index: 4,
                position: 0
            },
        ]))
    );
    decoder.reset();
    assert_eq!(decoder.positions(), None);
    assert_eq!(
        decoder.decode_report(&raw),
        Ok(DecodedM005S::Events(expected))
    );
}

#[test]
fn invalid_fader_tail_and_unsupported_selector_do_not_change_positions() {
    let mut decoder = M005SDecoder::new();
    decoder
        .decode_report(&frame(0x2a, &[2, 5, 0, 25, 50, 75, 100]))
        .unwrap();
    let previous = decoder.positions();
    for value in [101, 0xff] {
        assert!(matches!(
            decoder.decode_report(&frame(0x2a, &[2, 5, 1, 2, 3, 4, value])),
            Err(InputError::InvalidControlValue {
                control_index: 4,
                ..
            })
        ));
        assert_eq!(decoder.positions(), previous);
    }
    let unknown = frame(0x2a, &[0, 5, 1, 2, 3, 4, 5]);
    assert_eq!(
        decoder.decode_report(&unknown),
        Ok(DecodedM005S::Unknown(&unknown))
    );
    assert_eq!(decoder.positions(), previous);
}

#[test]
fn roller_updates_keep_wire_order_and_repeat_without_fabricated_magnitude() {
    let mut decoder = M032LEDecoder::new();
    let raw = frame(0x2a, &[1, 2, 0, 1]);
    let expected = vec![
        RollerEvent {
            roller_index: 0,
            direction: RollerDirection::Up,
        },
        RollerEvent {
            roller_index: 1,
            direction: RollerDirection::Down,
        },
    ];
    for _ in 0..2 {
        assert_eq!(
            decoder.decode_report(&raw),
            Ok(DecodedM032LE::Events(expected.clone()))
        );
    }
    assert_eq!(
        decoder.decode_report(&frame(0x2a, &[1, 2, 0xff, 0xff])),
        Ok(DecodedM032LE::Events(vec![]))
    );
    assert_eq!(
        decoder.decode_report(&frame(0x2a, &[1, 2, 0xff, 0])),
        Ok(DecodedM032LE::Events(vec![RollerEvent {
            roller_index: 1,
            direction: RollerDirection::Up
        }]))
    );
    assert!(matches!(
        decoder.decode_report(&frame(0x2a, &[1, 2, 1, 2])),
        Err(InputError::InvalidControlValue {
            control_index: 1,
            ..
        })
    ));
    decoder.reset();
    assert_eq!(
        decoder.decode_report(&raw),
        Ok(DecodedM032LE::Events(expected))
    );
}

#[test]
fn known_counts_and_logical_lengths_are_exact() {
    for payload in [
        vec![2, 14],
        vec![2, 15],
        vec![2, 15, 0],
        vec![2, 15].into_iter().chain([0; 16]).collect(),
    ] {
        assert!(matches!(
            parse_m4315_payload(&payload),
            Err(InputError::InvalidControlLength {
                module: "M4315",
                ..
            })
        ));
    }
    for payload in [
        &[2, 4, 0, 0, 0, 0][..],
        &[2, 5, 0, 0, 0, 0][..],
        &[2, 5, 0, 0, 0, 0, 0, 0][..],
    ] {
        assert!(matches!(
            parse_m005s_payload(payload),
            Err(InputError::InvalidControlLength {
                module: "M005S",
                ..
            })
        ));
    }
    for payload in [&[1, 1, 0][..], &[1, 2, 0][..], &[1, 2, 0, 0, 0][..]] {
        assert!(matches!(
            parse_m032le_payload(payload),
            Err(InputError::InvalidControlLength {
                module: "M032LE",
                ..
            })
        ));
    }
}

#[test]
fn every_known_frame_prefix_is_rejected_and_outer_padding_is_ignored() {
    let key = keys(2, &[14]);
    let fader = frame(0x2a, &[2, 5, 0, 25, 50, 75, 100]);
    let roller = frame(0x2a, &[1, 2, 0, 0xff]);
    for length in 0..key.len() {
        assert!(M4315Decoder::new().decode_report(&key[..length]).is_err());
    }
    for length in 0..fader.len() {
        assert!(M005SDecoder::new().decode_report(&fader[..length]).is_err());
    }
    for length in 0..roller.len() {
        assert!(
            M032LEDecoder::new()
                .decode_report(&roller[..length])
                .is_err()
        );
    }
    let padded = |raw: &[u8]| {
        let mut result = raw.to_vec();
        result.extend_from_slice(&[0xaa; 30]);
        result
    };
    assert_eq!(
        M4315Decoder::new().decode_report(&padded(&key)).unwrap(),
        M4315Decoder::new().decode_report(&key).unwrap()
    );
    assert_eq!(
        M005SDecoder::new().decode_report(&padded(&fader)).unwrap(),
        M005SDecoder::new().decode_report(&fader).unwrap()
    );
    assert_eq!(
        M032LEDecoder::new()
            .decode_report(&padded(&roller))
            .unwrap(),
        M032LEDecoder::new().decode_report(&roller).unwrap()
    );
}

#[test]
fn unsupported_outer_channels_are_retained_without_framing_assumptions() {
    for raw in [
        &[1, 0x35, 0xff, 0xff][..],
        &[1, 0x36][..],
        &[1, 0x37][..],
        &[2][..],
    ] {
        assert_eq!(
            M4315Decoder::new().decode_report(raw),
            Ok(DecodedM4315::Unknown(raw))
        );
    }
    for raw in [&[1, 0x2b, 0xff, 0xff][..], &[1, 0x2d][..], &[2][..]] {
        assert_eq!(
            M005SDecoder::new().decode_report(raw),
            Ok(DecodedM005S::Unknown(raw))
        );
        assert_eq!(
            M032LEDecoder::new().decode_report(raw),
            Ok(DecodedM032LE::Unknown(raw))
        );
    }
    let too_long = [1, 0x34, 0, 1, 2, 15];
    assert_eq!(
        M4315Decoder::new().decode_report(&too_long),
        Err(InputError::TruncatedPayload {
            declared: 256,
            available: 2
        })
    );
}

#[test]
fn supported_value_domains_are_exhaustive_without_clamping() {
    for value in 0..=u8::MAX {
        assert_eq!(
            parse_m005s_payload(&[2, 5, 0, 0, 0, 0, value]).is_ok(),
            value <= 100
        );
        assert_eq!(
            parse_m032le_payload(&[1, 2, 0xff, value]).is_ok(),
            matches!(value, 0 | 1 | 0xff)
        );
        let mut payload = [0; 17];
        payload[0] = 2;
        payload[1] = 15;
        payload[16] = value;
        assert_eq!(
            parse_m4315_payload(&payload).is_ok(),
            matches!(value, 0 | 1)
        );
    }
}
