//! Synthetic `.mhz` fixtures only: every archive is built here, in memory.
//! The serials are made up and follow no real device's numbering.

use super::mapping::Outcome;
use super::*;
use cooltek_sdk_service::{Action, Gesture, KeypadTile, Modifier, ServiceModule};
use flate2::{Compression, write::DeflateEncoder};
use serde_json::{Value, json};
use std::io::Write;

pub const BASE_SERIAL: &str = "MHBSFAKE0001";
pub const KEYPAD_SERIAL: &str = "MHKDFAKE0002";
const KNOB_SERIAL: &str = "MHKBFAKE0003";
const DIAL_SERIAL: &str = "MHDDFAKE0004";
const ROLLER_SERIAL: &str = "MHRRFAKE0005";
const FADER_SERIAL: &str = "MHFDFAKE0006";
const OTHER_SERIAL: &str = "MHBSFAKE0099";

#[derive(Clone)]
pub struct Item {
    name: Vec<u8>,
    data: Vec<u8>,
    deflate: bool,
    flags: u16,
    made_by: u16,
    external: u32,
    extra: Vec<u8>,
    method: Option<u16>,
    declared_size: Option<u32>,
    crc: Option<u32>,
    /// Point the directory at this offset instead of writing a local header.
    offset: Option<u32>,
}

pub fn item(name: &str, data: &[u8]) -> Item {
    Item {
        name: name.as_bytes().to_vec(),
        data: data.to_vec(),
        deflate: true,
        flags: 0,
        made_by: 20,
        external: 0,
        extra: Vec::new(),
        method: None,
        declared_size: None,
        crc: None,
        offset: None,
    }
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

fn local(item: &Item, compressed: &[u8], method: u16, crc: u32, size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&item.flags.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&(item.name.len() as u16).to_le_bytes());
    out.extend_from_slice(&(item.extra.len() as u16).to_le_bytes());
    out.extend_from_slice(&item.name);
    out.extend_from_slice(&item.extra);
    out.extend_from_slice(compressed);
    out
}

pub fn zip(items: &[Item]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for item in items {
        let compressed = if item.deflate {
            deflate(&item.data)
        } else {
            item.data.clone()
        };
        let method = item.method.unwrap_or(if item.deflate { 8 } else { 0 });
        let crc = item.crc.unwrap_or_else(|| crc32fast::hash(&item.data));
        let size = item.declared_size.unwrap_or(item.data.len() as u32);
        let offset = match item.offset {
            Some(offset) => offset,
            None => {
                let offset = out.len() as u32;
                out.extend(local(item, &compressed, method, crc, size));
                offset
            }
        };
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&item.made_by.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&item.flags.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0; 4]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(item.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&(item.extra.len() as u16).to_le_bytes());
        // Comment length, start disk, internal attributes.
        central.extend_from_slice(&[0; 6]);
        central.extend_from_slice(&item.external.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(&item.name);
        central.extend_from_slice(&item.extra);
    }
    let directory_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(items.len() as u16).to_le_bytes());
    out.extend_from_slice(&(items.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

pub fn mhz(profile: &Value) -> Vec<u8> {
    zip(&[item(
        &format!("profile2_{BASE_SERIAL}"),
        &serde_json::to_vec(profile).unwrap(),
    )])
}

pub fn tile(id: usize, func: &str, extra: Value) -> Value {
    let mut tile = json!({
        "id": id, "btnFunc": func, "title": "", "volumeAction": "volume-up", "direction": "up",
        "hotkey": {"key": [], "code": []}, "hotKeySettingMode": "hotkey",
        "btnColor": {"r": 1, "g": 2, "b": 3}
    });
    for (key, value) in extra.as_object().unwrap() {
        tile[key] = value.clone();
    }
    tile
}

pub fn page(tiles: Vec<Value>) -> Vec<Value> {
    let mut page: Vec<Value> = (0..15).map(|id| tile(id, "", json!({}))).collect();
    for tile in tiles {
        let id = tile["id"].as_u64().unwrap() as usize;
        page[id] = tile;
    }
    page
}

fn hotkey(keys: &[&str]) -> Value {
    json!({"key": keys, "code": [17, 67]})
}

fn profile_with(pages: Vec<Vec<Value>>) -> Value {
    json!({
        "serialNumber": BASE_SERIAL,
        "profileId": 2,
        "profileName": "Desk",
        "keypad": { KEYPAD_SERIAL: pages },
    })
}

/// Covers every mapped and every unbound action type.
pub fn full_profile() -> Value {
    let root = page(vec![
        tile(
            0,
            "hot-key",
            json!({"title": "Copy", "hotkey": hotkey(&["Control", "C"])}),
        ),
        tile(
            1,
            "hot-key",
            json!({"hotKeySettingMode": "macro", "hotkeyMacro": {"keys": [{"action": "Press", "name": "A"}]}}),
        ),
        tile(2, "web-url", json!({"url": "https://example.com/path?q=1"})),
        tile(
            3,
            "text",
            json!({"content": "hello", "imageIcon": "QUJD", "imageBuffer": {"type": "Buffer", "data": [255, 216, 255]}}),
        ),
        tile(
            4,
            "text",
            json!({"content": "ls -la\n", "afterEnter": true}),
        ),
        tile(
            5,
            "multi-tasking",
            json!({"actionList": [
                {"action": "hot-key", "hotkey": hotkey(&["Win", "X"])},
                {"action": "hot-key", "hotkey": hotkey(&["U"])},
                {"action": "hot-key", "hotkey": hotkey(&["Enter"])}
            ]}),
        ),
        tile(
            6,
            "multi-tasking",
            json!({"actionList": [
                {"action": "hot-key", "hotkey": hotkey(&["Win", "R"])},
                {"action": "app-files", "appFilePath": "C:\\Users\\someone\\tool.exe"}
            ]}),
        ),
        tile(7, "profile", json!({"dropdownOption": 0, "profileId": 7})),
        tile(
            8,
            "volume-control",
            json!({"dropdownOption": 1, "volumeAction": "volume-down"}),
        ),
        tile(9, "media-track-control", json!({"dropdownOption": 3})),
        tile(10, "undo-redo", json!({"dropdownOption": 1})),
        tile(11, "screen-capture", json!({})),
        tile(
            12,
            "app-files",
            json!({"actionAppMode": "Open", "appFilePath": "C:\\Users\\someone\\evil tool.exe"}),
        ),
        tile(
            13,
            "new-folder",
            json!({"title": "Tools", "folder": [
                {"id": 0, "btnFunc": ""},
                {"id": 1, "btnFunc": "profile", "dropdownOption": 1},
                {"id": 2, "btnFunc": "timer", "title": "Timer"},
                {"id": 3, "btnFunc": "new-folder", "folder": [{"id": 1, "btnFunc": "text", "content": "x"}]}
            ]}),
        ),
        tile(14, "next-page", json!({})),
    ]);
    let second = page(vec![
        tile(0, "text", json!({"content": "x"})),
        tile(
            1,
            "app-files",
            json!({"actionAppMode": "Close", "appFilePath": "/Applications/Thing.app"}),
        ),
        tile(
            2,
            "play-pause",
            json!({"appFilePath": "C:\\music\\song.mp3"}),
        ),
        tile(3, "previous-page", json!({})),
        tile(4, "lighting-effect", json!({"dropdownOption": 3})),
        tile(5, "record-start-stop", json!({})),
        tile(6, "chat-message", json!({"content": "hello chat"})),
        tile(7, "discord-mute", json!({})),
        tile(8, "streamlabs-record", json!({})),
        tile(9, "ctrl-scenes", json!({})),
        tile(10, "clock", json!({})),
        tile(11, "ps-flatten-image", json!({})),
        tile(12, "do-evil; rm -rf ~", json!({})),
        tile(13, "timer", json!({})),
        tile(14, "ps-brush", json!({"hotkey": hotkey(&["B"])})),
    ]);
    json!({
        "serialNumber": BASE_SERIAL,
        "profileId": 2,
        "profileName": "  Desk\u{202E} setup\u{0007} ",
        "keypad": {
            "CT99999999999": [page(vec![])],
            KEYPAD_SERIAL: [root, second],
        },
        "keypadBtnConfig": { KEYPAD_SERIAL: {"currentPage": 0, "totalPage": 2} },
        "knobsList": { KNOB_SERIAL: {
            "rotateR": {"id": 0, "btnFunc": "volume-control", "right": {"volumeAction": "volume-up"}, "left": {"volumeAction": "volume-down"}, "functionList": [{}, {}]},
            "rotateG": {"id": 1, "btnFunc": "ps-brush-size", "right": {"hotkey": hotkey(&["]"])}, "left": {"hotkey": hotkey(&["Control", "Z"])}},
            "rotateB": {"id": 2, "btnFunc": "brightness", "right": {}, "left": {}}
        }},
        "dialEncoder": { DIAL_SERIAL: {
            "lcdDisplayMode": "Clock",
            "frames": [{"type": "Buffer", "data": [1, 2]}],
            "gifPath": "C:\\Users\\someone\\AppData\\Roaming\\MasterHub\\uploadImage/1.gif",
            "functionList": [
                {"btnFunc": "volume-control", "btnName": "Volume", "right": {}, "left": {}},
                {"btnFunc": "hot-key", "btnName": "Volume", "right": {"hotkey": hotkey(&["Control", "Right"])}, "left": {"hotkey": hotkey(&["Control", "Left"])}},
                {"btnFunc": "obs-profile", "btnName": DIAL_SERIAL, "right": {}, "left": {}}
            ]
        }},
        "rollerBlock": { ROLLER_SERIAL: {
            "left": {"btnFunc": "volume-control", "up": {}, "down": {}},
            "right": {"btnFunc": "scroll-up-down", "up": {}, "down": {}}
        }},
        "sliders5List": { FADER_SERIAL: {
            "0": {"btnFunc": "volume-control"},
            "1": {"btnFunc": "volume-control", "volumeControlProcessId": 4242},
            "2": {"btnFunc": ""},
            "3": {"btnFunc": "streamlabs-audio-mixer"}
        }},
        "deviceLightingEffect": {"m4315": { KEYPAD_SERIAL: {"effect": 1, "color": "#ffffff"}, "undefined": {}}},
        "devices": {"basex64": [{"serialNumber": OTHER_SERIAL, "parentSerialNumber": "", "moduleName": "base"}]},
        "touch46s": {}, "touch50s": {},
        "filtered": true,
        "colorPickerFavoriteColors": []
    })
}

fn convert_one(profile: &Value) -> Converted {
    let mut converted = convert(&mhz(profile)).unwrap();
    assert_eq!(converted.len(), 1);
    converted.remove(0)
}

fn all_actions(profile: &Profile) -> Vec<Action> {
    fn push(action: &Action, out: &mut Vec<Action>) {
        out.push(action.clone());
        if let Action::MultiAction { actions } = action {
            actions.iter().for_each(|step| push(step, out));
        }
    }
    let mut out = Vec::new();
    profile
        .bindings
        .iter()
        .for_each(|b| push(&b.action, &mut out));
    for page in &profile.keypad.as_ref().unwrap().pages {
        for tile in page.slots.iter().flatten() {
            if let KeypadTile::Action { bindings, .. } = tile {
                bindings.iter().for_each(|b| push(&b.action, &mut out));
            }
        }
    }
    for function in profile.dial_functions.iter().flatten() {
        function
            .bindings
            .iter()
            .for_each(|b| push(&b.action, &mut out));
    }
    out
}

fn slot<'a>(profile: &'a Profile, page: &str, index: usize) -> &'a Option<KeypadTile> {
    &profile.keypad.as_ref().unwrap().page(page).unwrap().slots[index]
}

