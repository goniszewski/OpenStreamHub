use crate::{Action, DeviceContext, Gesture, Profile};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A rotation or long-press action belonging to one dial function.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialBinding {
    /// `right`, `left`, or `long_press`; a short press always switches functions.
    pub gesture: Gesture,
    /// Exact serial selector, or absent for any serial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// Complete host action; input-dependent fader templates are unsupported.
    pub action: Action,
}

/// One named group of M131E bindings selected by short-click cycling.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialFunction {
    /// Unique human-readable name of at most 64 bytes.
    pub name: String,
    /// Unique gesture/serial bindings active while this function is current.
    pub bindings: Vec<DialBinding>,
}

impl DialFunction {
    /// Maximum dial functions in one profile.
    pub const MAX_FUNCTIONS: usize = 16;
    /// Maximum function name length in bytes.
    pub const MAX_NAME_BYTES: usize = 64;
}

/// A completed short-click function switch; it never carries a host action.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DialFunctionNavigation {
    /// Function that was current before the short click.
    pub from_function: String,
    /// Function that is current after the short click.
    pub to_function: String,
    /// Zero-based index of the new current function.
    pub index: usize,
    /// Number of functions in the cycle.
    pub count: usize,
}

/// What the M131E dial screen shows while a profile runs, when the app drives
/// the display. Absent means the default: the active dial function's name, or
/// the calm analog clock for a profile with nothing on the dial.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialDisplay {
    /// The face the dial screen shows.
    pub content: DialDisplayContent,
    /// The custom face for [`DialDisplayContent::Image`]: a 240×240 baseline
    /// JPEG, base64-encoded, at most [`Self::MAX_IMAGE_BYTES`] decoded.
    /// Refused with any other content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The picture the custom face was prepared from and how it was fitted,
    /// kept so the fit, rotation and background can be changed after a
    /// reopen. Refused unless an image is present; the dial never reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<DialSource>,
}

/// How a source picture is fitted onto the round screen.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialFit {
    /// Keep the whole picture and fill the margins with the background.
    Contain,
    /// Crop a centred square so the picture fills the screen.
    Cover,
    /// Scale width and height independently.
    Stretch,
}

/// The picture a custom dial face was made from, with the look it was given.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DialSource {
    /// A PNG or JPEG, base64-encoded, at most [`DialSource::MAX_IMAGE_BYTES`] decoded.
    pub image: String,
    /// How the picture was fitted.
    pub fit: DialFit,
    /// Clockwise quarter turns, 0 through 3.
    pub quarter_turns: u8,
    /// Margin colour as red, green and blue.
    pub background: [u8; 3],
}

impl DialSource {
    /// Maximum decoded size of a kept source picture.
    pub const MAX_IMAGE_BYTES: usize = 256 * 1024;
}

impl DialDisplay {
    /// Maximum decoded size of a custom dial image (2× a key artwork frame).
    pub const MAX_IMAGE_BYTES: usize = 64 * 1024;
}

/// The face the M131E dial screen shows. The values match the
/// `masterhub-service --dial-content` spellings, with `-` for `_`
/// (`analog_clock` is `analog-clock`); a parity test in the app keeps the two
/// lists in step.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialDisplayContent {
    /// Nothing is drawn on the dial.
    Off,
    /// The active dial function's name, or the profile name without functions.
    #[default]
    Function,
    /// A digital `HH:MM` clock in local time.
    Clock,
    /// An analog clock in local time.
    AnalogClock,
    /// Whole-machine CPU usage in percent (macOS only; `--` elsewhere).
    Cpu,
    /// The picture stored in [`DialDisplay::image`].
    Image,
}

impl DialDisplayContent {
    /// Every value, in the `--dial-content` command-line order.
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::Function,
        Self::Clock,
        Self::AnalogClock,
        Self::Cpu,
        Self::Image,
    ];
}

