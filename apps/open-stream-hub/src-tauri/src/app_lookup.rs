//! Picking an app for the "Open app" action and showing its everyday name.
//! The picker is the system's own file dialog (through `osascript`), opened on
//! the Applications folder; the name comes from Spotlight's record of the app
//! (`mdfind`, `mdls`), with the bundle's own `Info.plist` and then the file
//! name as fallbacks. Nothing here launches or changes an app.

use serde::Serialize;
use std::path::Path;
use std::process::{Command, Stdio};

const OSASCRIPT: &str = "/usr/bin/osascript";
const MDFIND: &str = "/usr/bin/mdfind";
const MDLS: &str = "/usr/bin/mdls";
const PLUTIL: &str = "/usr/bin/plutil";

const PICKER_SCRIPT: &str = "POSIX path of (choose file of type {\"com.apple.application-bundle\"} with prompt \"Choose the app to open\" default location (path to applications folder))";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    pub bundle_id: String,
    pub name: String,
}

/// A bundle id is made of letters, digits, dots, hyphens and underscores; anything else is never looked up.
pub fn valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 300
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The name Finder shows: the bundle's file name without `.app`.
pub fn name_from_path(path: &str) -> Option<String> {
    let stem = Path::new(path.trim().trim_end_matches('/'))
        .file_name()?
        .to_str()?;
    let stem = stem.strip_suffix(".app").unwrap_or(stem).trim();
    (!stem.is_empty()).then(|| stem.to_owned())
}

/// `mdls -raw` prints `(null)` for a missing attribute.
fn attribute(path: &str, name: &str) -> Option<String> {
    let out = Command::new(MDLS)
        .args(["-raw", "-name", name, path])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty() && text != "(null)").then_some(text)
}

fn plist_value(path: &str, key: &str) -> Option<String> {
    let plist = format!("{}/Contents/Info.plist", path.trim_end_matches('/'));
    let out = Command::new(PLUTIL)
        .args(["-extract", key, "raw", "-o", "-", &plist])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// The id and everyday name of the app at `path`.
pub fn info_at(path: &str) -> Option<AppInfo> {
    let bundle_id = plist_value(path, "CFBundleIdentifier")
        .or_else(|| attribute(path, "kMDItemCFBundleIdentifier"))?;
    if !valid_bundle_id(&bundle_id) {
        return None;
    }
    let name = attribute(path, "kMDItemDisplayName")
        .map(|name| name.strip_suffix(".app").unwrap_or(&name).to_owned())
        .or_else(|| name_from_path(path))
        .unwrap_or_else(|| bundle_id.clone());
    Some(AppInfo { bundle_id, name })
}

/// The system's file dialog on Applications. `None` when the user cancels or the choice is not an app.
pub fn choose() -> Result<Option<AppInfo>, String> {
    let out = Command::new(OSASCRIPT)
        .args(["-e", PICKER_SCRIPT])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not open the app picker: {error}"))?;
    if !out.status.success() {
        let message = String::from_utf8_lossy(&out.stderr);
        // -128 is "User canceled".
        if message.contains("-128") {
            return Ok(None);
        }
        return Err("Could not open the app picker. Type the bundle ID instead.".into());
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(info_at(&path))
}

/// The everyday name of an installed app, or `None` when Spotlight does not know the id.
pub fn name_for(bundle_id: &str) -> Option<String> {
    if !valid_bundle_id(bundle_id) {
        return None;
    }
    let out = Command::new(MDFIND)
        .arg(format!("kMDItemCFBundleIdentifier == \"{bundle_id}\""))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let listing = String::from_utf8(out.stdout).ok()?;
    let path = listing
        .lines()
        .find(|line| line.ends_with(".app") || line.ends_with(".app/"))?;
    info_at(path).map(|info| info.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_ids_are_plain() {
        assert!(valid_bundle_id("com.apple.finder"));
        assert!(valid_bundle_id("org.my-app_2.Tool"));
        assert!(!valid_bundle_id(""));
        assert!(!valid_bundle_id("com.apple.finder\" || 1"));
        assert!(!valid_bundle_id("a b"));
    }

    #[test]
    fn names_come_from_the_file_name() {
        assert_eq!(
            name_from_path("/Applications/Visual Studio Code.app/\n").as_deref(),
            Some("Visual Studio Code")
        );
        assert_eq!(
            name_from_path("/Applications/Safari.app").as_deref(),
            Some("Safari")
        );
        assert_eq!(name_from_path("/"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_is_found() {
        assert_eq!(name_for("com.apple.finder").as_deref(), Some("Finder"));
        assert_eq!(name_for("not a real id"), None);
    }
}
