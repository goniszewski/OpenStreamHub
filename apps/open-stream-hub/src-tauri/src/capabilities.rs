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

/// The item of a key that sets `agent_decision`, as the confirmation names it.
const AGENT_ANSWER: &str = "answers AI agent permission prompts";

/// Every distinct capability of `profile`, as comparable items.
fn items(profile: &Profile) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let actions = top_level_actions(profile);
    for action in &actions {
        visit(action, &mut set);
    }
    feeding_values(profile, &actions, &mut set);
    set
}

/// The action of every binding: flat bindings, every keypad tile and every
/// dial function (steps inside them are walked by the callers).
fn top_level_actions(profile: &Profile) -> Vec<&Action> {
    let mut actions: Vec<&Action> = profile
        .bindings
        .iter()
        .map(|binding| &binding.action)
        .collect();
    if let Some(keypad) = &profile.keypad {
        for page in &keypad.pages {
            for slot in &page.slots {
                if let Some(KeypadTile::Action { bindings, .. }) = slot {
                    actions.extend(bindings.iter().map(|binding| &binding.action));
                }
            }
        }
    }
    if let Some(functions) = &profile.dial_functions {
        for function in functions {
            actions.extend(function.bindings.iter().map(|binding| &binding.action));
        }
    }
    actions
}

/// Calls `each` on `action` and every step that can run inside it.
fn walk<'a>(action: &'a Action, each: &mut impl FnMut(&'a Action)) {
    each(action);
    match action {
        Action::Conditional {
            then, otherwise, ..
        } => then
            .iter()
            .chain(otherwise)
            .for_each(|step| walk(step, each)),
        Action::MultiAction { actions } => actions.iter().for_each(|step| walk(step, each)),
        _ => {}
    }
}

/// A variable a `run_program` fills into its arguments, or a `plugin` action
/// or plugin condition into its text parameters, decides what runs, so
/// what the profile itself can make it (its declared value and every write of
/// it) is an item too: a changed starting value, or a key that sets it to a
/// new value, needs a new confirmation. Other programs on this Mac can still
/// set it, which the confirmation says.
fn feeding_values(profile: &Profile, actions: &[&Action], set: &mut BTreeSet<String>) {
    let mut filled_in = BTreeSet::new();
    for action in actions {
        walk(action, &mut |step| {
            if let Action::RunProgram {
                args,
                variables: true,
                ..
            } = step
            {
                for arg in args {
                    // Malformed text is refused by the service and fills in nothing.
                    filled_in.extend(
                        cooltek_sdk_service::variable_references(arg)
                            .unwrap_or_default()
                            .into_iter()
                            .map(str::to_owned),
                    );
                }
            }
            // Every plugin action and condition in this step (and in the
            // steps inside it, which the walk reaches again: a set).
            for found in step.plugin_uses() {
                for name in found.variables {
                    if let Some(cooltek_sdk_service::PluginValue::Text(text)) =
                        found.params.get(name)
                    {
                        filled_in.extend(
                            cooltek_sdk_service::variable_references(text)
                                .unwrap_or_default()
                                .into_iter()
                                .map(str::to_owned),
                        );
                    }
                }
            }
        });
    }
    if filled_in.is_empty() {
        return;
    }
    let item = |name: &str, write: String| format!("feeds\0{name}\0{write}");
    for variable in &profile.variables {
        if filled_in.contains(&variable.name) {
            let value =
                serde_json::to_string(&variable.value).unwrap_or_else(|_| "unnameable".into());
            set.insert(item(&variable.name, format!("declared\0{value}")));
        }
    }
    for action in actions {
        walk(action, &mut |step| {
            let name = match step {
                Action::SetVariable { name, .. }
                | Action::ToggleVariable { name }
                | Action::AddToVariable { name, .. } => name,
                _ => return,
            };
            if filled_in.contains(name) {
                let write = serde_json::to_string(step).unwrap_or_else(|_| "unnameable".into());
                set.insert(item(name, write));
            }
        });
    }
}

