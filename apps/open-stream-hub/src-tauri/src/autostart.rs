//! Start OpenStreamHub when the user logs in, through a per-user LaunchAgent
//! (`~/Library/LaunchAgents/dev.cooltek.openstreamhub.plist`). The agent only
//! runs `open -a <this app bundle>` once at login, so it needs no signature,
//! helper or extra permission, and turning it off deletes the file.
//!
//! It is offered only when the running binary sits inside an `.app` bundle: a
//! development build (`target/debug/...`) has nothing a login item could open.

use serde::Serialize;
use std::path::{Path, PathBuf};

pub const LABEL: &str = "dev.cooltek.openstreamhub";

/// What Settings shows: the real state of the login item, and whether this
/// copy of the app can have one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AutostartStatus {
    pub enabled: bool,
    pub available: bool,
    /// Why it is unavailable, for the UI to say; empty when available.
    pub reason: String,
}

/// The `.app` bundle that contains `exe` (`X.app/Contents/MacOS/binary`).
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let in_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    in_bundle.then(|| bundle.to_path_buf())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The LaunchAgent property list for one app bundle.
pub fn plist_for(bundle: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>{LABEL}</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>/usr/bin/open</string>\n\
\t\t<string>-a</string>\n\
\t\t<string>{}</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
</dict>\n\
</plist>\n",
        escape(&bundle.to_string_lossy())
    )
}

fn agent_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

fn status_in(home: &Path, exe: &Path) -> AutostartStatus {
    let enabled = agent_path(home).is_file();
    match bundle_of(exe) {
        Some(_) => AutostartStatus {
            enabled,
            available: true,
            reason: String::new(),
        },
        None => AutostartStatus {
            enabled,
            available: false,
            reason: "Starting at login works from the installed app, not from a development build."
                .into(),
        },
    }
}

fn set_in(home: &Path, exe: &Path, enabled: bool) -> Result<AutostartStatus, String> {
    let path = agent_path(home);
    if enabled {
        let bundle = bundle_of(exe).ok_or_else(|| {
            "Starting at login works from the installed app, not from a development build."
                .to_string()
        })?;
        let directory = path.parent().ok_or("the LaunchAgents folder is unknown")?;
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let temporary = path.with_extension("plist.tmp");
        std::fs::write(&temporary, plist_for(&bundle))
            .and_then(|()| std::fs::rename(&temporary, &path))
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    } else if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        return Err(format!("could not remove {}: {error}", path.display()));
    }
    Ok(status_in(home, exe))
}

fn environment() -> Result<(PathBuf, PathBuf), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or("the home folder is unknown")?;
    let exe =
        std::env::current_exe().map_err(|error| format!("could not find the app: {error}"))?;
    Ok((home, exe))
}

pub fn status() -> Result<AutostartStatus, String> {
    let (home, exe) = environment()?;
    Ok(status_in(&home, &exe))
}

pub fn set(enabled: bool) -> Result<AutostartStatus, String> {
    if !cfg!(target_os = "macos") {
        return Err("starting at login is supported on macOS only".into());
    }
    let (home, exe) = environment()?;
    set_in(&home, &exe, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALLED: &str = "/Applications/OpenStreamHub.app/Contents/MacOS/open-stream-hub-app";

    #[test]
    fn finds_the_bundle_only_for_an_installed_app() {
        assert_eq!(
            bundle_of(Path::new(INSTALLED)),
            Some(PathBuf::from("/Applications/OpenStreamHub.app"))
        );
        assert_eq!(
            bundle_of(Path::new(
                "/repo/src-tauri/target/debug/open-stream-hub-app"
            )),
            None
        );
        assert_eq!(bundle_of(Path::new("/x/Foo.app/Resources/bin")), None);
    }

    #[test]
    fn the_agent_opens_the_bundle_once_at_login_and_escapes_the_path() {
        let plist = plist_for(Path::new("/Apps/Tom & Jerry <1>.app"));
        assert!(plist.contains("<string>/Apps/Tom &amp; Jerry &lt;1&gt;.app</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(!plist.contains("KeepAlive"));
        assert!(plist.contains(&format!("<string>{LABEL}</string>")));
    }

    #[test]
    fn turning_it_on_and_off_writes_and_removes_the_file() {
        let home = tempfile::tempdir().unwrap();
        let exe = Path::new(INSTALLED);
        assert!(!status_in(home.path(), exe).enabled);
        let on = set_in(home.path(), exe, true).unwrap();
        assert!(on.enabled && on.available);
        let written = std::fs::read_to_string(agent_path(home.path())).unwrap();
        assert!(written.contains("/Applications/OpenStreamHub.app"));
        // Off twice: removing a missing file is not an error.
        assert!(!set_in(home.path(), exe, false).unwrap().enabled);
        assert!(!set_in(home.path(), exe, false).unwrap().enabled);
    }

    #[test]
    fn a_development_build_cannot_turn_it_on() {
        let home = tempfile::tempdir().unwrap();
        let dev = Path::new("/repo/target/debug/open-stream-hub-app");
        let status = status_in(home.path(), dev);
        assert!(!status.available && !status.reason.is_empty());
        assert!(set_in(home.path(), dev, true).is_err());
        assert!(!agent_path(home.path()).exists());
    }
}