fn bound(profile: &Profile, page: &str, index: usize) -> Option<Action> {
    match slot(profile, page, index) {
        Some(KeypadTile::Action { bindings, .. }) => {
            assert!(bindings.len() <= 1);
            bindings.first().map(|b| {
                assert_eq!(b.gesture, Gesture::Pressed);
                assert_eq!(b.serial_number, None);
                b.action.clone()
            })
        }
        _ => None,
    }
}

fn row<'a>(report: &'a ImportReport, path: &str) -> &'a mapping::ReportRow {
    report
        .rows
        .iter()
        .find(|row| row.path == path)
        .unwrap_or_else(|| panic!("no row {path}: {:#?}", report.rows))
}

#[test]
fn every_mapped_action_type_is_converted_and_every_unbound_type_is_reported() {
    let Converted { profile, report } = convert_one(&full_profile());
    profile.validate().unwrap();
    assert_eq!(profile.schema_version, Profile::CURRENT_SCHEMA);
    assert_eq!(profile.name, "Desk setup");
    assert_eq!(report.source_name, "Desk setup");

    let key = |modifiers: Vec<Modifier>, key: &str| Action::Shortcut {
        key: key.into(),
        modifiers,
    };
    assert_eq!(
        bound(&profile, "root", 0),
        Some(key(vec![Modifier::Control], "c"))
    );
    assert_eq!(bound(&profile, "root", 1), None);
    assert_eq!(
        bound(&profile, "root", 2),
        Some(Action::OpenUrl {
            url: "https://example.com/path?q=1".into()
        })
    );
    assert_eq!(
        bound(&profile, "root", 3),
        Some(Action::Text {
            text: "hello".into(),
            press_enter: false
        })
    );
    assert_eq!(
        bound(&profile, "root", 4),
        Some(Action::Text {
            text: "ls -la\n".into(),
            press_enter: true
        })
    );
    assert_eq!(
        bound(&profile, "root", 5),
        Some(Action::MultiAction {
            actions: vec![
                key(vec![Modifier::Command], "x"),
                key(vec![], "u"),
                key(vec![], "return")
            ]
        })
    );
    assert_eq!(bound(&profile, "root", 6), None);
    assert_eq!(
        bound(&profile, "root", 7),
        Some(Action::Profile {
            direction: cooltek_sdk_service::ProfileDirection::Next
        })
    );
    assert_eq!(
        bound(&profile, "root", 8),
        Some(Action::Volume { delta: -5 })
    );
    assert_eq!(
        bound(&profile, "root", 9),
        Some(Action::Media {
            command: cooltek_sdk_service::MediaCommand::NextTrack
        })
    );
    assert_eq!(
        bound(&profile, "root", 10),
        Some(Action::System {
            command: cooltek_sdk_service::SystemCommand::Redo
        })
    );
    assert_eq!(
        bound(&profile, "root", 11),
        Some(Action::System {
            command: cooltek_sdk_service::SystemCommand::Screenshot
        })
    );
    assert_eq!(bound(&profile, "root", 12), None);
    // Folder and chained page.
    assert_eq!(
        slot(&profile, "root", 13),
        &Some(KeypadTile::Folder {
            page_id: "root_key14".into()
        })
    );
    let folder = profile.keypad.as_ref().unwrap().page("root_key14").unwrap();
    assert_eq!(folder.name, "Tools");
    assert_eq!(folder.slots[0], None);
    assert_eq!(
        bound(&profile, "root_key14", 1),
        Some(Action::Profile {
            direction: cooltek_sdk_service::ProfileDirection::Previous
        })
    );
    assert!(matches!(
        slot(&profile, "root_key14", 2),
        Some(KeypadTile::Action { label, bindings }) if label == "Timer" && bindings.is_empty()
    ));
    assert_eq!(
        slot(&profile, "root", 14),
        &Some(KeypadTile::Folder {
            page_id: "page2".into()
        })
    );
    let second = profile.keypad.as_ref().unwrap().page("page2").unwrap();
    assert_eq!(second.slots[0], None);
    assert_eq!(bound(&profile, "page2", 14), Some(key(vec![], "b")));
    for index in 1..14 {
        assert_eq!(bound(&profile, "page2", index), None, "page2 key {index}");
    }
    // The empty placeholder keypad was skipped for the one with tiles.
    assert_eq!(profile.keypad.as_ref().unwrap().pages.len(), 3);

    // Knobs, dial, rollers and faders.
    let flat: Vec<_> = profile
        .bindings
        .iter()
        .map(|b| (b.module, b.control, b.gesture, b.action.clone()))
        .collect();
    assert_eq!(
        flat,
        vec![
            (
                ServiceModule::M003K,
                0,
                Gesture::Clockwise,
                Action::Volume { delta: 5 }
            ),
            (
                ServiceModule::M003K,
                0,
                Gesture::CounterClockwise,
                Action::Volume { delta: -5 }
            ),
            (
                ServiceModule::M003K,
                1,
                Gesture::CounterClockwise,
                key(vec![Modifier::Control], "z")
            ),
            (
                ServiceModule::M032LE,
                0,
                Gesture::Up,
                Action::Volume { delta: 5 }
            ),
            (
                ServiceModule::M032LE,
                0,
                Gesture::Down,
                Action::Volume { delta: -5 }
            ),
            (
                ServiceModule::M005S,
                0,
                Gesture::PositionChanged,
                Action::VolumeFromPosition
            ),
        ]
    );
    let functions = profile.dial_functions.as_ref().unwrap();
    let names: Vec<_> = functions.iter().map(|f| f.name.as_str()).collect();
    // A duplicate name is made unique; a serial in a name is replaced.
    assert_eq!(names, ["Volume", "Volume 2", "Function 3"]);
    assert_eq!(functions[0].bindings.len(), 2);
    assert_eq!(
        functions[1].bindings[0].action,
        key(vec![Modifier::Control], "arrow_right")
    );
    assert!(functions[2].bindings.is_empty());

    // Never a launch_app, never a serial selector.
    let actions = all_actions(&profile);
    assert!(!actions.is_empty());
    assert!(
        actions
            .iter()
            .all(|a| !matches!(a, Action::LaunchApp { .. }))
    );
    assert!(profile.bindings.iter().all(|b| b.serial_number.is_none()));

    // Report rows.
    assert_eq!(
        row(&report, "Keypad · page 1 · key 1").outcome,
        Outcome::ImportedLossy
    );
    assert_eq!(
        row(&report, "Keypad · page 1 · key 2").outcome,
        Outcome::UnboundUnsupported
    );
    let url = row(&report, "Keypad · page 1 · key 3");
    assert!(url.high_risk);
    assert_eq!(url.detail.as_deref(), Some("https://example.com"));
    assert_eq!(
        row(&report, "Keypad · page 1 · key 4").outcome,
        Outcome::Imported
    );
    let enter = row(&report, "Keypad · page 1 · key 5");
    assert!(enter.high_risk && enter.detail.as_deref() == Some("<7 chars>"));
    let multi = row(&report, "Keypad · page 1 · key 7");
    assert_eq!(multi.outcome, Outcome::UnboundDangerous);
    assert!(multi.reason.starts_with("step 2:"), "{}", multi.reason);
    let launch = row(&report, "Keypad · page 1 · key 13");
    assert_eq!(launch.outcome, Outcome::UnboundDangerous);
    assert_eq!(launch.detail.as_deref(), Some("evil tool.exe"));
    assert!(row(&report, "Keypad · page 1 · key 12").high_risk);
    assert_eq!(
        row(&report, "Keypad · page 1 · key 14 folder · key 4").outcome,
        Outcome::UnboundUnsupported
    );
    assert_eq!(
        row(&report, "Keypad · page 2 · key 1").reason,
        "key 1 of a folder page is reserved for Back"
    );
    let close = row(&report, "Keypad · page 2 · key 2");
    assert_eq!(
        (close.outcome, close.detail.as_deref()),
        (Outcome::UnboundUnsupported, Some("Thing.app"))
    );
    assert!(
        row(&report, "Keypad · page 1 · key 10")
            .reason
            .contains("macOS media key")
    );
    let stop = convert_one(&profile_with(vec![vec![tile(
        0,
        "media-track-control",
        json!({"dropdownOption": 5}),
    )]]));
    assert_eq!(
        bound(&stop.profile, "root", 0),
        Some(Action::Media {
            command: cooltek_sdk_service::MediaCommand::Stop
        })
    );
    assert!(
        row(&stop.report, "Keypad · page 1 · key 1")
            .reason
            .contains("Apple Music or pauses Spotify")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 7")
            .reason
            .contains("Twitch")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 8")
            .reason
            .contains("Discord")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 9")
            .reason
            .contains("Streamlabs")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 10")
            .reason
            .contains("Master CTRL")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 6")
            .reason
            .contains("OBS")
    );
    assert!(
        row(&report, "Keypad · page 2 · key 12")
            .reason
            .contains("script")
    );
    assert_eq!(
        row(&report, "Keypad · page 2 · key 13").official_action,
        "unknown"
    );
    assert_eq!(
        row(&report, "Knob 2 · clockwise").outcome,
        Outcome::UnboundUnsupported
    );
    assert_eq!(
        row(&report, "Knob 3 · clockwise").outcome,
        Outcome::UnboundUnsupported
    );
    assert_eq!(
        row(&report, "Right roller · up").outcome,
        Outcome::UnboundUnsupported
    );
    assert_eq!(row(&report, "Fader 2").outcome, Outcome::UnboundUnsupported);
    assert_eq!(row(&report, "Fader 4").outcome, Outcome::UnboundUnsupported);

    // Counts add up per category and per action.
    let total = |c: &mapping::Counts| {
        c.imported + c.imported_lossy + c.unbound_unsupported + c.unbound_dangerous
    };
    assert_eq!(total(&report.totals), report.rows.len());
    assert_eq!(
        report
            .categories
            .iter()
            .map(|c| total(&c.counts))
            .sum::<usize>(),
        report.rows.len()
    );
    assert_eq!(
        report
            .actions
            .iter()
            .map(|c| total(&c.counts))
            .sum::<usize>(),
        report.rows.len()
    );
    assert_eq!(report.totals.unbound_dangerous, 2);
    assert_eq!(
        report.high_risk,
        report.rows.iter().filter(|r| r.high_risk).count()
    );
    let categories: Vec<_> = report
        .categories
        .iter()
        .map(|c| c.category.as_str())
        .collect();
    assert_eq!(categories, ["keypad", "knobs", "dial", "rollers", "faders"]);
    for warning in [
        "Lighting",
        "Controls with images or GIFs",
        "Shortcuts recorded on Windows",
        "keypad sections",
        "Knob 1 had 2 functions",
        "contained a device serial number",
    ] {
        assert!(
            report.warnings.iter().any(|w| w.contains(warning)),
            "{warning}: {:#?}",
            report.warnings
        );
    }
}

