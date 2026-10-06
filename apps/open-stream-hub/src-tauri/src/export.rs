//! Local profile export. `export_profile` writes one pretty JSON file under
//! `<app data>/exports/`. Nothing is uploaded; the user decides where the file
//! goes. Files are never overwritten and only the newest `KEEP_EXPORTS` files
//! of each profile slug matching the export name pattern are kept.

use crate::{diagnostics::utc_stamps, replay};
use cooltek_sdk_service::Profile;
use serde::Serialize;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const DIRECTORY: &str = "exports";
pub const KEEP_EXPORTS: usize = 20;
const MAX_SLUG: usize = 40;
const STAMP_LEN: usize = 15;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub bytes: u64,
}

/// Lowercase ASCII letters and digits joined by single dashes, at most 40
/// characters; `profile` when nothing usable remains.
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.truncate(MAX_SLUG);
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "profile".into()
    } else {
        slug.into()
    }
}

pub fn file_name(slug: &str, stamp: &str) -> String {
    format!("{slug}-{stamp}.json")
}

/// `(slug, sort stamp)` of `<slug>-yyyymmdd-hhmmss.json`, or None for any
/// other name.
fn export_stamp(name: &str) -> Option<(&str, &str)> {
    let body = name.strip_suffix(".json")?;
    let stamp = body.get(body.len().checked_sub(STAMP_LEN)?..)?;
    let slug = body[..body.len() - STAMP_LEN].strip_suffix('-')?;
    let stamp_ok = stamp.bytes().enumerate().all(|(index, byte)| {
        if index == 8 {
            byte == b'-'
        } else {
            byte.is_ascii_digit()
        }
    });
    let slug_ok = !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    (stamp_ok && slug_ok).then_some((slug, stamp))
}

pub(crate) fn create_private_dir(directory: &Path, label: &str) -> Result<(), String> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(directory)
        .map_err(|error| format!("could not create the {label} folder: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not secure the {label} folder: {error}"))?;
    }
    Ok(())
}

/// Writes a new 0600 file without ever replacing one: content goes to a
/// private temporary file that is hard-linked to its final name (failing if
/// that name exists) and unlinked, then the folder itself is flushed so the
/// new name survives a crash.
pub(crate) fn write_new(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    what: &str,
) -> Result<PathBuf, String> {
    create_private_dir(directory, what)?;
    let path = directory.join(name);
    if path.symlink_metadata().is_ok() {
        return Err(format!(
            "{} already exists; wait a second and try again",
            path.display()
        ));
    }
    let temporary = directory.join(format!(".export-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options
        .open(&temporary)
        .and_then(|mut file| {
            io::Write::write_all(&mut file, bytes)?;
            file.sync_all()
        })
        .and_then(|()| fs::hard_link(&temporary, &path));
    let _ = fs::remove_file(&temporary);
    written.map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => format!(
            "{} already exists; wait a second and try again",
            path.display()
        ),
        _ => format!("could not write the {what}: {error}"),
    })?;
    #[cfg(unix)]
    fs::File::open(directory)
        .and_then(|folder| folder.sync_all())
        .map_err(|error| format!("could not flush the {what} folder: {error}"))?;
    Ok(path)
}

/// Deletes all but the newest `keep` files of each slug among the names that
/// `key` maps to `(slug, sort stamp)`; other files are ignored.
pub(crate) fn prune_per_slug(directory: &Path, keep: usize, key: fn(&str) -> Option<(&str, &str)>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files: Vec<(String, String, String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (slug, stamp) = key(&name)?;
            Some((slug.to_owned(), stamp.to_owned(), name, entry.path()))
        })
        .collect();
    // Newest first within each slug.
    files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| (&b.1, &b.2).cmp(&(&a.1, &a.2))));
    let mut kept = 0;
    for (index, (slug, _, _, path)) in files.iter().enumerate() {
        if index == 0 || files[index - 1].0 != *slug {
            kept = 0;
        }
        kept += 1;
        if kept > keep {
            let _ = fs::remove_file(path);
        }
    }
}

/// Deletes all but the newest `keep` export files of each profile slug.
pub fn prune(directory: &Path, keep: usize) {
    prune_per_slug(directory, keep, export_stamp);
}