/// Checks the profile's optional dial display setting.
pub(crate) fn validate_display(display: &DialDisplay) -> Result<(), String> {
    if let Some(source) = &display.source {
        if display.content != DialDisplayContent::Image || display.image.is_none() {
            return Err("dial_display: a source picture belongs to image content".into());
        }
        validate_source(source)?;
    }
    match (display.content, display.image.as_deref()) {
        (DialDisplayContent::Image, Some(image)) => validate_display_image(image),
        (DialDisplayContent::Image, None) => {
            Err("dial_display: image content requires the image field".into())
        }
        (_, Some(_)) => Err("dial_display: only image content carries an image".into()),
        (_, None) => Ok(()),
    }
}

fn validate_source(source: &DialSource) -> Result<(), String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    if source.quarter_turns > 3 {
        return Err("dial_display: source quarter_turns must be 0 through 3".into());
    }
    if source.image.len() > DialSource::MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(format!(
            "dial_display: the source picture exceeds {} KiB",
            DialSource::MAX_IMAGE_BYTES / 1024
        ));
    }
    let bytes = STANDARD
        .decode(&source.image)
        .map_err(|_| "dial_display: the source picture is not valid base64".to_string())?;
    if bytes.len() > DialSource::MAX_IMAGE_BYTES {
        return Err(format!(
            "dial_display: the source picture exceeds {} KiB",
            DialSource::MAX_IMAGE_BYTES / 1024
        ));
    }
    let png = bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    let jpeg = bytes.starts_with(&[0xff, 0xd8, 0xff]);
    if !png && !jpeg {
        return Err("dial_display: the source picture must be a PNG or JPEG".into());
    }
    Ok(())
}

/// Longest base64 text that can decode to [`DialDisplay::MAX_IMAGE_BYTES`]
/// bytes, checked before any decoding.
const MAX_IMAGE_BASE64_LEN: usize = DialDisplay::MAX_IMAGE_BYTES.div_ceil(3) * 4;

fn validate_display_image(image: &str) -> Result<(), String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    if image.is_empty() {
        return Err("dial_display: the image must not be empty".into());
    }
    if image.len() > MAX_IMAGE_BASE64_LEN {
        return Err(format!(
            "dial_display: the image exceeds {} KiB",
            DialDisplay::MAX_IMAGE_BYTES / 1024
        ));
    }
    let jpeg = STANDARD
        .decode(image)
        .map_err(|_| "dial_display: the image is not valid base64".to_string())?;
    if jpeg.len() > DialDisplay::MAX_IMAGE_BYTES {
        return Err(format!(
            "dial_display: the image exceeds {} KiB",
            DialDisplay::MAX_IMAGE_BYTES / 1024
        ));
    }
    cooltek_sdk_masterhub::dial::validate_jpeg(&jpeg)
        .map_err(|error| format!("dial_display: {error}"))
}

