use cooltek_sdk_service::Profile;
use serde_json::{Value, json};

pub fn folders_json() -> Value {
    let mut root = vec![Value::Null; 15];
    let mut tools = vec![Value::Null; 15];
    let mut deeper = vec![Value::Null; 15];
    root[0] = json!({ "type": "folder", "page_id": "tools" });
    root[14] = json!({ "type": "action", "label": "Root action", "bindings": [
        { "gesture": "pressed", "action": { "type": "volume", "delta": 7 } },
        { "gesture": "released", "action": { "type": "toggle_mute" } }
    ] });
    tools[1] = json!({ "type": "folder", "page_id": "deeper" });
    tools[2] = json!({ "type": "action", "label": "Calculator", "bindings": [
        { "gesture": "pressed", "action": { "type": "launch_app", "bundle_id": "com.apple.calculator" } },
        { "gesture": "pressed", "serial_number": "unit-1", "action": { "type": "volume", "delta": 11 } },
        { "gesture": "released", "action": { "type": "volume", "delta": -2 } }
    ] });
    deeper[1] = json!({ "type": "action", "label": "Paste", "bindings": [
        { "gesture": "pressed", "action": { "type": "shortcut", "key": "v", "modifiers": ["command"] } }
    ] });
    json!({
        "schema_version": 2, "name": "Folders",
        "bindings": [{ "module": "m003k", "control": 0, "gesture": "clockwise",
            "action": { "type": "volume", "delta": 5 } }],
        "keypad": { "pages": [
            { "id": "root", "name": "Home", "slots": root },
            { "id": "tools", "name": "Tools", "slots": tools },
            { "id": "deeper", "name": "Deeper", "slots": deeper }
        ] }
    })
}

pub fn folders() -> Profile {
    Profile::from_json(&serde_json::to_vec(&folders_json()).unwrap()).unwrap()
}
