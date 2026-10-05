use crate::store::{MAX_PROFILES, is_hidden_entry, validate_id};
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

pub fn prepare_store(app_data: &Path) -> Result<PathBuf, String> {
    let parent = app_data
        .parent()
        .ok_or_else(|| "application data has no parent directory".to_string())?;
    check_directory(parent, "application data parent")?;
    check_directory(app_data, "OpenStreamHub application data")?;
    let destination = app_data.join("profiles");
    if check_directory(&destination, "OpenStreamHub profile storage")? {
        // An existing destination is authoritative, including an empty or
        // corrupt store. Never merge legacy data into it or overwrite it.
        return Ok(destination);
    }

    let legacy_app = parent.join(LEGACY_APP_ID);
    if !check_directory(&legacy_app, "legacy application data")? {
        return Ok(destination);
    }
    let legacy_profiles = legacy_app.join("profiles");
    if !check_directory(&legacy_profiles, "legacy profile storage")? {
        return Ok(destination);
    }

    // Capture and validate every bounded file before writing any new store.
    // Raw JSON bytes and file modification times are preserved exactly.
    let snapshots = snapshots(&legacy_profiles)?;
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
    Ok(destination)
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

fn snapshots(directory: &Path) -> Result<Vec<Snapshot>, String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("could not read legacy profile storage: {error}"))?;
    let mut snapshots = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not read legacy profile entry: {error}"))?;
        // Hidden entries such as .DS_Store are never profiles. They are not
        // copied, and the legacy store is never modified.
        if is_hidden_entry(&entry.file_name()) {
            continue;
        }
        if snapshots.len() == MAX_PROFILES {
            return Err(format!(
                "legacy profile storage exceeds {MAX_PROFILES} profiles"
            ));
        }
        let filename = entry
            .file_name()
            .into_string()
            .map_err(|_| "legacy profile filename is not valid UTF-8".to_string())?;
        let id = filename.strip_suffix(".json").ok_or_else(|| {
            format!("legacy profile storage contains an unexpected entry: {filename}")
        })?;
        validate_id(id)
            .map_err(|error| format!("invalid legacy profile filename {filename}: {error}"))?;
        let path = entry.path();
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
        let metadata = file.metadata().map_err(|error| {
            format!("could not inspect opened legacy profile {filename}: {error}")
        })?;
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
                .map_err(|_| {
                    "legacy profile modification time predates the Unix epoch".to_string()
                })?
                .as_millis(),
        )
        .map_err(|_| "legacy profile modification time is out of range".to_string())?;
        let mut bytes = Vec::new();
        file.take((Profile::MAX_JSON_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read legacy profile {filename}: {error}"))?;
        Profile::from_json(&bytes)
            .map_err(|error| format!("invalid legacy profile {filename}: {error}"))?;
        snapshots.push(Snapshot {
            filename,
            bytes,
            modified,
        });
    }
    Ok(snapshots)
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

    #[test]
    fn invalid_legacy_profiles_abort_before_publication_and_never_reseed() {
        for bytes in [
            b"corrupt".to_vec(),
            vec![b' '; Profile::MAX_JSON_BYTES + 1],
            br#"{"schema_version":1,"name":"Bad","bindings":[],"extra":true}"#.to_vec(),
        ] {
            let (_temporary, legacy, new_app) = paths();
            let id = Uuid::new_v4().to_string();
            let old_file = legacy.join(format!("{id}.json"));
            fs::write(&old_file, &bytes).unwrap();
            fs::write(
                legacy.join(format!("{}.json", Uuid::new_v4())),
                profile_bytes("Valid"),
            )
            .unwrap();
            assert!(ProfileStore::open_for_app_data(new_app.clone()).is_err());
            assert!(!new_app.join("profiles").exists());
            assert_eq!(fs::read(&old_file).unwrap(), bytes);
            assert_eq!(fs::read_dir(legacy).unwrap().count(), 2);
        }
    }

    #[test]
    fn unexpected_filenames_are_not_silently_migrated_or_seeded() {
        for filename in ["invalid-id.json", "notes.txt"] {
            let (_temporary, legacy, new_app) = paths();
            fs::write(legacy.join(filename), profile_bytes("Legacy")).unwrap();
            assert!(ProfileStore::open_for_app_data(new_app.clone()).is_err());
            assert!(!new_app.join("profiles").exists());
            assert!(legacy.join(filename).exists());
        }
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
            assert_eq!(prepare_store(&new_app).unwrap(), destination);
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
            assert!(
                ProfileStore::open_for_app_data(app_data).is_err(),
                "accepted {link_target} link"
            );
            assert_eq!(fs::read(outside.join(&filename)).unwrap(), bytes);
            assert!(!outside.join("profiles").exists());
        }
    }
}
