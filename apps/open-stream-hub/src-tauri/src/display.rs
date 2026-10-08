//! Module control and module display settings.
//!
//! "Module control" says who drives the M4315 15-key module and the M131E
//! dial while a profile runs: `App` (the default) puts them in software mode
//! and keeps their displays in sync (`--software-mode --display-sync`, plus
//! `--dial-content` for the dial); `BuiltInAnimation` leaves them in the
//! module's own mode, exactly as before this setting existed. The M131E dial
//! content is kept beside it. Other modules never get software mode or display
//! flags. The settings live in `<app data>/display-settings.json`, beside the
//! profiles and never inside one, with the same bounded, symlink-safe read and
//! atomic 0600 write as the profile store and `focus-rules.json`. A missing
//! file means `App`.
//!
//! The file of the earlier experimental toggle (`{"enabled":…,
//! "dial_content":…}`) is still read: `enabled: true` is `App`, an explicit
//! `enabled: false` is `BuiltInAnimation`. The next save writes the current
//! shape. A damaged or unreadable file is never overwritten, and a session
//! started meanwhile uses `ModuleControl::Unreadable`: no software mode for
//! any module, nothing drawn on a display, and a schema-2 keypad profile is
//! refused with a message that names the damaged file.
//!
//! The settings are read once when the user starts a managed session
//! (`session::Driver::start`) and kept with that session, so wake, reconnect,
//! crash and profile-switch restarts use the value the session started with.
//! Changing the setting never stops or restarts anything; it applies at the
//! next Start. `display_args` turns the session's copy into
//! `masterhub-service` arguments for each spawn. Hardware results are in
//! `docs/masterhub-hardware-results.md`; see also
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
    /// The machine's CPU usage in percent (macOS only; `--` elsewhere).
    Cpu,
}

impl DialContent {
    /// Every value, in the service's `--dial-content` order.
    #[cfg(test)]
    pub const ALL: [Self; 5] = [
        Self::Off,
        Self::Function,
        Self::Clock,
        Self::AnalogClock,
        Self::Cpu,
    ];

    /// The `masterhub-service --dial-content` value.
    pub fn arg(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Function => "function",
            Self::Clock => "clock",
            Self::AnalogClock => "analog-clock",
            Self::Cpu => "cpu",
        }
    }
}

/// Who drives the 15-key module and the dial while a profile runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleControl {
    /// Software mode and display sync on the M4315 and M131E.
    #[default]
    App,
    /// The module's own mode; no software mode is requested for it unless the
    /// user asks in the Run panel, and nothing is drawn on its display.
    BuiltInAnimation,
    /// The settings file could not be read. Never stored or sent to the
    /// frontend (`get` reports the error instead); a session started with it
    /// never enters software mode and draws nothing.
    #[serde(skip)]
    Unreadable,
}

/// The stored settings. `App` and the dial function name by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredSettings")]
pub struct DisplaySettings {
    pub module_control: ModuleControl,
    pub dial_content: DialContent,
}

impl DisplaySettings {
    /// What a session uses when the stored settings cannot be read: no
    /// software mode and no display writes, so a damaged file never starts
    /// either.
    pub const UNREADABLE: Self = Self {
        module_control: ModuleControl::Unreadable,
        dial_content: DialContent::Function,
    };
}

/// The current file shape, or the earlier experimental toggle's.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredSettings {
    Current(CurrentSettings),
    Legacy(LegacySettings),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentSettings {
    module_control: ModuleControl,
    dial_content: DialContent,
}

/// `{"enabled":…,"dial_content":…}`, written by the experimental toggle.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySettings {
    enabled: bool,
    dial_content: DialContent,
}

impl From<StoredSettings> for DisplaySettings {
    fn from(stored: StoredSettings) -> Self {
        match stored {
            StoredSettings::Current(CurrentSettings {
                module_control,
                dial_content,
            }) => Self {
                module_control,
                dial_content,
            },
            StoredSettings::Legacy(LegacySettings {
                enabled,
                dial_content,
            }) => Self {
                module_control: if enabled {
                    ModuleControl::App
                } else {
                    ModuleControl::BuiltInAnimation
                },
                dial_content,
            },
        }
    }
}

