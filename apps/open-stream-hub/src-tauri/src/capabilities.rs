//! Which capabilities a saved profile adds over the profile a session holds.
//!
//! A save applies to a running session in place. In Live mode the swap may
//! restart the process executing without asking again, but only when the new
//! profile can do nothing the confirmed profile could not: every capability
//! of the new profile must already exist in the held one. The items here
//! mirror what the Execute confirmation discloses in the app (the kinds of
//! things a profile can do and their distinct parameters); they are an
//! identity model, not a risk model. Anything not already in the held
//! profile — a new snippet, a new shortcut, a sequence in a new order, a
//! kind of action the run never had — is an addition, and the session pauses
//! for one confirmation that names it.

use cooltek_sdk_service::{Action, KeypadTile, Profile};
use std::collections::BTreeSet;

/// Every distinct capability of `profile`, as comparable items.
fn items(profile: &Profile) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for binding in &profile.bindings {
        visit(&binding.action, &mut set);
    }
    if let Some(keypad) = &profile.keypad {
        for page in &keypad.pages {
            for slot in &page.slots {
                if let Some(KeypadTile::Action { bindings, .. }) = slot {
                    for binding in bindings {
                        visit(&binding.action, &mut set);
                    }
                }
            }
        }
    }
    if let Some(functions) = &profile.dial_functions {
        for function in functions {
            for binding in &function.bindings {
                visit(&binding.action, &mut set);
            }
        }
    }
    set
}

fn visit(action: &Action, set: &mut BTreeSet<String>) {
    match action {
        // Volume, fader-template and mute effects are all of a kind.
        Action::Volume { .. } | Action::VolumeFromPosition | Action::ToggleMute => {
            set.insert("volume".into());
        }
        // An absolute volume target is a fact the confirmation names, so a
        // new one counts as new.
        Action::SetVolume { percent } => {
            set.insert(format!("set_volume\0{percent}"));
        }
        Action::Shortcut { key, modifiers } => {
            let mut names: Vec<String> = modifiers
                .iter()
                .map(|modifier| format!("{modifier:?}"))
                .collect();
            names.sort();
            set.insert(format!("shortcut\0{}\0{key}", names.join("+")));
        }
        Action::LaunchApp { bundle_id } => {
            set.insert(format!("app\0{bundle_id}"));
        }
        // Only the origin is an item, as the confirmation shows it: a path
        // or query change of a known origin is not a new capability.
        Action::OpenUrl { url } => {
            set.insert(format!("url\0{}", url_origin(url)));
        }
        Action::Media { command } => {
            set.insert(format!("media\0{command:?}"));
        }
        Action::System { command } => {
            set.insert(format!("system\0{command:?}"));
        }
        // Text cannot contain NUL (validation allows only newline and tab
        // beyond the printable rules), so it separates the item fields.
        Action::Text { text, press_enter } => {
            set.insert(format!("text\0{text}\0{press_enter}"));
        }
        Action::MultiAction { actions } => {
            // The JSON of the typed steps is a stable identity, so a known
            // set of steps in a new order is still a new sequence item.
            match serde_json::to_string(actions) {
                Ok(steps) => set.insert(format!("sequence\0{steps}")),
                // Never produced (the steps are plain typed values); fail
                // closed if it ever were: always new.
                Err(_) => set.insert("sequence\0unnameable".into()),
            };
            for step in actions {
                visit(step, set);
            }
        }
        Action::Profile { direction } => {
            set.insert(format!("profile_key\0{direction:?}"));
        }
        // The label is the notification text, so it is part of the item.
        Action::Timer {
            seconds,
            label,
            sound,
        } => {
            set.insert(format!("timer\0{seconds}\0{label:?}\0{sound:?}"));
        }
        Action::ObsScene { scene } => {
            set.insert(format!("obs_scene\0{scene}"));
        }
        Action::ObsVirtualCamera { op } => {
            set.insert(format!("obs_camera\0{op:?}"));
        }
        Action::ObsReplayBuffer { op } => {
            set.insert(format!("obs_replay\0{op:?}"));
        }
        Action::ObsStudioMode { op } => {
            set.insert(format!("obs_studio_mode\0{op:?}"));
        }
        Action::ObsTransition => {
            set.insert("obs_transition".into());
        }
        Action::ObsFilter { source, filter, op } => {
            set.insert(format!("obs_filter\0{source}\0{filter}\0{op:?}"));
        }
        Action::ObsSourceVisibility { scene, source, op } => {
            set.insert(format!("obs_visibility\0{scene}\0{source}\0{op:?}"));
        }
        Action::ObsSceneCollection { name } => {
            set.insert(format!("obs_collection\0{name}"));
        }
        Action::ObsProfile { name } => {
            set.insert(format!("obs_profile\0{name}"));
        }
        // A fader-driven input volume names only the input; a fixed one also
        // names its level, like `set_volume`.
        Action::ObsInputVolumeFromPosition { input } => {
            set.insert(format!("obs_input_volume\0{input}"));
        }
        Action::ObsSetInputVolume { input, percent } => {
            set.insert(format!("obs_set_input_volume\0{input}\0{percent}"));
        }
    }
}

/// `scheme://authority` of a URL, lowercase, never a path, query or user
/// information. Mirrors the app's `urlOrigin`, which the Execute
/// confirmation shows.
fn url_origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or((url, ""));
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit('@')
        .next()
        .unwrap_or_default();
    format!("{scheme}://{authority}").to_lowercase()
}