#[test]
fn serials_paths_and_secrets_never_reach_the_profile_or_the_report() {
    let mut source = full_profile();
    source["profileName"] = json!(format!("Desk {KEYPAD_SERIAL}"));
    source["keypad"][KEYPAD_SERIAL][0][0]["title"] = json!(format!("{BASE_SERIAL} copy"));
    source["keypad"][KEYPAD_SERIAL][0][3]["content"] =
        json!(format!("my unit is {}", OTHER_SERIAL.to_lowercase()));
    source["keypad"][KEYPAD_SERIAL][0][2]["url"] =
        json!(format!("https://example.com/{KNOB_SERIAL}"));
    let Converted { profile, report } = convert_one(&source);
    assert_eq!(profile.name, "Imported profile");
    assert_eq!(bound(&profile, "root", 3), None);
    assert_eq!(bound(&profile, "root", 2), None);
    assert!(
        matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1")
    );
    let text = serde_json::to_string(&profile).unwrap() + &serde_json::to_string(&report).unwrap();
    for secret in [
        BASE_SERIAL,
        KEYPAD_SERIAL,
        KNOB_SERIAL,
        DIAL_SERIAL,
        ROLLER_SERIAL,
        FADER_SERIAL,
        OTHER_SERIAL,
        "CT99999999999",
        "someone",
        "C:\\",
        "AppData",
        "song.mp3",
        "hello chat",
    ] {
        assert!(
            !text.to_lowercase().contains(&secret.to_lowercase()),
            "{secret} leaked"
        );
    }
}

