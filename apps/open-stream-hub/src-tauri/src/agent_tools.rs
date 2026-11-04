//! The "AI agents" panel in Settings: finds the `osh` bundled next to the app, links it as `~/.local/bin/osh` so a
//! terminal can call it, and shows the hook configuration `osh agent setup` would print.
//!
//! Only one file is ever written or removed: the `osh` symbolic link in `~/.local/bin`, and only when it is missing
//! or already points at an OpenStreamHub `osh`. A regular file, or a link to anything else, is left alone. The
//! agents' own settings files are never touched; the panel shows what to paste into them.

use serde::Serialize;
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

/// The name of the bundled tool and of the link to it.
const TOOL: &str = "osh";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentToolStatus {
    /// The bundled `osh`, when it exists, is executable and is not translocated.
    pub osh_path: Option<String>,
    /// macOS runs the app from an App Translocation copy (a quarantined app opened where it was downloaded): its
    /// path changes at every launch, so no link or hook is offered for it. `osh_path` is then empty.
    pub translocated: bool,
    /// Where the link goes: `~/.local/bin/osh`, as an absolute path.
    pub link_path: String,
    /// The link exists and points at the bundled `osh`.
    pub linked: bool,
    /// The link's directory is on the app's own `PATH`. A terminal's login shell may differ, so false is not proof.
    pub on_path: bool,
    /// The Claude Code and Codex hook configurations for the bundled `osh`; empty when it is not found.
    pub claude_config: String,
    pub codex_config: String,
}

/// What is at the link path now.
#[derive(Debug, PartialEq)]
enum LinkState {
    Missing,
    /// A link to the bundled `osh`.
    Current,
    /// A link to the `osh` of an OpenStreamHub app bundle, but not this one (a moved or older copy of the app).
    OtherBundle,
    /// A link to something else, or not a link at all: never replaced or removed.
    Foreign(String),
}

/// The bundled `osh` in `exe_dir`, resolved, when it is an executable file.
fn bundled_osh(exe_dir: &Path) -> Option<PathBuf> {
    let path = exe_dir.join(TOOL).canonicalize().ok()?;
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() || !is_executable(&metadata) {
        return None;
    }
    Some(path)
}

#[cfg(unix)]
fn is_executable(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_: &std::fs::Metadata) -> bool {
    false
}

/// `~/.local/bin/osh` under `home`.
fn link_path(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join(TOOL)
}

/// Whether `target` is the `osh` inside an OpenStreamHub app bundle: `…/OpenStreamHub*.app/Contents/MacOS/osh`.
fn is_bundle_osh(target: &Path) -> bool {
    let names: Vec<&OsStr> = target.iter().collect();
    let [.., app, contents, macos, tool] = names.as_slice() else {
        return false;
    };
    let app = app.to_string_lossy();
    *tool == TOOL
        && *macos == "MacOS"
        && *contents == "Contents"
        && app.starts_with("OpenStreamHub")
        && app.ends_with(".app")
}

fn classify(link: &Path, bundled: Option<&Path>) -> io::Result<LinkState> {
    let metadata = match link.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(LinkState::Missing),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_symlink() {
        return Ok(LinkState::Foreign(format!(
            "{} exists and is not a link, so it was left alone",
            link.display()
        )));
    }
    let target = std::fs::read_link(link)?;
    // A relative target is relative to the link's own directory.
    let target = match link.parent() {
        Some(parent) if target.is_relative() => parent.join(target),
        _ => target,
    };
    let resolved = target.canonicalize().ok();
    if bundled.is_some_and(|bundled| target == bundled || resolved.as_deref() == Some(bundled)) {
        return Ok(LinkState::Current);
    }
    if is_bundle_osh(&target) || resolved.as_deref().is_some_and(is_bundle_osh) {
        return Ok(LinkState::OtherBundle);
    }
    Ok(LinkState::Foreign(format!(
        "{} already links to {}, so it was left alone",
        link.display(),
        target.display()
    )))
}

/// Whether `dir` is one of the directories in `path_var` (a `PATH` value).
fn dir_on_path(dir: &Path, path_var: Option<&OsStr>) -> bool {
    let Some(path_var) = path_var else {
        return false;
    };
    let wanted = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    std::env::split_paths(path_var)
        .any(|entry| entry == dir || entry.canonicalize().is_ok_and(|entry| entry == wanted))
}

/// The bundled `osh` unless it is translocated, and whether it is.
fn offered_osh(exe_dir: Option<&Path>) -> (Option<PathBuf>, bool) {
    let found = exe_dir.and_then(bundled_osh);
    let translocated = found
        .as_deref()
        .or(exe_dir)
        .is_some_and(|path| osh::is_translocated(&path.to_string_lossy()));
    (found.filter(|_| !translocated), translocated)
}

