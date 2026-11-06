//! App preset packs the user drops into `<app data>/packs/`, next to the ones shipped with the app.
//!
//! This module only reads the files. The webview checks each one with the same rules as a shipped pack
//! (`preset-packs.ts`) before the chooser offers it, and a pack's profile is validated again when it is created
//! (`create_pack_profile`), which also makes it dry run only until the user allows it.
//!
//! Reading is defensive because anything can be put in the folder: only `*.json` regular files are read (no symbolic
//! links, FIFOs or devices), each at most [`MAX_FILE_BYTES`], at most [`MAX_FILES`] of them in name order, and a file
//! that cannot be read is reported with the reason instead of failing the whole listing.

use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

/// The folder's name inside the app data folder.
pub const FOLDER: &str = "packs";
/// The largest pack file read, the same limit as a pack file picked in the chooser.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
/// The most pack files read; the rest are counted in [`PackFolder::skipped`].
pub const MAX_FILES: usize = 64;
/// The most folder entries looked at, so a folder with a huge number of files cannot stall the listing.
pub const MAX_ENTRIES: usize = 10_000;

const OPEN: &str = "/usr/bin/open";

/// One `*.json` file in the folder: its text, or why it could not be read.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PackFile {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the folder holds.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct PackFolder {
    /// The folder's full path, shown in the chooser.
    pub path: String,
    pub files: Vec<PackFile>,
    /// `*.json` files past [`MAX_FILES`], not read.
    pub skipped: usize,
    /// The folder holds more than [`MAX_ENTRIES`] entries, so some `*.json` files may not have been looked at.
    pub truncated: bool,
}

/// Where the folder lives.
pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join(FOLDER)
}

/// Creates the folder when it is missing. An existing path that is not a real folder (a file, or a symbolic link
/// even to a folder) is refused and left alone.
fn ensure(folder: &Path) -> Result<(), String> {
    match fs::symlink_metadata(folder) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => Err(format!(
            "{} must be a folder, not a file or a symbolic link",
            folder.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(folder)
            .map_err(|error| format!("could not create the packs folder: {error}")),
        Err(error) => Err(format!("could not inspect the packs folder: {error}")),
    }
}

fn is_pack_name(name: &str) -> bool {
    !name.starts_with('.')
        && Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
}

fn too_large() -> String {
    format!("it is larger than {} KiB", MAX_FILE_BYTES / 1024)
}

fn read_one(path: &Path) -> Result<String, String> {
    // A plain reason for the usual mistakes; read_private_file checks the opened file again, so a file swapped after
    // this check (for a link, a FIFO or a device) is still refused, and is opened without blocking.
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("could not inspect it: {error}"))?;
    let kind = metadata.file_type();
    if kind.is_symlink() {
        return Err("it is a symbolic link; put the file itself in the folder".into());
    }
    if kind.is_dir() {
        return Err("it is a folder, not a file".into());
    }
    if !kind.is_file() {
        return Err("it is not a regular file".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err(too_large());
    }
    let (bytes, _) = crate::store::read_private_file(path, MAX_FILE_BYTES, "pack file")?
        .ok_or("the file was removed while the folder was read")?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(too_large());
    }
    String::from_utf8(bytes).map_err(|_| "a pack file must be UTF-8 text".to_string())
}

/// Lists and reads the folder, creating it first when it is missing.
pub fn list(app_data: &Path) -> Result<PackFolder, String> {
    list_with(app_data, MAX_ENTRIES)
}

fn list_with(app_data: &Path, max_entries: usize) -> Result<PackFolder, String> {
    let folder = dir(app_data);
    ensure(&folder)?;
    let mut entries = fs::read_dir(&folder)
        .map_err(|error| format!("could not read the packs folder: {error}"))?;
    let mut names: Vec<String> = entries
        .by_ref()
        .take(max_entries)
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| is_pack_name(name))
        .collect();
    let truncated = entries.next().is_some();
    // A plain check that the folder was not swapped for a symbolic link while it was listed. It does not close the
    // race (each file is then opened by its path), but only a process running as the user could win it, and that
    // process could as well write the files here; whatever is read is still checked like any pack file.
    ensure(&folder)?;
    names.sort();
    let skipped = names.len().saturating_sub(MAX_FILES);
    let files = names
        .into_iter()
        .take(MAX_FILES)
        .map(|file| match read_one(&folder.join(&file)) {
            Ok(text) => PackFile {
                file,
                text: Some(text),
                error: None,
            },
            Err(error) => PackFile {
                file,
                text: None,
                error: Some(error),
            },
        })
        .collect();
    Ok(PackFolder {
        path: folder.display().to_string(),
        files,
        skipped,
        truncated,
    })
}