/// Checks the bounded, unambiguous dial function list of a profile.
pub(crate) fn validate(functions: &[DialFunction]) -> Result<(), String> {
    if functions.is_empty() || functions.len() > DialFunction::MAX_FUNCTIONS {
        return Err(format!(
            "dial_functions must contain 1 through {} functions",
            DialFunction::MAX_FUNCTIONS
        ));
    }
    let mut names = HashSet::new();
    for (index, function) in functions.iter().enumerate() {
        if function.name.trim().is_empty()
            || function.name.trim() != function.name
            || function.name.len() > DialFunction::MAX_NAME_BYTES
            || function
                .name
                .chars()
                .any(|c| c.is_control() || is_format_character(c))
        {
            return Err(format!(
                "dial function {index}: name must be nonempty, without leading or trailing whitespace, free of control and format characters, and at most {} bytes",
                DialFunction::MAX_NAME_BYTES
            ));
        }
        if !names.insert(function.name.as_str()) {
            return Err(format!(
                "dial function {index}: duplicate name {:?}",
                function.name
            ));
        }
        let mut selectors = HashSet::new();
        for binding in &function.bindings {
            match binding.gesture {
                Gesture::Right | Gesture::Left | Gesture::LongPress => {}
                Gesture::ShortPress => {
                    return Err(format!(
                        "dial function {index}: short_press is reserved for switching dial functions"
                    ));
                }
                _ => {
                    return Err(format!(
                        "dial function {index}: bindings require right, left, or long_press"
                    ));
                }
            }
            crate::profile::validate_serial_number(binding.serial_number.as_deref())
                .map_err(|error| format!("dial function {index}: {error}"))?;
            // A blank device serial is normalized to absent, so this could never match.
            if binding
                .serial_number
                .as_deref()
                .is_some_and(|serial| serial.trim().is_empty())
            {
                return Err(format!(
                    "dial function {index}: serial number must not be only whitespace"
                ));
            }
            if !selectors.insert((binding.gesture, binding.serial_number.as_deref())) {
                return Err(format!("dial function {index}: duplicate dial selector"));
            }
            binding
                .action
                .validate_executable()
                .map_err(|error| format!("dial function {index}: {error}"))?;
        }
    }
    Ok(())
}

