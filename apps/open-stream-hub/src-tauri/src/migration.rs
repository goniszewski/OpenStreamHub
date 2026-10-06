use crate::store::{MAX_PROFILES, UnreadableProfile, is_hidden_entry, is_stale, validate_id};
use cooltek_sdk_service::Profile;
use std::{
    fs::{self, File, FileTimes, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};
use uuid::Uuid;

// The only migration source is the previous application identifier's sibling
// data directory. Neither this path nor the destination is provided through IPC.
const LEGACY_APP_ID: &str = "dev.cooltek.masterhub";

struct Snapshot {
    filename: String,
    bytes: Vec<u8>,
    modified: SystemTime,
}

/// Returns the profile storage path, migrating legacy profiles into it when it
/// does not exist yet, plus the legacy entries that were left behind. The
/// legacy folder is only ever read.
pub fn prepare_store(app_data: &Path) -> Result<(PathBuf, Vec<UnreadableProfile>), String> {
    let parent = app_data
        .parent()
        .ok_or_else(|| "application data has no parent directory".to_string())?;
    check_directory(parent, "application data parent")?;
    if check_directory(app_data, "OpenStreamHub application data")? {
        remove_stale_staging(app_data);
    }
    let destination = app_data.join("profiles");
    if check_directory(&destination, "OpenStreamHub profile storage")? {
        // An existing destination is authoritative, including an empty or
        // corrupt store. Never merge legacy data into it or overwrite it.
        return Ok((destination, Vec::new()));
    }

    let legacy_app = parent.join(LEGACY_APP_ID);
    if !check_directory(&legacy_app, "legacy application data")? {
        return Ok((destination, Vec::new()));
    }
    let legacy_profiles = legacy_app.join("profiles");
    if !check_directory(&legacy_profiles, "legacy profile storage")? {
        return Ok((destination, Vec::new()));
    }

    // Capture and validate every bounded file before writing any new store.
    // Raw JSON bytes and file modification times are preserved exactly.
    let (snapshots, skipped) = snapshots(&legacy_profiles)?;
    if !check_directory(app_data, "OpenStreamHub application data")? {
        fs::create_dir_all(app_data)
            .map_err(|error| format!("could not create OpenStreamHub application data: {error}"))?;
    }
    let staging_path = app_data.join(format!(".profiles-migration-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&staging_path)
        .map_err(|error| format!("could not stage legacy profile migration: {error}"))?;
    let mut staging = StagingDirectory {
        path: staging_path,
        published: false,
    };
    protect_directory(&staging.path)?;
    for snapshot in snapshots {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(staging.path.join(&snapshot.filename))
            .map_err(|error| {
                format!(
                    "could not stage legacy profile {}: {error}",
                    snapshot.filename
                )
            })?;
        file.write_all(&snapshot.bytes).map_err(|error| {
            format!(
                "could not copy legacy profile {}: {error}",
                snapshot.filename
            )
        })?;
        file.set_times(FileTimes::new().set_modified(snapshot.modified))
            .map_err(|error| {
                format!("could not preserve legacy profile modification time: {error}")
            })?;
        file.sync_all()
            .map_err(|error| format!("could not flush migrated profile: {error}"))?;
    }
    File::open(&staging.path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("could not flush staged profile storage: {error}"))?;
    // The exclusive rename is atomic and refuses an existing destination even
    // if another app process creates an empty profiles directory during copying.
    publish_exclusive(&staging.path, &destination)?;
    staging.published = true;
    File::open(app_data)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("could not flush migrated profile storage: {error}"))?;
    Ok((destination, skipped))
}

/// Deletes stale `.profiles-migration-*.tmp` folders left by an interrupted
/// migration. Recent ones may belong to another app process mid-migration.
fn remove_stale_staging(app_data: &Path) {
    let Ok(entries) = fs::read_dir(app_data) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(".profiles-migration-") && name.ends_with(".tmp")) {
            continue;
        }
        // DirEntry metadata does not follow symbolic links.
        if entry
            .metadata()
            .is_ok_and(|metadata| metadata.is_dir() && is_stale(&metadata))
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

fn check_directory(path: &Path, label: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(true),
        Ok(_) => Err(format!(
            "{label} must be a directory, not a symbolic link or file"
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("could not inspect {label}: {error}")),
    }
}

/// Valid legacy profiles, at most `MAX_PROFILES` by filename, plus a report
/// for every visible entry that is not migrated. Hidden entries such as
/// .DS_Store are never profiles and are neither copied nor reported.
fn snapshots(directory: &Path) -> Result<(Vec<Snapshot>, Vec<UnreadableProfile>), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("could not read legacy profile storage: {error}"))?;
    let mut skipped = Vec::new();
    let mut skip = |filename: &str, error: String| {
        // The report is bounded even for a folder full of stray files.
        if skipped.len() < MAX_PROFILES {
            skipped.push(UnreadableProfile {
                id: filename
                    .strip_suffix(".json")
                    .unwrap_or(filename)
                    .to_owned(),
                error: format!(
                    "not migrated; left unchanged in {}: {error}",
                    directory.display()
                ),
            });
        }
    };
    let mut filenames = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read legacy profile entry: {error}"))?;
        if is_hidden_entry(&entry.file_name()) {
            continue;
        }
        match entry.file_name().into_string() {
            Ok(filename) => filenames.push(filename),
            Err(filename) => skip(
                &filename.to_string_lossy(),
                "legacy profile filename is not valid UTF-8".into(),
            ),
        }
    }
    filenames.sort();
    let mut snapshots = Vec::new();
    for filename in filenames {
        if snapshots.len() == MAX_PROFILES {
            skip(
                &filename,
                format!("only the first {MAX_PROFILES} legacy profiles are migrated"),
            );
            continue;
        }
        match snapshot(directory, filename.clone()) {
            Ok(snapshot) => snapshots.push(snapshot),
            Err(error) => skip(&filename, error),
        }
    }
    Ok((snapshots, skipped))
}