#[test]
fn dangerous_values_stay_unbound_and_are_redacted() {
    let cases = [
        (
            tile(
                0,
                "web-url",
                json!({"url": "https://example.com/\"; rm -rf ~ #"}),
            ),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "https://example.com/`id`"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "file:///etc/passwd"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "javascript:alert(1)"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "https://ex ample.com"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "https://example.com/\u{200B}"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "web-url", json!({"url": "example.com"})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "web-url", json!({"url": "https://user:pw@example.com"})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "web-url",
                json!({"url": format!("https://example.com/{}", "a".repeat(70_000))}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "app-files",
                json!({"appFilePath": "\"C:\\a.exe\" & calc.exe | whoami"}),
            ),
            Outcome::UnboundDangerous,
        ),
        (
            tile(
                0,
                "text",
                json!({"content": "x".repeat(Action::MAX_TEXT_BYTES + 1)}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "text", json!({"content": "y".repeat(70_000)})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "text", json!({"content": "safe\u{202E}txt"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "text", json!({"content": "echo hi\r"})),
            Outcome::UnboundDangerous,
        ),
        (
            tile(0, "text", json!({"content": ""})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "hot-key",
                json!({"hotkey": hotkey(&["Control", "Minus"])}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "hot-key", json!({"hotkey": hotkey(&["Control", "-"])})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "hot-key", json!({"hotkey": hotkey(&["A", "B"])})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "hot-key", json!({"hotkey": hotkey(&["Shift"])})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "hot-key", json!({"hotkey": {"key": ["Control", 67]}})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "hot-key",
                json!({"hotkey": {"key": ["\" & do shell script \"x"]}}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "volume-control",
                json!({"dropdownOption": 0, "volumeControlProcessId": 99}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "volume-control",
                json!({"dropdownOption": 1, "volumeAction": "volume-up"}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "profile", json!({"dropdownOption": 9})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, "multi-tasking", json!({"actionList": []})),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "multi-tasking",
                json!({"actionList": vec![json!({"action": "hot-key", "hotkey": hotkey(&["A"])}); 9]}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "multi-tasking",
                json!({"actionList": [{"action": "profile", "dropdownOption": 0}, {"action": "hot-key", "hotkey": hotkey(&["A"])}]}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(
                0,
                "multi-tasking",
                json!({"actionList": [{"action": "web-url", "url": "file:///x"}, {"action": "hot-key", "hotkey": hotkey(&["A"])}]}),
            ),
            Outcome::UnboundDangerous,
        ),
        (
            tile(
                0,
                "multi-tasking",
                json!({"actionList": [{"action": "multi-tasking"}, {"action": "hot-key", "hotkey": hotkey(&["A"])}]}),
            ),
            Outcome::UnboundUnsupported,
        ),
        (
            tile(0, json!(42).to_string().as_str(), json!({})),
            Outcome::UnboundUnsupported,
        ),
    ];
    for (tile, expected) in cases {
        let source = profile_with(vec![page(vec![tile.clone()])]);
        let Converted { profile, report } = convert_one(&source);
        assert_eq!(bound(&profile, "root", 0), None, "{tile}");
        let found = row(&report, "Keypad · page 1 · key 1");
        assert_eq!(found.outcome, expected, "{tile}: {found:?}");
        let text = serde_json::to_string(&report).unwrap();
        for leaked in [
            "rm -rf",
            "calc.exe",
            "whoami",
            "etc/passwd",
            "alert(",
            "do shell script",
            "pw@",
        ] {
            assert!(!text.contains(leaked), "{leaked} leaked for {tile}");
        }
    }
    // Labels lose invisible and control characters and stay within 128 bytes.
    let source = profile_with(vec![page(vec![tile(
        0,
        "text",
        json!({"content": "a", "title": format!("Ke\u{202E}y\u{0000}{}", "é".repeat(100))}),
    )])]);
    let Converted { profile, .. } = convert_one(&source);
    let Some(KeypadTile::Action { label, .. }) = slot(&profile, "root", 0) else {
        panic!()
    };
    assert!(label.starts_with("Key") && label.len() <= 128, "{label}");
    // A one-step sequence becomes a plain binding; a non-string btnFunc is unknown.
    let source = profile_with(vec![page(vec![
        tile(
            0,
            "multi-tasking",
            json!({"actionList": [{"action": "text", "content": "hi", "afterEnter": true}]}),
        ),
        tile(1, "x", json!({"btnFunc": 5})),
    ])]);
    let Converted { profile, report } = convert_one(&source);
    assert_eq!(
        bound(&profile, "root", 0),
        Some(Action::Text {
            text: "hi".into(),
            press_enter: true
        })
    );
    assert!(row(&report, "Keypad · page 1 · key 1").high_risk);
    assert_eq!(
        row(&report, "Keypad · page 1 · key 2").official_action,
        "unknown"
    );
}

#[test]
fn url_mixing_sequences_are_high_risk() {
    let source = profile_with(vec![page(vec![tile(
        0,
        "multi-tasking",
        json!({"actionList": [
            {"action": "web-url", "url": "https://example.com"},
            {"action": "hot-key", "hotkey": hotkey(&["Control", "V"])}
        ]}),
    )])]);
    let Converted { report, .. } = convert_one(&source);
    assert!(row(&report, "Keypad · page 1 · key 1").high_risk);
}

#[test]
fn chained_pages_and_folders_respect_the_depth_page_and_binding_limits() {
    // Twelve chained pages: page 9 is at depth 8, the deepest allowed.
    let pages: Vec<Vec<Value>> = (0..12)
        .map(|_| {
            page(vec![
                tile(1, "text", json!({"content": "a"})),
                tile(14, "next-page", json!({})),
            ])
        })
        .collect();
    let Converted { profile, report } = convert_one(&profile_with(pages));
    profile.validate().unwrap();
    let keypad = profile.keypad.as_ref().unwrap();
    assert_eq!(keypad.pages.len(), 9);
    assert!(keypad.page("page9").is_some() && keypad.page("page10").is_none());
    assert!(
        row(&report, "Keypad · page 9 · key 15")
            .reason
            .contains("folder limits")
    );
    assert!(
        row(&report, "Keypad · page 10 · key 2")
            .reason
            .contains("not reachable")
    );
    assert!(
        row(&report, "Keypad · page 12 · key 15")
            .reason
            .contains("not reachable")
    );

    // Folders on chained pages: at most 64 pages and 256 bindings.
    let folder = |id: usize| {
        let children: Vec<Value> = (1..15)
            .map(|c| json!({"id": c, "btnFunc": "media-track-control", "dropdownOption": 4}))
            .collect();
        tile(id, "new-folder", json!({"folder": children}))
    };
    let pages: Vec<Vec<Value>> = (0..8)
        .map(|_| {
            let mut tiles: Vec<Value> = (1..14).map(folder).collect();
            tiles.push(tile(14, "next-page", json!({})));
            page(tiles)
        })
        .collect();
    let Converted { profile, report } = convert_one(&profile_with(pages));
    profile.validate().unwrap();
    assert_eq!(profile.keypad.as_ref().unwrap().pages.len(), 64);
    assert_eq!(profile.binding_count(), Profile::MAX_BINDINGS);
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.reason.contains("reached 256 bindings"))
    );
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.reason.contains("would exceed 8 levels or 64 pages"))
    );
    assert!(report.rows.iter().any(|r| {
        r.reason
            .contains("inside a folder that could not be imported")
    }));
    assert!(serde_json::to_vec(&profile).unwrap().len() <= Profile::MAX_JSON_BYTES);
}

#[test]
fn an_empty_or_minimal_profile_still_imports_an_empty_root_page() {
    let Converted { profile, report } = convert_one(&json!({"profilename": "Only name"}));
    assert_eq!(profile.name, "Only name");
    assert_eq!(profile.keypad.as_ref().unwrap().pages.len(), 1);
    assert!(profile.bindings.is_empty() && profile.dial_functions.is_none());
    assert!(report.rows.is_empty());
    let Converted { profile, .. } = convert_one(&json!({"profileName": 7}));
    assert_eq!(profile.name, "Imported profile");
}

#[test]
fn names_never_collide_and_stay_within_the_label_limit() {
    let existing = vec!["Desk (imported)".to_string(), "desk (IMPORTED 2) ".into()];
    assert_eq!(unique_name("Desk", &existing).unwrap(), "Desk (imported 3)");
    assert_eq!(unique_name("Desk", &[]).unwrap(), "Desk (imported)");
    let long = "é".repeat(100);
    let name = unique_name(&long, &[]).unwrap();
    assert!(name.len() <= 128 && name.ends_with(" (imported)"), "{name}");
    assert_eq!(unique_name("", &[]).unwrap(), "Imported profile (imported)");
}

#[test]
fn unsafe_entry_names_are_rejected() {
    let json = serde_json::to_vec(&json!({"profileName": "x"})).unwrap();
    for name in [
        "../profile1_A",
        "/profile1_A",
        "uploadImages/../profile1_A",
        "uploadImages\\x.png",
        "C:profile1_A",
        "uploadImages/a/b.png",
        "uploadImages//x.png",
        "./profile1_A",
        "uploadImages/x\0.png",
        "uploadImages/x\u{202E}.png",
        "readme.txt",
        "otherFolder/",
        "profile1_A/",
        "profile_A",
        "profileX_A",
        "profile1_",
        "profile1_A-B",
    ] {
        let archive = zip(&[item("profile1_A", &json), item(name, b"x")]);
        assert!(convert(&archive).is_err(), "{name:?} accepted");
    }
    let archive = zip(&[item("profile1_A", &json)]);
    assert!(convert(&archive).is_ok());
}

#[test]
fn archive_structure_limits_are_enforced() {
    let json = serde_json::to_vec(&json!({"profileName": "x"})).unwrap();
    let ok = |items: &[Item]| convert(&zip(items));
    let error = |items: &[Item]| convert(&zip(items)).unwrap_err();

    // Duplicates, also when they differ only in case or a folder slash.
    assert!(error(&[item("profile1_A", &json), item("profile1_A", &json)]).contains("duplicate"));
    assert!(
        error(&[
            item("profile1_A", &json),
            item("uploadImages/a.png", b"x"),
            item("uploadImages/A.png", b"x")
        ])
        .contains("duplicate")
    );
    // Exactly one profile entry.
    assert!(
        error(&[item("profile1_A", &json), item("profile2_B", &json)]).contains("more than one")
    );
    assert!(error(&[item("uploadImages/a.png", b"x")]).contains("does not contain"));
    // Encryption, methods and special files.
    let mut encrypted = item("profile1_A", &json);
    encrypted.flags = 1;
    assert!(error(&[encrypted]).contains("encrypted"));
    let mut aes = item("profile1_A", &json);
    aes.method = Some(99);
    assert!(error(&[aes]).contains("encrypted"));
    let mut aes_extra = item("profile1_A", &json);
    aes_extra.extra = vec![0x01, 0x99, 7, 0, 1, 0, b'A', b'E', 3, 8, 0];
    assert!(error(&[aes_extra]).contains("encrypted"));
    let mut bzip = item("profile1_A", &json);
    bzip.method = Some(12);
    assert!(error(&[bzip]).contains("compression method"));
    let mut link = item("profile1_A", &json);
    link.made_by = 3 << 8;
    link.external = 0o120_777 << 16;
    assert!(error(&[link]).contains("link"));
    let mut regular = item("profile1_A", &json);
    regular.made_by = 3 << 8;
    regular.external = 0o100_644 << 16;
    assert!(ok(&[regular]).is_ok());
    // CRC and size lies.
    let mut crc = item("profile1_A", &json);
    crc.crc = Some(0xdead_beef);
    assert!(error(&[crc]).contains("CRC"));
    let mut short = item("profile1_A", &json);
    short.declared_size = Some(json.len() as u32 - 1);
    assert!(convert(&zip(&[short])).is_err());
    let mut stored = item("profile1_A", &json);
    stored.deflate = false;
    assert!(ok(&[stored]).is_ok());
    // Too many entries.
    let mut many = vec![item("profile1_A", &json)];
    many.extend((0..archive::MAX_ENTRIES).map(|i| item(&format!("gifPools/{i}"), b"x")));
    assert!(error(&many).contains("more than 256 entries"));
    // Not a ZIP, empty, truncated, oversize.
    for bytes in [&b""[..], b"PK\x03\x04", b"not a zip at all, just text"] {
        assert!(convert(bytes).is_err());
    }
    let valid = zip(&[item("profile1_A", &json)]);
    assert!(convert(&valid[..valid.len() - 1]).is_err());
    assert!(
        convert(&vec![0; archive::MAX_ARCHIVE_BYTES + 1])
            .unwrap_err()
            .contains("larger than 32 MiB")
    );
}

#[test]
fn image_entries_are_skipped_without_being_read() {
    let json = serde_json::to_vec(&json!({"profileName": "x"})).unwrap();
    // Garbage that is neither valid deflate nor matches its CRC: never read.
    let mut gif = item("gifPools/abc", b"");
    gif.deflate = false;
    gif.method = Some(8);
    gif.data = vec![0xff; 32];
    gif.crc = Some(1);
    let archive = zip(&[
        item("uploadImages/", b""),
        item("profile2_A", &json),
        item("uploadImages/1.gif", b"GIF89a"),
        gif,
    ]);
    let converted = convert(&archive).unwrap();
    assert_eq!(converted.len(), 1);
    assert!(converted[0].report.warnings[0].contains("skipped unread: 2"));
}

#[test]
fn decompression_bombs_and_oversized_entries_are_rejected() {
    // 2 MiB of whitespace after a tiny object compresses far beyond 100:1.
    let mut bomb = b"{}".to_vec();
    bomb.resize(2 * 1024 * 1024, b' ');
    assert!(
        convert(&zip(&[item("profile1_A", &bomb)]))
            .unwrap_err()
            .contains("100:1")
    );
    // A header that understates the size cannot slip a bomb past the check.
    let mut lying = item("profile1_A", &bomb);
    lying.declared_size = Some(1024);
    assert!(convert(&zip(&[lying])).is_err());
    // Declared sizes are summed across entries, images included.
    let mut huge = item("uploadImages/big.png", b"x");
    huge.deflate = false;
    huge.declared_size = Some(65 * 1024 * 1024);
    huge.method = Some(8);
    assert!(convert(&zip(&[item("profile1_A", b"{}"), huge])).is_err());
    // A profile JSON over 16 MiB is rejected even when it compresses well
    // enough to pass the ratio rule.
    let mut big: Vec<u8> = Vec::with_capacity(17 * 1024 * 1024);
    let mut state: u32 = 1;
    big.extend_from_slice(b"{\"a\":\"");
    while big.len() < 17 * 1024 * 1024 {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        big.push(b'a' + (state >> 16) as u8 % 26);
    }
    big.extend_from_slice(b"\"}");
    assert!(
        convert(&zip(&[item("profile1_A", &big)]))
            .unwrap_err()
            .contains("larger than 16 MiB")
    );
}

#[test]
fn overlapping_entries_are_rejected() {
    let json = serde_json::to_vec(&json!({"profileName": "x"})).unwrap();
    // Entry B's local record is hidden inside entry A's stored data.
    let inner = item("uploadImages/b.png", b"img");
    let compressed = deflate(&inner.data);
    let inner_local = local(&inner, &compressed, 8, crc32fast::hash(&inner.data), 3);
    let mut outer = item("uploadImages/a.png", &inner_local);
    outer.deflate = false;
    let name_len = outer.name.len() as u32;
    let mut hidden = inner;
    hidden.offset = Some(30 + name_len);
    let archive = zip(&[outer, hidden, item("profile1_A", &json)]);
    assert!(convert(&archive).unwrap_err().contains("overlap"));
}

#[test]
fn bad_json_and_deep_nesting_are_rejected() {
    for bytes in [
        &b"{"[..],
        b"[1,2]",
        b"\"text\"",
        b"{} trailing",
        b"\xff\xfe",
    ] {
        let archive = zip(&[item("profile1_A", bytes)]);
        assert!(convert(&archive).is_err(), "{bytes:?}");
    }
    let deep = format!("{{\"keypad\":{}{}}}", "[".repeat(80), "]".repeat(80));
    let error = convert(&zip(&[item("profile1_A", deep.as_bytes())])).unwrap_err();
    assert!(error.contains("nested too deeply"), "{error}");
    // Wrong types never panic: they are unsupported or ignored.
    let odd = json!({"keypad": {"S": "pages"}, "knobsList": [1, 2], "dialEncoder": {"S": {"functionList": "x"}}, "sliders5List": {"S": [{"btnFunc": "volume-control"}]}});
    let Converted { profile, .. } = convert_one(&odd);
    profile.validate().unwrap();
}

#[test]
fn nested_multi_profile_archives_are_one_level_and_bounded() {
    let first = mhz(&json!({"profileName": "One"}));
    let second = mhz(&json!({"profileName": "Two"}));
    let outer = zip(&[
        item("masterHubConfig_profile_One.mhz", &first),
        item("masterHubConfig_profile_Two.MHZ", &second),
    ]);
    let converted = convert(&outer).unwrap();
    let names: Vec<_> = converted.iter().map(|c| c.profile.name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    // A second level is refused.
    let deeper = zip(&[item("inner.mhz", &outer)]);
    assert!(convert(&deeper).is_err());
    // At most eight.
    let many: Vec<Item> = (0..=archive::MAX_NESTED)
        .map(|i| item(&format!("p{i}.mhz"), &first))
        .collect();
    assert!(convert(&zip(&many)).unwrap_err().contains("more than 8"));
    // No mixing of a profile entry with nested files, or images in the outer.
    let json = serde_json::to_vec(&json!({})).unwrap();
    assert!(convert(&zip(&[item("profile1_A", &json), item("a.mhz", &first)])).is_err());
    assert!(convert(&zip(&[item("a.mhz", &first), item("uploadImages/x", b"x")])).is_err());
    // A broken inner archive rejects everything.
    assert!(convert(&zip(&[item("a.mhz", &first), item("b.mhz", b"garbage")])).is_err());
}

#[test]
fn corrupted_archives_never_panic() {
    let valid = mhz(&full_profile());
    let small = mhz(
        &json!({"profileName": "x", "keypad": {"S": [page(vec![tile(0, "text", json!({"content": "a"}))])]}}),
    );
    for length in 0..small.len() {
        let _ = convert(&small[..length]);
    }
    for index in 0..small.len() {
        let mut flipped = small.clone();
        flipped[index] ^= 0x5a;
        let _ = convert(&flipped);
    }
    assert!(convert(&valid).is_ok());
}

fn first_row(source: &Value) -> (Profile, mapping::ReportRow, String) {
    let Converted { profile, report } = convert_one(source);
    let text = serde_json::to_string(&report).unwrap();
    let row = report.rows[0].clone();
    (profile, row, text)
}

#[test]
fn serials_and_user_folders_never_reach_report_details_or_reasons() {
    let base = |tile: Value| {
        let mut source = profile_with(vec![page(vec![tile])]);
        source["devices"] = json!({"m003": [{"serialNumber": KNOB_SERIAL}]});
        source
    };
    let cases = [
        tile(
            0,
            "web-url",
            json!({"url": format!("https://{BASE_SERIAL}.example.com/x")}),
        ),
        tile(
            0,
            "web-url",
            json!({"url": format!("ftp://{BASE_SERIAL}.example.com/x")}),
        ),
        tile(
            0,
            "app-files",
            json!({"appFilePath": format!("C:\\Tools\\{KEYPAD_SERIAL}.exe")}),
        ),
        tile(0, "app-files", json!({"appFilePath": "C:\\Users\\bob"})),
        tile(0, "app-files", json!({"appFilePath": "/home/bob/"})),
        tile(
            0,
            "hot-key",
            json!({"hotkey": hotkey(&["Control", KNOB_SERIAL])}),
        ),
        tile(
            0,
            "multi-tasking",
            json!({"actionList": [
                {"action": "web-url", "url": format!("https://{BASE_SERIAL}.example.com")},
                {"action": "hot-key", "hotkey": hotkey(&["A"])}
            ]}),
        ),
    ];
    for case in cases {
        let (profile, row, text) = first_row(&base(case.clone()));
        assert_eq!(bound(&profile, "root", 0), None, "{case}");
        for secret in [BASE_SERIAL, KEYPAD_SERIAL, KNOB_SERIAL, "bob"] {
            assert!(
                !text.to_lowercase().contains(&secret.to_lowercase()),
                "{secret} in {row:?}"
            );
        }
    }
    let (_, row, _) = first_row(&base(tile(
        0,
        "app-files",
        json!({"appFilePath": "C:\\Users\\bob"}),
    )));
    assert_eq!(row.detail.as_deref(), Some("<12 chars>"));
    let (_, row, _) = first_row(&base(tile(
        0,
        "hot-key",
        json!({"hotkey": hotkey(&["Control", KNOB_SERIAL])}),
    )));
    assert!(row.reason.contains("the key a key"), "{}", row.reason);
    let (_, row, _) = first_row(&base(tile(
        0,
        "app-files",
        json!({"appFilePath": "C:\\Tools\\tool.exe"}),
    )));
    assert_eq!(row.detail.as_deref(), Some("tool.exe"));
}

fn knob_profile(rotate_r: Value) -> Value {
    json!({"profileName": "K", "knobsList": {KNOB_SERIAL: {"rotateR": rotate_r}}})
}

#[test]
fn rotating_sides_refuse_mute_macros_and_per_app_volume() {
    let (profile, row, _) = first_row(&knob_profile(
        json!({"btnFunc": "volume-control", "right": {"volumeAction": "volume-mute"}, "left": {}}),
    ));
    assert!(row.reason.contains("every detent"), "{}", row.reason);
    assert!(
        profile
            .bindings
            .iter()
            .all(|b| b.action != Action::ToggleMute)
    );
    assert_eq!(profile.bindings.len(), 1);
    for knob in [
        json!({"btnFunc": "hot-key", "hotKeySettingMode": "macro", "right": {"hotkey": hotkey(&["A"])}, "left": {"hotkey": hotkey(&["B"])}}),
        json!({"btnFunc": "hot-key", "right": {"hotKeySettingMode": "macro", "hotkey": hotkey(&["A"])}, "left": {"hotKeySettingMode": "macro", "hotkey": hotkey(&["B"])}}),
    ] {
        let Converted { profile, report } = convert_one(&knob_profile(knob));
        assert!(profile.bindings.is_empty());
        assert!(
            report.rows.iter().all(|r| r.reason.contains("macro")),
            "{:?}",
            report.rows
        );
    }
    let dial = json!({"profileName": "D", "dialEncoder": {DIAL_SERIAL: {"functionList": [
        {"btnFunc": "volume-control", "btnName": "App", "volumeControlProcessId": 77, "right": {}, "left": {}},
        {"btnFunc": "volume-control", "btnName": "Main", "right": {}, "left": {}}
    ]}}});
    let Converted { profile, report } = convert_one(&dial);
    let functions = profile.dial_functions.unwrap();
    assert!(functions[0].bindings.is_empty());
    assert_eq!(functions[1].bindings.len(), 2);
    assert!(report.rows[0].reason.contains("per-application"));
}

#[test]
fn odd_after_enter_values_sequence_details_and_warnings_follow_what_is_bound() {
    for value in [json!("yes"), json!(2), json!({})] {
        let (profile, row, _) = first_row(&profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "hi", "afterEnter": value}),
        )])]));
        assert_eq!(row.outcome, Outcome::UnboundUnsupported, "{value}");
        assert!(row.reason.contains("afterEnter"), "{}", row.reason);
        assert!(bound(&profile, "root", 0).is_none());
    }
    let (_, row, _) = first_row(&profile_with(vec![page(vec![tile(
        0,
        "text",
        json!({"content": "hi", "afterEnter": true}),
    )])]));
    assert_eq!(row.outcome, Outcome::Imported);
    assert!(row.reason.contains("presses Return"));
    // Step reasons and redacted details survive into the sequence row.
    let (_, row, _) = first_row(&profile_with(vec![page(vec![tile(
        0,
        "multi-tasking",
        json!({"actionList": [
            {"action": "web-url", "url": "https://example.com/path"},
            {"action": "text", "content": "go", "afterEnter": true}
        ]}),
    )])]));
    assert!(
        row.reason.contains("1. opens a web page") && row.reason.contains("presses Return"),
        "{}",
        row.reason
    );
    assert_eq!(
        row.detail.as_deref(),
        Some("https://example.com, <2 chars>")
    );
    // A dropped sequence's shortcuts are not counted in the warning.
    let Converted { report, .. } = convert_one(&profile_with(vec![page(vec![tile(
        0,
        "multi-tasking",
        json!({"actionList": [
            {"action": "hot-key", "hotkey": hotkey(&["Control", "A"])},
            {"action": "app-files", "appFilePath": "C:\\x.exe"}
        ]}),
    )])]));
    assert!(
        !report
            .warnings
            .iter()
            .any(|w| w.contains("Shortcuts recorded")),
        "{:?}",
        report.warnings
    );
}

