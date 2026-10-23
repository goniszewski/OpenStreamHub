//! Startup profiles for the sessions the session manager starts on its own.
//!
//! With module control `App` the session manager starts a session on every
//! M4315 15-key module, M131E dial and M003K knob module when the app opens
//! and whenever one is connected (`session::Driver::inventory`). That run
//! uses, in order (`choose`): the global default profile the user picked in
//! Settings, while it still loads; else the saved profile last used on that
//! module's serial number, while it still loads; else `blank_profile`, which
//! binds nothing: the keys show one empty page and the dial its idle face
//! (`display::display_args`). The same default drives every module, since a
//! profile holds bindings per module.
//!
//! The default and the last profile per serial number live in
//! `<app data>/module-profiles.json`, beside `display-settings.json`, with the
//! same bounded, symlink-safe read and atomic 0600 write. It holds saved
//! profile ids only, never a profile body or an execute flag: whether a
//! session executes is the run mode (`display::DisplaySettings::run_mode`,
//! the header badge). Only modules the
//! session manager takes over with a nonblank serial number are remembered.
//! A missing file means no default and no last profiles. A damaged or
//! unreadable file fails closed: every automatic start uses the blank
//! profile and the file is never overwritten.

use crate::{display, store};
use cooltek_sdk_service::{Profile, ServiceModule};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE_NAME: &str = "module-profiles.json";
const MAX_FILE_BYTES: usize = 64 * 1024;
/// Most module serial numbers remembered; the least recently used is dropped.
pub const MAX_ENTRIES: usize = 64;
/// Name of the built-in profile an automatic start runs without a last one.
pub const BLANK_PROFILE_NAME: &str = "Blank (built-in)";

/// The built-in startup profile: no bindings, so nothing is planned or run,
/// and nothing is drawn but an empty key page and the dial's idle face.
pub fn blank_profile() -> Profile {
    Profile {
        schema_version: 1,
        name: BLANK_PROFILE_NAME.into(),
        bindings: Vec::new(),
        keypad: None,
        dial_functions: None,
        dial_display: None,
        screen_brightness: None,
        variables: Vec::new(),
    }
}

/// The profile an automatic start runs and the saved profile id it came
/// from: the global default profile while it still loads (`load` reads and
/// validates it), else the last saved profile used on the module serial
/// while it still loads, else the blank profile with no id. A record that
/// cannot be read counts as absent, so a damaged file gives the blank one.
pub fn choose(
    default: Result<Option<String>, String>,
    last: Result<Option<String>, String>,
    mut load: impl FnMut(&str) -> Result<Profile, String>,
) -> (Option<String>, Profile) {
    for id in [default, last]
        .into_iter()
        .filter_map(|record| record.ok().flatten())
    {
        if let Ok(profile) = load(&id) {
            return (Some(id), profile);
        }
    }
    (None, blank_profile())
}

