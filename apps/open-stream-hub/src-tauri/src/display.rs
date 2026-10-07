//! Experimental module display settings.
//!
//! One opt-in switch, "send images to module displays", and the M131E dial
//! content. They live in `<app data>/display-settings.json`, beside the
//! profiles and never inside one, with the same bounded, symlink-safe read and
//! atomic 0600 write as the profile store and `focus-rules.json`. A missing
//! file means off.
//!
//! The settings are read once when the user starts a managed session
//! (`session::Driver::start`) and kept with that session, so wake, reconnect,
//! crash and profile-switch restarts use the value the session started with.
//! Changing the setting never stops or restarts anything; it applies at the
//! next Start. `display_args` turns the session's copy into
//! `masterhub-service` arguments for each spawn; everything it passes is
//! experimental and has never run on hardware. See
//! `docs/masterhub-display-sync.md` and `docs/masterhub-session-manager.md`.

use crate::{service::StartRequest, store};
use cooltek_sdk_service::{Profile, ServiceModule};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILE_BYTES: usize = 4 * 1024;
const FILE_NAME: &str = "display-settings.json";

/// What the M131E dial screen shows while display images are on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialContent {
    /// Nothing is drawn on the dial.
    Off,
    /// The active dial function's name (the profile name without functions).
    #[default]
    Function,
    /// A digital clock.
    Clock,
    /// An analog clock.
    AnalogClock,
}

impl DialContent {
    /// The `masterhub-service --dial-content` value.
    pub fn arg(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Function => "function",
            Self::Clock => "clock",
            Self::AnalogClock => "analog-clock",
        }
    }
}

/// The stored settings. Off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplaySettings {
    /// "Experimental: send images to module displays".
    pub enabled: bool,
    pub dial_content: DialContent,
}

/// The settings a managed session started with, and its module. Set by the
/// session driver only; the frontend cannot send it (`StartRequest` skips it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayLaunch {
    pub settings: DisplaySettings,
    pub module: ServiceModule,
    /// Whether the run the user started (its profile and software mode) got
    /// display flags, so the Run panel showed its hardware-unverified warning.
    /// When it did not, no later process of the session gets them either: a
    /// focused-app rule or profile key switching to a keypad-folder profile
    /// then fails closed, exactly as with the setting off, even when the
    /// setting is on by then. The service's refusal says to start a session
    /// with that profile, software mode and the setting on, because a session
    /// started without display images cannot switch into one.
    pub active_at_start: bool,
}

impl DisplayLaunch {
    /// The launch for a Start of `profile` on `module`, recording whether that
    /// first process gets display flags.
    pub fn at_start(
        settings: DisplaySettings,
        module: ServiceModule,
        software_mode: bool,
        profile: &Profile,
    ) -> Self {
        Self {
            settings,
            module,
            active_at_start: !flags(settings, module, software_mode, profile).is_empty(),
        }
    }
}

/// Extra `masterhub-service run` arguments for one spawn of a managed session.
///
/// Nothing unless display images are on and the run requests software mode,
/// which `--display-sync` requires. An M4315 gets `--display-sync` only for a
/// schema-2 keypad profile; an M131E gets `--display-sync --dial-content`
/// unless the dial content is off. Every other case, including a flat profile
/// on an M4315 after a profile switch, runs with exactly the arguments it had
/// without the setting. A session whose first process got no flags never gets
/// them later (`DisplayLaunch::active_at_start`).
pub fn display_args(
    launch: Option<&DisplayLaunch>,
    software_mode: bool,
    profile: &Profile,
) -> Vec<OsString> {
    match launch {
        Some(launch) if launch.active_at_start => {
            flags(launch.settings, launch.module, software_mode, profile)
        }
        _ => Vec::new(),
    }
}

/// The display arguments for one spawn: only a session-managed process gets
/// any, never an unmanaged run (`lib.rs` `run_service`).
pub fn spawn_args(managed: bool, request: &StartRequest) -> Vec<OsString> {
    if !managed {
        return Vec::new();
    }
    display_args(
        request.display.as_ref(),
        request.software_mode,
        &request.profile,
    )
}

fn flags(
    settings: DisplaySettings,
    module: ServiceModule,
    software_mode: bool,
    profile: &Profile,
) -> Vec<OsString> {
    if !settings.enabled || !software_mode {
        return Vec::new();
    }
    match module {
        ServiceModule::M4315 if profile.schema_version == 2 && profile.keypad.is_some() => {
            vec!["--display-sync".into()]
        }
        ServiceModule::M131E if settings.dial_content != DialContent::Off => vec![
            "--display-sync".into(),
            "--dial-content".into(),
            settings.dial_content.arg().into(),
        ],
        _ => Vec::new(),
    }
}