/// Whether `next` can do anything `held` could not. `held` is the profile a
/// session runs (the one a confirmation covered); `next` is the saved edit.
/// An equal or smaller profile adds nothing.
pub fn adds_capabilities(held: &Profile, next: &Profile) -> bool {
    !items(held).is_superset(&items(next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::{
        Binding, DialBinding, DialFunction, Gesture, KeypadBinding, KeypadLayout, KeypadPage,
        Modifier, ProfileDirection, ServiceModule,
    };

    fn profile(bindings: Vec<Action>) -> Profile {
        Profile {
            schema_version: 1,
            name: "T".into(),
            bindings: bindings
                .into_iter()
                .map(|action| Binding {
                    module: ServiceModule::M003K,
                    control: 0,
                    gesture: Gesture::Pressed,
                    serial_number: None,
                    action,
                })
                .collect(),
            keypad: None,
            dial_functions: None,
            dial_display: None,
        }
    }

    fn text(text: &str, press_enter: bool) -> Action {
        Action::Text {
            text: text.into(),
            press_enter,
        }
    }

    #[test]
    fn a_subset_of_the_held_capabilities_adds_nothing() {
        let held = profile(vec![
            text("hello", true),
            Action::Shortcut {
                key: "c".into(),
                modifiers: vec![Modifier::Control],
            },
        ]);
        // Fewer bindings only remove (the shortcut is dropped).
        let next = profile(vec![text("hello", true)]);
        assert!(!adds_capabilities(&held, &next));
        assert!(!adds_capabilities(&held, &held));
        // A changed snippet is an addition, and so is the same snippet with
        // the Return press dropped: a different item, confirmed never.
        assert!(adds_capabilities(
            &held,
            &profile(vec![text("hello!", true)])
        ));
        assert!(adds_capabilities(
            &held,
            &profile(vec![text("hello", false)])
        ));
    }

    #[test]
    fn a_new_kind_of_action_is_an_addition() {
        let held = profile(vec![Action::ToggleMute]);
        assert!(adds_capabilities(
            &held,
            &profile(vec![
                Action::ToggleMute,
                Action::OpenUrl {
                    url: "https://example.com/a?token=1".into()
                },
            ])
        ));
        // A path, query or case change of a known origin is not.
        assert!(!adds_capabilities(
            &profile(vec![Action::OpenUrl {
                url: "https://example.com/a".into()
            }]),
            &profile(vec![Action::OpenUrl {
                url: "https://EXAMPLE.com/b?q=2".into()
            }]),
        ));
    }

    #[test]
    fn a_known_sequence_in_a_new_order_is_an_addition() {
        let paste = text("user", false);
        let tab = Action::Shortcut {
            key: "tab".into(),
            modifiers: vec![],
        };
        let held = profile(vec![Action::MultiAction {
            actions: vec![paste.clone(), tab.clone()],
        }]);
        assert!(adds_capabilities(
            &held,
            &profile(vec![Action::MultiAction {
                actions: vec![tab.clone(), paste.clone()],
            }])
        ));
        // The same sequence again adds nothing, and so does a profile whose
        // extra steps all exist inside known sequences.
        assert!(!adds_capabilities(&held, &held));
        assert!(!adds_capabilities(&held, &profile(vec![paste, tab])));
    }

    #[test]
    fn keypad_pages_and_dial_functions_are_walked() {
        let mut held = profile(vec![Action::ToggleMute]);
        let mut page = KeypadPage {
            id: "root".into(),
            name: "Root".into(),
            slots: (0..15).map(|_| None).collect(),
            artwork: Default::default(),
        };
        page.slots[1] = Some(KeypadTile::Action {
            label: "Paste".into(),
            bindings: vec![KeypadBinding {
                gesture: Gesture::Pressed,
                serial_number: None,
                action: text("secret", false),
            }],
        });
        held.keypad = Some(KeypadLayout { pages: vec![page] });
        // The same snippet moved to the dial: nothing new.
        let mut next = held.clone();
        next.keypad = None;
        next.dial_functions = Some(vec![DialFunction {
            name: "Text".into(),
            bindings: vec![DialBinding {
                gesture: Gesture::Right,
                serial_number: None,
                action: text("secret", false),
            }],
        }]);
        assert!(!adds_capabilities(&held, &next));
        let mut edited = next;
        edited.dial_functions.as_mut().unwrap()[0]
            .bindings
            .push(DialBinding {
                gesture: Gesture::Left,
                serial_number: None,
                action: Action::Timer {
                    seconds: 300,
                    label: Some("Break".into()),
                    sound: None,
                },
            });
        assert!(adds_capabilities(&held, &edited));
    }

    #[test]
    fn url_origins_never_carry_user_information_or_a_path() {
        assert_eq!(
            url_origin("https://User@example.com:8443/watch?v=1#top"),
            "https://example.com:8443"
        );
        assert_eq!(url_origin("http://example.com"), "http://example.com");
        assert_eq!(url_origin("not a url"), "not a url://");
    }

    #[test]
    fn a_profile_key_direction_is_part_of_the_item() {
        let held = profile(vec![Action::Profile {
            direction: ProfileDirection::Next,
        }]);
        assert!(adds_capabilities(
            &held,
            &profile(vec![Action::Profile {
                direction: ProfileDirection::Previous,
            }])
        ));
    }
}