#[test]
fn numbers_among_steps_keys_or_functions_are_malformed() {
    let (profile, row, _) = first_row(&profile_with(vec![page(vec![tile(
        0,
        "multi-tasking",
        json!({"actionList": [
            {"action": "hot-key", "hotkey": hotkey(&["A"])}, 7, {"action": "hot-key", "hotkey": hotkey(&["B"])}
        ]}),
    )])]));
    assert_eq!(bound(&profile, "root", 0), None);
    assert!(row.reason.contains("malformed"));
    // A number among the keys of a page would shift every later key.
    let mut tiles: Vec<Value> = vec![json!(5)];
    tiles.extend(
        (0..15).map(|id| json!({"btnFunc": if id == 3 { "text" } else { "" }, "content": "x"})),
    );
    let Converted { profile, report } = convert_one(&json!({"keypad": {"S": [tiles]}}));
    assert!(
        profile
            .keypad
            .unwrap()
            .page("root")
            .unwrap()
            .slots
            .iter()
            .all(Option::is_none)
    );
    assert!(!report.rows.is_empty());
    assert!(report.rows.iter().all(|r| r.reason.contains("malformed")));
    let Converted { profile, report } = convert_one(
        &json!({"dialEncoder": {"S": {"functionList": [1, {"btnFunc": "volume-control"}]}}}),
    );
    assert!(profile.dial_functions.is_none());
    assert!(report.rows[0].reason.contains("malformed"));
}