/// `display-settings.json` inside the app data folder.
pub struct SettingsFile {
    directory: PathBuf,
}

impl SettingsFile {
    pub fn new(app_data: &Path) -> Self {
        Self {
            directory: app_data.to_path_buf(),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join(FILE_NAME)
    }

    /// A missing file means off. A damaged file is an error and is never
    /// overwritten, like an unreadable profile.
    pub fn load(&self) -> Result<DisplaySettings, String> {
        let Some((bytes, _)) =
            store::read_private_file(&self.path(), MAX_FILE_BYTES, "display settings file")?
        else {
            return Ok(DisplaySettings::default());
        };
        serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "display settings in {} are unreadable ({error}); fix or remove the file",
                self.path().display()
            )
        })
    }

    pub fn save(&self, settings: &DisplaySettings) -> Result<(), String> {
        match std::fs::symlink_metadata(&self.directory) {
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
        let bytes = serde_json::to_vec_pretty(settings)
            .map_err(|error| format!("could not encode display settings: {error}"))?;
        store::write_private_file(
            &self.directory,
            &self.path(),
            &bytes,
            "display settings file",
        )
    }
}

/// Settings storage; the lock serializes read-modify-write.
pub struct DisplayState {
    file: Result<SettingsFile, String>,
    lock: Mutex<()>,
}

impl DisplayState {
    pub fn open(app_data: Result<&Path, String>) -> Self {
        Self {
            file: app_data.map(SettingsFile::new),
            lock: Mutex::new(()),
        }
    }