fn status_for(exe_dir: Option<&Path>, home: &Path, path_var: Option<&OsStr>) -> AgentToolStatus {
    let (bundled, translocated) = offered_osh(exe_dir);
    let link = link_path(home);
    let linked = matches!(classify(&link, bundled.as_deref()), Ok(LinkState::Current));
    let on_path = link.parent().is_some_and(|dir| dir_on_path(dir, path_var));
    let config = |agent| {
        bundled
            .as_deref()
            .map(|osh| osh::setup(agent, &osh.to_string_lossy()))
            .unwrap_or_default()
    };
    AgentToolStatus {
        osh_path: bundled
            .as_deref()
            .map(|path| path.to_string_lossy().into_owned()),
        translocated,
        link_path: link.to_string_lossy().into_owned(),
        linked,
        on_path,
        claude_config: config(osh::Agent::Claude),
        codex_config: config(osh::Agent::Codex),
    }
}

/// Creates `link` pointing at `bundled`, or repoints a link to another OpenStreamHub `osh`.
fn create_link(link: &Path, bundled: &Path) -> Result<(), String> {
    let dir = link.parent().ok_or("the link has no directory")?;
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o755);
    builder
        .create(dir)
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    match classify(link, Some(bundled)).map_err(|error| error.to_string())? {
        LinkState::Current => return Ok(()),
        LinkState::Missing => {}
        // Checked a moment ago to be a link to an OpenStreamHub `osh`; remove_file never removes a directory.
        LinkState::OtherBundle => std::fs::remove_file(link)
            .map_err(|error| format!("could not replace {}: {error}", link.display()))?,
        LinkState::Foreign(reason) => return Err(reason),
    }
    symlink(bundled, link).map_err(|error| format!("could not create {}: {error}", link.display()))
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "links are made on macOS only",
    ))
}

/// Removes `link` when it points at an OpenStreamHub `osh`; anything else is left alone.
fn remove_link(link: &Path, bundled: Option<&Path>) -> Result<(), String> {
    match classify(link, bundled).map_err(|error| error.to_string())? {
        LinkState::Missing => Ok(()),
        LinkState::Current | LinkState::OtherBundle => std::fs::remove_file(link)
            .map_err(|error| format!("could not remove {}: {error}", link.display())),
        LinkState::Foreign(reason) => Err(reason),
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "could not find your home folder".to_string())
}

pub fn status() -> Result<AgentToolStatus, String> {
    Ok(status_for(
        exe_dir().as_deref(),
        &home()?,
        std::env::var_os("PATH").as_deref(),
    ))
}

pub fn link() -> Result<AgentToolStatus, String> {
    let home = home()?;
    let (bundled, translocated) = offered_osh(exe_dir().as_deref());
    if translocated {
        return Err(format!("{}.", osh::TRANSLOCATED));
    }
    let bundled = bundled.ok_or("the bundled osh was not found; reinstall the app")?;
    create_link(&link_path(&home), &bundled)?;
    status()
}

