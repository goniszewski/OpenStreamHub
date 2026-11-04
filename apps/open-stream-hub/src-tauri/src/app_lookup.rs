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

/// What the picker's `osascript` run means: the chosen app, a cancelled dialog (`None`), or a failure.
fn picked(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<Option<AppInfo>, String> {
    if !success {
        let message = String::from_utf8_lossy(stderr);
        // -128 is "User canceled".
        if message.contains("-128") {
            return Ok(None);
        }
        return Err("Could not open the app picker. Type the bundle ID instead.".into());
    }
    let path = String::from_utf8_lossy(stdout).trim().to_owned();
    Ok(info_at(&path))
}

/// The system's file dialog on Applications. `None` when the user cancels or the choice is not an app.
pub fn choose() -> Result<Option<AppInfo>, String> {
    let out = Command::new(OSASCRIPT)
        .args(["-e", PICKER_SCRIPT])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not open the app picker: {error}"))?;
    picked(out.status.success(), &out.stdout, &out.stderr)
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

    #[test]
    fn a_cancelled_picker_is_no_choice_and_other_failures_are_errors() {
        let cancelled = b"execution error: User canceled. (-128)";
        assert_eq!(picked(false, b"", cancelled), Ok(None));
        assert!(picked(false, b"", b"some other failure").is_err());
        assert!(picked(false, b"", b"").is_err());
    }

    #[test]
    fn a_pick_that_is_not_an_app_is_no_choice() {
        assert_eq!(picked(true, b"", b""), Ok(None));
        assert_eq!(picked(true, b"/no/such/place.app/\n", b""), Ok(None));
    }

    #[test]
    fn names_ignore_trailing_slashes_and_blank_stems() {
        assert_eq!(
            name_from_path("/Applications/Safari.app/").as_deref(),
            Some("Safari")
        );
        assert_eq!(name_from_path("/Applications/.app"), None);
        assert_eq!(name_from_path(""), None);
        assert_eq!(
            name_from_path("/Applications/Plain").as_deref(),
            Some("Plain")
        );
    }

    #[test]
    fn bundle_ids_have_a_length_limit() {
        assert!(valid_bundle_id(&"a".repeat(300)));
        assert!(!valid_bundle_id(&"a".repeat(301)));
    }

    #[cfg(target_os = "macos")]
    mod bundles {
        use super::*;
        use std::fs;

        /// A throwaway `.app` folder with an `Info.plist` holding `body`, removed again when dropped.
        struct Fake(std::path::PathBuf);
        impl Fake {
            fn new(stem: &str, plist: Option<&str>) -> Self {
                let root = std::env::temp_dir()
                    .join(format!("osh-app-lookup-{}-{stem}", std::process::id()));
                let app = root.join(format!("{stem}.app"));
                fs::create_dir_all(app.join("Contents")).unwrap();
                if let Some(body) = plist {
                    fs::write(app.join("Contents/Info.plist"), body).unwrap();
                }
                Fake(app)
            }
            fn path(&self) -> &str {
                self.0.to_str().unwrap()
            }
        }
        impl Drop for Fake {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(self.0.parent().unwrap());
            }
        }

        fn plist(id: &str) -> String {
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>CFBundleIdentifier</key><string>{id}</string></dict></plist>"
            )
        }

        #[test]
        fn a_bundle_gives_its_id_and_falls_back_to_the_file_name() {
            let app = Fake::new("Zorblax Unit", Some(&plist("test.osh.zorblax")));
            let info = info_at(app.path()).expect("bundle");
            assert_eq!(info.bundle_id, "test.osh.zorblax");
            // Spotlight has not indexed a throwaway bundle, so the name is the file's own.
            assert_eq!(info.name, "Zorblax Unit");
        }

        #[test]
        fn a_bundle_without_a_plist_or_with_a_bad_id_is_not_an_app() {
            let bare = Fake::new("Bare", None);
            assert_eq!(info_at(bare.path()), None);
            let odd = Fake::new("Odd", Some(&plist("not a valid id")));
            assert_eq!(info_at(odd.path()), None);
            assert_eq!(info_at("/no/such/place.app"), None);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn finder_is_found() {
        assert_eq!(name_for("com.apple.finder").as_deref(), Some("Finder"));
        assert_eq!(name_for("not a real id"), None);
    }
}