#[test]
fn too_many_serials_are_refused_fast() {
    let keys: Vec<String> = (0..480_000)
        .map(|i| format!("\"MHKD{i:08}\":null"))
        .collect();
    let json = format!(
        "{{\"profileName\":\"x\",\"keypad\":{{{}}}}}",
        keys.join(",")
    );
    let root = json::parse(json.as_bytes()).unwrap();
    let started = std::time::Instant::now();
    let before = mapping::SCANNED.with(std::cell::Cell::get);
    let error = mapping::map(&root, "A").unwrap_err();
    // Refused before any text is scanned (an operation count, not a clock).
    assert_eq!(mapping::SCANNED.with(std::cell::Cell::get), before);
    // Wall-clock only in optimized builds, with a generous limit.
    #[cfg(not(debug_assertions))]
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let _ = started;
    assert!(error.contains("more than 64"), "{error}");
    // 64 distinct serials are still fine (63 keys plus the entry serial).
    let keys: Vec<String> = (0..63).map(|i| format!("\"MHKD{i:08}\":null")).collect();
    let json = format!("{{\"keypad\":{{{}}}}}", keys.join(","));
    assert!(mapping::map(&json::parse(json.as_bytes()).unwrap(), "A1B2").is_ok());
}

#[test]
fn report_rows_are_capped_but_still_counted() {
    let tiles: Vec<Value> = (0..99_000).map(|_| json!({"btnFunc": "timer"})).collect();
    // Stored, not deflated: identical tiles would compress past 100:1.
    let mut entry = item(
        "profile1_A",
        &serde_json::to_vec(&json!({"keypad": {"S": [tiles]}})).unwrap(),
    );
    entry.deflate = false;
    let Converted { report, .. } = convert(&zip(&[entry])).unwrap().remove(0);
    assert_eq!(report.rows.len(), mapping::MAX_REPORT_ROWS);
    assert_eq!(report.totals.unbound_unsupported, 99_000);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.contains("98000 more controls")),
        "{:?}",
        report.warnings
    );
    assert!(serde_json::to_vec(&report).unwrap().len() < 400_000);
}

#[test]
fn bound_text_stops_at_the_payload_budget_instead_of_failing() {
    let steps: Vec<Value> = (0..8)
        .map(|i| json!({"action": "text", "content": format!("{i}{}", "x".repeat(2000))}))
        .collect();
    let pages: Vec<Vec<Value>> = (0..3)
        .map(|_| {
            let mut tiles: Vec<Value> = (0..14)
                .map(|id| tile(id, "multi-tasking", json!({"actionList": steps})))
                .collect();
            tiles.push(tile(14, "next-page", json!({})));
            page(tiles)
        })
        .collect();
    let Converted { profile, report } = convert_one(&profile_with(pages));
    profile.validate().unwrap();
    assert!(serde_json::to_vec(&profile).unwrap().len() <= Profile::MAX_JSON_BYTES);
    assert!(
        report
            .rows
            .iter()
            .any(|r| r.reason.contains("reached 400 KiB"))
    );
    assert!(report.totals.imported_lossy >= 20);
}

#[test]
fn r3_action_ids_are_a_closed_catalogue_with_prefix_buckets() {
    let source = profile_with(vec![page(vec![
        tile(0, &format!("ps-{}", BASE_SERIAL.to_lowercase()), json!({})),
        tile(1, "discord-johndoe", json!({})),
        tile(2, "streamlabs-secret-name", json!({})),
        tile(3, "record-start-stop", json!({})),
    ])]);
    let Converted { report, .. } = convert_one(&source);
    let ids: Vec<_> = report
        .rows
        .iter()
        .map(|r| r.official_action.as_str())
        .collect();
    assert_eq!(
        ids,
        ["ps-*", "discord-*", "streamlabs-*", "record-start-stop"]
    );
    let text = serde_json::to_string(&report).unwrap().to_lowercase();
    for leak in [
        BASE_SERIAL.to_lowercase().as_str(),
        "johndoe",
        "secret-name",
    ] {
        assert!(!text.contains(leak), "{leak}");
    }
}

#[test]
fn r3_serials_are_checked_before_names_are_clamped() {
    let title = format!("{} {BASE_SERIAL}", "x".repeat(124));
    let mut source = profile_with(vec![page(vec![tile(
        0,
        "text",
        json!({"content": "a", "title": title}),
    )])]);
    source["profileName"] = json!(format!("{} {BASE_SERIAL}", "y".repeat(124)));
    source["dialEncoder"] = json!({DIAL_SERIAL: {"functionList": [
        {"btnFunc": "volume-control", "btnName": format!("{} {DIAL_SERIAL}", "z".repeat(60)), "right": {}, "left": {}}
    ]}});
    let Converted { profile, .. } = convert_one(&source);
    assert!(
        matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1")
    );
    assert_eq!(profile.name, "Imported profile");
    assert_eq!(profile.dial_functions.unwrap()[0].name, "Function 1");
}

#[test]
fn r3_basenames_need_a_program_extension_outside_user_folders() {
    for (path, expected) in [
        ("C:\\Users\\john.doe", "<17 chars>"),
        ("/home/jane.smith", "<16 chars>"),
        ("C:\\Users\\bob\\notes.txt", "<22 chars>"),
        ("C:\\Tools\\tool.exe", "tool.exe"),
        ("C:\\Tools\\Run.LNK", "Run.LNK"),
    ] {
        let source = profile_with(vec![page(vec![tile(
            0,
            "app-files",
            json!({"appFilePath": path}),
        )])]);
        let Converted { report, .. } = convert_one(&source);
        assert_eq!(report.rows[0].detail.as_deref(), Some(expected), "{path}");
    }
}

#[test]
fn r3_the_report_stays_small_with_many_distinct_unknown_ids() {
    let first: Vec<Value> = (0..99_000)
        .map(|i| json!({"btnFunc": format!("ps-x{i}")}))
        .collect();
    let second: Vec<Value> = (0..99_000)
        .map(|i| json!({"btnFunc": format!("discord-y{i}")}))
        .collect();
    let mut entry = item(
        "profile1_A",
        &serde_json::to_vec(&json!({"keypad": {"S": [first, second]}})).unwrap(),
    );
    entry.deflate = false;
    let Converted { report, .. } = convert(&zip(&[entry])).unwrap().remove(0);
    assert!(report.actions.len() <= 2, "{}", report.actions.len());
    assert!(serde_json::to_vec(&report).unwrap().len() < 400_000);
}

#[test]
fn r3_quote_heavy_text_never_breaks_the_store_size_limit() {
    let steps: Vec<Value> = (0..8)
        .map(|i| json!({"action": "text", "content": format!("{i}{}", "\"".repeat(2000))}))
        .collect();
    let pages: Vec<Vec<Value>> = (0..3)
        .map(|_| {
            let mut tiles: Vec<Value> = (0..14)
                .map(|id| {
                    tile(
                        id,
                        "multi-tasking",
                        json!({"actionList": steps, "title": "\"".repeat(128)}),
                    )
                })
                .collect();
            tiles.push(tile(14, "next-page", json!({})));
            page(tiles)
        })
        .collect();
    let mut entry = item(
        "profile1_A",
        &serde_json::to_vec(&profile_with(pages)).unwrap(),
    );
    entry.deflate = false;
    let Converted { profile, report } = convert(&zip(&[entry])).unwrap().remove(0);
    let pretty = serde_json::to_vec_pretty(&profile.clone().into_current().unwrap()).unwrap();
    assert!(pretty.len() <= Profile::MAX_JSON_BYTES, "{}", pretty.len());
    assert!(report.rows.iter().any(|r| r.reason.contains("KiB")));
}

#[test]
fn r3_dropped_pages_count_every_tile_and_numbers_never_shift_positions() {
    let mut tiles: Vec<Value> = (0..20).map(|_| json!({"btnFunc": "timer"})).collect();
    tiles[5] = json!({"btnFunc": "new-folder", "folder": [{"btnFunc": "timer"}, {"btnFunc": "timer"}, {"btnFunc": "timer"}]});
    // Page 2 is unreachable: page 1 has no next-page key.
    let Converted { report, .. } = convert_one(&json!({"keypad": {"S": [[], tiles]}}));
    assert_eq!(report.totals.unbound_unsupported, 23);
    // Numbers among the pages of a keypad, or among the faders.
    let Converted { profile, report } =
        convert_one(&json!({"keypad": {"S": [7, [{"btnFunc": "text", "content": "x"}]]}}));
    assert!(
        profile
            .keypad
            .unwrap()
            .page("root")
            .unwrap()
            .slots
            .iter()
            .all(Option::is_none)
    );
    assert!(
        report.rows.iter().all(|r| r.reason.contains("malformed")),
        "{:?}",
        report.rows
    );
    let Converted { profile, report } =
        convert_one(&json!({"sliders5List": {"S": [3, {"btnFunc": "volume-control"}]}}));
    assert!(profile.bindings.is_empty());
    assert!(
        report.rows.iter().any(|r| r.reason.contains("malformed")),
        "{:?}",
        report.rows
    );
}

#[test]
fn r3_percent_encoded_serials_in_urls_are_found() {
    let url = format!("https://example.com/%4D%48{}", &BASE_SERIAL[2..]);
    let (profile, row, text) = first_row(&profile_with(vec![page(vec![tile(
        0,
        "web-url",
        json!({"url": url}),
    )])]));
    assert_eq!(bound(&profile, "root", 0), None);
    assert!(row.reason.contains("serial"));
    assert!(!text.contains(&BASE_SERIAL[2..]));
}