/// Reads and validates one legacy profile without following links.
fn snapshot(directory: &Path, filename: String) -> Result<Snapshot, String> {
    let id = filename.strip_suffix(".json").ok_or_else(|| {
        format!("legacy profile storage contains an unexpected entry: {filename}")
    })?;
    validate_id(id)
        .map_err(|error| format!("invalid legacy profile filename {filename}: {error}"))?;
    let path = directory.join(&filename);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("could not inspect legacy profile {filename}: {error}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "legacy profile {filename} must be a regular file, not a symbolic link"
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&path)
        .map_err(|error| format!("could not open legacy profile {filename}: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("could not inspect opened legacy profile {filename}: {error}"))?;
    if !metadata.is_file() || metadata.len() > Profile::MAX_JSON_BYTES as u64 {
        return Err(format!(
            "legacy profile {filename} is not a bounded regular profile file"
        ));
    }
    let modified = metadata
        .modified()
        .map_err(|error| format!("could not read legacy profile modification time: {error}"))?;
    u64::try_from(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "legacy profile modification time predates the Unix epoch".to_string())?
            .as_millis(),
    )
    .map_err(|_| "legacy profile modification time is out of range".to_string())?;
    let mut bytes = Vec::new();
    file.take((Profile::MAX_JSON_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read legacy profile {filename}: {error}"))?;
    Profile::from_json(&bytes)
        .map_err(|error| format!("invalid legacy profile {filename}: {error}"))?;
    Ok(Snapshot {
        filename,
        bytes,
        modified,
    })
}

fn protect_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("could not protect migrated profile storage: {error}"))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn publish_exclusive(source: &Path, destination: &Path) -> Result<(), String> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| "migration staging path contains a NUL byte".to_string())?;
    let destination = CString::new(destination.as_os_str().as_bytes())
        .map_err(|_| "migration destination path contains a NUL byte".to_string())?;
    // SAFETY: Both arguments are live NUL-terminated paths. RENAME_EXCL is the
    // macOS no-replacement flag; it changes directory names without touching
    // the legacy source directory or an existing destination.
    let result =
        unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
    if result != 0 {
        return Err(format!(
            "could not publish legacy profiles without replacing existing storage: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn publish_exclusive(_source: &Path, _destination: &Path) -> Result<(), String> {
    Err("legacy profile migration requires macOS exclusive directory rename support".into())
}

struct StagingDirectory {
    path: PathBuf,
    published: bool,
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::store::ProfileStore;
    use std::time::{Duration, UNIX_EPOCH};

    fn paths() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temporary = tempfile::tempdir().unwrap();
        let legacy = temporary.path().join(LEGACY_APP_ID).join("profiles");
        fs::create_dir_all(&legacy).unwrap();
        let new_app = temporary.path().join("dev.cooltek.openstreamhub");
        (temporary, legacy, new_app)
    }

    fn profile_bytes(name: &str) -> Vec<u8> {
        let mut profile =
            Profile::from_json(include_bytes!("../../../../examples/profiles/macos.json")).unwrap();
        profile.name = name.into();
        let mut bytes = serde_json::to_vec_pretty(&profile).unwrap();
        bytes.extend_from_slice(b" \n");
        bytes
    }

    #[test]
    fn migration_preserves_ids_raw_contents_mtime_and_original_store_and_is_idempotent() {
        let (_temporary, legacy, new_app) = paths();
        let id = Uuid::new_v4().to_string();
        let old_file = legacy.join(format!("{id}.json"));
        let bytes = profile_bytes("Custom profile");
        fs::write(&old_file, &bytes).unwrap();
        File::options()
            .write(true)
            .open(&old_file)
            .unwrap()
            .set_times(
                FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_789)),
            )
            .unwrap();
        let modified = fs::metadata(&old_file).unwrap().modified().unwrap();
        let store = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
        let new_file = new_app.join("profiles").join(format!("{id}.json"));
        assert_eq!(store.bootstrap().unwrap().profile.unwrap().id, id);
        assert_eq!(fs::read(&new_file).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&new_file).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(fs::read(&old_file).unwrap(), bytes);
        assert_eq!(
            fs::metadata(&old_file).unwrap().modified().unwrap(),
            modified
        );
        // Later legacy edits never replace the now-existing destination.
        fs::write(&old_file, profile_bytes("Later legacy edit")).unwrap();
        let restarted = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
        assert_eq!(restarted.load(&id).unwrap().profile.name, "Custom profile");
        assert_eq!(fs::read(&new_file).unwrap(), bytes);
        assert_eq!(fs::read_dir(new_app).unwrap().count(), 1);
    }

    fn store_names(new_app: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(new_app.join("profiles"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn invalid_legacy_profiles_are_reported_and_valid_ones_still_migrate() {
        for bytes in [
            b"corrupt".to_vec(),
            vec![b' '; Profile::MAX_JSON_BYTES + 1],
            br#"{"schema_version":1,"name":"Bad","bindings":[],"extra":true}"#.to_vec(),
        ] {
            let (_temporary, legacy, new_app) = paths();
            let id = Uuid::new_v4().to_string();
            let old_file = legacy.join(format!("{id}.json"));
            fs::write(&old_file, &bytes).unwrap();
            let valid = Uuid::new_v4().to_string();
            fs::write(legacy.join(format!("{valid}.json")), profile_bytes("Valid")).unwrap();
            let store = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
            let listing = store.list().unwrap();
            assert_eq!(listing.profiles.len(), 1);
            assert_eq!(listing.profiles[0].id, valid);
            assert_eq!(listing.unreadable.len(), 1);
            assert_eq!(listing.unreadable[0].id, id);
            assert!(listing.unreadable[0].error.contains("not migrated"));
            assert_eq!(store_names(&new_app), vec![format!("{valid}.json")]);
            assert_eq!(fs::read(&old_file).unwrap(), bytes);
            assert_eq!(fs::read_dir(&legacy).unwrap().count(), 2);
        }
    }

    #[test]
    fn unexpected_legacy_entries_are_reported_not_copied() {
        for filename in ["invalid-id.json", "notes.txt"] {
            let (_temporary, legacy, new_app) = paths();
            fs::write(legacy.join(filename), profile_bytes("Legacy")).unwrap();
            fs::create_dir(legacy.join("folder")).unwrap();
            let store = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
            let ids: Vec<_> = store
                .list()
                .unwrap()
                .unreadable
                .into_iter()
                .map(|item| item.id)
                .collect();
            assert_eq!(ids.len(), 2, "{ids:?}");
            assert!(ids.contains(&"folder".to_string()));
            assert!(
                ids.contains(
                    &filename
                        .strip_suffix(".json")
                        .unwrap_or(filename)
                        .to_string()
                )
            );
            assert!(store_names(&new_app).is_empty());
            assert_eq!(
                fs::read(legacy.join(filename)).unwrap(),
                profile_bytes("Legacy")
            );
            assert!(legacy.join("folder").is_dir());
            // The store holds no data of its own, so it gets the sample
            // profile while the legacy entries stay reported for this launch.
            let bootstrap = store.bootstrap().unwrap();
            assert_eq!(bootstrap.profiles.len(), 1);
            assert_eq!(bootstrap.unreadable.len(), 2);
        }
    }

    #[test]
    fn legacy_profiles_beyond_the_limit_are_reported_and_the_rest_migrate() {
        let (_temporary, legacy, new_app) = paths();
        let bytes = profile_bytes("Many");
        let mut ids: Vec<String> = (0..MAX_PROFILES + 2)
            .map(|_| Uuid::new_v4().to_string())
            .collect();
        for id in &ids {
            fs::write(legacy.join(format!("{id}.json")), &bytes).unwrap();
        }
        ids.sort();
        let store = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
        let listing = store.list().unwrap();
        assert_eq!(listing.profiles.len(), MAX_PROFILES);
        let skipped: Vec<_> = listing
            .unreadable
            .iter()
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(skipped, ids[MAX_PROFILES..].to_vec());
        assert_eq!(store_names(&new_app).len(), MAX_PROFILES);
        assert_eq!(fs::read_dir(&legacy).unwrap().count(), MAX_PROFILES + 2);
    }

    #[test]
    fn stale_staging_folders_are_removed_and_recent_ones_kept() {
        let (_temporary, legacy, new_app) = paths();
        fs::write(
            legacy.join(format!("{}.json", Uuid::new_v4())),
            profile_bytes("Legacy"),
        )
        .unwrap();
        fs::create_dir_all(&new_app).unwrap();
        let stale = new_app.join(format!(".profiles-migration-{}.tmp", Uuid::new_v4()));
        let recent = new_app.join(format!(".profiles-migration-{}.tmp", Uuid::new_v4()));
        let other = new_app.join(".other.tmp");
        for folder in [&stale, &recent, &other] {
            fs::create_dir(folder).unwrap();
            fs::write(folder.join("partial.json"), b"partial").unwrap();
        }
        let old = SystemTime::now() - crate::store::STALE_TEMPORARY - Duration::from_secs(60);
        for folder in [&stale, &other] {
            File::open(folder)
                .unwrap()
                .set_times(FileTimes::new().set_modified(old))
                .unwrap();
        }
        let (destination, skipped) = prepare_store(&new_app).unwrap();
        assert!(skipped.is_empty());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 1);
        assert!(!stale.exists());
        assert!(recent.join("partial.json").exists());
        assert!(other.join("partial.json").exists());
    }

    #[test]
    fn hidden_legacy_entries_are_skipped_without_aborting_or_being_copied() {
        let (_temporary, legacy, new_app) = paths();
        let id = Uuid::new_v4().to_string();
        let bytes = profile_bytes("Legacy");
        fs::write(legacy.join(format!("{id}.json")), &bytes).unwrap();
        for name in [".DS_Store", "._sidecar.json", ".interrupted.tmp"] {
            fs::write(legacy.join(name), b"hidden").unwrap();
        }
        let store = ProfileStore::open_for_app_data(new_app.clone()).unwrap();
        let bootstrap = store.bootstrap().unwrap();
        assert_eq!(bootstrap.profile.unwrap().id, id);
        assert!(bootstrap.unreadable.is_empty());
        let migrated: Vec<_> = fs::read_dir(new_app.join("profiles"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(migrated, vec![format!("{id}.json")]);
        assert_eq!(fs::read(legacy.join(".DS_Store")).unwrap(), b"hidden");
        assert_eq!(fs::read_dir(&legacy).unwrap().count(), 4);
    }

    #[test]
    fn existing_destination_is_never_merged_or_overwritten_even_if_empty() {
        for empty in [true, false] {
            let (_temporary, legacy, new_app) = paths();
            fs::write(legacy.join("invalid.json"), b"invalid legacy data").unwrap();
            let destination = new_app.join("profiles");
            fs::create_dir_all(&destination).unwrap();
            let sentinel = destination.join("sentinel.data");
            if !empty {
                fs::write(&sentinel, b"existing destination").unwrap();
            }
            assert_eq!(
                prepare_store(&new_app).unwrap(),
                (destination.clone(), Vec::new())
            );
            assert_eq!(
                fs::read_dir(&destination).unwrap().count(),
                usize::from(!empty)
            );
            if !empty {
                assert_eq!(fs::read(&sentinel).unwrap(), b"existing destination");
            }
        }
    }

    #[test]
    fn exclusive_publication_refuses_a_destination_created_during_copying() {
        let temporary = tempfile::tempdir().unwrap();
        let staging = temporary.path().join("staging");
        let destination = temporary.path().join("profiles");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("profile.json"), b"staged data").unwrap();
        fs::create_dir(&destination).unwrap();
        assert!(publish_exclusive(&staging, &destination).is_err());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
        assert_eq!(
            fs::read(staging.join("profile.json")).unwrap(),
            b"staged data"
        );
    }

    #[test]
    fn symbolic_legacy_roots_and_profiles_are_rejected_without_following_or_reseeding() {
        for link_target in ["app", "profiles", "file", "parent", "destination"] {
            let (temporary, legacy, new_app) = paths();
            let bytes = profile_bytes("Legacy");
            let filename = format!("{}.json", Uuid::new_v4());
            let outside = temporary.path().join("outside");
            fs::create_dir(&outside).unwrap();
            fs::write(outside.join(&filename), &bytes).unwrap();
            let app_data = match link_target {
                "app" => {
                    fs::remove_dir_all(legacy.parent().unwrap()).unwrap();
                    std::os::unix::fs::symlink(&outside, legacy.parent().unwrap()).unwrap();
                    new_app.clone()
                }
                "profiles" => {
                    fs::remove_dir(&legacy).unwrap();
                    std::os::unix::fs::symlink(&outside, &legacy).unwrap();
                    new_app.clone()
                }
                "file" => {
                    std::os::unix::fs::symlink(outside.join(&filename), legacy.join(&filename))
                        .unwrap();
                    new_app.clone()
                }
                "parent" => {
                    let parent_link = temporary.path().join("linked-parent");
                    std::os::unix::fs::symlink(&outside, &parent_link).unwrap();
                    parent_link.join("dev.cooltek.openstreamhub")
                }
                "destination" => {
                    std::os::unix::fs::symlink(&outside, &new_app).unwrap();
                    new_app.clone()
                }
                _ => unreachable!(),
            };
            if link_target == "file" {
                // A linked legacy profile is reported, never followed or copied.
                let store = ProfileStore::open_for_app_data(app_data.clone()).unwrap();
                let unreadable = store.list().unwrap().unreadable;
                assert_eq!(unreadable.len(), 1);
                assert!(unreadable[0].error.contains("symbolic link"));
                assert!(store_names(&app_data).is_empty());
                assert!(
                    legacy
                        .join(&filename)
                        .symlink_metadata()
                        .unwrap()
                        .is_symlink()
                );
            } else {
                assert!(
                    ProfileStore::open_for_app_data(app_data).is_err(),
                    "accepted {link_target} link"
                );
            }
            assert_eq!(fs::read(outside.join(&filename)).unwrap(), bytes);
            assert!(!outside.join("profiles").exists());
        }
    }
}
