use cooltek_sdk_service::Profile;
use serde::Serialize;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    time::UNIX_EPOCH,
};
use uuid::Uuid;

pub(crate) const MAX_PROFILES: usize = 128;
const SAMPLE: &str = include_str!("../../../../examples/profiles/macos.json");

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

#[derive(Debug, Serialize)]
pub struct AppBootstrap {
    pub profile: StoredProfile,
    pub profiles: Vec<ProfileSummary>,
    pub storage_path: String,
    pub desktop: bool,
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
        let mut profiles = self.list()?;
        if profiles.is_empty() {
            // An interrupted write or unfamiliar file is retained for recovery.
            // Seed only an empty store, never replace an unreadable profile.
            if fs::read_dir(&self.directory)
                .map_err(|error| format!("could not inspect profile storage: {error}"))?
                .next()
                .is_some()
            {
                return Err("profile storage contains data but no usable profiles; preserve and inspect the files before recovery".into());
            }
            let mut profile = Profile::from_json(SAMPLE.as_bytes())?;
            profile.name = "Everyday".into();
            self.save(None, profile)?;
            profiles = self.list()?;
        }
        let selected = profiles
            .first()
            .ok_or_else(|| "profile storage is empty".to_string())?;
        Ok(AppBootstrap {
            profile: self.load(&selected.id)?,
            profiles,
            storage_path: self.directory.to_string_lossy().into_owned(),
            desktop: true,
        })
    }

    pub fn list(&self) -> Result<Vec<ProfileSummary>, String> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("could not list profiles: {error}"))?;
        let mut profiles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("could not read profile entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            if profiles.len() == MAX_PROFILES {
                return Err(format!("profile storage exceeds {MAX_PROFILES} profiles"));
            }
            let id = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| "profile filename is not valid UTF-8".to_string())?;
            let stored = self
                .load(id)
                .map_err(|error| format!("could not load profile {id}: {error}"))?;
            profiles.push(ProfileSummary {
                id: stored.id,
                name: stored.profile.name,
                binding_count: stored.profile.bindings.len(),
                modified_at_ms: stored.modified_at_ms,
            });
        }
        profiles.sort_by(|left, right| {
            right
                .modified_at_ms
                .cmp(&left.modified_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(profiles)
    }

    pub fn load(&self, id: &str) -> Result<StoredProfile, String> {
        let path = self.path(id)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("could not inspect profile: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err("profile must be a regular file, not a symbolic link".into());
        }
        if metadata.len() > Profile::MAX_JSON_BYTES as u64 {
            return Err("stored profile exceeds the size limit".into());
        }
        let mut bytes = Vec::new();
        File::open(&path)
            .map_err(|error| format!("could not open profile: {error}"))?
            .take((Profile::MAX_JSON_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("could not read profile: {error}"))?;
        let profile = Profile::from_json(&bytes)?;
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

    pub fn save(&self, id: Option<&str>, profile: Profile) -> Result<StoredProfile, String> {
        profile.validate()?;
        let id = match id {
            Some(id) => {
                // Never overwrite an unreadable existing profile during editing.
                self.load(id)?;
                id.to_owned()
            }
            None => {
                if self.list()?.len() >= MAX_PROFILES {
                    return Err(format!(
                        "profile storage permits at most {MAX_PROFILES} profiles"
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
        self.load(&id)
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
        assert_eq!(initial.profile.profile.name, "Everyday");
        let id = initial.profile.id;
        let mut profile = initial.profile.profile;
        profile.name = "Work".into();
        profile.bindings[0].action = cooltek_sdk_service::Action::Volume { delta: 8 };
        let saved = store.save(Some(&id), profile.clone()).unwrap();
        assert_eq!(saved.profile, profile);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.load(&id).unwrap().profile, profile);
        assert_eq!(restarted.bootstrap().unwrap().profile.id, id);
        assert_eq!(restarted.list().unwrap().len(), 1);
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
                    .save(Some(id), initial.profile.profile.clone())
                    .is_err()
            );
        }
        let mut invalid = initial.profile.profile;
        invalid.bindings[0].control = 3;
        assert!(store.save(None, invalid).is_err());
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn corrupt_or_oversized_data_is_preserved_and_never_reseeded() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        fs::write(&path, b"corrupt").unwrap();
        assert!(store.bootstrap().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"corrupt");
        assert!(
            store
                .save(Some(&id), Profile::from_json(SAMPLE.as_bytes()).unwrap())
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
        fs::write(
            directory.path().join("profiles/.interrupted.tmp"),
            b"partial",
        )
        .unwrap();
        assert!(store.bootstrap().is_err());
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
        assert!(store.save(Some(&id), profile).is_err());
        assert_eq!(fs::read(outside).unwrap(), before);
    }
}