/// Opens the folder in the Finder, creating it first when it is missing. The Finder is named so that a path which is
/// not a folder is not handed to whatever app claims it; the folder is checked just before, though not atomically.
pub fn open(app_data: &Path) -> Result<(), String> {
    let folder = dir(app_data);
    ensure(&folder)?;
    let status = Command::new(OPEN)
        .args(["-a", "Finder"])
        .arg(&folder)
        .status()
        .map_err(|_| "Could not open the packs folder.".to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Could not open the packs folder.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_missing_folder_is_created_and_lists_nothing() {
        let app_data = temp();
        let listed = list(app_data.path()).unwrap();
        assert!(dir(app_data.path()).is_dir());
        assert_eq!(listed.files, Vec::new());
        assert_eq!(listed.skipped, 0);
        assert!(!listed.truncated);
        assert_eq!(listed.path, dir(app_data.path()).display().to_string());
    }

    #[test]
    fn only_json_regular_files_are_read_in_name_order() {
        let app_data = temp();
        let folder = dir(app_data.path());
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("b.json"), "{\"b\":1}").unwrap();
        fs::write(folder.join("A.JSON"), "{}").unwrap();
        fs::write(folder.join("notes.txt"), "not a pack").unwrap();
        fs::write(folder.join(".hidden.json"), "{}").unwrap();
        fs::create_dir(folder.join("dir.json")).unwrap();
        std::os::unix::fs::symlink(folder.join("b.json"), folder.join("link.json")).unwrap();
        fs::write(folder.join("big.json"), vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
        fs::write(folder.join("latin1.json"), [0xff, 0xfe]).unwrap();
        // Refused by the check before opening; a FIFO swapped in after it is refused by read_private_file, which opens
        // without blocking (that race is not exercised here).
        let fifo = std::ffi::CString::new(folder.join("fifo.json").as_os_str().as_encoded_bytes())
            .unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let listed = list(app_data.path()).unwrap();
        let names: Vec<&str> = listed.files.iter().map(|file| file.file.as_str()).collect();
        assert_eq!(
            names,
            [
                "A.JSON",
                "b.json",
                "big.json",
                "dir.json",
                "fifo.json",
                "latin1.json",
                "link.json"
            ]
        );
        let by_name = |name: &str| listed.files.iter().find(|file| file.file == name).unwrap();
        assert_eq!(by_name("b.json").text.as_deref(), Some("{\"b\":1}"));
        for (refused, reason) in [
            ("big.json", "larger than 256 KiB"),
            ("dir.json", "a folder"),
            ("fifo.json", "not a regular file"),
            ("latin1.json", "UTF-8"),
            ("link.json", "symbolic link"),
        ] {
            assert!(by_name(refused).text.is_none(), "{refused} was read");
            let error = by_name(refused).error.as_deref().unwrap_or_default();
            assert!(error.contains(reason), "{refused}: {error}");
        }
    }

    #[test]
    fn files_past_the_limit_are_counted_not_read() {
        let app_data = temp();
        let folder = dir(app_data.path());
        fs::create_dir(&folder).unwrap();
        for index in 0..MAX_FILES + 3 {
            fs::write(folder.join(format!("{index:03}.json")), "{}").unwrap();
        }
        let listed = list(app_data.path()).unwrap();
        assert_eq!(listed.files.len(), MAX_FILES);
        assert_eq!(listed.skipped, 3);
        assert!(!listed.truncated);
    }

    #[test]
    fn a_folder_with_too_many_entries_is_marked_truncated() {
        let app_data = temp();
        let folder = dir(app_data.path());
        fs::create_dir(&folder).unwrap();
        for index in 0..5 {
            fs::write(folder.join(format!("{index}.txt")), "").unwrap();
        }
        assert!(!list_with(app_data.path(), 5).unwrap().truncated);
        fs::write(folder.join("extra.json"), "{}").unwrap();
        assert!(list_with(app_data.path(), 5).unwrap().truncated);
    }

    #[test]
    fn a_folder_that_is_a_link_or_a_file_is_refused() {
        let app_data = temp();
        let elsewhere = temp();
        std::os::unix::fs::symlink(elsewhere.path(), dir(app_data.path())).unwrap();
        assert!(list(app_data.path()).is_err());
        let other = temp();
        fs::write(dir(other.path()), "file").unwrap();
        assert!(list(other.path()).is_err());
    }
}