/// The serial number a module's last profile is kept under, or `None` when it
/// is not kept: a module the session manager does not take over, or no
/// serial number.
fn remembered_serial(module: ServiceModule, serial: Option<&str>) -> Option<&str> {
    serial
        .map(str::trim)
        .filter(|serial| display::auto_takes_over(module) && !serial.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LastProfile {
    module: ServiceModule,
    serial_number: String,
    profile_id: String,
}

/// The file: the global default, then the last profiles, most recently used
/// first.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredProfiles {
    /// The saved profile every automatic start uses first; absent for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_profile_id: Option<String>,
    last_profiles: Vec<LastProfile>,
}

impl StoredProfiles {
    fn validate(&self) -> Result<(), String> {
        if let Some(id) = &self.default_profile_id {
            store::validate_id(id)?;
        }
        if self.last_profiles.len() > MAX_ENTRIES {
            return Err(format!("more than {MAX_ENTRIES} entries"));
        }
        let mut keys = HashSet::new();
        for entry in &self.last_profiles {
            if remembered_serial(entry.module, Some(&entry.serial_number))
                != Some(entry.serial_number.as_str())
                || entry.serial_number.len() > 256
                || entry.serial_number.chars().any(char::is_control)
            {
                return Err("an entry names another module or an invalid serial number".into());
            }
            store::validate_id(&entry.profile_id)?;
            if !keys.insert((entry.module, entry.serial_number.as_str())) {
                return Err("a module serial number is listed twice".into());
            }
        }
        Ok(())
    }

    fn find(&self, module: ServiceModule, serial: &str) -> Option<&LastProfile> {
        self.last_profiles
            .iter()
            .find(|entry| entry.module == module && entry.serial_number == serial)
    }
}

/// `module-profiles.json`; the lock serializes read-modify-write.
pub struct LastProfiles {
    directory: Result<PathBuf, String>,
    lock: Mutex<()>,
}

impl LastProfiles {
    pub fn open(app_data: Result<&Path, String>) -> Self {
        Self {
            directory: app_data.map(Path::to_path_buf),
            lock: Mutex::new(()),
        }
    }

    fn load(directory: &Path) -> Result<StoredProfiles, String> {
        let path = directory.join(FILE_NAME);
        let Some((bytes, _)) =
            store::read_private_file(&path, MAX_FILE_BYTES, "last module profiles file")?
        else {
            return Ok(StoredProfiles::default());
        };
        let stored: StoredProfiles = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "the last module profiles in {} are unreadable ({error}); fix or delete the file",
                path.display()
            )
        })?;
        stored.validate().map_err(|error| {
            format!(
                "the last module profiles in {} are invalid ({error}); fix or delete the file",
                path.display()
            )
        })?;
        Ok(stored)
    }

    /// The saved profile last used on `module` with `serial`, if any. `Err`
    /// when the file cannot be read (the caller then uses the blank profile).
    pub fn get(
        &self,
        module: ServiceModule,
        serial: Option<&str>,
    ) -> Result<Option<String>, String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let Some(serial) = remembered_serial(module, serial) else {
            return Ok(None);
        };
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "last module profiles lock is unavailable".to_string())?;
        Ok(Self::load(directory)?
            .find(module, serial)
            .map(|entry| entry.profile_id.clone()))
    }

    /// Records that a session on `module` with `serial` now runs the saved
    /// profile `profile_id`. Modules that are not kept are ignored. A damaged
    /// file is reported and never overwritten.
    pub fn remember(
        &self,
        module: ServiceModule,
        serial: Option<&str>,
        profile_id: &str,
    ) -> Result<(), String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let Some(serial) = remembered_serial(module, serial) else {
            return Ok(());
        };
        store::validate_id(profile_id)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "last module profiles lock is unavailable".to_string())?;
        let mut stored = Self::load(directory)?;
        let entry = LastProfile {
            module,
            serial_number: serial.to_owned(),
            profile_id: profile_id.to_owned(),
        };
        if stored.last_profiles.first() == Some(&entry) {
            return Ok(());
        }
        stored
            .last_profiles
            .retain(|other| !(other.module == module && other.serial_number == serial));
        stored.last_profiles.insert(0, entry);
        stored.last_profiles.truncate(MAX_ENTRIES);
        Self::save(directory, &stored)
    }

    /// The global default profile id, if one is set. `Err` when the file
    /// cannot be read (the caller then has no default).
    pub fn default_profile(&self) -> Result<Option<String>, String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "last module profiles lock is unavailable".to_string())?;
        Ok(Self::load(directory)?.default_profile_id)
    }

    /// Sets (or with `None` clears) the global default profile id. The caller
    /// checks that it names a saved profile. A damaged file is reported and
    /// never overwritten.
    pub fn set_default_profile(&self, profile_id: Option<&str>) -> Result<(), String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        if let Some(id) = profile_id {
            store::validate_id(id)?;
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "last module profiles lock is unavailable".to_string())?;
        let mut stored = Self::load(directory)?;
        if stored.default_profile_id.as_deref() == profile_id {
            return Ok(());
        }
        stored.default_profile_id = profile_id.map(str::to_owned);
        Self::save(directory, &stored)
    }

    fn save(directory: &Path, stored: &StoredProfiles) -> Result<(), String> {
        match std::fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(
                    "the application data folder must be a directory, not a symbolic link".into(),
                );
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect the application data folder: {error}"
                ));
            }
        }
        let bytes = serde_json::to_vec_pretty(stored)
            .map_err(|error| format!("could not encode the last module profiles: {error}"))?;
        store::write_private_file(
            directory,
            &directory.join(FILE_NAME),
            &bytes,
            "last module profiles file",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: ServiceModule = ServiceModule::M4315;
    const DIAL: ServiceModule = ServiceModule::M131E;
    const FIRST: &str = "00000000-0000-4000-8000-000000000001";
    const SECOND: &str = "00000000-0000-4000-8000-000000000002";

    #[test]
    fn the_last_profile_is_kept_per_module_serial_and_survives_a_restart() {
        let directory = tempfile::tempdir().unwrap();
        let last = LastProfiles::open(Ok(directory.path()));
        // A missing file: nothing remembered and nothing written.
        assert_eq!(last.get(KEYS, Some("K-1")), Ok(None));
        assert!(!directory.path().join(FILE_NAME).exists());
        last.remember(KEYS, Some("K-1"), FIRST).unwrap();
        last.remember(KEYS, Some("K-2"), SECOND).unwrap();
        last.remember(DIAL, Some("K-1"), SECOND).unwrap();
        let reopened = LastProfiles::open(Ok(directory.path()));
        assert_eq!(reopened.get(KEYS, Some("K-1")), Ok(Some(FIRST.into())));
        assert_eq!(reopened.get(KEYS, Some(" K-2 ")), Ok(Some(SECOND.into())));
        assert_eq!(reopened.get(DIAL, Some("K-1")), Ok(Some(SECOND.into())));
        assert_eq!(reopened.get(DIAL, Some("K-2")), Ok(None));
        // The newest use replaces the older one.
        reopened.remember(KEYS, Some("K-1"), SECOND).unwrap();
        assert_eq!(last.get(KEYS, Some("K-1")), Ok(Some(SECOND.into())));
        // Serial-less modules are never kept; faders and rollers are, like
        // every module the app starts on its own.
        last.remember(KEYS, None, FIRST).unwrap();
        last.remember(KEYS, Some("  "), FIRST).unwrap();
        last.remember(ServiceModule::M005S, Some("F-1"), FIRST)
            .unwrap();
        assert_eq!(last.get(KEYS, None), Ok(None));
        assert_eq!(
            last.get(ServiceModule::M005S, Some("F-1")),
            Ok(Some(FIRST.into()))
        );
        let stored = LastProfiles::load(directory.path()).unwrap();
        assert_eq!(stored.last_profiles.len(), 4);
        assert!(last.remember(KEYS, Some("K-1"), "not-a-uuid").is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = directory.path().join(FILE_NAME);
            let mode = std::fs::metadata(path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn the_default_profile_is_stored_beside_the_last_profiles() {
        let directory = tempfile::tempdir().unwrap();
        let last = LastProfiles::open(Ok(directory.path()));
        assert_eq!(last.default_profile(), Ok(None));
        last.remember(ServiceModule::M003K, Some("N-1"), SECOND)
            .unwrap();
        last.set_default_profile(Some(FIRST)).unwrap();
        let reopened = LastProfiles::open(Ok(directory.path()));
        assert_eq!(reopened.default_profile(), Ok(Some(FIRST.into())));
        // Setting it keeps the last profiles, which now include the knobs.
        assert_eq!(
            reopened.get(ServiceModule::M003K, Some("N-1")),
            Ok(Some(SECOND.into()))
        );
        reopened.set_default_profile(None).unwrap();
        assert_eq!(last.default_profile(), Ok(None));
        assert!(last.set_default_profile(Some("not-a-uuid")).is_err());
        // A file with a bad default id is damaged as a whole.
        std::fs::write(
            directory.path().join(FILE_NAME),
            br#"{"default_profile_id":"x","last_profiles":[]}"#,
        )
        .unwrap();
        assert!(last.default_profile().is_err());
        assert!(last.get(KEYS, Some("K")).is_err());
        // Faders and rollers are kept like any module the app starts itself.
        let directory = tempfile::tempdir().unwrap();
        let last = LastProfiles::open(Ok(directory.path()));
        for module in [ServiceModule::M005S, ServiceModule::M032LE] {
            last.remember(module, Some("X-1"), FIRST).unwrap();
            assert_eq!(last.get(module, Some("X-1")), Ok(Some(FIRST.into())));
        }
    }

    #[test]
    fn the_list_is_bounded_by_dropping_the_least_recently_used() {
        let directory = tempfile::tempdir().unwrap();
        let last = LastProfiles::open(Ok(directory.path()));
        for index in 0..=MAX_ENTRIES {
            last.remember(KEYS, Some(&format!("K-{index}")), FIRST)
                .unwrap();
        }
        assert_eq!(last.get(KEYS, Some("K-0")), Ok(None));
        assert_eq!(
            last.get(KEYS, Some(&format!("K-{MAX_ENTRIES}"))),
            Ok(Some(FIRST.into()))
        );
        assert_eq!(
            LastProfiles::load(directory.path())
                .unwrap()
                .last_profiles
                .len(),
            MAX_ENTRIES
        );
    }

    #[test]
    fn a_damaged_file_fails_closed_and_is_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let last = LastProfiles::open(Ok(directory.path()));
        let duplicate = format!(
            r#"{{"last_profiles":[{{"module":"m4315","serial_number":"K","profile_id":"{FIRST}"}},{{"module":"m4315","serial_number":"K","profile_id":"{SECOND}"}}]}}"#
        );
        for bytes in [
            b"not json".to_vec(),
            br#"{"last_profiles":[],"extra":1}"#.to_vec(),
            br#"{"last_profiles":[{"module":"m4315","serial_number":"K","profile_id":"x"}]}"#
                .to_vec(),
            format!(r#"{{"last_profiles":[{{"module":"m4315","serial_number":"K","profile_id":"{FIRST}"}},{{"module":"m4315","serial_number":"K","profile_id":"{SECOND}"}}]}}"#)
                .into_bytes(),
            duplicate.into_bytes(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(last.get(KEYS, Some("K")).is_err());
            assert!(last.remember(KEYS, Some("K"), FIRST).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
            // The startup choice falls back to the blank profile.
            let (id, profile) = choose(
                last.default_profile(),
                last.get(KEYS, Some("K")),
                |_| unreachable!(),
            );
            assert!(last.set_default_profile(Some(FIRST)).is_err());
            assert_eq!((id, profile), (None, blank_profile()));
        }
        assert!(
            LastProfiles::open(Err("no app data".into()))
                .get(KEYS, Some("K"))
                .is_err()
        );
    }

    #[test]
    fn the_startup_choice_is_the_default_then_the_last_used_then_blank() {
        let folders = crate::test_profiles::folders();
        let mut renamed = folders.clone();
        renamed.name = "Last".into();
        let load = |id: &str| match id {
            FIRST => Ok(folders.clone()),
            SECOND => Ok(renamed.clone()),
            _ => Err("no longer saved".to_string()),
        };
        // The default wins over the last used profile.
        assert_eq!(
            choose(Ok(Some(FIRST.into())), Ok(Some(SECOND.into())), load),
            (Some(FIRST.into()), folders.clone())
        );
        // No default, or one that no longer loads: the last used profile.
        let gone = "00000000-0000-4000-8000-0000000000ff";
        for default in [Ok(None), Ok(Some(gone.into())), Err("damaged".into())] {
            assert_eq!(
                choose(default, Ok(Some(SECOND.into())), load),
                (Some(SECOND.into()), renamed.clone())
            );
        }
        // Neither loads: blank, without an id.
        assert_eq!(
            choose(Ok(Some(gone.into())), Ok(Some(gone.into())), load),
            (None, blank_profile())
        );
        assert_eq!(
            choose(Ok(None), Ok(None), |_| unreachable!()),
            (None, blank_profile())
        );
        assert_eq!(
            choose(
                Err("damaged".into()),
                Err("damaged".into()),
                |_| unreachable!()
            ),
            (None, blank_profile())
        );
        // The blank profile is valid, binds nothing and runs no action.
        let blank = blank_profile();
        crate::replay::validate_profile(&blank).unwrap();
        assert_eq!(blank.binding_count(), 0);
    }
}
