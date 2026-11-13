//! Installing, updating and removing a plugin from Settings > Plugins (step 6a in `docs/plugins-design.md`).
//!
//! The user picks the plugin's folder wherever it is (the system's folder dialog, through `osascript`). The app reads
//! its `plugin.json` by the usual rules except the folder name, counts what it holds, and shows the review; nothing is
//! copied until the user confirms. Installing copies the folder into a hidden staging folder in the plugins folder
//! (`.install-<random>/<id>`, which the catalog skips), checks the copy with [`plugins::load`], and renames it into
//! place as `<id>`. A failure at any point removes the staging folder, so the plugins folder never holds half a plugin.
//!
//! The copy holds only regular files and folders: a symbolic link, a device, a pipe or a socket anywhere refuses the
//! install and names it. While copying, every file and folder is opened from its parent folder's descriptor with
//! `O_NOFOLLOW`, so a folder swapped for a link after the scan is refused, not followed, and the picked folder must be
//! the one scanned (device and inode). A hard link (a regular file with other names) is refused, naming it, since it
//! may be a name for a file the user does not own. An error after the copy is in place (the approval could not be
//! written) starts with `installed: ` and reads on its own after it; any other error means nothing was installed.
//! `.git` folders are skipped; other hidden files are copied. At most [`MAX_ENTRIES`] files and folders and [`MAX_BYTES`] are copied, counted before copying and again while copying (a file is read
//! only up to what is left of the limit, so one growing meanwhile cannot pass it). Every copy belongs to the user and
//! is writable by nobody else (`mode & 0o755`, so execute bits stay and set-id bits go; folders also keep `u+rwx`).
//! File data and extended attributes are copied, so macOS's quarantine flag stays; ACLs are not. The source folder is
//! never changed.
//!
//! An update stops the running plugin first and waits until none of its processes is left (its pending presses fail,
//! as on disable), renames the old folder aside (`.old-<id>-<random>`), renames the new one in and deletes the old
//! one; if the rename in fails, the old folder is put back. Anything at `plugins/<id>` that is not a real folder (a
//! link, a file) refuses the install. A plain Install keeps the plugin enabled only when the copy installed now has
//! identical `plugin.json` bytes and is enabled; in every other case its approval is removed. Installs and removes run
//! one at a time. After an interrupted one (a crash, a power cut), an installed copy that was moved aside is put back
//! when `<id>` is missing, and every other leftover is deleted, at app start and before each install or remove.

use crate::{
    plugin_host::PluginHost,
    plugin_secrets::PluginSecrets,
    plugins::{self, Approval, Approvals, Approved, Installed, Listed, MANIFEST_FILE, Manifest},
};
use cooltek_sdk_service::plugin::{parse_version, validate_plugin_id};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs, io,
    os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};

const OSASCRIPT: &str = "/usr/bin/osascript";
const PICKER_SCRIPT: &str =
    "POSIX path of (choose folder with prompt \"Choose the plugin's folder (unzip a .zip first)\")";

/// Most files and folders one plugin may hold.
pub const MAX_ENTRIES: usize = if cfg!(test) { 20 } else { 5_000 };
/// Most bytes one plugin's files may hold together.
pub const MAX_BYTES: u64 = if cfg!(test) {
    64 * 1024
} else {
    256 * 1024 * 1024
};

const STAGING_PREFIX: &str = ".install-";
const ASIDE_PREFIX: &str = ".old-";
/// A folder being removed, deleted (never restored) when found later.
const REMOVED_PREFIX: &str = ".removed-";
const CHANGED: &str = "plugin.json changed since you reviewed it; review it again";

/// One install or remove at a time.
static LOCK: Mutex<()> = Mutex::new(());

/// What the install review shows.
#[derive(Debug, Serialize)]
pub struct InstallReview {
    /// The folder that will be copied (its real path).
    pub path: String,
    /// The plugin as Settings would list it once installed: `folder` is the id it is installed as, `approval` and
    /// `secrets` are what installing gives it.
    pub plugin: Listed,
    /// Regular files that will be copied.
    pub files: usize,
    /// Their size together.
    pub bytes: u64,
    /// The plugin this replaces, when the id is installed already.
    pub installed: Option<Replaced>,
    /// Whether the new version is older than the installed one.
    pub older: bool,
}

/// The installed copy an install would replace.
#[derive(Debug, Serialize, PartialEq)]
pub struct Replaced {
    /// `None` when its manifest cannot be used.
    pub version: Option<String>,
    pub approval: Option<Approval>,
    /// Whether its `plugin.json` has exactly the new one's bytes, so its approval carries over.
    pub same_manifest: bool,
}

/// What the folder dialog's `osascript` run means: the chosen folder, a cancelled dialog (`None`), or a failure.
fn picked(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<Option<String>, String> {
    if !success {
        // -128 is "User canceled".
        if String::from_utf8_lossy(stderr).contains("-128") {
            return Ok(None);
        }
        return Err("Could not open the folder picker.".into());
    }
    let path = String::from_utf8_lossy(stdout)
        .trim_end_matches('\n')
        .to_owned();
    Ok((!path.is_empty()).then_some(path))
}

/// The system's folder dialog. `None` when the user cancels.
pub fn choose_folder() -> Result<Option<String>, String> {
    let out = Command::new(OSASCRIPT)
        .args(["-e", PICKER_SCRIPT])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("Could not open the folder picker: {error}"))?;
    picked(out.status.success(), &out.stdout, &out.stderr)
}

/// What the folder holds: every file and folder below it (relative paths, parents before their contents), whether
/// each is a folder, and the regular files' count and size.
#[derive(Default)]
struct Scan {
    entries: Vec<(PathBuf, bool)>,
    files: usize,
    bytes: u64,
}

fn too_many() -> String {
    format!(
        "the folder holds more than {MAX_ENTRIES} files and folders; choose the plugin's own folder"
    )
}

fn too_large() -> String {
    format!(
        "the folder is larger than {} MiB; choose the plugin's own folder",
        MAX_BYTES / (1024 * 1024)
    )
}

/// Why an entry that is neither a regular file nor a folder refuses the install.
fn special(path: &Path, kind: fs::FileType) -> String {
    let what = if kind.is_symlink() {
        "a symbolic link"
    } else if kind.is_fifo() {
        "a named pipe"
    } else if kind.is_socket() {
        "a socket"
    } else if kind.is_block_device() || kind.is_char_device() {
        "a device"
    } else {
        "a special file"
    };
    format!(
        "{} is {what}; a plugin may hold only files and folders",
        path.display()
    )
}

/// Why `path` below `root` could not be opened without following a link (`ELOOP`) or as the folders the scan saw
/// (`ENOTDIR`): the link on the way, named, or else that something on the way was replaced (by a file, say). Looked
/// up by path only to word the error; nothing is opened through it.
fn not_opened(root: &Path, path: &Path) -> String {
    let mut at = PathBuf::new();
    for name in path.components() {
        at.push(name);
        if fs::symlink_metadata(root.join(&at)).is_ok_and(|metadata| metadata.is_symlink()) {
            return format!(
                "{} is a symbolic link; a plugin may hold only files and folders",
                at.display()
            );
        }
    }
    format!(
        "{} was replaced since it was read; review it again",
        path.display()
    )
}

/// Why a file with more than one name refuses the install: it may be a name for a file the user does not own.
fn hard_link(path: &Path) -> String {
    format!(
        "{} is a hard link; a plugin may hold only its own files",
        path.display()
    )
}

/// Walks `root` without following links, refusing special files and the limits, skipping `.git` folders.
fn scan(root: &Path) -> Result<Scan, String> {
    let mut scan = Scan::default();
    let mut folders = vec![PathBuf::new()];
    while let Some(folder) = folders.pop() {
        let shown = |path: &Path| {
            if path.as_os_str().is_empty() {
                "the folder".to_string()
            } else {
                path.display().to_string()
            }
        };
        let entries = fs::read_dir(root.join(&folder))
            .map_err(|error| format!("could not read {}: {error}", shown(&folder)))?;
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("could not read {}: {error}", shown(&folder)))?;
            let name = entry.file_name();
            let path = folder.join(&name);
            let kind = fs::symlink_metadata(root.join(&path))
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            if kind.is_dir() {
                if name == ".git" {
                    continue;
                }
                folders.push(path.clone());
                scan.entries.push((path, true));
            } else if kind.is_file() {
                if kind.nlink() > 1 {
                    return Err(hard_link(&path));
                }
                scan.files += 1;
                scan.bytes += kind.len();
                scan.entries.push((path, false));
            } else {
                return Err(special(&path, kind.file_type()));
            }
            if scan.entries.len() > MAX_ENTRIES {
                return Err(too_many());
            }
            if scan.bytes > MAX_BYTES {
                return Err(too_large());
            }
        }
    }
    Ok(scan)
}