/// The modules `ModuleControl::App` drives in software mode.
pub fn app_controls(module: ServiceModule) -> bool {
    matches!(module, ServiceModule::M4315 | ServiceModule::M131E)
}

/// Why a schema-2 keypad profile cannot run on an M4315 with module control
/// set to built-in animation. The service refuses it too; this names the
/// setting instead of the service's `--display-sync` flag.
pub const BUILT_IN_KEYPAD_REFUSAL: &str = "This profile's 15-key pages run live only when OpenStreamHub controls the module. Module control in Settings is set to Built-in animation: set it to App, then start the profile again.";

/// The same refusal while the settings file cannot be read.
pub const UNREADABLE_KEYPAD_REFUSAL: &str = "This profile's 15-key pages run live only when OpenStreamHub controls the module, but the Module control settings file (display-settings.json) cannot be read. Settings shows the error: fix or delete the file, then start the profile again.";

/// Why `profile` cannot run on `module` under `settings`, or `None`. The
/// service refuses a schema-2 profile on an M4315 without `--display-sync`,
/// which only `ModuleControl::App` passes.
pub fn keypad_refusal(
    settings: DisplaySettings,
    module: ServiceModule,
    profile: &Profile,
) -> Option<&'static str> {
    if module != ServiceModule::M4315 || profile.schema_version != 2 {
        return None;
    }
    match settings.module_control {
        ModuleControl::App => None,
        ModuleControl::BuiltInAnimation => Some(BUILT_IN_KEYPAD_REFUSAL),
        ModuleControl::Unreadable => Some(UNREADABLE_KEYPAD_REFUSAL),
    }
}

/// The software mode a managed run on `module` gets: always under `App` for
/// the keypad and the dial, the Run panel's request under built-in animation,
/// and never for other modules or while the settings cannot be read.
pub fn software_mode(settings: DisplaySettings, module: ServiceModule, requested: bool) -> bool {
    if !app_controls(module) {
        return false;
    }
    match settings.module_control {
        ModuleControl::App => true,
        ModuleControl::BuiltInAnimation => requested,
        ModuleControl::Unreadable => false,
    }
}

/// The settings a managed session started with, and its module. Set by the
/// session driver only; the frontend cannot send it (`StartRequest` skips it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayLaunch {
    pub settings: DisplaySettings,
    pub module: ServiceModule,
}

