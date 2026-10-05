use cooltek_sdk_service::Profile;
use serde::Serialize;
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    time::UNIX_EPOCH,
};
use uuid::Uuid;

pub(crate) const MAX_PROFILES: usize = 128;
const SAMPLE: &str = include_str!("../../../../examples/profiles/macos.json");
pub(crate) const CONFLICT_ERROR: &str = "this profile changed on disk after it was loaded, possibly in another OpenStreamHub window; reload the saved profile before saving again";

#[derive(Clone, Debug, Serialize)]
pub struct StoredProfile {
    pub id: String,
    pub profile: Profile,
    pub modified_at_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub binding_count: usize,
    pub modified_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnreadableProfile {
    pub id: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileListing {
    pub profiles: Vec<ProfileSummary>,
    pub unreadable: Vec<UnreadableProfile>,
}

#[derive(Debug, Serialize)]
pub struct AppBootstrap {
    /// The most recently modified readable profile, if any. `None` means the
    /// store holds only unreadable data, which is preserved and never reseeded.
    pub profile: Option<StoredProfile>,
    pub profiles: Vec<ProfileSummary>,
    pub unreadable: Vec<UnreadableProfile>,
    pub storage_path: String,
    pub desktop: bool,
}

/// Hidden entries (`.DS_Store`, AppleDouble `._*` files, interrupted
/// `.<uuid>.tmp` writes) are never profiles. They are left untouched.
pub(crate) fn is_hidden_entry(name: &OsStr) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

pub struct ProfileStore {
    directory: PathBuf,
}

impl ProfileStore {
    pub fn open_for_app_data(app_data: PathBuf) -> Result<Self, String> {
        let directory = crate::migration::prepare_store(&app_data)?;
        Self::open(directory)
    }

    pub fn open(directory: PathBuf) -> Result<Self, String> {
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err("profile storage must be a directory, not a symbolic link".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&directory)
                    .map_err(|error| format!("could not create profile storage: {error}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                        .map_err(|error| format!("could not protect profile storage: {error}"))?;
                }
            }
            Err(error) => return Err(format!("could not inspect profile storage: {error}")),
        }
        Ok(Self { directory })
    }

    pub fn bootstrap(&self) -> Result<AppBootstrap, String> {
        let mut listing = self.list()?;
        if listing.profiles.is_empty() && listing.unreadable.is_empty() {
            // Seed only an empty store, never beside unfamiliar or unreadable
            // data, which is retained for recovery. Hidden entries such as
            // .DS_Store are ignored.
            let mut entries = fs::read_dir(&self.directory)
                .map_err(|error| format!("could not inspect profile storage: {error}"))?;
            let has_data = entries.try_fold(false, |found, entry| {
                entry
                    .map(|entry| found || !is_hidden_entry(&entry.file_name()))
                    .map_err(|error| format!("could not inspect profile storage: {error}"))
            })?;
            if has_data {
                return Err("profile storage contains data but no usable profiles; preserve and inspect the files before recovery".into());
            }
            let mut profile = Profile::from_json(SAMPLE.as_bytes())?;
            profile.name = "Everyday".into();
            self.save(None, profile, None)?;
            listing = self.list()?;
        }
        let profile = match listing.profiles.first() {
            Some(selected) => Some(self.load(&selected.id)?),
            None => None,
        };
        Ok(AppBootstrap {
            profile,
            profiles: listing.profiles,
            unreadable: listing.unreadable,
            storage_path: self.directory.to_string_lossy().into_owned(),
            desktop: true,
        })
    }

    /// Lists readable profiles and reports each unreadable `.json` entry
    /// separately, so one damaged file never hides the others.
    pub fn list(&self) -> Result<ProfileListing, String> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("could not list profiles: {error}"))?;
        let mut profiles = Vec::new();
        let mut unreadable = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("could not read profile entry: {error}"))?;
            let path = entry.path();
            if is_hidden_entry(&entry.file_name())
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                continue;
            }
            if profiles.len() + unreadable.len() == MAX_PROFILES {
                return Err(format!("profile storage exceeds {MAX_PROFILES} profiles"));
            }
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                unreadable.push(UnreadableProfile {
                    id: entry.file_name().to_string_lossy().into_owned(),
                    error: "profile filename is not valid UTF-8".into(),
                });
                continue;
            };
            match self.load(id) {
                Ok(stored) => {
                    let binding_count = stored.profile.binding_count();
                    profiles.push(ProfileSummary {
                        id: stored.id,
                        name: stored.profile.name,
                        binding_count,
                        modified_at_ms: stored.modified_at_ms,
                    });
                }
                Err(error) => unreadable.push(UnreadableProfile {
                    id: id.to_owned(),
                    error,
                }),
            }
        }
        profiles.sort_by(|left, right| {
            right
                .modified_at_ms
                .cmp(&left.modified_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        unreadable.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(ProfileListing {
            profiles,
            unreadable,
        })
    }

    pub fn load(&self, id: &str) -> Result<StoredProfile, String> {
        let path = self.path(id)?;
        // Checked before opening so a FIFO or device node never blocks open().
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "profile no longer exists in storage".to_string()
            } else {
                format!("could not inspect profile: {error}")
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err("profile must be a regular file, not a symbolic link".into());
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
            .map_err(|error| format!("could not open profile: {error}"))?;
        // Re-check the opened file, not the path, in case it was swapped.
        let metadata = file
            .metadata()
            .map_err(|error| format!("could not inspect opened profile: {error}"))?;
        if !metadata.is_file() {
            return Err("profile must be a regular file, not a symbolic link".into());
        }
        if metadata.len() > Profile::MAX_JSON_BYTES as u64 {
            return Err("stored profile exceeds the size limit".into());
        }
        let mut bytes = Vec::new();
        file.take((Profile::MAX_JSON_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read profile: {error}"))?;
        // Legacy profiles are upgraded for the editor only. Reading must not
        // change file bytes, modification time, filename, or opaque identity.
        let profile = Profile::from_json(&bytes)?.into_v2()?;
        let modified_at_ms = metadata
            .modified()
            .map_err(|error| format!("could not read profile modification time: {error}"))?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "profile modification time predates the Unix epoch".to_string())?
            .as_millis()
            .try_into()
            .map_err(|_| "profile modification time is out of range".to_string())?;
        Ok(StoredProfile {
            id: id.to_owned(),
            profile,
            modified_at_ms,
        })
    }

    /// Saves a profile. For an existing ID, `expected_modified_at_ms` is the
    /// modification time the editor loaded; a mismatch means another window or
    /// process changed the file, and the save is rejected instead of
    /// silently overwriting it.
    pub fn save(
        &self,
        id: Option<&str>,
        profile: Profile,
        expected_modified_at_ms: Option<u64>,
    ) -> Result<StoredProfile, String> {
        // Explicit Save is the only operation that persists a schema upgrade.
        let profile = profile.into_v2()?;
        let id = match id {
            Some(id) => {
                // Never overwrite an unreadable existing profile during editing.
                let current = self.load(id)?;
                if expected_modified_at_ms
                    .is_some_and(|expected| expected != current.modified_at_ms)
                {
                    return Err(CONFLICT_ERROR.into());
                }
                id.to_owned()
            }
            None => {
                let listing = self.list()?;
                if listing.profiles.len() + listing.unreadable.len() >= MAX_PROFILES {
                    return Err(format!(
                        "profile storage permits at most {MAX_PROFILES} profiles; delete a profile before creating another"
                    ));
                }
                Uuid::new_v4().to_string()
            }
        };
        let path = self.path(&id)?;
        let bytes = serde_json::to_vec_pretty(&profile)
            .map_err(|error| format!("could not encode profile: {error}"))?;
        if bytes.len() > Profile::MAX_JSON_BYTES {
            return Err("encoded profile exceeds the size limit".into());
        }
        let temporary = self.directory.join(format!(".{}.tmp", Uuid::new_v4()));
        let result = (|| -> Result<(), String> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&temporary)
                .map_err(|error| format!("could not create temporary profile: {error}"))?;
            file.write_all(&bytes)
                .map_err(|error| format!("could not write profile: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("could not flush profile: {error}"))?;
            fs::rename(&temporary, &path)
                .map_err(|error| format!("could not atomically save profile: {error}"))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        // Persist the rename itself, not only the file contents.
        self.sync_directory()?;
        self.load(&id)
    }

    /// Deletes a readable profile. `expected_modified_at_ms` protects against
    /// removing a version changed elsewhere since it was listed or loaded.
    pub fn delete(&self, id: &str, expected_modified_at_ms: Option<u64>) -> Result<(), String> {
        let current = self.load(id)?;
        if expected_modified_at_ms.is_some_and(|expected| expected != current.modified_at_ms) {
            return Err(CONFLICT_ERROR.into());
        }
        fs::remove_file(self.path(id)?)
            .map_err(|error| format!("could not delete profile: {error}"))?;
        self.sync_directory()
    }

    fn sync_directory(&self) -> Result<(), String> {
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("could not flush profile storage: {error}"))
    }

    fn path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.directory.join(format!("{id}.json")))
    }
}

pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    let parsed =
        Uuid::parse_str(id).map_err(|_| "profile ID must be an opaque UUID".to_string())?;
    if id != parsed.to_string() {
        return Err("profile ID must be a canonical lowercase UUID".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;

    fn store() -> (tempfile::TempDir, ProfileStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(directory.path().join("profiles")).unwrap();
        (directory, store)
    }

    #[test]
    fn seed_save_restart_roundtrip_keeps_bindings_and_identity() {
        let (directory, store) = store();
        let initial = store.bootstrap().unwrap();
        assert!(initial.desktop);
        assert_eq!(initial.profiles.len(), 1);
        let initial = initial.profile.unwrap();
        assert_eq!(initial.profile.name, "Everyday");
        let id = initial.id;
        let mut profile = initial.profile;
        profile.name = "Work".into();
        profile.bindings[0].action = cooltek_sdk_service::Action::Volume { delta: 8 };
        let saved = store.save(Some(&id), profile.clone(), None).unwrap();
        assert_eq!(saved.profile, profile);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.load(&id).unwrap().profile, profile);
        assert_eq!(restarted.bootstrap().unwrap().profile.unwrap().id, id);
        assert_eq!(restarted.list().unwrap().profiles.len(), 1);
        assert_eq!(
            fs::read_dir(directory.path().join("profiles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn legacy_load_upgrades_only_in_memory_until_explicit_save() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        let bytes = include_bytes!("../../../../examples/profiles/all-modules.json");
        fs::write(&path, bytes).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let expected = Profile::from_json(bytes).unwrap().into_v2().unwrap();

        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.profile, expected);
        assert_eq!(loaded.profile.schema_version, 2);
        assert!(loaded.profile.keypad.is_some());
        assert_eq!(
            store.bootstrap().unwrap().profile.unwrap().profile,
            expected
        );
        assert_eq!(
            store.list().unwrap().profiles[0].binding_count,
            expected.binding_count()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

        let saved = store.save(Some(&id), loaded.profile, None).unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.profile, expected);
        let saved_bytes = fs::read(&path).unwrap();
        assert_ne!(saved_bytes, bytes);
        assert_eq!(Profile::from_json(&saved_bytes).unwrap().schema_version, 2);
        assert_eq!(
            fs::read_dir(directory.path().join("profiles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn schema_two_roundtrip_and_summary_include_all_nested_tile_bindings() {
        let (directory, store) = store();
        let profile = test_profiles::folders();
        let saved = store.save(None, profile.clone(), None).unwrap();
        assert_eq!(saved.profile, profile);
        assert_eq!(saved.profile.binding_count(), 7);
        assert_eq!(store.list().unwrap().profiles[0].binding_count, 7);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.load(&saved.id).unwrap().profile, profile);
        assert_eq!(restarted.bootstrap().unwrap().profile.unwrap().id, saved.id);
    }

    #[test]
    fn invalid_folder_profiles_cannot_overwrite_or_reseed_storage() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        let original = fs::read(&path).unwrap();
        let mut invalid = test_profiles::folders_json();
        invalid["keypad"]["pages"][0]["slots"][0]["page_id"] = "missing".into();
        let invalid_profile: Profile = serde_json::from_value(invalid.clone()).unwrap();
        assert!(store.save(Some(&saved.id), invalid_profile, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &invalid_bytes).unwrap();
        assert!(store.load(&saved.id).is_err());
        let recovered = store.bootstrap().unwrap();
        assert!(recovered.profile.is_none());
        assert!(recovered.profiles.is_empty());
        assert_eq!(recovered.unreadable.len(), 1);
        assert_eq!(recovered.unreadable[0].id, saved.id);
        assert_eq!(fs::read(&path).unwrap(), invalid_bytes);
        assert_eq!(
            fs::read_dir(directory.path().join("profiles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn invalid_ids_and_invalid_profiles_cannot_write() {
        let (_directory, store) = store();
        let initial = store.bootstrap().unwrap();
        for id in [
            "../escape",
            "",
            "/tmp/outside",
            "a.json",
            "00000000-0000-0000-0000-00000000000A",
        ] {
            assert!(store.load(id).is_err());
            assert!(
                store
                    .save(
                        Some(id),
                        initial.profile.as_ref().unwrap().profile.clone(),
                        None
                    )
                    .is_err()
            );
        }
        let mut invalid = initial.profile.unwrap().profile;
        invalid.bindings[0].control = 3;
        assert!(store.save(None, invalid, None).is_err());
        assert_eq!(store.list().unwrap().profiles.len(), 1);
    }

    #[test]
    fn corrupt_or_oversized_data_is_preserved_and_never_reseeded() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        fs::write(&path, b"corrupt").unwrap();
        let recovered = store.bootstrap().unwrap();
        assert!(recovered.profile.is_none());
        assert_eq!(recovered.unreadable[0].id, id);
        assert_eq!(fs::read(&path).unwrap(), b"corrupt");
        assert!(
            store
                .save(
                    Some(&id),
                    Profile::from_json(SAMPLE.as_bytes()).unwrap(),
                    None
                )
                .is_err()
        );
        fs::write(&path, vec![b' '; Profile::MAX_JSON_BYTES + 1]).unwrap();
        assert!(store.load(&id).unwrap_err().contains("size limit"));
        assert_eq!(
            fs::read_dir(directory.path().join("profiles"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn strict_unknown_fields_and_partial_writes_are_rejected() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        fs::write(
            &path,
            br#"{"schema_version":1,"name":"Desk","bindings":[],"extra":true}"#,
        )
        .unwrap();
        assert!(store.load(&id).is_err());
        fs::remove_file(path).unwrap();
        // Unfamiliar visible data still blocks seeding.
        let unfamiliar = directory.path().join("profiles/notes.txt");
        fs::write(&unfamiliar, b"keep me").unwrap();
        assert!(store.bootstrap().is_err());
        assert_eq!(fs::read(&unfamiliar).unwrap(), b"keep me");
    }

    #[test]
    fn hidden_entries_never_block_seeding_and_are_left_untouched() {
        let (directory, store) = store();
        let profiles = directory.path().join("profiles");
        for (name, bytes) in [
            (".DS_Store", &b"finder"[..]),
            ("._sidecar.json", &b"appledouble"[..]),
            (".interrupted.tmp", &b"partial"[..]),
        ] {
            fs::write(profiles.join(name), bytes).unwrap();
        }
        let initial = store.bootstrap().unwrap();
        assert_eq!(initial.profiles.len(), 1);
        assert!(initial.unreadable.is_empty());
        assert_eq!(fs::read(profiles.join(".DS_Store")).unwrap(), b"finder");
        assert_eq!(
            fs::read(profiles.join("._sidecar.json")).unwrap(),
            b"appledouble"
        );
        assert_eq!(
            fs::read(profiles.join(".interrupted.tmp")).unwrap(),
            b"partial"
        );
    }

    #[test]
    fn one_unreadable_profile_does_not_hide_others_or_block_saving() {
        let (directory, store) = store();
        let good = store.save(None, test_profiles::folders(), None).unwrap();
        let bad_id = Uuid::new_v4().to_string();
        let bad_path = directory
            .path()
            .join("profiles")
            .join(format!("{bad_id}.json"));
        fs::write(&bad_path, b"corrupt").unwrap();
        fs::write(directory.path().join("profiles/not-a-uuid.json"), b"{}").unwrap();

        let listing = store.list().unwrap();
        assert_eq!(listing.profiles.len(), 1);
        assert_eq!(listing.profiles[0].id, good.id);
        let ids: Vec<_> = listing
            .unreadable
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&bad_id.as_str()) && ids.contains(&"not-a-uuid"));
        assert!(listing.unreadable.iter().all(|item| !item.error.is_empty()));

        let bootstrap = store.bootstrap().unwrap();
        assert_eq!(bootstrap.profile.unwrap().id, good.id);
        assert_eq!(bootstrap.unreadable, listing.unreadable);

        let created = store.save(None, test_profiles::folders(), None).unwrap();
        let updated = store
            .save(
                Some(&good.id),
                test_profiles::folders(),
                Some(good.modified_at_ms),
            )
            .unwrap();
        assert_eq!(updated.id, good.id);
        assert_eq!(store.list().unwrap().profiles.len(), 2);
        assert!(store.load(&created.id).is_ok());
        assert_eq!(fs::read(&bad_path).unwrap(), b"corrupt");
    }

    #[test]
    fn stale_saves_and_deletes_are_rejected_as_conflicts() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        // Simulate another instance writing the file later.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                UNIX_EPOCH + std::time::Duration::from_millis(saved.modified_at_ms + 5_000),
            ))
            .unwrap();
        let before = fs::read(&path).unwrap();
        let mut edited = test_profiles::folders();
        edited.name = "Mine".into();
        let error = store
            .save(Some(&saved.id), edited.clone(), Some(saved.modified_at_ms))
            .unwrap_err();
        assert_eq!(error, CONFLICT_ERROR);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            store
                .delete(&saved.id, Some(saved.modified_at_ms))
                .unwrap_err(),
            CONFLICT_ERROR
        );
        assert!(path.exists());

        let current = store.load(&saved.id).unwrap();
        let resaved = store
            .save(Some(&saved.id), edited, Some(current.modified_at_ms))
            .unwrap();
        assert_eq!(resaved.profile.name, "Mine");
        store
            .delete(&saved.id, Some(resaved.modified_at_ms))
            .unwrap();
        assert!(!path.exists());
        assert!(
            store
                .load(&saved.id)
                .unwrap_err()
                .contains("no longer exists")
        );
    }

    #[test]
    fn deleting_frees_capacity_and_never_touches_invalid_ids_or_unreadable_files() {
        let (directory, store) = store();
        let mut last = None;
        for _ in 0..MAX_PROFILES {
            last = Some(store.save(None, test_profiles::folders(), None).unwrap());
        }
        let error = store
            .save(None, test_profiles::folders(), None)
            .unwrap_err();
        assert!(error.contains("delete a profile"));
        let last = last.unwrap();
        store.delete(&last.id, None).unwrap();
        assert!(store.save(None, test_profiles::folders(), None).is_ok());
        for id in ["../escape", "", "a.json"] {
            assert!(store.delete(id, None).is_err());
        }
        let bad_id = Uuid::new_v4().to_string();
        let bad_path = directory
            .path()
            .join("profiles")
            .join(format!("{bad_id}.json"));
        fs::write(&bad_path, b"corrupt").unwrap();
        assert!(store.delete(&bad_id, None).is_err());
        assert_eq!(fs::read(&bad_path).unwrap(), b"corrupt");
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_profile_paths_do_not_read_or_overwrite_external_data() {
        let (directory, store) = store();
        let outside = directory.path().join("outside.json");
        let profile = Profile::from_json(SAMPLE.as_bytes()).unwrap();
        fs::write(&outside, serde_json::to_vec(&profile).unwrap()).unwrap();
        let before = fs::read(&outside).unwrap();
        let id = Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(&outside, store.path(&id).unwrap()).unwrap();
        assert!(store.load(&id).is_err());
        assert!(store.save(Some(&id), profile, None).is_err());
        assert!(store.delete(&id, None).is_err());
        assert!(store.path(&id).unwrap().symlink_metadata().is_ok());
        assert_eq!(fs::read(outside).unwrap(), before);
    }
}