/// A picked folder, read and counted.
struct Source {
    root: PathBuf,
    /// The device and inode of `root` when it was scanned, so the copy reads the same folder.
    identity: (u64, u64),
    manifest: Manifest,
    fingerprint: String,
    scan: Scan,
}

/// The manifest of the picked folder by the usual rules, except that the folder's name does not matter.
fn source_manifest(root: &Path) -> Result<(Manifest, String), String> {
    let file = root.join(MANIFEST_FILE);
    let metadata = match fs::symlink_metadata(&file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(format!(
                "the folder has no {MANIFEST_FILE}; choose the plugin's own folder (unzip a .zip first)"
            ));
        }
        Err(error) => return Err(format!("could not read {MANIFEST_FILE}: {error}")),
        Ok(metadata) => metadata,
    };
    if !metadata.is_file() {
        return Err(special(Path::new(MANIFEST_FILE), metadata.file_type()));
    }
    if metadata.len() > plugins::MAX_MANIFEST_BYTES {
        return Err(format!("{MANIFEST_FILE} is larger than 64 KiB"));
    }
    let bytes =
        fs::read(&file).map_err(|error| format!("could not read {MANIFEST_FILE}: {error}"))?;
    Ok((plugins::parse(&bytes)?, plugins::fingerprint(&bytes)))
}

/// Reads the folder at `path`: its manifest first (so a folder that is not a plugin is refused before it is walked),
/// then its contents. A folder inside the plugins folder, or holding it, is refused.
fn read_source(app_data: &Path, path: &Path) -> Result<Source, String> {
    let root = fs::canonicalize(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    if !root.is_dir() {
        return Err(format!("{} is not a folder", root.display()));
    }
    if let Ok(installed) = fs::canonicalize(plugins::dir(app_data)) {
        if root.starts_with(&installed) {
            return Err(
                "this folder is in the plugins folder already; choose the plugin's folder where you downloaded it"
                    .into(),
            );
        }
        if installed.starts_with(&root) {
            return Err(
                "this folder holds the plugins folder; choose the plugin's own folder".into(),
            );
        }
    }
    let identity = fs::symlink_metadata(&root)
        .map(|metadata| (metadata.dev(), metadata.ino()))
        .map_err(|error| format!("could not open {}: {error}", root.display()))?;
    let (manifest, fingerprint) = source_manifest(&root)?;
    let scan = scan(&root)?;
    Ok(Source {
        root,
        identity,
        manifest,
        fingerprint,
        scan,
    })
}

/// What installing the folder at `path` would do, for the review. `approved` is what is enabled now and `is_set` tells
/// whether a secret holds a value (`None` when that cannot be told).
pub fn review(
    app_data: &Path,
    path: &str,
    approved: &Approved,
    is_set: &dyn Fn(&str, &str) -> Option<bool>,
) -> Result<InstallReview, String> {
    let source = read_source(app_data, Path::new(path))?;
    let id = source.manifest.id.clone();
    let target = plugins::dir(app_data).join(&id);
    let installed = target_exists(&target)?.then(|| match plugins::load(&target) {
        Ok((manifest, fingerprint)) => Replaced {
            version: Some(manifest.version),
            approval: Some(Approval::of(approved, &id, &fingerprint)),
            same_manifest: fingerprint == source.fingerprint,
        },
        Err(_) => Replaced {
            version: None,
            approval: None,
            same_manifest: false,
        },
    });
    let older = installed
        .as_ref()
        .and_then(|installed| installed.version.as_deref())
        .and_then(parse_version)
        .zip(parse_version(&source.manifest.version))
        .is_some_and(|(installed, new)| new < installed);
    let secrets = source
        .manifest
        .secret_params()
        .into_iter()
        .map(|param| plugins::SecretStatus {
            id: param.id.clone(),
            label: param.label.clone(),
            set: is_set(&id, &param.id),
        })
        .collect();
    Ok(InstallReview {
        path: source.root.display().to_string(),
        plugin: Listed {
            // What a plain Install leaves; Install and enable always enables.
            approval: Some(
                if keeps_approval(&target, approved, &id, &source.fingerprint) {
                    Approval::Enabled
                } else {
                    Approval::NotEnabled
                },
            ),
            installed: Installed {
                folder: id,
                manifest: Some(source.manifest),
                fingerprint: Some(source.fingerprint),
                error: None,
            },
            secrets,
            status: None,
        },
        files: source.scan.files,
        bytes: source.scan.bytes,
        installed,
        older,
    })
}

/// Whether `plugins/<id>` is there, refusing anything there that is not a real folder (a link, the socket, a file),
/// which an install must never move aside or replace.
fn target_exists(target: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(target) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not read the installed copy: {error}")),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(metadata) => Err(format!(
            "{} in the plugins folder is {}, not a plugin folder; remove it in the Finder first",
            target.file_name().unwrap_or_default().to_string_lossy(),
            if metadata.is_symlink() {
                "a symbolic link"
            } else {
                "not a folder"
            }
        )),
    }
}

/// Whether a plain Install of `fingerprint` keeps the plugin enabled: only when the copy installed now has exactly
/// these bytes and is enabled. In every other case its approval is removed (a leftover approval of a folder deleted by
/// hand, or one of other bytes, must not enable what is installed).
fn keeps_approval(target: &Path, approved: &Approved, id: &str, fingerprint: &str) -> bool {
    approved
        .get(id)
        .is_some_and(|approved| approved == fingerprint)
        && plugins::load(target).is_ok_and(|(_, installed)| installed == fingerprint)
}

