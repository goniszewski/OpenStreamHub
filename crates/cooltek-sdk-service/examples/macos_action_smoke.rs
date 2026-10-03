//! Explicit, device-free checks for application launch and a scratch keystroke.
//! Focus a disposable TextEdit document during `type-marker`'s five-second delay.

use cooltek_sdk_service::{Action, ActionDispatcher, macos::MacOsDispatcher};
use serde_json::json;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("macos_action_smoke: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let (execute, command) = match arguments.as_slice() {
        [command] => (false, command.as_str()),
        [flag, command] if flag == "--execute" => (true, command.as_str()),
        _ => {
            return Err("usage: macos_action_smoke [--execute] launch-textedit|type-marker".into());
        }
    };
    let action = match command {
        "launch-textedit" => Action::LaunchApp {
            bundle_id: "com.apple.TextEdit".into(),
        },
        "type-marker" => Action::Shortcut {
            key: "m".into(),
            modifiers: vec![],
        },
        _ => return Err("expected launch-textedit or type-marker".into()),
    };
    action.validate()?;
    println!("{}", json!({"type":"action_planned", "action":action}));
    if execute {
        if command == "type-marker" {
            eprintln!(
                "Focus a disposable TextEdit document now; the marker is sent in five seconds."
            );
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        MacOsDispatcher::new().dispatch(&action)?;
        println!("{}", json!({"type":"action_completed", "action":action}));
    }
    Ok(())
}