/// Unicode General Category `Cf` (format) characters, which include the bidi
/// embedding, override, isolate, and mark controls plus invisible joiners.
pub(crate) fn is_format_character(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Counts bindings across every dial function.
pub(crate) fn binding_count(profile: &Profile) -> usize {
    profile.dial_functions.as_deref().map_or(0, |functions| {
        functions.iter().map(|f| f.bindings.len()).sum()
    })
}

/// Selects an exact serial binding before a wildcard, independent of order.
pub(crate) fn select_action(
    function: &DialFunction,
    device: &DeviceContext,
    gesture: Gesture,
) -> Option<Action> {
    let mut wildcard = None;
    for binding in function.bindings.iter().filter(|b| b.gesture == gesture) {
        match binding.serial_number.as_deref() {
            Some(serial) if device.serial_number.as_deref() == Some(serial) => {
                return Some(binding.action.clone());
            }
            None => wildcard = Some(binding.action.clone()),
            Some(_) => {}
        }
    }
    wildcard
}

#[cfg(test)]
mod tests {
    use crate::{
        Action, DeviceContext, DialFunction, Gesture, InputEvent, KeypadResolution, Profile,
        ProfileResolution, ProfileSession, ServiceModule,
    };
    use serde_json::{Value, json};

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/dial-functions.json");

    fn fixture() -> Profile {
        Profile::from_json(FIXTURE).unwrap()
    }
    fn dial(serial: Option<&str>) -> DeviceContext {
        DeviceContext {
            module: ServiceModule::M131E,
            serial_number: serial.map(str::to_owned),
        }
    }
    fn event(gesture: Gesture) -> InputEvent {
        InputEvent {
            timestamp_micros: 0,
            control: 0,
            gesture,
            value: None,
        }
    }
    fn click(session: &mut ProfileSession, device: &DeviceContext) -> Vec<ProfileResolution> {
        // InputSession emits pressed, released, then the classified gesture.
        [Gesture::Pressed, Gesture::Released, Gesture::ShortPress]
            .into_iter()
            .map(|gesture| session.route(device, &event(gesture)).unwrap())
            .collect()
    }
    fn rejects(value: &Value) -> String {
        Profile::from_json(&serde_json::to_vec(value).unwrap()).unwrap_err()
    }

    #[test]
    fn fixture_round_trips_and_existing_profiles_stay_json_compatible() {
        let profile = fixture();
        assert_eq!(profile.dial_functions.as_ref().unwrap().len(), 3);
        let encoded = serde_json::to_vec(&profile).unwrap();
        assert_eq!(Profile::from_json(&encoded).unwrap(), profile);
        for bytes in [
            include_bytes!("../../../examples/profiles/macos.json").as_slice(),
            include_bytes!("../../../examples/profiles/all-modules.json").as_slice(),
            include_bytes!("../../../examples/profiles/keypad-folders.json").as_slice(),
        ] {
            let original: Value = serde_json::from_slice(bytes).unwrap();
            let profile = Profile::from_json(bytes).unwrap();
            assert_eq!(profile.dial_functions, None);
            let reencoded = serde_json::to_value(&profile).unwrap();
            assert!(reencoded.get("dial_functions").is_none());
            assert_eq!(reencoded, original);
        }
        // Canonical compact encodings of profiles without functions are byte-identical.
        let compact: &[u8] = br#"{"schema_version":1,"name":"Desk","bindings":[{"module":"m131e","control":0,"gesture":"short_press","serial_number":"unit","action":{"type":"toggle_mute"}}]}"#;
        assert_eq!(
            serde_json::to_vec(&Profile::from_json(compact).unwrap()).unwrap(),
            compact
        );
        let v2 = Profile::from_json(compact).unwrap().into_v2().unwrap();
        let v2_bytes = serde_json::to_vec(&v2).unwrap();
        assert!(!v2_bytes.windows(14).any(|w| w == b"dial_functions"));
        assert_eq!(
            serde_json::to_vec(&Profile::from_json(&v2_bytes).unwrap()).unwrap(),
            v2_bytes
        );
        // Explicit null is accepted as absent and never written back.
        let mut value: Value = serde_json::from_slice(FIXTURE).unwrap();
        value["dial_functions"] = Value::Null;
        let profile = Profile::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(
            serde_json::to_value(&profile)
                .unwrap()
                .get("dial_functions")
                .is_none()
        );
    }

    #[test]
    fn dial_functions_are_valid_under_schema_1_and_survive_v2_upgrade() {
        let mut value: Value = serde_json::from_slice(FIXTURE).unwrap();
        value["schema_version"] = json!(1);
        value.as_object_mut().unwrap().remove("keypad");
        let flat = Profile::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        let upgraded = flat.clone().into_v2().unwrap();
        assert_eq!(upgraded.schema_version, 2);
        assert_eq!(upgraded.dial_functions, flat.dial_functions);
        assert_eq!(upgraded.binding_count(), flat.binding_count());
    }

    #[test]
    fn invalid_dial_function_configs_are_rejected() {
        let original: Value = serde_json::from_slice(FIXTURE).unwrap();
        type Mutation = Box<dyn Fn(&mut Value)>;
        let cases: Vec<(Mutation, &str)> = vec![
            (Box::new(|v| v["dial_functions"] = json!([])), "1 through"),
            (
                Box::new(|v| {
                    let one = v["dial_functions"][0].clone();
                    let many: Vec<Value> = (0..=DialFunction::MAX_FUNCTIONS)
                        .map(|i| {
                            let mut f = one.clone();
                            f["name"] = json!(format!("f{i}"));
                            f
                        })
                        .collect();
                    v["dial_functions"] = Value::Array(many);
                }),
                "1 through",
            ),
            (
                Box::new(|v| {
                    v["dial_functions"][1]["name"] = v["dial_functions"][0]["name"].clone()
                }),
                "duplicate name",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["name"] = json!("  ")),
                "name must",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["name"] = json!("a\nb")),
                "name must",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["name"] = json!("x".repeat(65))),
                "name must",
            ),
            (
                // leading space
                Box::new(|v| v["dial_functions"][0]["name"] = json!(" Volume")),
                "name must",
            ),
            (
                // trailing space
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Volume ")),
                "name must",
            ),
            (
                // bidi override
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Vol\u{202E}ume")),
                "name must",
            ),
            (
                // bidi isolate
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Vol\u{2066}ume")),
                "name must",
            ),
            (
                // zero width space
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Vol\u{200B}ume")),
                "name must",
            ),
            (
                // byte order mark
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Vol\u{FEFF}ume")),
                "name must",
            ),
            (
                // soft hyphen
                Box::new(|v| v["dial_functions"][0]["name"] = json!("Vol\u{00AD}ume")),
                "name must",
            ),
            (
                Box::new(|v| {
                    v["dial_functions"][0]["bindings"][0]["gesture"] = json!("short_press")
                }),
                "reserved for switching",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["gesture"] = json!("pressed")),
                "right, left, or long_press",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["gesture"] = json!("released")),
                "right, left, or long_press",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["gesture"] = json!("clockwise")),
                "right, left, or long_press",
            ),
            (
                Box::new(|v| {
                    let first = v["dial_functions"][0]["bindings"][0].clone();
                    v["dial_functions"][0]["bindings"]
                        .as_array_mut()
                        .unwrap()
                        .push(first);
                }),
                "duplicate dial selector",
            ),
            (
                Box::new(|v| {
                    v["dial_functions"][0]["bindings"][0]["action"] =
                        json!({"type":"volume_from_position"})
                }),
                "resolved",
            ),
            (
                Box::new(|v| {
                    v["dial_functions"][0]["bindings"][0]["action"] =
                        json!({"type":"volume","delta":0})
                }),
                "volume delta",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["serial_number"] = json!("")),
                "serial number",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["serial_number"] = json!("  ")),
                "only whitespace",
            ),
            (
                // A short-press binding outside the cycle would make a short click ambiguous.
                Box::new(|v| {
                    v["bindings"].as_array_mut().unwrap().push(json!({
                        "module":"m131e","control":0,"gesture":"short_press",
                        "action":{"type":"toggle_mute"}
                    }))
                }),
                "belong in dial_functions",
            ),
            (
                Box::new(|v| {
                    v["bindings"].as_array_mut().unwrap().push(json!({
                        "module":"m131e","control":0,"gesture":"right",
                        "action":{"type":"volume","delta":1}
                    }))
                }),
                "belong in dial_functions",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["extra"] = json!(true)),
                "unknown field",
            ),
            (
                Box::new(|v| v["dial_functions"][0]["bindings"][0]["extra"] = json!(true)),
                "unknown field",
            ),
        ];
        for (index, (mutate, expected)) in cases.iter().enumerate() {
            let mut value = original.clone();
            mutate(&mut value);
            let error = rejects(&value);
            assert!(error.contains(expected), "case {index}: {error}");
        }
    }

    #[test]
    fn dial_bindings_count_toward_the_profile_limit() {
        let mut profile = fixture();
        assert_eq!(profile.binding_count(), 1 + 1 + 4 + 2 + 1);
        let filler = profile.dial_functions.as_ref().unwrap()[0].bindings[0].clone();
        let mut serial = 0;
        while profile.binding_count() <= Profile::MAX_BINDINGS {
            let mut binding = filler.clone();
            binding.serial_number = Some(format!("unit-{serial}"));
            serial += 1;
            profile.dial_functions.as_mut().unwrap()[0]
                .bindings
                .push(binding);
        }
        assert!(profile.validate().unwrap_err().contains("exceeds"));
        profile.dial_functions.as_mut().unwrap()[0].bindings.pop();
        assert_eq!(profile.binding_count(), Profile::MAX_BINDINGS);
        profile.validate().unwrap();
        assert!(ProfileSession::new(profile).is_ok());
    }

    #[test]
    fn function_names_allow_inner_spaces_and_non_ascii_letters() {
        let mut profile = fixture();
        profile.dial_functions.as_mut().unwrap()[0].name = "Głośność Mix".into();
        profile.validate().unwrap();
        // Flat profile names keep their existing, looser rule.
        profile.name = " Dial functions ".into();
        profile.validate().unwrap();
    }

    #[test]
    fn empty_function_is_accepted_and_resolves_nothing() {
        let mut profile = fixture();
        profile.dial_functions.as_mut().unwrap()[1].bindings.clear();
        let mut session = ProfileSession::new(profile).unwrap();
        session.set_dial_function(1).unwrap();
        for gesture in [Gesture::Right, Gesture::Left, Gesture::LongPress] {
            assert_eq!(
                session.route(&dial(None), &event(gesture)).unwrap(),
                ProfileResolution::Unbound
            );
        }
    }

    #[test]
    fn single_function_short_click_wraps_to_itself_without_dispatch() {
        let mut profile = fixture();
        profile.dial_functions.as_mut().unwrap().truncate(1);
        let mut session = ProfileSession::new(profile).unwrap();
        for _ in 0..2 {
            let outcomes = click(&mut session, &dial(None));
            assert_eq!(
                outcomes[..2],
                [ProfileResolution::Unbound, ProfileResolution::Unbound]
            );
            assert_eq!(
                outcomes[2],
                ProfileResolution::DialFunction {
                    navigation: crate::DialFunctionNavigation {
                        from_function: "Volume".into(),
                        to_function: "Volume".into(),
                        index: 0,
                        count: 1,
                    }
                }
            );
            assert!(
                outcomes
                    .into_iter()
                    .all(|o| !matches!(KeypadResolution::from(o), KeypadResolution::Action { .. }))
            );
            assert_eq!(session.dial_function_index(), Some(0));
        }
    }

    #[test]
    fn short_click_cycles_with_wraparound_and_never_dispatches() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        assert_eq!(session.dial_function_index(), Some(0));
        let names = ["Volume", "Scroll", "Apps"];
        for step in 1..=4 {
            let outcomes = click(&mut session, &dial(None));
            assert_eq!(outcomes[0], ProfileResolution::Unbound);
            assert_eq!(outcomes[1], ProfileResolution::Unbound);
            let ProfileResolution::DialFunction { navigation } = &outcomes[2] else {
                panic!("short press must switch functions: {:?}", outcomes[2]);
            };
            assert_eq!(navigation.from_function, names[(step - 1) % 3]);
            assert_eq!(navigation.to_function, names[step % 3]);
            assert_eq!(navigation.index, step % 3);
            assert_eq!(navigation.count, 3);
            assert_eq!(
                session.current_dial_function().unwrap().name,
                names[step % 3]
            );
            // The compatibility view used by existing callers dispatches nothing.
            for outcome in outcomes {
                assert!(!matches!(
                    KeypadResolution::from(outcome),
                    KeypadResolution::Action { .. }
                ));
            }
        }
        let navigation = serde_json::to_value(click(&mut session, &dial(None)).pop()).unwrap();
        assert_eq!(navigation["type"], "dial_function");
        assert_eq!(navigation["navigation"]["from_function"], "Scroll");
        assert_eq!(navigation["navigation"]["to_function"], "Apps");
    }

    #[test]
    fn rotation_and_long_press_use_the_current_function() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        let device = dial(None);
        assert_eq!(
            session.route(&device, &event(Gesture::Right)).unwrap(),
            ProfileResolution::Action {
                action: Action::Volume { delta: 5 },
                page_id: None,
                dial_function: Some("Volume".into()),
            }
        );
        assert_eq!(
            session.route(&device, &event(Gesture::LongPress)).unwrap(),
            ProfileResolution::Action {
                action: Action::ToggleMute,
                page_id: None,
                dial_function: Some("Volume".into()),
            }
        );
        click(&mut session, &device);
        assert_eq!(
            session.route(&device, &event(Gesture::Left)).unwrap(),
            ProfileResolution::Action {
                action: Action::Shortcut {
                    key: "k".into(),
                    modifiers: vec![],
                },
                page_id: None,
                dial_function: Some("Scroll".into()),
            }
        );
        // Scroll has no long-press binding; nothing falls through to another function.
        assert_eq!(
            session.route(&device, &event(Gesture::LongPress)).unwrap(),
            ProfileResolution::Unbound
        );
        click(&mut session, &device);
        assert_eq!(
            session
                .resolve(&device, &event(Gesture::LongPress))
                .unwrap(),
            KeypadResolution::Action {
                action: Action::LaunchApp {
                    bundle_id: "com.apple.finder".into()
                },
                page_id: None,
            }
        );
        // A long press does not advance the cycle.
        assert_eq!(session.current_dial_function().unwrap().name, "Apps");
    }

    #[test]
    fn serial_specific_bindings_win_inside_a_function() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        for (serial, delta) in [(Some("desk-dial"), 1), (Some("other"), 5), (None, 5)] {
            assert!(matches!(
                session.route(&dial(serial), &event(Gesture::Right)).unwrap(),
                ProfileResolution::Action { action: Action::Volume { delta: d }, .. } if d == delta
            ));
        }
    }

    #[test]
    fn reset_and_new_session_return_to_the_first_function() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        click(&mut session, &dial(None));
        click(&mut session, &dial(None));
        assert_eq!(session.dial_function_index(), Some(2));
        session.reset();
        assert_eq!(session.dial_function_index(), Some(0));
        click(&mut session, &dial(None));
        assert!(session.set_dial_function(3).is_err());
        assert_eq!(session.dial_function_index(), Some(1));
        session.set_dial_function(2).unwrap();
        assert_eq!(session.current_dial_function().unwrap().name, "Apps");
        let fresh = ProfileSession::new(session.profile().clone()).unwrap();
        assert_eq!(fresh.dial_function_index(), Some(0));
    }

    #[test]
    fn other_modules_and_keypad_routing_are_unaffected() {
        let mut session = ProfileSession::new(fixture()).unwrap();
        click(&mut session, &dial(None));
        let knob = DeviceContext {
            module: ServiceModule::M003K,
            serial_number: None,
        };
        let turn = InputEvent {
            gesture: Gesture::Clockwise,
            ..event(Gesture::Clockwise)
        };
        assert_eq!(
            session.route(&knob, &turn).unwrap(),
            ProfileResolution::Action {
                action: Action::Volume { delta: 2 },
                page_id: None,
                dial_function: None,
            }
        );
        let keypad = DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        };
        let press = InputEvent {
            gesture: Gesture::Pressed,
            ..event(Gesture::Pressed)
        };
        assert_eq!(
            session.route(&keypad, &press).unwrap(),
            ProfileResolution::Action {
                action: Action::ToggleMute,
                page_id: Some("root".into()),
                dial_function: None,
            }
        );
        assert_eq!(session.current_dial_function().unwrap().name, "Scroll");
        // Flat binding lookup never exposes dial function bindings.
        assert!(
            session
                .binding_for(&dial(None), &event(Gesture::Right))
                .is_none()
        );
        assert!(session.binding_for(&knob, &turn).is_some());
    }

    #[test]
    fn profiles_without_dial_functions_keep_flat_dial_behavior() {
        let profile =
            Profile::from_json(include_bytes!("../../../examples/profiles/macos.json")).unwrap();
        let mut session = ProfileSession::new(profile).unwrap();
        assert_eq!(session.dial_function_index(), None);
        assert!(session.current_dial_function().is_none());
        assert!(session.set_dial_function(0).is_err());
        assert!(matches!(
            session
                .route(&dial(None), &event(Gesture::ShortPress))
                .unwrap(),
            ProfileResolution::Action {
                dial_function: None,
                ..
            }
        ));
    }

    #[test]
    fn stateless_engine_refuses_dial_function_profiles() {
        let flat = Profile {
            schema_version: 1,
            keypad: None,
            ..fixture()
        };
        flat.validate().unwrap();
        let error = crate::BindingEngine::new(flat.clone()).unwrap_err();
        assert!(error.contains("ProfileSession"), "{error}");
        assert!(ProfileSession::new(flat).is_ok());
    }
}