#[test]
fn r3_non_boolean_after_enter_is_left_unbound() {
    for value in [json!("yes"), json!(2), json!({})] {
        let (profile, row, _) = first_row(&profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "hi", "afterEnter": value}),
        )])]));
        assert_eq!(row.outcome, Outcome::UnboundUnsupported, "{value}");
        assert!(bound(&profile, "root", 0).is_none());
    }
}

#[test]
fn r3_serials_with_separators_or_stored_as_numbers_are_scrubbed() {
    let cases = [
        (json!({"serialNumber": "MHBS-01AA-1234"}), "MHBS01AA1234"),
        (json!({"serialNumber": "MHBS01AA1234"}), "MHBS-01AA 1234"),
        (
            json!({"devices": {"basex64": [{"serialNumber": 123_456_789}]}}),
            "unit 123456789",
        ),
        (json!({"serialNumber": 98_765_432}), "98765432"),
    ];
    for (extra, title) in cases {
        let mut source = profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "a", "title": title}),
        )])]);
        for (key, value) in extra.as_object().unwrap() {
            source[key] = value.clone();
        }
        let Converted { profile, .. } = convert_one(&source);
        assert!(
            matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1"),
            "{title}"
        );
    }
}

#[test]
fn r4_a_malformed_page_after_a_next_page_key_never_fails_the_import() {
    let mut bad: Vec<Value> = vec![json!(7)];
    bad.extend((0..3).map(|_| json!({"btnFunc": "timer"})));
    let source = json!({"keypad": {"S": [page(vec![tile(14, "next-page", json!({}))]), bad]}});
    let Converted { profile, report } = convert_one(&source);
    profile.validate().unwrap();
    assert!(profile.keypad.as_ref().unwrap().page("page2").is_none());
    assert_eq!(
        row(&report, "Keypad · page 1 · key 15").outcome,
        Outcome::UnboundUnsupported
    );
    assert_eq!(report.totals.unbound_unsupported, 4);
}

/// Every array and object of a valid file, replaced by a number, an empty
/// object or an array with a number in front: mapping may refuse the file
/// for a limit, but never because it built an invalid profile.
#[test]
fn r4_mutated_files_never_fail_for_structural_reasons() {
    fn paths(value: &Value, path: Vec<String>, out: &mut Vec<Vec<String>>) {
        match value {
            Value::Object(map) => {
                out.push(path.clone());
                for (key, item) in map {
                    let mut next = path.clone();
                    next.push(key.clone());
                    paths(item, next, out);
                }
            }
            Value::Array(items) => {
                out.push(path.clone());
                for (index, item) in items.iter().enumerate() {
                    let mut next = path.clone();
                    next.push(index.to_string());
                    paths(item, next, out);
                }
            }
            _ => {}
        }
    }
    fn at<'a>(value: &'a mut Value, path: &[String]) -> &'a mut Value {
        path.iter().fold(value, |node, key| match node {
            Value::Array(items) => &mut items[key.parse::<usize>().unwrap()],
            other => &mut other[key.as_str()],
        })
    }
    let base = full_profile();
    let mut all = Vec::new();
    paths(&base, Vec::new(), &mut all);
    for path in all {
        for variant in 0..3 {
            let mut value = base.clone();
            let node = at(&mut value, &path);
            *node = match (variant, node.clone()) {
                (0, _) => json!(7),
                (1, _) => json!({}),
                (_, Value::Array(mut items)) => {
                    items.insert(0, json!(3));
                    Value::Array(items)
                }
                (_, other) => json!([3, other]),
            };
            let root = json::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
            if let Err(error) = mapping::map(&root, "A") {
                assert!(
                    !error.contains("invalid"),
                    "{path:?} variant {variant}: {error}"
                );
            }
        }
    }
}

fn many_text_tiles(text: &str, folders: usize) -> Value {
    let children: Vec<Value> = (1..15)
        .map(|id| json!({"id": id, "btnFunc": "text", "content": text}))
        .collect();
    let root: Vec<Value> = (1..=folders.min(13))
        .map(|id| tile(id, "new-folder", json!({"folder": children})))
        .chain([tile(14, "next-page", json!({}))])
        .collect();
    let second: Vec<Value> = (1..=folders.saturating_sub(13).min(13))
        .map(|id| tile(id, "new-folder", json!({"folder": children})))
        .collect();
    json!({"profileName": "Big", "keypad": {"S": [page(root), page(second)]}})
}

#[test]
fn r4_the_final_fit_updates_the_report_and_is_fast() {
    let root =
        json::parse(&serde_json::to_vec(&many_text_tiles(&"q".repeat(1400), 26)).unwrap()).unwrap();
    let limit = 100 * 1024;
    let started = std::time::Instant::now();
    let passes = mapping::FIT_PASSES.with(std::cell::Cell::get);
    let (profile, report) = mapping::map_with_limit(&root, "A", limit).unwrap();
    // Linear: a few serializations, not one per removed binding.
    let passes = mapping::FIT_PASSES.with(std::cell::Cell::get) - passes;
    assert!(passes <= 6, "{passes} serializations");
    #[cfg(not(debug_assertions))]
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    let _ = started;
    assert!(serde_json::to_vec_pretty(&profile).unwrap().len() <= limit - 1024);
    let fitted = report
        .rows
        .iter()
        .filter(|r| r.reason.contains("fits 1 MiB"))
        .count();
    assert!(fitted > 100, "{fitted}");
    // Every bound action has an imported row (folder and page rows aside),
    // and every removed one a row that says so.
    let bound = all_actions(&profile).len();
    let imported = report
        .rows
        .iter()
        .filter(|r| {
            matches!(r.outcome, Outcome::Imported | Outcome::ImportedLossy)
                && !matches!(r.official_action.as_str(), "new-folder" | "next-page")
        })
        .count();
    assert_eq!(imported, bound);
    let total = |c: &mapping::Counts| {
        c.imported + c.imported_lossy + c.unbound_unsupported + c.unbound_dangerous
    };
    assert_eq!(total(&report.totals), report.rows.len());
    assert_eq!(
        report
            .categories
            .iter()
            .map(|c| total(&c.counts))
            .sum::<usize>(),
        report.rows.len()
    );
}

#[test]
fn r4_mapping_a_large_valid_file_is_fast() {
    let mut source = many_text_tiles(&"t".repeat(2000), 26);
    source["devices"] = json!({"m": (0..60).map(|i| json!({"serialNumber": format!("MHXX{i:08}")})).collect::<Vec<_>>()});
    // Sequences make the file big: every key carries eight long text steps.
    let steps: Vec<Value> = (0..8)
        .map(|_| json!({"action": "text", "content": "s".repeat(2000)}))
        .collect();
    for page in source["keypad"]["S"].as_array_mut().unwrap() {
        for tile in page.as_array_mut().unwrap() {
            for child in tile["folder"].as_array_mut().into_iter().flatten() {
                *child =
                    json!({"id": child["id"], "btnFunc": "multi-tasking", "actionList": steps});
            }
        }
    }
    let bytes = serde_json::to_vec(&source).unwrap();
    assert!(bytes.len() > 5_000_000, "{}", bytes.len());
    let root = json::parse(&bytes).unwrap();
    let started = std::time::Instant::now();
    let before = mapping::SCANNED.with(std::cell::Cell::get);
    mapping::map(&root, "A").unwrap();
    // Text past the budget is never scanned for serials.
    let scanned = mapping::SCANNED.with(std::cell::Cell::get) - before;
    assert!(
        scanned < 1_000_000,
        "{scanned} bytes scanned of {}",
        bytes.len()
    );
    #[cfg(not(debug_assertions))]
    assert!(
        started.elapsed() < std::time::Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    let _ = started;
}

#[test]
fn r4_percent_encoded_serial_hosts_never_reach_report_details() {
    let host = format!("%4D%48{}", &BASE_SERIAL[2..]);
    for url in [
        format!("https://{host}.example.com/ x"),
        format!("https://{host}.example.com/\"q"),
        format!("https://user@{host}.example.com/"),
    ] {
        let (_, row, text) = first_row(&profile_with(vec![page(vec![tile(
            0,
            "web-url",
            json!({"url": url}),
        )])]));
        assert!(!text.to_lowercase().contains("bsfake0001"), "{row:?}");
    }
}

#[test]
fn r4_serials_hidden_by_odd_separators_or_fullwidth_forms_are_scrubbed() {
    for title in [
        "MHBS\u{a0}FAKE\u{2010}0001".to_owned(),
        "ＭＨＢＳＦＡＫＥ０００１".to_owned(),
        "MHBS+FAKE#0001".to_owned(),
    ] {
        let source = profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "a", "title": title}),
        )])]);
        let Converted { profile, .. } = convert_one(&source);
        assert!(
            matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1"),
            "{title}"
        );
    }
}

#[test]
fn r4_short_serials_do_not_scrub_ordinary_labels() {
    for (serial, title) in [(json!("AB12"), "AB12 deck"), (json!(2024), "2024")] {
        let mut source = profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "a", "title": title}),
        )])]);
        source["devices"] = json!({"m": [{"serialNumber": serial}]});
        let Converted { profile, .. } = convert_one(&source);
        assert!(
            matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == title),
            "{title}"
        );
    }
}

#[test]
fn r5_lighting_setting_names_are_not_serials() {
    // The shape of the official templates' lighting section (synthetic).
    let mut source = profile_with(vec![page(vec![
        tile(0, "brightness", json!({"title": "Brightness up"})),
        tile(
            1,
            "lighting-effect",
            json!({"title": "Lighting", "dropdownOption": 3}),
        ),
        tile(
            2,
            "text",
            json!({"content": "brightness", "title": "brightness"}),
        ),
    ])]);
    source["deviceLightingEffect"] = json!({
        "wholeDevice": {"brightness": 80, "effect": "static", "color": "#ffffff", "lightingEffect": 1},
        "currentLightingEffectDevice": {"basex64": true, "m4315": false},
        "m4315": {KEYPAD_SERIAL: {"effect": 1, "color": "#ffffff", "brightness": 50}},
    });
    let Converted { profile, report } = convert_one(&source);
    assert!(
        matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Brightness up")
    );
    assert!(
        row(&report, "Keypad · page 1 · key 1")
            .reason
            .contains("not supported yet")
    );
    assert!(
        row(&report, "Keypad · page 1 · key 2")
            .reason
            .contains("not supported yet")
    );
    assert_eq!(
        bound(&profile, "root", 2),
        Some(Action::Text {
            text: "brightness".into(),
            press_enter: false
        })
    );
}