/// Whether `suffix` is what [`random`] makes (12 hex digits), so only folders this module named are tidied.
fn is_random(suffix: &str) -> bool {
    suffix.len() == 12 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn random() -> Result<String, String> {
    crate::ingress::new_token()
        .map(|token| token[..12].to_string())
        .map_err(|error| {
            // Its text names the ingress token; only the cause is of use here.
            let cause = error
                .split_once(": ")
                .map_or(error.as_str(), |(_, cause)| cause);
            format!("could not create a random name: {cause}")
        })
}

/// Opens `path` (relative, of plain names) below the open folder `root`, one name at a time and never through a
/// symbolic link, so a folder swapped for a link after the scan is not followed. The last name is opened as a folder
/// with `folder`, otherwise as a file without blocking (a pipe does not wait for a writer).
fn open_below(root: &fs::File, path: &Path, folder: bool) -> io::Result<fs::File> {
    use std::os::{fd::AsRawFd, fd::FromRawFd, unix::ffi::OsStrExt};
    let names: Vec<_> = path.components().collect();
    let mut current: Option<fs::File> = None;
    for (index, name) in names.iter().enumerate() {
        let std::path::Component::Normal(name) = name else {
            return Err(io::Error::other("not a plain name"));
        };
        let name = std::ffi::CString::new(name.as_bytes())?;
        let last = index + 1 == names.len();
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | if !last || folder {
                libc::O_DIRECTORY
            } else {
                libc::O_NONBLOCK
            };
        let parent = current.as_ref().unwrap_or(root).as_raw_fd();
        // SAFETY: `parent` is an open folder and `name` a NUL-terminated name.
        let fd = unsafe { libc::openat(parent, name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `fd` was just opened and is owned by nothing else.
        current = Some(unsafe { fs::File::from_raw_fd(fd) });
    }
    current.ok_or_else(|| io::Error::other("an empty path"))
}

/// Copies one regular file below `root`, its data and extended attributes (the quarantine flag among them), never
/// through a link, and adds its size to `copied`. At most what is left of [`MAX_BYTES`] is read, so a file that grows
/// while it is copied cannot take the copy past the limit.
fn copy_file(
    root: &fs::File,
    root_path: &Path,
    path: &Path,
    to: &Path,
    copied: &mut u64,
) -> Result<(), String> {
    let failed = |error: io::Error| format!("could not copy {}: {error}", path.display());
    let source = open_below(root, path, false).map_err(|error| match error.raw_os_error() {
        Some(libc::ELOOP | libc::ENOTDIR) => not_opened(root_path, path),
        _ => failed(error),
    })?;
    let metadata = source.metadata().map_err(failed)?;
    if !metadata.is_file() {
        return Err(special(path, metadata.file_type()));
    }
    if metadata.nlink() > 1 {
        return Err(hard_link(path));
    }
    let left = MAX_BYTES - *copied;
    if metadata.len() > left {
        return Err(too_large());
    }
    let target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(to)
        .map_err(failed)?;
    let written = io::copy(&mut io::Read::take(&source, left + 1), &mut &target).map_err(failed)?;
    if written > left {
        return Err(too_large());
    }
    *copied += written;
    copy_attributes(&source, &target).map_err(failed)?;
    target
        .set_permissions(fs::Permissions::from_mode(
            metadata.permissions().mode() & 0o755,
        ))
        .map_err(failed)
}

/// Copies a file's extended attributes (not its ACL).
#[cfg(target_os = "macos")]
fn copy_attributes(from: &fs::File, to: &fs::File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    // SAFETY: both descriptors are open for as long as the call runs; a null state is allowed.
    let done = unsafe {
        libc::fcopyfile(
            from.as_raw_fd(),
            to.as_raw_fd(),
            std::ptr::null_mut(),
            libc::COPYFILE_XATTR,
        )
    };
    if done < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn copy_attributes(_: &fs::File, _: &fs::File) -> io::Result<()> {
    Ok(())
}

/// Copies what `source` scanned into the new folder `to`.
fn copy_tree(source: &Source, to: &Path) -> Result<(), String> {
    let root = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&source.root)
        .map_err(|error| format!("could not open the folder: {error}"))?;
    let opened = root
        .metadata()
        .map_err(|error| format!("could not read the folder: {error}"))?;
    if (opened.dev(), opened.ino()) != source.identity {
        return Err("the folder was replaced since it was read; review it again".into());
    }
    let mode = |metadata: fs::Metadata| (metadata.permissions().mode() & 0o755) | 0o700;
    let mode_of = |path: &Path| {
        open_below(&root, path, true)
            .and_then(|folder| folder.metadata())
            .map(mode)
            .map_err(|error| match error.raw_os_error() {
                Some(libc::ELOOP | libc::ENOTDIR) => not_opened(&source.root, path),
                _ => format!("could not read {}: {error}", path.display()),
            })
    };
    let create = |path: &Path| {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path)
            .map_err(|error| format!("could not create the copy: {error}"))
    };
    create(to)?;
    // Folders get their own mode last, so a read-only one is still filled in.
    let root_mode = root
        .metadata()
        .map(mode)
        .map_err(|error| format!("could not read the folder: {error}"))?;
    let mut folders = vec![(to.to_path_buf(), root_mode)];
    let mut copied = 0;
    for (path, folder) in &source.scan.entries {
        let target = to.join(path);
        if *folder {
            create(&target)?;
            folders.push((target, mode_of(path)?));
        } else {
            copy_file(&root, &source.root, path, &target, &mut copied)?;
        }
    }
    for (folder, mode) in folders.into_iter().rev() {
        fs::set_permissions(&folder, fs::Permissions::from_mode(mode))
            .map_err(|error| format!("could not set the copy's permissions: {error}"))?;
    }
    Ok(())
}

/// Deletes a file or a folder tree without following links. A folder that is not writable is made writable first.
fn remove_tree(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        other => other?,
    };
    if !metadata.is_dir() {
        return fs::remove_file(path);
    }
    if fs::remove_dir_all(path).is_ok() {
        return Ok(());
    }
    let mut folders = vec![path.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let _ = fs::set_permissions(&folder, fs::Permissions::from_mode(0o700));
        for entry in fs::read_dir(&folder)?.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
        }
    }
    fs::remove_dir_all(path)
}

/// Whether `path` is a real folder (not a link to one); a missing one is not an error.
fn real_folder_or_missing(path: &Path, what: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not read the {what}: {error}")),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(format!(
            "the {what} {} is not a plain folder (a symbolic link or a file), so nothing in it is changed",
            path.display()
        )),
    }
}

/// Tidies what an interrupted install or remove left in the plugins folder: an update stopped between moving the
/// installed copy aside (`.old-<id>-<random>`) and putting the new one in place gets the installed copy back (the
/// newest, when there are several) when `<id>` is missing; every other leftover is deleted. Nothing is done in a
/// plugins folder that is a link.
pub fn clean_leftovers(plugins_dir: &Path) {
    if !real_folder_or_missing(plugins_dir, "plugins folder").unwrap_or(false) {
        return;
    }
    let Ok(entries) = fs::read_dir(plugins_dir) else {
        return;
    };
    let mut aside: BTreeMap<String, Vec<(i64, i64, PathBuf)>> = BTreeMap::new();
    let mut delete = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix(ASIDE_PREFIX) {
            let Some((id, suffix)) = rest.rsplit_once('-') else {
                continue;
            };
            if !is_random(suffix) {
                // Not one of ours (a backup made by hand, say): left alone.
                continue;
            }
            let id = id.to_string();
            match fs::symlink_metadata(entry.path()) {
                Ok(metadata) if metadata.is_dir() && validate_plugin_id(&id).is_ok() => {
                    aside.entry(id).or_default().push((
                        metadata.ctime(),
                        metadata.ctime_nsec(),
                        entry.path(),
                    ));
                }
                _ => delete.push(entry.path()),
            }
        } else if name.strip_prefix(STAGING_PREFIX).is_some_and(is_random)
            || name
                .strip_prefix(REMOVED_PREFIX)
                .and_then(|rest| rest.rsplit_once('-'))
                .is_some_and(|(_, suffix)| is_random(suffix))
        {
            delete.push(entry.path());
        }
    }
    for (id, mut copies) in aside {
        copies.sort();
        let target = plugins_dir.join(&id);
        if fs::symlink_metadata(&target).is_err()
            && let Some((_, _, newest)) = copies.pop()
        {
            match fs::rename(&newest, &target) {
                Ok(()) => eprintln!("put {id} back after an interrupted update"),
                Err(error) => {
                    eprintln!("could not put {id} back from {}: {error}", newest.display());
                    continue;
                }
            }
        }
        delete.extend(copies.into_iter().map(|(_, _, path)| path));
    }
    for path in delete {
        if let Err(error) = remove_tree(&path) {
            eprintln!("could not delete {}: {error}", path.display());
        }
    }
}