pub fn unlink() -> Result<AgentToolStatus, String> {
    let home = home()?;
    let bundled = offered_osh(exe_dir().as_deref()).0;
    remove_link(&link_path(&home), bundled.as_deref())?;
    status()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    /// A fake app bundle with an executable `osh`, and a home folder, in a temporary directory.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let macos = root
            .path()
            .join("Applications/OpenStreamHub.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let osh = macos.join("osh");
        std::fs::write(&osh, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&osh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        (root, macos, home)
    }

    #[test]
    fn finds_only_an_executable_bundled_osh() {
        let (_root, macos, _home) = setup();
        let osh = macos.join("osh").canonicalize().unwrap();
        assert_eq!(bundled_osh(&macos), Some(osh.clone()));
        std::fs::set_permissions(&osh, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(bundled_osh(&macos), None);
        std::fs::remove_file(&osh).unwrap();
        assert_eq!(bundled_osh(&macos), None);
    }

    #[test]
    fn recognises_bundle_paths() {
        assert!(is_bundle_osh(Path::new(
            "/Applications/OpenStreamHub.app/Contents/MacOS/osh"
        )));
        assert!(is_bundle_osh(Path::new(
            "/Users/a/Downloads/OpenStreamHub 2.app/Contents/MacOS/osh"
        )));
        assert!(!is_bundle_osh(Path::new(
            "/Applications/Other.app/Contents/MacOS/osh"
        )));
        assert!(!is_bundle_osh(Path::new(
            "/Applications/OpenStreamHub.app/Contents/MacOS/other"
        )));
        assert!(!is_bundle_osh(Path::new("/usr/local/bin/osh")));
        assert!(!is_bundle_osh(Path::new("osh")));
    }

    #[test]
    fn links_creating_the_directory_and_unlinks() {
        let (_root, macos, home) = setup();
        let status = status_for(Some(&macos), &home, None);
        assert!(!status.linked);
        assert!(status.osh_path.is_some());
        let link = link_path(&home);
        assert_eq!(status.link_path, link.to_string_lossy());

        let bundled = bundled_osh(&macos).unwrap();
        create_link(&link, &bundled).unwrap();
        let mode = std::fs::metadata(link.parent().unwrap())
            .unwrap()
            .permissions()
            .mode();
        // 0755 less the umask.
        assert_eq!(mode & 0o700, 0o700);
        assert_eq!(mode & 0o022, 0);
        assert_eq!(std::fs::read_link(&link).unwrap(), bundled);
        assert!(status_for(Some(&macos), &home, None).linked);
        // Linking again changes nothing.
        create_link(&link, &bundled).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), bundled);

        remove_link(&link, Some(&bundled)).unwrap();
        assert!(link.symlink_metadata().is_err());
        // Nothing to remove is not an error.
        remove_link(&link, Some(&bundled)).unwrap();
        assert!(!status_for(Some(&macos), &home, None).linked);
    }

    #[test]
    fn replaces_a_link_to_another_openstreamhub_bundle() {
        let (root, macos, home) = setup();
        let bundled = bundled_osh(&macos).unwrap();
        let link = link_path(&home);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        // A copy of the app that was moved away: the target does not even exist.
        let old = root.path().join("Old/OpenStreamHub.app/Contents/MacOS/osh");
        symlink(&old, &link).unwrap();
        assert_eq!(
            classify(&link, Some(&bundled)).unwrap(),
            LinkState::OtherBundle
        );
        create_link(&link, &bundled).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), bundled);

        // An old link is also removable from Settings.
        std::fs::remove_file(&link).unwrap();
        symlink(&old, &link).unwrap();
        remove_link(&link, Some(&bundled)).unwrap();
        assert!(link.symlink_metadata().is_err());
    }

    #[test]
    fn leaves_a_regular_file_and_a_foreign_link_alone() {
        let (root, macos, home) = setup();
        let bundled = bundled_osh(&macos).unwrap();
        let link = link_path(&home);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();

        std::fs::write(&link, "mine").unwrap();
        assert!(
            create_link(&link, &bundled)
                .unwrap_err()
                .contains("not a link")
        );
        assert!(remove_link(&link, Some(&bundled)).is_err());
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "mine");
        std::fs::remove_file(&link).unwrap();

        let other = root.path().join("other-osh");
        std::fs::write(&other, "other").unwrap();
        symlink(&other, &link).unwrap();
        assert!(
            create_link(&link, &bundled)
                .unwrap_err()
                .contains("already links")
        );
        assert!(remove_link(&link, Some(&bundled)).is_err());
        assert_eq!(std::fs::read_link(&link).unwrap(), other);
        assert!(!status_for(Some(&macos), &home, None).linked);
    }

    #[test]
    fn a_relative_link_to_the_bundled_osh_counts() {
        let (root, macos, home) = setup();
        let bundled = bundled_osh(&macos).unwrap();
        let link = link_path(&home);
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        let relative = Path::new("../../../Applications/OpenStreamHub.app/Contents/MacOS/osh");
        assert!(link.parent().unwrap().join(relative).exists(), "{root:?}");
        symlink(relative, &link).unwrap();
        assert_eq!(classify(&link, Some(&bundled)).unwrap(), LinkState::Current);
    }

    #[test]
    fn reports_whether_the_link_directory_is_on_path() {
        let (_root, macos, home) = setup();
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let path = std::env::join_paths([Path::new("/usr/bin"), &bin]).unwrap();
        assert!(status_for(Some(&macos), &home, Some(&path)).on_path);
        let path = std::env::join_paths([Path::new("/usr/bin"), Path::new("/bin")]).unwrap();
        assert!(!status_for(Some(&macos), &home, Some(&path)).on_path);
        assert!(!status_for(Some(&macos), &home, None).on_path);
    }

    #[test]
    fn the_configurations_match_what_osh_agent_setup_prints() {
        let (_root, macos, home) = setup();
        let status = status_for(Some(&macos), &home, None);
        let osh = status.osh_path.clone().unwrap();
        assert_eq!(status.claude_config, osh::setup(osh::Agent::Claude, &osh));
        assert_eq!(status.codex_config, osh::setup(osh::Agent::Codex, &osh));
        assert!(status.claude_config.contains(&format!("{osh} agent ask")));

        let missing = status_for(None, &home, None);
        assert_eq!(missing.osh_path, None);
        assert_eq!(missing.claude_config, "");
        assert_eq!(missing.codex_config, "");
    }

    #[test]
    fn a_translocated_copy_offers_no_osh() {
        let root = tempfile::tempdir().unwrap();
        let macos = root
            .path()
            .join("AppTranslocation/1234/d/OpenStreamHub.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let osh = macos.join("osh");
        std::fs::write(&osh, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&osh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let home = root.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let status = status_for(Some(&macos), &home, None);
        assert!(status.translocated);
        assert_eq!(status.osh_path, None);
        assert_eq!(status.claude_config, "");
        assert_eq!(status.codex_config, "");
        // Not translocated in the usual place.
        let (_root, macos, home) = setup();
        let status = status_for(Some(&macos), &home, None);
        assert!(!status.translocated);
        assert!(status.osh_path.is_some());
    }
}