fn visit(action: &Action, set: &mut BTreeSet<String>) {
    match action {
        // Volume, fader-template and mute effects are all of a kind.
        Action::Volume { .. } | Action::VolumeFromPosition | Action::ToggleMute => {
            set.insert("volume".into());
        }
        // The microphone's gain is its own effect, apart from the output volume.
        Action::MicInputVolumeFromPosition | Action::SetMicInputVolume { .. } => {
            set.insert("mic_volume".into());
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
        // The whole command line is the item: the app, the directory and the
        // command all decide what runs. Neither field can hold NUL.
        Action::TerminalCommand {
            app,
            command,
            directory,
        } => {
            set.insert(format!("terminal\0{app:?}\0{directory:?}\0{command}"));
        }
        // The path, every argument, the directory and whether variables are
        // filled in decide what runs, so all of them are the item. JSON keeps
        // the argument boundaries exact.
        Action::RunProgram {
            program,
            args,
            directory,
            variables,
        } => {
            let identity = serde_json::to_string(&(program, args, directory, variables))
                .unwrap_or_else(|_| "unnameable".into());
            set.insert(format!("program\0{identity}"));
        }
        // The plugin, the action, every parameter and which ones take
        // variables decide what the plugin is asked to do, so all of them are
        // the item, as for `run_program`. The minimum version is not: it
        // changes which plugin may answer, not what is asked.
        Action::Plugin {
            plugin,
            action,
            params,
            variables,
            ..
        } => {
            let identity = serde_json::to_string(&(plugin, action, params, variables))
                .unwrap_or_else(|_| "unnameable".into());
            set.insert(format!("plugin\0{identity}"));
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
        // A variable is the profile's own memory and adds no capability (the
        // actions it steers do), with one exception: `agent_decision` reaches
        // outside the service, since `osh agent ask` answers an AI agent's
        // permission prompt with it. Any write of it is one item, whatever
        // the value: a key that answers agent prompts.
        Action::SetVariable { name, .. }
        | Action::ToggleVariable { name }
        | Action::AddToVariable { name, .. } => {
            if name == osh::AGENT_DECISION {
                set.insert(AGENT_ANSWER.into());
            }
        }
        Action::Conditional {
            when,
            then,
            otherwise,
        } => {
            // The test and both branches are the identity: a changed
            // condition or a swapped branch is a new item. Every step of
            // either branch could run, so each counts on its own too.
            match serde_json::to_string(&(when, then, otherwise)) {
                Ok(identity) => set.insert(format!("conditional\0{identity}")),
                Err(_) => set.insert("conditional\0unnameable".into()),
            };
            // Each plugin condition asks its plugin, which the confirmation
            // names like a plugin action: the plugin, the condition, every
            // parameter and which ones take variables are the item.
            let mut asked = Vec::new();
            when.plugin_uses(&mut asked);
            for found in asked {
                let identity =
                    serde_json::to_string(&(found.plugin, found.id, found.params, found.variables))
                        .unwrap_or_else(|_| "unnameable".into());
                set.insert(format!("plugin_condition\0{identity}"));
            }
            for step in then.iter().chain(otherwise) {
                visit(step, set);
            }
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
        // A macro is never a multi_action step, so its typed steps are the
        // whole identity, judged like a sequence's.
        Action::Macro { steps } => {
            match serde_json::to_string(steps) {
                Ok(steps) => set.insert(format!("macro\0{steps}")),
                Err(_) => set.insert("macro\0unnameable".into()),
            };
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
        Action::ObsStream { op, confirm } => {
            set.insert(format!("obs_stream\0{op:?}\0{confirm:?}"));
        }
        Action::ObsRecord { op } => {
            set.insert(format!("obs_record\0{op:?}"));
        }
        Action::ObsReplayBuffer { op } => {
            set.insert(format!("obs_replay\0{op:?}"));
        }
        Action::ObsStudioMode { op } => {
            set.insert(format!("obs_studio_mode\0{op:?}"));
        }
        Action::ObsScreenshot => {
            set.insert("obs_screenshot".into());
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
        Action::ObsInputMute { input, op } => {
            set.insert(format!("obs_input_mute\0{input}\0{op:?}"));
        }
        Action::ObsAdjustInputVolume { input, step_db } => {
            set.insert(format!("obs_adjust_input_volume\0{input}\0{step_db}"));
        }
        Action::ObsMediaSource { input, op } => {
            set.insert(format!("obs_media_source\0{input}\0{op:?}"));
        }
        Action::ObsSetInputVolume { input, percent } => {
            set.insert(format!("obs_set_input_volume\0{input}\0{percent}"));
        }
        Action::StreamlabsRecord => {
            set.insert("streamlabs_record".into());
        }
        Action::StreamlabsStream { confirm } => {
            set.insert(format!("streamlabs_stream\0{confirm:?}"));
        }
        Action::StreamlabsStudioMode => {
            set.insert("streamlabs_studio_mode".into());
        }
        Action::StreamlabsTransition => {
            set.insert("streamlabs_transition".into());
        }
        Action::StreamlabsScene { scene, collection } => {
            set.insert(format!("streamlabs_scene\0{scene}\0{collection:?}"));
        }
        Action::StreamlabsSourceVisibility { scene, source, op } => {
            set.insert(format!("streamlabs_visibility\0{scene}\0{source}\0{op:?}"));
        }
        // Like `obs_set_input_volume`, the level (or step, or mute mode) is
        // part of the item.
        Action::StreamlabsAudio { source, op } => {
            set.insert(format!("streamlabs_audio\0{source}\0{op:?}"));
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
            screen_brightness: None,
            variables: Vec::new(),
        }
    }

    fn text(text: &str, press_enter: bool) -> Action {
        Action::Text {
            text: text.into(),
            press_enter,
        }
    }

    #[test]
    fn a_conditional_adds_its_test_its_branches_and_every_step() {
        use cooltek_sdk_service::Condition;
        let conditional = |when: Condition, then: Vec<Action>, otherwise: Vec<Action>| {
            profile(vec![Action::Conditional {
                when,
                then,
                otherwise,
            }])
        };
        let held = conditional(
            Condition::Recording(true),
            vec![Action::ToggleMute],
            vec![text("hi", false)],
        );
        assert!(!adds_capabilities(&held, &held));
        // The same steps under another condition, or swapped between the
        // branches, are a new conditional.
        assert!(adds_capabilities(
            &held,
            &conditional(
                Condition::Streaming(true),
                vec![Action::ToggleMute],
                vec![text("hi", false)]
            )
        ));
        assert!(adds_capabilities(
            &held,
            &conditional(
                Condition::Recording(true),
                vec![text("hi", false)],
                vec![Action::ToggleMute]
            )
        ));
        // A step of a branch counts on its own: a new snippet is an addition.
        assert!(adds_capabilities(
            &held,
            &conditional(
                Condition::Recording(true),
                vec![Action::ToggleMute],
                vec![text("other", false)]
            )
        ));
        // Without a conditional, the held steps alone do not cover it.
        assert!(adds_capabilities(
            &profile(vec![Action::ToggleMute, text("hi", false)]),
            &held
        ));
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
            faces: Vec::new(),
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
    fn a_streamlabs_target_or_level_is_part_of_the_item() {
        use cooltek_sdk_service::StreamlabsAudioOp;
        let audio = |op| {
            profile(vec![Action::StreamlabsAudio {
                source: "Mic".into(),
                op,
            }])
        };
        let held = audio(StreamlabsAudioOp::Set(40));
        assert!(!adds_capabilities(&held, &held));
        assert!(adds_capabilities(&held, &audio(StreamlabsAudioOp::Set(90))));
        let scene = |collection: Option<&str>| {
            profile(vec![Action::StreamlabsScene {
                scene: "Live".into(),
                collection: collection.map(Into::into),
            }])
        };
        assert!(adds_capabilities(&scene(None), &scene(Some("Podcast"))));
        // The OBS action of the same name is a different capability.
        assert!(adds_capabilities(
            &profile(vec![Action::ObsTransition]),
            &profile(vec![Action::StreamlabsTransition])
        ));
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
    #[test]
    fn a_program_with_its_arguments_and_directory_is_the_whole_item() {
        let program = |path: &str, args: &[&str], directory: Option<&str>| Action::RunProgram {
            program: path.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            directory: directory.map(Into::into),
            variables: false,
        };
        let held = profile(vec![program("/usr/bin/tool", &["a b"], None)]);
        assert!(!adds_capabilities(&held, &held.clone()));
        for next in [
            program("/usr/bin/other", &["a b"], None),
            program("/usr/bin/tool", &["a", "b"], None),
            program("/usr/bin/tool", &["a b", "--force"], None),
            program("/usr/bin/tool", &[], None),
            program("/usr/bin/tool", &["a b"], Some("~")),
            // Filling in variables changes what can run.
            Action::RunProgram {
                program: "/usr/bin/tool".into(),
                args: vec!["a b".into()],
                directory: None,
                variables: true,
            },
        ] {
            assert!(adds_capabilities(&held, &profile(vec![next])));
        }
    }

    #[test]
    fn a_plugin_action_with_its_parameters_is_the_whole_item() {
        let plugin =
            |value: serde_json::Value| -> Action { serde_json::from_value(value).unwrap() };
        let held = profile(vec![plugin(serde_json::json!({
            "type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"set",
            "params":{"entity":"light.desk","level":40}
        }))]);
        assert!(!adds_capabilities(&held, &held.clone()));
        // A newer minimum version asks the same thing.
        assert!(!adds_capabilities(
            &held,
            &profile(vec![plugin(serde_json::json!({
                "type":"plugin","plugin":"com.example.lights","min_version":"1.4.0","action":"set",
                "params":{"entity":"light.desk","level":40}
            }))])
        ));
        for next in [
            serde_json::json!({"type":"plugin","plugin":"com.example.other","min_version":"1.0.0","action":"set","params":{"entity":"light.desk","level":40}}),
            serde_json::json!({"type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"off","params":{"entity":"light.desk","level":40}}),
            serde_json::json!({"type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"set","params":{"entity":"light.hall","level":40}}),
            serde_json::json!({"type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"set","params":{"entity":"light.desk","level":41}}),
            serde_json::json!({"type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"set","params":{"entity":"light.desk","level":40},"variables":["entity"]}),
        ] {
            assert!(adds_capabilities(&held, &profile(vec![plugin(next)])));
        }
    }

    #[test]
    fn a_plugin_condition_with_its_parameters_is_an_item_and_feeds_its_variables() {
        use cooltek_sdk_service::{Variable, VariableValue};
        let asking = |condition: serde_json::Value| -> Action {
            serde_json::from_value(serde_json::json!({
                "type":"conditional","when":{"any":[{"output_muted":true},{"plugin":condition}]},
                "then":[{"type":"toggle_mute"}]
            }))
            .unwrap()
        };
        let base = serde_json::json!({"plugin":"com.example.lights","min_version":"1.0.0",
            "condition":"is_on","params":{"entity":"light.{{room}}"},"variables":["entity"],"value":true});
        let held = profile(vec![asking(base.clone())]);
        assert!(!adds_capabilities(&held, &held.clone()));
        // The conditional's own item already changes with its test; the plugin condition is an item of its own,
        // so the same condition moved under another if / else is still known.
        assert!(
            items(&held)
                .iter()
                .any(|item| item.starts_with("plugin_condition\0"))
        );
        for change in [
            serde_json::json!({"plugin":"com.example.other"}),
            serde_json::json!({"condition":"is_off"}),
            serde_json::json!({"params":{"entity":"light.hall"},"variables":[]}),
            serde_json::json!({"value":false}),
        ] {
            let mut next = base.clone();
            for (field, value) in change.as_object().unwrap() {
                next[field] = value.clone();
            }
            assert!(
                adds_capabilities(&held, &profile(vec![asking(next)])),
                "{change}"
            );
        }
        // A variable it fills in is fed by the profile's own writes.
        let mut fed = profile(vec![asking(base)]);
        fed.variables = vec![Variable {
            name: "room".into(),
            value: VariableValue::Text("desk".into()),
        }];
        let mut changed = fed.clone();
        changed.variables[0].value = VariableValue::Text("hall".into());
        assert!(adds_capabilities(&fed, &changed));
    }

    #[test]
    fn what_the_profile_writes_into_a_filled_in_variable_is_part_of_the_item() {
        use cooltek_sdk_service::{Variable, VariableValue};
        let text = |value: &str| VariableValue::Text(value.into());
        let rm = Action::RunProgram {
            program: "/bin/rm".into(),
            args: vec!["{{target}}".into()],
            directory: None,
            variables: true,
        };
        let set = |value: &str| Action::SetVariable {
            name: "target".into(),
            value: text(value),
        };
        let with = |initial: &str, actions: Vec<Action>| {
            let mut profile = profile(actions);
            profile.variables = vec![
                Variable {
                    name: "target".into(),
                    value: text(initial),
                },
                Variable {
                    name: "other".into(),
                    value: text("x"),
                },
            ];
            profile
        };
        let held = with(
            "tmp/a",
            vec![
                rm.clone(),
                Action::MultiAction {
                    actions: vec![set("tmp/scratch")],
                },
            ],
        );
        assert!(!adds_capabilities(&held, &held.clone()));
        // Dropping a write adds nothing.
        assert!(!adds_capabilities(&held, &with("tmp/a", vec![rm.clone()])));
        // A new starting value, or a key that writes a new value, is new.
        for next in [
            with("/Users/me/Documents", vec![rm.clone()]),
            with("tmp/a", vec![rm.clone(), set("/Users/me/Documents")]),
            with(
                "tmp/a",
                vec![
                    rm.clone(),
                    Action::Conditional {
                        when: cooltek_sdk_service::Condition::Recording(true),
                        then: vec![set("/Users/me/Documents")],
                        otherwise: vec![],
                    },
                ],
            ),
        ] {
            assert!(adds_capabilities(&held, &next));
        }
        // A variable no program fills in stays the profile's own memory.
        let mut other = with(
            "tmp/a",
            vec![
                rm.clone(),
                Action::SetVariable {
                    name: "other".into(),
                    value: text("y"),
                },
            ],
        );
        other.variables[1].value = text("changed");
        assert!(!adds_capabilities(&held, &other));
    }

    #[test]
    fn a_terminal_command_line_is_the_whole_item() {
        use cooltek_sdk_service::TerminalApp;
        let terminal =
            |app: TerminalApp, command: &str, directory: Option<&str>| Action::TerminalCommand {
                app,
                command: command.into(),
                directory: directory.map(Into::into),
            };
        let held = profile(vec![terminal(TerminalApp::Terminal, "claude", None)]);
        assert!(!adds_capabilities(&held, &held.clone()));
        for next in [
            terminal(TerminalApp::Terminal, "claude --yolo", None),
            terminal(TerminalApp::Iterm2, "claude", None),
            terminal(TerminalApp::Terminal, "claude", Some("~/code")),
        ] {
            assert!(adds_capabilities(&held, &profile(vec![next])));
        }
    }

    #[test]
    fn a_key_that_answers_agent_prompts_is_an_addition() {
        use cooltek_sdk_service::{Condition, VariableValue};
        let decide = |value: &str| Action::SetVariable {
            name: osh::AGENT_DECISION.into(),
            value: VariableValue::Text(value.into()),
        };
        let held = profile(vec![Action::ToggleMute]);
        assert!(items(&profile(vec![decide("allow")])).contains(AGENT_ANSWER));
        assert!(adds_capabilities(&held, &profile(vec![decide("allow")])));
        // Inside a sequence or an if / else too, and through a toggle or an add.
        let nested = [
            Action::MultiAction {
                actions: vec![Action::ToggleMute, decide("allow")],
            },
            Action::Conditional {
                when: Condition::OutputMuted(true),
                then: vec![Action::ToggleMute],
                otherwise: vec![decide("deny")],
            },
            Action::ToggleVariable {
                name: osh::AGENT_DECISION.into(),
            },
            Action::AddToVariable {
                name: osh::AGENT_DECISION.into(),
                amount: 1,
            },
        ];
        for action in nested {
            assert!(
                items(&profile(vec![action.clone()])).contains(AGENT_ANSWER),
                "{action:?}"
            );
        }
        // Once held, another value adds nothing new; other variables never count.
        let answering = profile(vec![decide("allow")]);
        assert!(!adds_capabilities(
            &answering,
            &profile(vec![decide("deny")])
        ));
        let other = profile(vec![Action::SetVariable {
            name: "busy".into(),
            value: VariableValue::Bool(true),
        }]);
        assert!(!adds_capabilities(&held, &other));
    }
}