/// Puts `staged` in place as `target` with `put` (a rename), first renaming an installed `target` to `aside`, and
/// putting it back when `put` fails. The old copy is deleted once the new one is in place.
fn swap(
    staged: &Path,
    target: &Path,
    aside: &Path,
    put: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> Result<(), String> {
    let replacing = fs::symlink_metadata(target).is_ok();
    if replacing {
        fs::rename(target, aside)
            .map_err(|error| format!("could not move the installed copy aside: {error}"))?;
    }
    if let Err(error) = put(staged, target) {
        if replacing && let Err(back) = fs::rename(aside, target) {
            return Err(format!(
                "could not put the new copy in place: {error}; nor the installed one back: {back} (it is {})",
                aside.display()
            ));
        }
        return Err(format!(
            "could not put the new copy in place: {error}; nothing was changed"
        ));
    }
    if replacing && let Err(error) = remove_tree(aside) {
        eprintln!("could not delete {}: {error}", aside.display());
    }
    Ok(())
}

/// Runs `then` with `id` paused in the host, so it neither runs from nor starts in a folder that is being changed,
/// and resumes it afterwards whatever happened: a Live run that uses it starts what is installed then.
fn paused(
    host: Option<&PluginHost>,
    id: &str,
    then: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let Some(host) = host else {
        return then();
    };
    if let Err(error) = host.pause(id) {
        // A process of the old copy may still run: start nothing beside it now (see `PluginHost::unpause`).
        host.unpause(id);
        return Err(error);
    }
    let result = then();
    host.resume(id);
    result
}

fn tell_host(host: Option<&PluginHost>, id: &str, fingerprint: Option<String>) {
    if let Some(host) = host {
        host.set_approval(id, fingerprint);
    }
}

/// Installs the folder at `path`, whose `plugin.json` the user reviewed with `fingerprint`, and with `enable` also
/// enables exactly that `plugin.json`. Replaces an installed copy of the same id.
pub fn install(
    app_data: &Path,
    approvals: &Approvals,
    host: Option<&PluginHost>,
    path: &str,
    fingerprint: &str,
    enable: bool,
) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let plugins_dir = plugins::dir(app_data);
    crate::ingress::prepare_private_dir(&plugins_dir, "plugins", "no plugin can be installed")?;
    clean_leftovers(&plugins_dir);
    let source = read_source(app_data, Path::new(path))?;
    if source.fingerprint != fingerprint {
        return Err(CHANGED.into());
    }
    let id = source.manifest.id.clone();
    let target = plugins_dir.join(&id);
    target_exists(&target)?;
    let staging = plugins_dir.join(format!("{STAGING_PREFIX}{}", random()?));
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&staging)
        .map_err(|error| format!("could not create the copy: {error}"))?;
    let staged = staging.join(&id);
    let result = copy_tree(&source, &staged)
        .and_then(|()| {
            // The copy is checked as it will be used, folder name included, before it is put in place.
            let (_, copied) = plugins::load(&staged)
                .map_err(|error| format!("the copy cannot be used: {error}"))?;
            if copied != fingerprint {
                return Err(CHANGED.to_string());
            }
            Ok(())
        })
        .and_then(|()| {
            let aside = plugins_dir.join(format!("{ASIDE_PREFIX}{id}-{}", random()?));
            paused(host, &id, || {
                // Decided before the swap, from the copy installed now.
                let before = approvals.read();
                let keep = keeps_approval(&target, &before, &id, fingerprint);
                target_exists(&target)?;
                // An approval a plain Install ends goes before the swap, so neither a failed write after it nor a
                // crash can leave the new copy enabled; a failed swap puts it back.
                let dropped = match before.get(&id) {
                    Some(earlier) if !enable && !keep => {
                        approvals
                            .set(&id, None, |_| tell_host(host, &id, None))
                            .map_err(|error| {
                                format!("the earlier approval could not be removed: {error}")
                            })?;
                        Some(earlier.clone())
                    }
                    _ => None,
                };
                if let Err(error) = swap(&staged, &target, &aside, |from, to| fs::rename(from, to)) {
                    if let Some(earlier) = dropped {
                        restore_approval(approvals, host, &id, &earlier);
                    }
                    return Err(error);
                }
                // From here on the new copy is in place: an error says so with the `installed: ` marker.
                approve(approvals, host, &id, fingerprint, enable, keep).map_err(|error| {
                    let name = &source.manifest.name;
                    if enable {
                        format!("installed: {name} could not be enabled: {error}")
                    } else {
                        format!("installed: the earlier approval of {name} could not be removed: {error}")
                    }
                })
            })
        });
    if let Err(error) = remove_tree(&staging) {
        eprintln!("could not delete {}: {error}", staging.display());
    }
    result
}

/// The approval after an install of `fingerprint`: enabled with `enable`; otherwise kept only with `keep` (see
/// [`keeps_approval`]) and removed in every other case.
fn approve(
    approvals: &Approvals,
    host: Option<&PluginHost>,
    id: &str,
    fingerprint: &str,
    enable: bool,
    keep: bool,
) -> Result<(), String> {
    if enable {
        approvals.set(id, Some(fingerprint), |_| {
            tell_host(host, id, Some(fingerprint.to_string()))
        })?;
    } else if !keep && approvals.read().contains_key(id) {
        // Removed before the swap already; only an enable racing the install lands here.
        approvals.set(id, None, |_| tell_host(host, id, None))?;
    }
    Ok(())
}

/// Puts back an approval removed for a change that then failed, so a plain error still means nothing changed.
fn restore_approval(approvals: &Approvals, host: Option<&PluginHost>, id: &str, fingerprint: &str) {
    if let Err(error) = approvals.set(id, Some(fingerprint), |_| {
        tell_host(host, id, Some(fingerprint.to_string()))
    }) {
        eprintln!("could not put back the approval of {id}: {error}");
    }
}