/// The command without Tauri: validates, writes, prunes.
pub fn export(
    profile: &Profile,
    app_data: &Path,
    generated_at: u64,
) -> Result<ExportResult, String> {
    replay::validate_profile(profile)?;
    let mut bytes = serde_json::to_vec_pretty(profile)
        .map_err(|error| format!("could not encode the profile: {error}"))?;
    bytes.push(b'\n');
    let directory = app_data.join(DIRECTORY);
    let (_, stamp) = utc_stamps(generated_at);
    let path = write_new(
        &directory,
        &file_name(&slug(&profile.name), &stamp),
        &bytes,
        "exported profile",
    )?;
    prune(&directory, KEEP_EXPORTS);
    Ok(ExportResult {
        path: path.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
    })
}

pub fn now_seconds() -> Result<u64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the system clock is before 1970".to_string())?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;

    #[test]
    fn slugs_are_lowercase_ascii_dashes_and_bounded() {
        assert_eq!(slug("Everyday"), "everyday");
        assert_eq!(slug("  My Desk -- Setup! "), "my-desk-setup");
        assert_eq!(slug("Zażółć"), "za");
        assert_eq!(slug("日本語"), "profile");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug(""), "profile");
        let long = slug(&format!("{}-tail", "a".repeat(39)));
        assert_eq!(long, "a".repeat(39));
        assert!(slug(&"ab ".repeat(40)).len() <= MAX_SLUG);
    }

    #[test]
    fn export_names_match_only_their_own_pattern() {
        assert!(export_stamp("folders-20261005-120000.json").is_some());
        assert!(export_stamp("a-b-20261005-120000.json").is_some());
        for name in [
            "notes.json",
            "-20261005-120000.json",
            "Folders-20261005-120000.json",
            "folders-20261005-120000.txt",
            "folders-2026100-5120000.json",
            "folders-20261005-12000x.json",
            "folders_20261005-120000.json",
        ] {
            assert!(export_stamp(name).is_none(), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn exports_are_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let profile = test_profiles::folders();
        let result = export(&profile, root.path(), 1_791_201_600).unwrap();
        let directory = root.path().join("exports");
        let path = directory.join("folders-20261005-120000.json");
        assert_eq!(PathBuf::from(&result.path), path);
        assert_eq!(result.bytes, fs::metadata(&path).unwrap().len());
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&directory), 0o700);
        let before = fs::read(&path).unwrap();
        let error = export(&profile, root.path(), 1_791_201_600).unwrap_err();
        assert!(error.contains("already exists"));
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        // The file is a valid profile that the app can import again.
        assert_eq!(Profile::from_json(&before).unwrap(), profile);
    }

    #[test]
    fn retention_keeps_the_newest_exports_per_slug_and_ignores_other_files() {
        let directory = tempfile::tempdir().unwrap();
        let directory = directory.path();
        // Two old exports of a rarely exported profile survive many newer
        // exports of another one.
        for index in 0..2 {
            let name = file_name("beta-desk", &format!("20000101-0000{index:02}"));
            fs::write(directory.join(name), "{}").unwrap();
        }
        for index in 0..23 {
            let name = file_name("alpha", &format!("20261005-1200{index:02}"));
            fs::write(directory.join(name), "{}").unwrap();
        }
        let unrelated = [
            "notes.json",
            "Gamma-20261005-120000.json",
            "gamma-20261005-120000.txt",
            ".export-x.tmp",
        ];
        for name in unrelated {
            fs::write(directory.join(name), "").unwrap();
        }
        fs::create_dir(directory.join("alpha-20000101-000000.json")).unwrap();
        prune(directory, KEEP_EXPORTS);
        let names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let kept = names.iter().filter(|name| name.contains("-1200")).count();
        // 20 export files plus the unrelated near-miss `Alpha-...120000.json`
        // and `.txt` that also contain "-1200".
        assert_eq!(kept, KEEP_EXPORTS + 2);
        for index in 0..3 {
            assert!(!names.contains(&file_name("alpha", &format!("20261005-1200{index:02}"))));
        }
        assert!(names.contains(&file_name("alpha", "20261005-120003")));
        assert!(names.contains(&file_name("alpha", "20261005-120022")));
        assert!(names.contains(&file_name("beta-desk", "20000101-000000")));
        assert!(names.contains(&file_name("beta-desk", "20000101-000001")));
        for name in unrelated {
            assert!(names.iter().any(|item| item == name), "{name}");
        }
        assert!(directory.join("alpha-20000101-000000.json").is_dir());
    }

    #[test]
    fn invalid_profiles_are_rejected_before_anything_is_written() {
        let root = tempfile::tempdir().unwrap();
        let mut profile = test_profiles::folders();
        profile.name = " ".into();
        let error = export(&profile, root.path(), 1_791_201_600).unwrap_err();
        assert!(!error.is_empty());
        assert!(!root.path().join("exports").exists());
    }
}