#[test]
fn r5_a_shortcut_only_profile_is_fitted_instead_of_failing() {
    let steps: Vec<Value> = (0..8)
        .map(|_| json!({"action": "hot-key", "hotkey": hotkey(&["Control", "Alt", "Shift", "Win", "Delete"])}))
        .collect();
    let children: Vec<Value> = (1..15)
        .map(|id| json!({"id": id, "btnFunc": "multi-tasking", "title": "\"".repeat(128), "actionList": steps}))
        .collect();
    let pages: Vec<Vec<Value>> = (0..5)
        .map(|_| {
            let mut tiles: Vec<Value> = (1..14)
                .map(|id| {
                    tile(
                        id,
                        "new-folder",
                        json!({"title": "\"".repeat(128), "folder": children}),
                    )
                })
                .collect();
            tiles.push(tile(14, "next-page", json!({})));
            page(tiles)
        })
        .collect();
    let mut entry = item(
        "profile1_A",
        &serde_json::to_vec(&profile_with(pages)).unwrap(),
    );
    entry.deflate = false;
    let Converted { profile, report } = convert(&zip(&[entry])).unwrap().remove(0);
    let pretty = serde_json::to_vec_pretty(&profile.clone().into_current().unwrap()).unwrap();
    assert!(
        pretty.len() <= Profile::MAX_JSON_BYTES - 1024,
        "{}",
        pretty.len()
    );
    let total = |c: &mapping::Counts| {
        c.imported + c.imported_lossy + c.unbound_unsupported + c.unbound_dangerous
    };
    assert_eq!(
        total(&report.totals),
        report.rows.len() + report_omitted(&report)
    );
}

fn report_omitted(report: &ImportReport) -> usize {
    report
        .warnings
        .iter()
        .find_map(|w| w.strip_suffix(" more controls are counted above but not listed."))
        .map_or(0, |n| n.parse().unwrap())
}

#[test]
fn r5_dial_names_are_checked_for_serials_after_the_duplicate_suffix() {
    let source = json!({"profileName": "D", "keypad": {KEYPAD_SERIAL: []}, "dialEncoder": {DIAL_SERIAL: {"functionList": [
        {"btnFunc": "volume-control", "btnName": &KEYPAD_SERIAL[..11], "right": {}, "left": {}},
        {"btnFunc": "volume-control", "btnName": &KEYPAD_SERIAL[..11], "right": {}, "left": {}}
    ]}}});
    let Converted { profile, .. } = convert_one(&source);
    let names: Vec<_> = profile
        .dial_functions
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, [&KEYPAD_SERIAL[..11], "Function 2"]);
}

#[test]
fn r5_compatibility_forms_of_serials_are_scrubbed() {
    // Mathematical bold capitals and digits, and circled letters.
    let bold: String = BASE_SERIAL
        .chars()
        .map(|c| match c {
            'A'..='Z' => char::from_u32(0x1D400 + (c as u32 - 'A' as u32)).unwrap(),
            '0'..='9' => char::from_u32(0x1D7CE + (c as u32 - '0' as u32)).unwrap(),
            other => other,
        })
        .collect();
    let circled: String = BASE_SERIAL
        .chars()
        .map(|c| match c {
            'A'..='Z' => char::from_u32(0x24B6 + (c as u32 - 'A' as u32)).unwrap(),
            other => other,
        })
        .collect();
    for title in [bold, circled] {
        let source = profile_with(vec![page(vec![tile(
            0,
            "text",
            json!({"content": "a", "title": title}),
        )])]);
        let Converted { profile, .. } = convert_one(&source);
        assert!(
            matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1"),
            "{title}"
        );
    }
}

#[test]
fn r5_keys_without_text_never_get_the_text_budget_reason() {
    // Long labels use up the payload budget; shortcut keys still bind.
    let long = "\"".repeat(128);
    let children: Vec<Value> = (1..15)
        .map(|id| json!({"id": id, "btnFunc": "hot-key", "title": long, "hotkey": hotkey(&["Control", "C"])}))
        .collect();
    // Page 1: sequences of quote-heavy text nearly fill the budget; the
    // labels of the folders behind it push it over.
    let steps: Vec<Value> = (0..8)
        .map(|i| json!({"action": "text", "content": format!("{i}{}", "\"".repeat(1990))}))
        .collect();
    let mut first: Vec<Value> = (0..14)
        .map(|id| tile(id, "multi-tasking", json!({"actionList": steps})))
        .collect();
    first.push(tile(14, "next-page", json!({})));
    let mut pages = vec![page(first)];
    pages.extend((0..3).map(|_| {
        let mut tiles: Vec<Value> = (1..14)
            .map(|id| tile(id, "new-folder", json!({"title": long, "folder": children})))
            .collect();
        tiles.push(tile(14, "next-page", json!({})));
        page(tiles)
    }));
    let mut entry = item(
        "profile1_A",
        &serde_json::to_vec(&profile_with(pages)).unwrap(),
    );
    entry.deflate = false;
    let Converted { report, .. } = convert(&zip(&[entry])).unwrap().remove(0);
    assert!(
        !report
            .rows
            .iter()
            .any(|r| r.official_action == "hot-key" && r.reason.contains("KiB")),
        "{:?}",
        report.rows.iter().find(|r| r.reason.contains("KiB"))
    );
}

fn scanned() -> usize {
    mapping::SCANNED.with(std::cell::Cell::get)
}

#[test]
fn r6_serial_scans_are_bounded_for_hostile_strings() {
    // U+FDFA expands about 18 times under NFKC; these strings are each
    // about 60 KiB.
    let wide = "\u{FDFA}".repeat(20_000);
    let mut tiles: Vec<Value> = (0..12)
        .map(|id| tile(id, "text", json!({"content": "a", "title": wide})))
        .collect();
    tiles.push(tile(
        12,
        "web-url",
        json!({"url": format!("https://example.com/{wide}")}),
    ));
    tiles.push(tile(
        13,
        "hot-key",
        json!({"hotkey": {"key": ["Control", wide]}}),
    ));
    let mut source = profile_with(vec![page(tiles)]);
    source["profileName"] = json!(wide);
    let root = json::parse(&serde_json::to_vec(&source).unwrap()).unwrap();
    let before = scanned();
    let (profile, _) = mapping::map(&root, "A").unwrap();
    let total = scanned() - before;
    assert!(total < 64 * 1024, "{total} bytes scanned");
    // A serial right after the cut is still caught.
    let title = format!("{} {BASE_SERIAL}", "x".repeat(127));
    let source = profile_with(vec![page(vec![tile(
        0,
        "text",
        json!({"content": "a", "title": title}),
    )])]);
    let Converted { profile: cut, .. } = convert_one(&source);
    assert!(
        matches!(slot(&cut, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1")
    );
    assert!(profile.name.len() <= 128);
}

#[test]
fn r6_serials_stored_on_tiles_steps_and_folder_children_are_collected() {
    let serial = "MHXXTILE0042";
    let mut source = profile_with(vec![page(vec![
        tile(
            0,
            "text",
            json!({"content": "a", "title": format!("A {serial}"), "serialNumber": serial}),
        ),
        tile(
            1,
            "multi-tasking",
            json!({"actionList": [
            {"action": "text", "content": "x", "parentSerialNumber": "MHXXSTEP0043"},
            {"action": "hot-key", "hotkey": hotkey(&["A"])}
        ], "title": "B MHXXSTEP0043"}),
        ),
        tile(
            2,
            "new-folder",
            json!({"folder": [
                {"id": 1, "btnFunc": "text", "content": "y", "title": "C MHXXKIDS0044", "SerialNumber": "MHXXKIDS0044"}
            ]}),
        ),
    ])]);
    source["serialNumber"] = json!("MHXXBASE0001");
    let Converted { profile, report } = convert_one(&source);
    assert!(
        matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1")
    );
    assert!(
        matches!(slot(&profile, "root", 1), Some(KeypadTile::Action { label, .. }) if label == "Key 2")
    );
    assert!(
        matches!(slot(&profile, "root_key3", 1), Some(KeypadTile::Action { label, .. }) if label == "Key 2")
    );
    let text = serde_json::to_string(&profile).unwrap() + &serde_json::to_string(&report).unwrap();
    for leak in ["MHXXTILE0042", "MHXXSTEP0043", "MHXXKIDS0044"] {
        assert!(!text.contains(leak), "{leak}");
    }
}

#[test]
fn r6_the_final_fit_removes_about_what_is_needed() {
    let root =
        json::parse(&serde_json::to_vec(&many_text_tiles(&"q".repeat(1400), 26)).unwrap()).unwrap();
    let limit = 100 * 1024;
    let (profile, _) = mapping::map_with_limit(&root, "A", limit).unwrap();
    let size = serde_json::to_vec_pretty(&profile).unwrap().len();
    assert!(size <= limit - 1024);
    assert!(
        size > (limit - 1024) * 85 / 100,
        "{size}: removed far more than needed"
    );
}

#[test]
fn r7_a_serial_split_by_a_long_separator_run_at_the_cut_is_caught() {
    let title = format!("{} MHBS{}FAKE0001", "x".repeat(120), " ".repeat(600));
    let source = profile_with(vec![page(vec![tile(
        0,
        "text",
        json!({"content": "a", "title": title}),
    )])]);
    let Converted { profile, .. } = convert_one(&source);
    assert!(
        matches!(slot(&profile, "root", 0), Some(KeypadTile::Action { label, .. }) if label == "Key 1")
    );
}