/// The secret parameter ids the plugin folder's `plugin.json` declares, read without the owner and name checks so a
/// folder that cannot be used still gives them; none when the manifest cannot be read at all.
fn declared_secrets(folder: &Path) -> Vec<String> {
    let file = folder.join(MANIFEST_FILE);
    let readable = fs::symlink_metadata(&file)
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= plugins::MAX_MANIFEST_BYTES);
    if !readable {
        return Vec::new();
    }
    fs::read(&file)
        .ok()
        .and_then(|bytes| plugins::parse(&bytes).ok())
        .map(|manifest| {
            manifest
                .secret_params()
                .into_iter()
                .map(|param| param.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Refuses a name that is not one visible entry directly in the plugins folder: empty, a path, `.` or `..`, or hidden
/// (which also keeps staging and set-aside folders out of reach).
fn check_folder_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.starts_with('.')
        || name.contains(['/', '\0'])
        || Path::new(name).components().count() != 1
    {
        return Err(format!("{name:?} is not a folder in the plugins folder"));
    }
    Ok(())
}

/// Removes the plugin in the plugins folder's entry `id`: stops it, removes its approval, deletes its folder and its
/// `plugin-data` folder, and forgets the Keychain secrets its manifest declares. Works for a folder that cannot be
/// used, and for a valid id that is gone already. `id` may also be the name of a folder that is not a plugin id (an
/// unusable folder as Settings lists it, such as `notify-main`): that folder must exist as a real folder, and only it
/// is deleted, since no approval, secret or data folder can belong to such a name. A symbolic link is never removed
/// or followed. Profiles are not changed.
pub fn remove(
    app_data: &Path,
    approvals: &Approvals,
    host: Option<&PluginHost>,
    secrets: &PluginSecrets,
    id: &str,
) -> Result<(), String> {
    check_folder_name(id)?;
    let is_id = validate_plugin_id(id).is_ok();
    let _guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let plugins_dir = plugins::dir(app_data);
    let data_dir = plugins::data_dir(app_data);
    real_folder_or_missing(&plugins_dir, "plugins folder")?;
    real_folder_or_missing(&data_dir, "plugin data folder")?;
    clean_leftovers(&plugins_dir);
    let folder = plugins_dir.join(id);
    let exists = match fs::symlink_metadata(&folder) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("could not read the plugin's folder: {error}")),
        Ok(metadata) if metadata.is_dir() => true,
        Ok(metadata) => {
            return Err(format!(
                "{id} is {} in the plugins folder, not a plugin folder; remove it in the Finder",
                if metadata.is_symlink() {
                    "a symbolic link"
                } else {
                    "a file"
                }
            ));
        }
    };
    if !is_id {
        if !exists {
            return Err(format!("{id} is not a folder in the plugins folder"));
        }
        let aside = plugins_dir.join(format!("{REMOVED_PREFIX}{id}-{}", random()?));
        fs::rename(&folder, &aside)
            .map_err(|error| format!("could not remove the folder: {error}"))?;
        if let Err(error) = remove_tree(&aside) {
            eprintln!("could not delete {}: {error}", aside.display());
        }
        return Ok(());
    }
    let declared = declared_secrets(&folder);
    paused(host, id, || {
        let earlier = approvals.read().get(id).cloned();
        approvals.set(id, None, |_| tell_host(host, id, None))?;
        // Renamed aside first, so a delete that fails half way leaves no half plugin under its id. A failed rename
        // puts the approval back: a plain error means nothing was removed.
        if exists {
            let renamed = random().and_then(|suffix| {
                let aside = plugins_dir.join(format!("{REMOVED_PREFIX}{id}-{suffix}"));
                fs::rename(&folder, &aside)
                    .map(|()| aside)
                    .map_err(|error| format!("could not remove the plugin's folder: {error}"))
            });
            let aside = match renamed {
                Ok(aside) => aside,
                Err(error) => {
                    if let Some(earlier) = earlier {
                        restore_approval(approvals, host, id, &earlier);
                    }
                    return Err(error);
                }
            };
            if let Err(error) = remove_tree(&aside) {
                eprintln!("could not delete {}: {error}", aside.display());
            }
        }
        // From here on its folder is gone: an error says so with the `removed: ` marker, and the rest is still done.
        let mut problems = Vec::new();
        // A link there is removed itself, never followed.
        if let Err(error) = remove_tree(&data_dir.join(id)) {
            problems.push(format!("its data folder could not be deleted: {error}"));
        }
        let unforgotten: Vec<String> = declared
            .iter()
            .filter_map(|param| {
                secrets
                    .forget(id, param)
                    .err()
                    .map(|error| format!("{param} ({error})"))
            })
            .collect();
        if !unforgotten.is_empty() {
            problems.push(format!(
                "the Keychain kept its secret {}",
                unforgotten.join(", ")
            ));
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(format!("removed: {}", problems.join("; ")))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_secrets::memory;
    use serde_json::json;
    use std::time::{Duration, Instant};

    const ID: &str = "com.example.lights";

    fn manifest(version: &str, command: serde_json::Value) -> String {
        json!({
            "schema": 1, "id": ID, "name": "Lights", "version": version, "command": command,
            "actions": [{ "id": "set", "label": "Set", "params": [
                { "id": "token", "label": "Token", "type": "text", "secret": true }
            ] }]
        })
        .to_string()
    }

    /// An app data folder (under /tmp, so a host's socket path stays short) and a download folder.
    struct Rig {
        root: tempfile::TempDir,
        approvals: Approvals,
    }

    impl Rig {
        fn new() -> Self {
            let root = tempfile::Builder::new()
                .prefix("i")
                .tempdir_in("/tmp")
                .unwrap();
            fs::create_dir(root.path().join("data")).unwrap();
            let approvals = Approvals::new(&root.path().join("data"));
            Self { root, approvals }
        }

        fn app_data(&self) -> PathBuf {
            self.root.path().join("data")
        }

        fn plugins(&self) -> PathBuf {
            plugins::dir(&self.app_data())
        }

        /// A download named `name` holding `plugin.json` with `manifest`; hands back its path and fingerprint.
        fn download(&self, name: &str, manifest: &str) -> (String, String) {
            let folder = self.root.path().join("Downloads").join(name);
            fs::create_dir_all(folder.join("bin")).unwrap();
            fs::write(folder.join(MANIFEST_FILE), manifest).unwrap();
            fs::write(folder.join("bin/lights"), "#!/bin/sh\n").unwrap();
            fs::set_permissions(folder.join("bin/lights"), fs::Permissions::from_mode(0o777))
                .unwrap();
            (
                folder.display().to_string(),
                plugins::fingerprint(manifest.as_bytes()),
            )
        }

        fn review(&self, path: &str) -> Result<InstallReview, String> {
            review(&self.app_data(), path, &self.approvals.read(), &|_, _| {
                Some(false)
            })
        }

        fn install(&self, path: &str, fingerprint: &str, enable: bool) -> Result<(), String> {
            install(
                &self.app_data(),
                &self.approvals,
                None,
                path,
                fingerprint,
                enable,
            )
        }

        fn hidden_leftovers(&self) -> Vec<String> {
            fs::read_dir(self.plugins())
                .unwrap()
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.'))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect()
        }
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn a_cancelled_folder_picker_is_no_choice_and_other_failures_are_errors() {
        let cancelled = b"execution error: User canceled. (-128)";
        assert_eq!(picked(false, b"", cancelled), Ok(None));
        assert!(picked(false, b"", b"some other failure").is_err());
        assert_eq!(
            picked(true, b"/Users/me/Downloads/notify-main/\n", b""),
            Ok(Some("/Users/me/Downloads/notify-main/".into()))
        );
        assert_eq!(picked(true, b"", b""), Ok(None));
    }

    #[test]
    fn the_review_counts_the_files_and_says_what_it_replaces() {
        let rig = Rig::new();
        let (path, fingerprint) =
            rig.download("notify-main", &manifest("1.2.0", json!(["bin/lights"])));
        fs::create_dir_all(Path::new(&path).join(".git/objects")).unwrap();
        fs::write(Path::new(&path).join(".git/objects/blob"), vec![0; 4000]).unwrap();
        fs::write(Path::new(&path).join(".env"), "A=1").unwrap();
        let review = rig.review(&path).unwrap();
        // plugin.json, bin/lights and .env; .git is not counted.
        assert_eq!(review.files, 3);
        assert_eq!(
            review.bytes,
            (manifest("1.2.0", json!(["bin/lights"])).len() + "#!/bin/sh\n".len() + 3) as u64
        );
        assert!(review.installed.is_none());
        assert!(!review.older);
        assert_eq!(review.plugin.installed.folder, ID);
        assert_eq!(
            review.plugin.installed.fingerprint.as_deref(),
            Some(fingerprint.as_str())
        );
        assert_eq!(review.plugin.approval, Some(Approval::NotEnabled));
        assert_eq!(review.plugin.secrets[0].id, "token");
        assert_eq!(
            review.path,
            fs::canonicalize(&path).unwrap().display().to_string()
        );
        let shown = serde_json::to_value(&review).unwrap();
        assert_eq!(shown["installed"], serde_json::Value::Null);
        assert_eq!(shown["plugin"]["approval"], "not_enabled");
        assert!(shown["plugin"].get("status").is_none());

        rig.install(&path, &fingerprint, true).unwrap();
        // The same bytes again: enabled, the same manifest.
        let again = rig.review(&path).unwrap();
        assert_eq!(again.plugin.approval, Some(Approval::Enabled));
        assert_eq!(
            again.installed,
            Some(Replaced {
                version: Some("1.2.0".into()),
                approval: Some(Approval::Enabled),
                same_manifest: true
            })
        );
        // An older version with another manifest: a plain Install leaves it not enabled.
        let (older, _) = rig.download("old", &manifest("1.1.9", json!(["bin/lights"])));
        let review = rig.review(&older).unwrap();
        assert!(review.older);
        assert_eq!(review.plugin.approval, Some(Approval::NotEnabled));
        let installed = review.installed.unwrap();
        assert!(!installed.same_manifest);
        assert_eq!(installed.version.as_deref(), Some("1.2.0"));
        // An installed folder that cannot be used has no version.
        fs::set_permissions(rig.plugins().join(ID), fs::Permissions::from_mode(0o777)).unwrap();
        let review = rig.review(&older).unwrap();
        assert_eq!(
            review.installed,
            Some(Replaced {
                version: None,
                approval: None,
                same_manifest: false
            })
        );
        assert!(!review.older);
    }

    #[test]
    fn links_and_special_files_are_refused_by_name() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let link = Path::new(&path).join("bin/data");
        std::os::unix::fs::symlink("/etc", &link).unwrap();
        let error = rig.review(&path).unwrap_err();
        assert_eq!(
            error,
            "bin/data is a symbolic link; a plugin may hold only files and folders"
        );
        assert_eq!(rig.install(&path, &fingerprint, false).unwrap_err(), error);
        fs::remove_file(&link).unwrap();
        let fifo = Path::new(&path).join("bin/pipe");
        let name = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        // SAFETY: a valid path string.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(
            rig.review(&path).unwrap_err(),
            "bin/pipe is a named pipe; a plugin may hold only files and folders"
        );
        assert!(!rig.plugins().join(ID).exists());
        assert!(rig.hidden_leftovers().is_empty());
    }

    #[test]
    fn a_hard_link_is_refused_by_name() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let other = rig.root.path().join("not-mine");
        fs::write(&other, "x").unwrap();
        fs::hard_link(&other, Path::new(&path).join("bin/data")).unwrap();
        let error = rig.review(&path).unwrap_err();
        assert_eq!(
            error,
            "bin/data is a hard link; a plugin may hold only its own files"
        );
        assert_eq!(rig.install(&path, &fingerprint, false).unwrap_err(), error);
        // Checked again while copying.
        let source = read_source(&rig.app_data(), Path::new(&path));
        assert!(source.is_err());
        fs::remove_file(Path::new(&path).join("bin/data")).unwrap();
        let source = read_source(&rig.app_data(), Path::new(&path)).unwrap();
        fs::hard_link(&other, Path::new(&path).join("bin/data")).unwrap();
        fs::remove_file(Path::new(&path).join("bin/lights")).unwrap();
        fs::hard_link(&other, Path::new(&path).join("bin/lights")).unwrap();
        let error = copy_tree(&source, &rig.root.path().join("copy")).unwrap_err();
        assert!(error.contains("is a hard link"), "{error}");
    }

    #[test]
    fn a_picked_folder_replaced_after_the_review_is_not_copied() {
        let rig = Rig::new();
        let (path, _) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let source = read_source(&rig.app_data(), Path::new(&path)).unwrap();
        fs::rename(&path, rig.root.path().join("moved")).unwrap();
        rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let error = copy_tree(&source, &rig.root.path().join("copy")).unwrap_err();
        assert!(error.contains("replaced"), "{error}");
    }

    #[test]
    fn a_folder_past_the_limits_is_refused() {
        let rig = Rig::new();
        let (path, _) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        for index in 0..MAX_ENTRIES {
            fs::write(Path::new(&path).join(format!("f{index}")), "x").unwrap();
        }
        assert!(
            rig.review(&path)
                .unwrap_err()
                .contains("more than 20 files")
        );
        let (path, _) = rig.download("big", &manifest("1.2.0", json!(["bin/lights"])));
        fs::write(Path::new(&path).join("big"), vec![0; MAX_BYTES as usize]).unwrap();
        assert!(rig.review(&path).unwrap_err().contains("larger than"));
    }

    #[test]
    fn a_folder_without_a_usable_manifest_or_inside_the_plugins_folder_is_refused() {
        let rig = Rig::new();
        let empty = rig.root.path().join("Downloads/empty");
        fs::create_dir_all(&empty).unwrap();
        assert!(
            rig.review(empty.to_str().unwrap())
                .unwrap_err()
                .contains("has no plugin.json")
        );
        let (path, _) = rig.download("broken", "{\"schema\": 1}");
        assert!(
            rig.review(&path)
                .unwrap_err()
                .contains("not a valid manifest")
        );
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        rig.install(&path, &fingerprint, false).unwrap();
        let inside = rig.plugins().join(ID);
        assert!(
            rig.review(inside.to_str().unwrap())
                .unwrap_err()
                .contains("in the plugins folder already")
        );
        assert!(
            rig.review(rig.app_data().to_str().unwrap())
                .unwrap_err()
                .contains("holds the plugins folder")
        );
    }

    #[test]
    fn a_copy_is_installed_as_its_id_without_git_and_writable_only_by_the_user() {
        let rig = Rig::new();
        let (path, fingerprint) =
            rig.download("lights-1.2.0", &manifest("1.2.0", json!(["bin/lights"])));
        let source = Path::new(&path);
        fs::create_dir(source.join(".git")).unwrap();
        fs::write(source.join(".git/HEAD"), "ref").unwrap();
        fs::write(source.join(".env"), "A=1").unwrap();
        fs::set_permissions(
            source.join(MANIFEST_FILE),
            fs::Permissions::from_mode(0o666),
        )
        .unwrap();
        fs::create_dir(source.join("ro")).unwrap();
        fs::write(source.join("ro/data"), "d").unwrap();
        fs::set_permissions(source.join("ro"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(source, fs::Permissions::from_mode(0o777)).unwrap();
        rig.install(&path, &fingerprint, false).unwrap();
        let installed = rig.plugins().join(ID);
        assert!(!installed.join(".git").exists());
        assert_eq!(fs::read_to_string(installed.join(".env")).unwrap(), "A=1");
        assert_eq!(mode(&installed), 0o755);
        assert_eq!(mode(&installed.join(MANIFEST_FILE)), 0o644);
        assert_eq!(mode(&installed.join("bin/lights")), 0o755);
        assert_eq!(mode(&installed.join("ro")), 0o755);
        plugins::load(&installed).unwrap();
        // The source is left as it was.
        assert!(source.join(".git/HEAD").exists());
        assert_eq!(mode(source), 0o777);
        assert!(rig.approvals.read().is_empty());
        assert!(rig.hidden_leftovers().is_empty());
        fs::set_permissions(source.join("ro"), fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_quarantine_flag_is_kept() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let program = Path::new(&path).join("bin/lights");
        let flag = "0081;6734a2b1;Safari;";
        let status = Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", flag])
            .arg(&program)
            .status()
            .unwrap();
        assert!(status.success());
        rig.install(&path, &fingerprint, false).unwrap();
        let out = Command::new("/usr/bin/xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(rig.plugins().join(ID).join("bin/lights"))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), flag);
    }

    #[test]
    fn install_and_enable_approves_exactly_the_reviewed_manifest() {
        let rig = Rig::new();
        let text = manifest("1.2.0", json!(["bin/lights"]));
        let (path, fingerprint) = rig.download("lights", &text);
        // plugin.json changes after the review.
        fs::write(Path::new(&path).join(MANIFEST_FILE), format!("{text} ")).unwrap();
        assert_eq!(rig.install(&path, &fingerprint, true).unwrap_err(), CHANGED);
        assert!(!rig.plugins().join(ID).exists());
        assert!(rig.approvals.read().is_empty());
        let current = plugins::fingerprint(format!("{text} ").as_bytes());
        rig.install(&path, &current, true).unwrap();
        assert_eq!(rig.approvals.read()[ID], current);
    }

    #[test]
    fn an_update_keeps_the_approval_only_for_identical_bytes() {
        let rig = Rig::new();
        let text = manifest("1.2.0", json!(["bin/lights"]));
        let (path, fingerprint) = rig.download("lights", &text);
        rig.install(&path, &fingerprint, true).unwrap();
        fs::write(Path::new(&path).join("bin/lights"), "#!/bin/sh\necho new\n").unwrap();
        rig.install(&path, &fingerprint, false).unwrap();
        assert_eq!(rig.approvals.read()[ID], fingerprint);
        assert!(
            fs::read_to_string(rig.plugins().join(ID).join("bin/lights"))
                .unwrap()
                .contains("new")
        );
        let (newer, other) = rig.download("lights-2", &manifest("2.0.0", json!(["bin/lights"])));
        rig.install(&newer, &other, false).unwrap();
        assert!(rig.approvals.read().is_empty());
        let review = rig.review(&path).unwrap();
        assert_eq!(review.plugin.approval, Some(Approval::NotEnabled));
        assert!(review.older);
        rig.install(&newer, &other, true).unwrap();
        assert_eq!(rig.approvals.read()[ID], other);
        assert!(rig.hidden_leftovers().is_empty());
    }

    #[test]
    fn an_approval_that_cannot_be_written_after_the_copy_says_the_plugin_is_installed() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        fs::create_dir_all(rig.plugins()).unwrap();
        // Refused before the swap: plain text, nothing installed.
        let error = rig.install(&path, "0000", true).unwrap_err();
        assert_eq!(error, CHANGED);
        assert!(!rig.plugins().join(ID).exists());
        // The approvals file cannot be written; the plugins folder still can.
        fs::set_permissions(rig.app_data(), fs::Permissions::from_mode(0o500)).unwrap();
        let error = rig.install(&path, &fingerprint, true).unwrap_err();
        fs::set_permissions(rig.app_data(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            error.starts_with("installed: Lights could not be enabled: "),
            "{error}"
        );
        plugins::load(&rig.plugins().join(ID)).unwrap();
    }

    #[test]
    fn a_failed_swap_puts_the_installed_copy_back() {
        let rig = Rig::new();
        let plugins_dir = rig.plugins();
        let target = plugins_dir.join(ID);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("old"), "old").unwrap();
        let staged = plugins_dir.join(".install-x").join(ID);
        fs::create_dir_all(&staged).unwrap();
        let aside = plugins_dir.join(format!(".old-{ID}-x"));
        let error = swap(&staged, &target, &aside, |_, _| {
            Err(io::Error::other("disk full"))
        })
        .unwrap_err();
        assert!(error.contains("disk full"), "{error}");
        assert_eq!(fs::read_to_string(target.join("old")).unwrap(), "old");
        assert!(!aside.exists());
        swap(&staged, &target, &aside, |from, to| fs::rename(from, to)).unwrap();
        assert!(!target.join("old").exists() && !aside.exists());
    }

    #[test]
    fn leftovers_of_an_interrupted_install_are_deleted() {
        let rig = Rig::new();
        let plugins_dir = rig.plugins();
        fs::create_dir_all(plugins_dir.join(".install-0123456789ab/com.example.x/ro")).unwrap();
        fs::set_permissions(
            plugins_dir.join(".install-0123456789ab/com.example.x/ro"),
            fs::Permissions::from_mode(0o555),
        )
        .unwrap();
        fs::create_dir_all(plugins_dir.join("com.example.x")).unwrap();
        fs::create_dir_all(plugins_dir.join(".old-com.example.x-0123456789ab")).unwrap();
        fs::create_dir_all(plugins_dir.join(".removed-com.example.y-0123456789ab")).unwrap();
        fs::create_dir_all(plugins_dir.join(".keep")).unwrap();
        // Names these do not make are not theirs: a backup made by hand is neither restored nor deleted.
        fs::create_dir_all(plugins_dir.join(".old-com.example.z-mine")).unwrap();
        fs::create_dir_all(plugins_dir.join(".install-mine")).unwrap();
        fs::create_dir_all(plugins_dir.join(".removed-com.example.y-mine")).unwrap();
        clean_leftovers(&plugins_dir);
        assert_eq!(
            rig.hidden_leftovers(),
            [
                ".install-mine",
                ".keep",
                ".old-com.example.z-mine",
                ".removed-com.example.y-mine"
            ]
        );
        assert!(!plugins_dir.join("com.example.z").exists());
        assert!(plugins_dir.join("com.example.x").exists());
        // Nothing is done in a plugins folder that is a link.
        let elsewhere = rig.root.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join(".install-0123456789ab")).unwrap();
        let linked = rig.root.path().join("linked");
        std::os::unix::fs::symlink(&elsewhere, &linked).unwrap();
        clean_leftovers(&linked);
        assert!(elsewhere.join(".install-0123456789ab").exists());
    }

    #[test]
    fn an_update_interrupted_between_its_renames_gets_the_installed_copy_back() {
        let rig = Rig::new();
        let plugins_dir = rig.plugins();
        let older = plugins_dir.join(format!(".old-{ID}-aaaaaaaaaaaa"));
        fs::create_dir_all(&older).unwrap();
        fs::write(older.join("which"), "older").unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let newest = plugins_dir.join(format!(".old-{ID}-bbbbbbbbbbbb"));
        fs::create_dir_all(&newest).unwrap();
        fs::write(newest.join("which"), "newest").unwrap();
        // The staging copy of the interrupted update is gone too.
        fs::create_dir_all(plugins_dir.join(".install-cccccccccccc").join(ID)).unwrap();
        clean_leftovers(&plugins_dir);
        assert_eq!(
            fs::read_to_string(plugins_dir.join(ID).join("which")).unwrap(),
            "newest"
        );
        assert!(rig.hidden_leftovers().is_empty());
    }

    #[test]
    fn only_a_real_folder_at_the_id_is_replaced_and_the_socket_name_is_no_id() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        fs::create_dir_all(rig.plugins()).unwrap();
        let elsewhere = rig.root.path().join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, rig.plugins().join(ID)).unwrap();
        assert!(rig.review(&path).unwrap_err().contains("a symbolic link"));
        assert!(
            rig.install(&path, &fingerprint, false)
                .unwrap_err()
                .contains("a symbolic link")
        );
        fs::remove_file(rig.plugins().join(ID)).unwrap();
        fs::write(rig.plugins().join(ID), "").unwrap();
        assert!(
            rig.install(&path, &fingerprint, false)
                .unwrap_err()
                .contains("not a folder")
        );
        assert!(elsewhere.exists() && rig.plugins().join(ID).is_file());
        // A plugin named like the host's socket is no plugin.
        let mut value: serde_json::Value =
            serde_json::from_str(&manifest("1.2.0", json!(["bin/lights"]))).unwrap();
        value["id"] = json!(crate::plugin_host::SOCKET_FILE);
        let (socket, _) = rig.download("socket", &value.to_string());
        assert!(rig.review(&socket).unwrap_err().contains("socket"));
    }

    #[test]
    fn a_plain_install_never_keeps_a_leftover_approval() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        // An approval left behind by a folder deleted by hand.
        rig.approvals.set(ID, Some(&fingerprint), |_| {}).unwrap();
        assert_eq!(
            rig.review(&path).unwrap().plugin.approval,
            Some(Approval::NotEnabled)
        );
        rig.install(&path, &fingerprint, false).unwrap();
        assert!(rig.approvals.read().is_empty());
        // An installed copy of other bytes showing "changed", whose approval is of the new bytes.
        let (other, other_fingerprint) =
            rig.download("lights-2", &manifest("2.0.0", json!(["bin/lights"])));
        rig.install(&other, &other_fingerprint, false).unwrap();
        rig.approvals.set(ID, Some(&fingerprint), |_| {}).unwrap();
        assert_eq!(
            rig.review(&path).unwrap().plugin.approval,
            Some(Approval::NotEnabled)
        );
        rig.install(&path, &fingerprint, false).unwrap();
        assert!(rig.approvals.read().is_empty());
        // Identical and enabled: kept.
        rig.install(&path, &fingerprint, true).unwrap();
        assert_eq!(
            rig.review(&path).unwrap().plugin.approval,
            Some(Approval::Enabled)
        );
        rig.install(&path, &fingerprint, false).unwrap();
        assert_eq!(rig.approvals.read()[ID], fingerprint);
    }

    #[test]
    fn a_folder_swapped_for_a_link_after_the_scan_is_not_followed() {
        let rig = Rig::new();
        let (path, _) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let source = read_source(&rig.app_data(), Path::new(&path)).unwrap();
        let secret = rig.root.path().join("secret");
        fs::create_dir_all(&secret).unwrap();
        fs::write(secret.join("lights"), "private").unwrap();
        fs::remove_dir_all(Path::new(&path).join("bin")).unwrap();
        std::os::unix::fs::symlink(&secret, Path::new(&path).join("bin")).unwrap();
        let to = rig.root.path().join("copy");
        let error = copy_tree(&source, &to).unwrap_err();
        assert!(error.contains("bin is a symbolic link"), "{error}");
        assert!(!to.join("bin/lights").exists());
    }

    #[test]
    fn remove_changes_nothing_through_a_linked_plugins_or_data_folder() {
        let rig = Rig::new();
        let secrets = memory::secrets(&memory::Store::default());
        let elsewhere = rig.root.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join(ID)).unwrap();
        std::os::unix::fs::symlink(&elsewhere, rig.plugins()).unwrap();
        let error = remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap_err();
        assert!(error.contains("not a plain folder"), "{error}");
        assert!(elsewhere.join(ID).exists());
        fs::remove_file(rig.plugins()).unwrap();
        std::os::unix::fs::symlink(&elsewhere, plugins::data_dir(&rig.app_data())).unwrap();
        assert!(remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).is_err());
        assert!(elsewhere.join(ID).exists());
        // A link as the plugin's own data folder is removed itself, not followed.
        fs::remove_file(plugins::data_dir(&rig.app_data())).unwrap();
        fs::create_dir_all(plugins::data_dir(&rig.app_data())).unwrap();
        std::os::unix::fs::symlink(&elsewhere, plugins::data_dir(&rig.app_data()).join(ID))
            .unwrap();
        remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap();
        assert!(elsewhere.join(ID).exists());
        assert!(fs::symlink_metadata(plugins::data_dir(&rig.app_data()).join(ID)).is_err());
    }

    #[test]
    fn remove_deletes_the_folders_the_approval_and_the_secrets() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        rig.install(&path, &fingerprint, true).unwrap();
        let data = plugins::data_dir(&rig.app_data()).join(ID);
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("state"), "s").unwrap();
        let store = memory::Store::default();
        let secrets = memory::secrets(&store);
        secrets.set(ID, "token", "s3cret").unwrap();
        secrets.set("com.example.other", "token", "keep").unwrap();
        remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap();
        assert!(!rig.plugins().join(ID).exists());
        assert!(!data.exists());
        assert!(rig.approvals.read().is_empty());
        assert_eq!(secrets.get(ID, "token"), Ok(None));
        assert_eq!(store.values.lock().unwrap().len(), 1);
        assert!(rig.hidden_leftovers().is_empty());
        // Gone already: nothing to do.
        remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap();
        assert!(remove(&rig.app_data(), &rig.approvals, None, &secrets, "../x").is_err());
    }

    #[test]
    fn a_folder_named_other_than_an_id_can_be_removed_but_nothing_outside_the_plugins_folder() {
        let rig = Rig::new();
        let store = memory::Store::default();
        let secrets = memory::secrets(&store);
        let remove = |name: &str| remove(&rig.app_data(), &rig.approvals, None, &secrets, name);
        let folder = rig.plugins().join("notify-main");
        fs::create_dir_all(folder.join("bin")).unwrap();
        fs::write(folder.join(MANIFEST_FILE), "{").unwrap();
        remove("notify-main").unwrap();
        assert!(!folder.exists());
        assert!(rig.hidden_leftovers().is_empty());
        assert!(remove("notify-main").unwrap_err().contains("not a folder"));
        // Paths, dot names and hidden entries are refused before anything is touched.
        let outside = rig.root.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(rig.plugins().join(".install-x")).unwrap();
        for name in ["", ".", "..", "a/b", "../outside", ".install-x", ".hidden"] {
            assert!(remove(name).is_err(), "{name:?}");
        }
        assert!(outside.exists() && rig.plugins().join(".install-x").exists());
        // A link is never removed or followed, whether its name is an id or not.
        std::os::unix::fs::symlink(&outside, rig.plugins().join("linked")).unwrap();
        std::os::unix::fs::symlink(&outside, rig.plugins().join(ID)).unwrap();
        assert!(remove("linked").unwrap_err().contains("symbolic link"));
        assert!(remove(ID).unwrap_err().contains("symbolic link"));
        assert!(outside.exists());
        assert!(fs::symlink_metadata(rig.plugins().join("linked")).is_ok());
        assert!(fs::symlink_metadata(rig.plugins().join(ID)).is_ok());
    }

    #[test]
    fn a_remove_whose_folder_cannot_be_moved_keeps_the_plugin_enabled() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        rig.install(&path, &fingerprint, true).unwrap();
        let secrets = memory::secrets(&memory::Store::default());
        // The plugins folder is read only, so its folder cannot be renamed aside.
        fs::set_permissions(rig.plugins(), fs::Permissions::from_mode(0o500)).unwrap();
        let error = remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap_err();
        fs::set_permissions(rig.plugins(), fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!error.starts_with("removed: "), "{error}");
        assert!(rig.plugins().join(ID).exists());
        assert_eq!(rig.approvals.read()[ID], fingerprint);
    }

    #[test]
    fn a_plain_install_that_cannot_end_the_approval_installs_nothing() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        rig.install(&path, &fingerprint, true).unwrap();
        let (other, other_fingerprint) =
            rig.download("lights-2", &manifest("2.0.0", json!(["bin/lights"])));
        // Approvals that cannot be written: the plain Install is refused before the swap, the old copy stays.
        let approvals_file = fs::read_dir(rig.app_data())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.is_file())
            .expect("an approvals file");
        fs::set_permissions(rig.app_data(), fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&approvals_file, fs::Permissions::from_mode(0o400)).unwrap();
        let error = rig.install(&other, &other_fingerprint, false);
        fs::set_permissions(rig.app_data(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&approvals_file, fs::Permissions::from_mode(0o600)).unwrap();
        let error = error.unwrap_err();
        assert!(!error.starts_with("installed: "), "{error}");
        assert_eq!(
            rig.review(&path)
                .unwrap()
                .installed
                .unwrap()
                .version
                .as_deref(),
            Some("1.2.0")
        );
        assert_eq!(rig.approvals.read()[ID], fingerprint);
    }

    #[test]
    fn what_fails_after_the_folder_is_gone_is_marked_removed() {
        let rig = Rig::new();
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        rig.install(&path, &fingerprint, true).unwrap();
        let store = memory::Store::default();
        let secrets = memory::secrets(&store);
        secrets.set(ID, "token", "s3cret").unwrap();
        store
            .failing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let error = remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap_err();
        assert!(
            error.starts_with("removed: the Keychain kept its secret token ("),
            "{error}"
        );
        assert!(!rig.plugins().join(ID).exists());
        assert!(rig.approvals.read().is_empty());
        // Before the folder goes: plain text.
        assert!(
            !remove(&rig.app_data(), &rig.approvals, None, &secrets, "a/b")
                .unwrap_err()
                .starts_with("removed: ")
        );
    }

    #[test]
    fn a_file_where_a_folder_was_scanned_is_reported_as_replaced() {
        let rig = Rig::new();
        let (path, _) = rig.download("lights", &manifest("1.2.0", json!(["bin/lights"])));
        let source = read_source(&rig.app_data(), Path::new(&path)).unwrap();
        fs::remove_dir_all(Path::new(&path).join("bin")).unwrap();
        fs::write(Path::new(&path).join("bin"), "").unwrap();
        let error = copy_tree(&source, &rig.root.path().join("copy")).unwrap_err();
        assert!(error.contains("was replaced since it was read"), "{error}");
    }

    #[test]
    fn a_folder_that_cannot_be_used_can_be_removed() {
        let rig = Rig::new();
        let folder = rig.plugins().join(ID);
        fs::create_dir_all(&folder).unwrap();
        // Writable by others: unusable, but its manifest still names its secrets.
        fs::write(folder.join(MANIFEST_FILE), manifest("1.2.0", json!(["x"]))).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(plugins::load(&folder).is_err());
        let store = memory::Store::default();
        let secrets = memory::secrets(&store);
        secrets.set(ID, "token", "s3cret").unwrap();
        remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap();
        assert!(!folder.exists());
        assert!(store.values.lock().unwrap().is_empty());
        // A broken manifest: the folder goes, and no secret can be named.
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(MANIFEST_FILE), "{").unwrap();
        remove(&rig.app_data(), &rig.approvals, None, &secrets, ID).unwrap();
        assert!(!folder.exists());
    }

    fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn alive(pid: i32) -> bool {
        // SAFETY: signal 0 only asks whether the process exists.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[test]
    fn an_update_stops_the_running_plugin_and_a_live_run_starts_the_new_copy() {
        let rig = Rig::new();
        // Writes its pid into its data folder, then waits.
        let command = json!([
            "/bin/sh",
            "-c",
            "echo $$ > \"$OSH_PLUGIN_DATA/pid\"; exec sleep 30"
        ]);
        let (path, fingerprint) = rig.download("lights", &manifest("1.2.0", command.clone()));
        rig.install(&path, &fingerprint, true).unwrap();
        let host = PluginHost::start(
            &rig.plugins(),
            rig.approvals.read(),
            memory::secrets(&memory::Store::default()),
            std::sync::Arc::new(|_: &serde_json::Value| Vec::new()),
        )
        .unwrap();
        struct Stop(PluginHost);
        impl Drop for Stop {
            fn drop(&mut self) {
                self.0.stop_all();
            }
        }
        let host = Stop(host);
        let pid_file = plugins::data_dir(&rig.app_data()).join(ID).join("pid");
        let pid = || -> i32 {
            fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
                .unwrap_or(0)
        };
        host.0.set_usage("m1", Some([ID.to_string()].into()));
        wait_for("the first start", || pid() > 0);
        let first = pid();
        // Identical plugin.json: still enabled, and the new copy starts.
        fs::write(Path::new(&path).join("bin/lights"), "#!/bin/sh\n# 2\n").unwrap();
        install(
            &rig.app_data(),
            &rig.approvals,
            Some(&host.0),
            &path,
            &fingerprint,
            false,
        )
        .unwrap();
        // Stopped before the folder was replaced.
        assert!(!alive(first));
        wait_for("the new copy to start", || pid() > 0 && pid() != first);
        let second = pid();
        // Another plugin.json, not enabled: the old copy stops and nothing starts.
        let (newer, other) = rig.download("lights-2", &manifest("2.0.0", command));
        install(
            &rig.app_data(),
            &rig.approvals,
            Some(&host.0),
            &newer,
            &other,
            false,
        )
        .unwrap();
        assert!(!alive(second));
        let status = &host.0.statuses()[0];
        assert_eq!(status.state, "not_enabled");
        // Removing it stops nothing more and says it is not installed.
        let secrets = memory::secrets(&memory::Store::default());
        remove(&rig.app_data(), &rig.approvals, Some(&host.0), &secrets, ID).unwrap();
        let status = &host.0.statuses()[0];
        assert!(
            status.error.as_ref().unwrap().contains("not installed"),
            "{status:?}"
        );
    }
}