/// Extra `masterhub-service run` arguments for one spawn of a managed session.
///
/// Nothing unless module control is `App` and the run requests software mode,
/// which `--display-sync` requires (`session::Driver::start` requests it for
/// every M4315 and M131E session with `App`). An M4315 gets `--display-sync`
/// only for a schema-2 keypad profile, so a profile switch between flat and
/// keypad profiles follows the profile of each spawn; an M131E gets
/// `--display-sync --dial-content` unless the dial content is off. Every
/// other module runs with exactly the arguments it had without the setting.
pub fn display_args(
    launch: Option<&DisplayLaunch>,
    software_mode: bool,
    profile: &Profile,
) -> Vec<OsString> {
    let Some(launch) = launch else {
        return Vec::new();
    };
    if launch.settings.module_control != ModuleControl::App || !software_mode {
        return Vec::new();
    }
    match launch.module {
        ServiceModule::M4315 if profile.schema_version == 2 && profile.keypad.is_some() => {
            vec!["--display-sync".into()]
        }
        ServiceModule::M131E if launch.settings.dial_content != DialContent::Off => vec![
            "--display-sync".into(),
            "--dial-content".into(),
            launch.settings.dial_content.arg().into(),
        ],
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

    /// A missing file means the defaults (`App`). A damaged file is an error
    /// and is never overwritten, like an unreadable profile.
    pub fn load(&self) -> Result<DisplaySettings, String> {
        let Some((bytes, _)) =
            store::read_private_file(&self.path(), MAX_FILE_BYTES, "display settings file")?
        else {
            return Ok(DisplaySettings::default());
        };
        serde_json::from_slice(&bytes).map_err(|error| {
            if error.is_data() {
                // The untagged reader's own message names no field.
                format!(
                    "the Module control settings in {} have an unexpected shape (expected {{\"module_control\": \"app\" or \"built_in_animation\", \"dial_content\": \"function\", \"clock\", \"analog_clock\", \"cpu\" or \"off\"}}); fix the file or delete it to go back to the defaults",
                    self.path().display()
                )
            } else {
                format!(
                    "display settings in {} are unreadable ({error}); fix or delete the file to go back to the defaults",
                    self.path().display()
                )
            }
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

    /// What a new session starts with: the stored settings, or
    /// `DisplaySettings::UNREADABLE` when they cannot be read, so a damaged
    /// file never turns display writes on.
    pub fn for_new_session(&self) -> DisplaySettings {
        self.get().unwrap_or(DisplaySettings::UNREADABLE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;

    const APP: DisplaySettings = DisplaySettings {
        module_control: ModuleControl::App,
        dial_content: DialContent::Clock,
    };
    const BUILT_IN: DisplaySettings = DisplaySettings {
        module_control: ModuleControl::BuiltInAnimation,
        dial_content: DialContent::Clock,
    };

    fn launch(module: ServiceModule, settings: DisplaySettings) -> DisplayLaunch {
        DisplayLaunch { settings, module }
    }

    fn flat() -> Profile {
        let mut profile = test_profiles::folders();
        profile.schema_version = 1;
        profile.keypad = None;
        profile
    }

    fn parse(json: &str) -> Result<DisplaySettings, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn settings_default_to_app_control_and_parse_strictly() {
        assert_eq!(
            DisplaySettings::default(),
            DisplaySettings {
                module_control: ModuleControl::App,
                dial_content: DialContent::Function
            }
        );
        let parsed = parse(r#"{"module_control":"app","dial_content":"analog_clock"}"#).unwrap();
        assert_eq!(parsed.dial_content, DialContent::AnalogClock);
        let parsed =
            parse(r#"{"module_control":"built_in_animation","dial_content":"cpu"}"#).unwrap();
        assert_eq!(
            parsed,
            DisplaySettings {
                module_control: ModuleControl::BuiltInAnimation,
                dial_content: DialContent::Cpu,
            }
        );
        // Always written in the current shape.
        assert_eq!(
            serde_json::to_value(BUILT_IN).unwrap(),
            serde_json::json!({"module_control":"built_in_animation","dial_content":"clock"})
        );
        for bad in [
            r#"{"module_control":"app"}"#,
            r#"{"module_control":"App","dial_content":"clock"}"#,
            r#"{"module_control":"builtin","dial_content":"clock"}"#,
            r#"{"module_control":"app","dial_content":"Clock"}"#,
            r#"{"module_control":"app","dial_content":"analog-clock"}"#,
            r#"{"module_control":"app","dial_content":"clock","extra":1}"#,
            r#"{"module_control":"app","enabled":true,"dial_content":"clock"}"#,
            r#"{"enabled":true}"#,
            r#"{"enabled":"yes","dial_content":"clock"}"#,
            r#"{"enabled":true,"dial_content":"gpu"}"#,
            r#"{"enabled":true,"dial_content":"clock","extra":1}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_experimental_toggle_file_migrates_on_read() {
        // An explicit "off" keeps the module's own mode; "on" is App control.
        assert_eq!(
            parse(r#"{"enabled":false,"dial_content":"function"}"#).unwrap(),
            DisplaySettings {
                module_control: ModuleControl::BuiltInAnimation,
                dial_content: DialContent::Function,
            }
        );
        assert_eq!(
            parse(r#"{"enabled":true,"dial_content":"clock"}"#).unwrap(),
            APP
        );
        // The file is rewritten in the current shape at the next save only.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let legacy = br#"{"enabled":false,"dial_content":"clock"}"#;
        std::fs::write(&path, legacy).unwrap();
        let state = DisplayState::open(Ok(directory.path()));
        assert_eq!(state.get().unwrap(), BUILT_IN);
        assert_eq!(state.for_new_session(), BUILT_IN);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            legacy,
            "read rewrote the file"
        );
        state.set(APP).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            saved,
            serde_json::json!({"module_control":"app","dial_content":"clock"})
        );
    }

    #[test]
    fn the_unreadable_value_is_never_stored_or_parsed() {
        assert!(parse(r#"{"module_control":"unreadable","dial_content":"clock"}"#).is_err());
        assert!(serde_json::to_vec(&DisplaySettings::UNREADABLE).is_err());
        let directory = tempfile::tempdir().unwrap();
        let state = DisplayState::open(Ok(directory.path()));
        assert!(state.set(DisplaySettings::UNREADABLE).is_err());
        assert!(!directory.path().join(FILE_NAME).exists());
    }

    #[test]
    fn software_mode_and_keypad_refusals_follow_module_control() {
        let unreadable = DisplaySettings::UNREADABLE;
        for module in [ServiceModule::M4315, ServiceModule::M131E] {
            for requested in [false, true] {
                assert!(software_mode(APP, module, requested));
                assert_eq!(software_mode(BUILT_IN, module, requested), requested);
                assert!(!software_mode(unreadable, module, requested));
            }
        }
        for module in [
            ServiceModule::M003K,
            ServiceModule::M005S,
            ServiceModule::M032LE,
        ] {
            for settings in [APP, BUILT_IN, unreadable] {
                assert!(!software_mode(settings, module, true));
                assert_eq!(
                    keypad_refusal(settings, module, &test_profiles::folders()),
                    None
                );
            }
        }
        let folders = test_profiles::folders();
        let keypad = ServiceModule::M4315;
        assert_eq!(keypad_refusal(APP, keypad, &folders), None);
        assert_eq!(
            keypad_refusal(BUILT_IN, keypad, &folders),
            Some(BUILT_IN_KEYPAD_REFUSAL)
        );
        assert_eq!(
            keypad_refusal(unreadable, keypad, &folders),
            Some(UNREADABLE_KEYPAD_REFUSAL)
        );
        assert!(UNREADABLE_KEYPAD_REFUSAL.contains("cannot be read"));
        assert!(!UNREADABLE_KEYPAD_REFUSAL.contains("Built-in animation"));
        assert_eq!(keypad_refusal(BUILT_IN, keypad, &flat()), None);
        assert_eq!(
            keypad_refusal(BUILT_IN, ServiceModule::M131E, &folders),
            None
        );
        // Unreadable settings never produce display arguments.
        for module in [ServiceModule::M4315, ServiceModule::M131E] {
            assert!(display_args(Some(&launch(module, unreadable)), true, &folders).is_empty());
        }
    }

    #[test]
    fn dial_content_arguments_match_the_service() {
        let ours: Vec<&str> = DialContent::ALL.into_iter().map(DialContent::arg).collect();
        let service: Vec<&str> = cooltek_sdk_cli::dial_artwork::DialContent::ALL
            .into_iter()
            .map(|content| content.as_str())
            .collect();
        assert_eq!(ours, service);
    }

    #[test]
    fn app_control_adds_display_arguments_for_the_keypad_and_dial_only() {
        let folders = test_profiles::folders();
        let none: Vec<OsString> = Vec::new();
        assert!(app_controls(ServiceModule::M4315) && app_controls(ServiceModule::M131E));
        // Built-in animation, a missing launch, or no software mode: nothing.
        for module in [ServiceModule::M4315, ServiceModule::M131E] {
            assert_eq!(
                display_args(Some(&launch(module, BUILT_IN)), true, &folders),
                none
            );
            assert_eq!(display_args(None, true, &folders), none);
            assert_eq!(
                display_args(Some(&launch(module, APP)), false, &folders),
                none
            );
        }
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, APP)), true, &folders),
            ["--display-sync"]
        );
        // A flat profile on the keypad: software mode only (the service
        // refuses --display-sync without a schema-2 keypad profile).
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, APP)), true, &flat()),
            none
        );
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M131E, APP)), true, &flat()),
            ["--display-sync", "--dial-content", "clock"]
        );
        // The default dial content is the active function's name.
        assert_eq!(
            display_args(
                Some(&launch(ServiceModule::M131E, DisplaySettings::default())),
                true,
                &flat()
            ),
            ["--display-sync", "--dial-content", "function"]
        );
        let dial_off = DisplaySettings {
            dial_content: DialContent::Off,
            ..APP
        };
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M131E, dial_off)), true, &flat()),
            none
        );
        // Knobs, faders and rollers never get display flags, whatever the
        // settings and even with software mode requested.
        for module in [
            ServiceModule::M003K,
            ServiceModule::M005S,
            ServiceModule::M032LE,
        ] {
            assert!(!app_controls(module));
            for settings in [APP, BUILT_IN, DisplaySettings::default()] {
                for profile in [&folders, &flat()] {
                    assert_eq!(
                        display_args(Some(&launch(module, settings)), true, profile),
                        none
                    );
                }
            }
        }
    }

    #[test]
    fn a_profile_switch_follows_the_profile_of_each_spawn() {
        let folders = test_profiles::folders();
        let keypad = launch(ServiceModule::M4315, APP);
        // Started flat, switched to keypad pages, and back.
        assert!(display_args(Some(&keypad), true, &flat()).is_empty());
        assert_eq!(
            display_args(Some(&keypad), true, &folders),
            ["--display-sync"]
        );
        // The dial is synced for any profile.
        let dial = launch(ServiceModule::M131E, APP);
        assert_eq!(display_args(Some(&dial), true, &folders).len(), 3);
        assert_eq!(display_args(Some(&dial), true, &flat()).len(), 3);
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
            display: Some(launch(ServiceModule::M4315, APP)),
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
        // Reading a missing file writes nothing and means App control.
        assert_eq!(state.get().unwrap(), DisplaySettings::default());
        assert_eq!(state.for_new_session().module_control, ModuleControl::App);
        assert!(!directory.path().join(FILE_NAME).exists());
        assert_eq!(state.set(BUILT_IN).unwrap(), BUILT_IN);
        assert_eq!(state.get().unwrap(), BUILT_IN);
        assert_eq!(
            DisplayState::open(Ok(directory.path())).get().unwrap(),
            BUILT_IN
        );
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
    fn a_damaged_file_is_preserved_and_new_sessions_use_the_built_in_mode() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let state = DisplayState::open(Ok(directory.path()));
        for bytes in [
            b"not json".to_vec(),
            br#"{"module_control":"app","dial_content":"clock","extra":1}"#.to_vec(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(state.get().is_err());
            assert!(state.set(APP).is_err());
            assert_eq!(state.for_new_session(), DisplaySettings::UNREADABLE);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
        }
        assert_eq!(
            DisplaySettings::UNREADABLE.module_control,
            ModuleControl::Unreadable
        );
        // The error names the file and says what to do.
        std::fs::write(
            &path,
            br#"{"module_control":"auto","dial_content":"clock"}"#,
        )
        .unwrap();
        let error = state.get().unwrap_err();
        assert!(error.contains(FILE_NAME), "{error}");
        assert!(error.contains("unexpected shape"), "{error}");
        assert!(error.contains("delete it"), "{error}");
        assert!(!error.contains("untagged"), "{error}");
        std::fs::write(&path, b"{").unwrap();
        let error = state.get().unwrap_err();
        assert!(
            error.contains("unreadable") && error.contains("delete the file"),
            "{error}"
        );
        assert!(DisplayState::open(Err("no app data".into())).get().is_err());
        assert_eq!(
            DisplayState::open(Err("no app data".into())).for_new_session(),
            DisplaySettings::UNREADABLE
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_settings_file_is_never_read_or_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(
            &outside,
            br#"{"module_control":"app","dial_content":"clock"}"#,
        )
        .unwrap();
        let app = directory.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::os::unix::fs::symlink(&outside, app.join(FILE_NAME)).unwrap();
        let state = DisplayState::open(Ok(&app));
        assert!(state.get().unwrap_err().contains("symbolic link"));
        assert!(state.set(DisplaySettings::default()).is_err());
        assert_eq!(state.for_new_session(), DisplaySettings::UNREADABLE);
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            br#"{"module_control":"app","dial_content":"clock"}"#
        );
    }
}