    pub fn get(&self) -> Result<DisplaySettings, String> {
        let file = self.file.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "display settings lock is unavailable".to_string())?;
        file.load()
    }

    /// Saves `settings` unless they are already stored. A damaged file is
    /// reported and left alone.
    pub fn set(&self, settings: DisplaySettings) -> Result<DisplaySettings, String> {
        let file = self.file.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "display settings lock is unavailable".to_string())?;
        if file.load()? != settings {
            file.save(&settings)?;
        }
        Ok(settings)
    }

    /// What a new session starts with: the stored settings, or off when they
    /// cannot be read, so a damaged file never turns display writes on.
    pub fn for_new_session(&self) -> DisplaySettings {
        self.get().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;

    const ON: DisplaySettings = DisplaySettings {
        enabled: true,
        dial_content: DialContent::Clock,
    };

    fn launch(module: ServiceModule, settings: DisplaySettings) -> DisplayLaunch {
        DisplayLaunch {
            settings,
            module,
            active_at_start: true,
        }
    }

    fn flat() -> Profile {
        let mut profile = test_profiles::folders();
        profile.schema_version = 1;
        profile.keypad = None;
        profile
    }

    #[test]
    fn settings_default_to_off_and_parse_strictly() {
        assert_eq!(
            DisplaySettings::default(),
            DisplaySettings {
                enabled: false,
                dial_content: DialContent::Function
            }
        );
        let parsed: DisplaySettings =
            serde_json::from_str(r#"{"enabled":true,"dial_content":"analog_clock"}"#).unwrap();
        assert_eq!(parsed.dial_content, DialContent::AnalogClock);
        for bad in [
            r#"{"enabled":true}"#,
            r#"{"enabled":true,"dial_content":"Clock"}"#,
            r#"{"enabled":true,"dial_content":"analog-clock"}"#,
            r#"{"enabled":"yes","dial_content":"clock"}"#,
            r#"{"enabled":true,"dial_content":"clock","extra":1}"#,
        ] {
            assert!(
                serde_json::from_str::<DisplaySettings>(bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn dial_content_arguments_match_the_service() {
        let ours: Vec<&str> = [
            DialContent::Off,
            DialContent::Function,
            DialContent::Clock,
            DialContent::AnalogClock,
        ]
        .into_iter()
        .map(DialContent::arg)
        .collect();
        let service: Vec<&str> = cooltek_sdk_cli::dial_artwork::DialContent::ALL
            .into_iter()
            .map(|content| content.as_str())
            .collect();
        assert_eq!(ours, service);
    }

    #[test]
    fn arguments_are_added_only_when_on_with_software_mode_for_a_display_module() {
        let folders = test_profiles::folders();
        let none: Vec<OsString> = Vec::new();
        // Off, missing, or without software mode: nothing changes.
        for module in [ServiceModule::M4315, ServiceModule::M131E] {
            let off = launch(module, DisplaySettings::default());
            assert_eq!(display_args(Some(&off), true, &folders), none);
            assert_eq!(display_args(None, true, &folders), none);
            assert_eq!(
                display_args(Some(&launch(module, ON)), false, &folders),
                none
            );
        }
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, ON)), true, &folders),
            ["--display-sync"]
        );
        // A flat profile on the keypad (for example after a profile switch).
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, ON)), true, &flat()),
            none
        );
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M131E, ON)), true, &flat()),
            ["--display-sync", "--dial-content", "clock"]
        );
        let dial_off = DisplaySettings {
            dial_content: DialContent::Off,
            ..ON
        };
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M131E, dial_off)), true, &flat()),
            none
        );
        for module in [
            ServiceModule::M003K,
            ServiceModule::M005S,
            ServiceModule::M032LE,
        ] {
            assert_eq!(
                display_args(Some(&launch(module, ON)), true, &folders),
                none
            );
        }
    }

    #[test]
    fn a_session_started_without_display_flags_never_gets_them_later() {
        let folders = test_profiles::folders();
        // Started on the keypad with a flat profile: no flags, no warning shown.
        let flat_start = DisplayLaunch::at_start(ON, ServiceModule::M4315, true, &flat());
        assert!(!flat_start.active_at_start);
        // A later switch to a keypad-folder profile fails closed like "off".
        assert!(display_args(Some(&flat_start), true, &folders).is_empty());
        // Started without software mode: the same.
        let no_mode = DisplayLaunch::at_start(ON, ServiceModule::M4315, false, &folders);
        assert!(!no_mode.active_at_start);
        assert!(display_args(Some(&no_mode), true, &folders).is_empty());
        // Started with folders: flags follow the profile of each spawn.
        let nested_start = DisplayLaunch::at_start(ON, ServiceModule::M4315, true, &folders);
        assert!(nested_start.active_at_start);
        assert_eq!(
            display_args(Some(&nested_start), true, &folders),
            ["--display-sync"]
        );
        assert!(display_args(Some(&nested_start), true, &flat()).is_empty());
        // The dial is active for any profile, so switches keep it.
        let dial = DisplayLaunch::at_start(ON, ServiceModule::M131E, true, &flat());
        assert!(dial.active_at_start);
        assert_eq!(display_args(Some(&dial), true, &folders).len(), 3);
        let off = DisplayLaunch::at_start(
            DisplaySettings::default(),
            ServiceModule::M131E,
            true,
            &flat(),
        );
        assert!(!off.active_at_start);
    }

    #[test]
    fn only_session_managed_spawns_get_display_arguments() {
        let request = StartRequest {
            profile: test_profiles::folders(),
            path_hex: "41".into(),
            execute: false,
            software_mode: true,
            profile_id: None,
            source_profile_id: None,
            display: Some(launch(ServiceModule::M4315, ON)),
            obs: None,
        };
        assert_eq!(spawn_args(true, &request), ["--display-sync"]);
        assert!(spawn_args(false, &request).is_empty());
        let unidentified = StartRequest {
            display: None,
            ..request
        };
        assert!(spawn_args(true, &unidentified).is_empty());
    }

    #[test]
    fn the_settings_file_round_trips_atomically_and_privately() {
        let directory = tempfile::tempdir().unwrap();
        let state = DisplayState::open(Ok(directory.path()));
        // Reading a missing file writes nothing.
        assert_eq!(state.get().unwrap(), DisplaySettings::default());
        assert!(!directory.path().join(FILE_NAME).exists());
        assert_eq!(state.set(ON).unwrap(), ON);
        assert_eq!(state.get().unwrap(), ON);
        assert_eq!(DisplayState::open(Ok(directory.path())).get().unwrap(), ON);
        let path = directory.path().join(FILE_NAME);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(FILE_NAME)]);
    }

    #[test]
    fn a_damaged_file_is_preserved_and_new_sessions_start_with_display_off() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let state = DisplayState::open(Ok(directory.path()));
        for bytes in [
            b"not json".to_vec(),
            br#"{"enabled":true,"dial_content":"clock","extra":1}"#.to_vec(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(state.get().is_err());
            assert!(state.set(ON).is_err());
            assert_eq!(state.for_new_session(), DisplaySettings::default());
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
        }
        assert!(DisplayState::open(Err("no app data".into())).get().is_err());
        assert_eq!(
            DisplayState::open(Err("no app data".into())).for_new_session(),
            DisplaySettings::default()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_settings_file_is_never_read_or_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(&outside, br#"{"enabled":true,"dial_content":"clock"}"#).unwrap();
        let app = directory.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::os::unix::fs::symlink(&outside, app.join(FILE_NAME)).unwrap();
        let state = DisplayState::open(Ok(&app));
        assert!(state.get().unwrap_err().contains("symbolic link"));
        assert!(state.set(DisplaySettings::default()).is_err());
        assert_eq!(state.for_new_session(), DisplaySettings::default());
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            br#"{"enabled":true,"dial_content":"clock"}"#
        );
    }
}
