//! Module control and module display settings.
//!
//! "Module control" says who drives the M4315 15-key module and the M131E
//! dial while a profile runs: `App` (the default) puts them in software mode
//! and keeps their displays in sync (`--software-mode --display-sync`, plus
//! `--dial-content` for the dial); `BuiltInAnimation` leaves them in the
//! module's own mode, exactly as before this setting existed. What the dial
//! screen shows is each profile's own `dial_display` setting, not a global
//! one; `display_args` derives the `--dial-content` value from the running
//! profile. Other modules never get software mode or display flags. The
//! settings live in `<app data>/display-settings.json`, beside the
//! profiles and never inside one, with the same bounded, symlink-safe read and
//! atomic 0600 write as the profile store and `focus-rules.json`. A missing
//! file means `App`.
//!
//! The two earlier file shapes are still read: the experimental toggle's
//! `{"enabled":…,"dial_content":…}` (`enabled: true` is `App`, an explicit
//! `enabled: false` is `BuiltInAnimation`) and the shape that kept a global
//! dial content choice beside module control (`dial_content` is ignored;
//! profiles carry their own since the choice moved into them). The next save
//! writes the current shape. A damaged or unreadable file is never
//! overwritten, and a session started meanwhile uses
//! `ModuleControl::Unreadable`: no software mode for any module, nothing
//! drawn on a display, and a schema-2 keypad profile is refused with a
//! message that names the damaged file.
//!
//! The settings are read once when a managed session starts (a user's Start,
//! or an automatic start, which happens only under `App`; see `startup.rs`)
//! and kept with that session, so wake, reconnect, crash and profile-switch
//! restarts use the value the session started with. Changing the setting
//! never stops or restarts anything; it applies at the next start.
//! `spawn_display` turns the session's copy into `masterhub-service`
//! arguments for each spawn, showing a flat keypad profile as one page
//! (`single_page`) and a calm dial face for a profile with nothing on the
//! dial (`dial_content`). Hardware results are in
//! `docs/masterhub-hardware-results.md`; see also
//! `docs/masterhub-display-sync.md` and `docs/masterhub-session-manager.md`.

use crate::{service::StartRequest, store};
use cooltek_sdk_service::{DialDisplayContent, Profile, ServiceModule};
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILE_BYTES: usize = 4 * 1024;
const FILE_NAME: &str = "display-settings.json";

/// The `masterhub-service --dial-content` spelling of a profile's dial
/// display content; a parity test keeps this in step with the service's own
/// values.
fn dial_arg(content: DialDisplayContent) -> &'static str {
    match content {
        DialDisplayContent::Off => "off",
        DialDisplayContent::Function => "function",
        DialDisplayContent::Clock => "clock",
        DialDisplayContent::AnalogClock => "analog-clock",
        DialDisplayContent::Cpu => "cpu",
        DialDisplayContent::Image => "image",
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

/// What a key press does when its profile runs: every action executes, or
/// everything is only logged. One mode for the whole app (the header badge),
/// held by the stored settings so it survives a restart and is read by the
/// session manager itself. The default is Live: a fresh install becomes live
/// at its first-launch confirmation, and until that confirmation nothing
/// executes (`live_offered` gates it, so the mode is never silently on).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunMode {
    /// Presses run their actions (macOS only, behind the per-profile
    /// confirmations).
    #[default]
    Live,
    /// Presses are logged only ("Would run: …"); nothing happens on the Mac.
    Test,
}

/// The stored settings. `App` control by default; the run mode defaults to
/// Live with the confirmation still owed (`live_offered: false` — a missing
/// settings file is a fresh install, which the frontend greets with the
/// what-will-run dialog once). The dial screen content is a profile setting
/// (`Profile`'s `dial_display`) and is no longer stored here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "StoredSettings")]
pub struct DisplaySettings {
    pub module_control: ModuleControl,
    /// What a press does in every run: Live executes, Test logs only. A file
    /// written before the mode existed reads as Test, and one written while
    /// "Run actions automatically" was on reads as Live.
    pub run_mode: RunMode,
    /// Whether the one-time Live confirmation has been answered. False only
    /// for a fresh install (no settings file): automatic sessions then stay
    /// dry with the reason shown until the frontend confirms the mode, so a
    /// new user's first presses never act silently. Files from before the
    /// field existed read as true: their user has seen the mode already.
    pub live_offered: bool,
    /// Set only while the frontend's editing hold ("Switch to Test mode while
    /// editing") has Test stored on the user's behalf: the Live it must
    /// restore when the editing ends — or at the next launch, when the app
    /// quit or crashed mid-edit and the hold never ended. The session manager
    /// reads `run_mode` alone and never interprets this; absent (and written
    /// as absent) whenever no hold is up, so files from before the field
    /// existed and files written outside a hold are byte-identical either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_from: Option<RunMode>,
    /// Whether a key on the 15-key module is drawn dimmed while it is held
    /// down (`--press-feedback`). On by default, and for every file written
    /// before the setting existed.
    pub press_feedback: bool,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            module_control: ModuleControl::default(),
            run_mode: RunMode::default(),
            live_offered: false,
            held_from: None,
            press_feedback: true,
        }
    }
}

impl DisplaySettings {
    /// What a session uses when the stored settings cannot be read: no
    /// software mode, no display writes and no execution, so a damaged file
    /// never starts either.
    pub const UNREADABLE: Self = Self {
        module_control: ModuleControl::Unreadable,
        run_mode: RunMode::Test,
        live_offered: true,
        held_from: None,
        press_feedback: false,
    };
}

/// The current file shape, or one of the earlier shapes: with the removed
/// `auto_execute` toggle (whose value becomes the run mode), that toggle
/// beside the removed dial content choice, and the experimental toggle's.
#[derive(Deserialize)]
#[serde(untagged)]
enum StoredSettings {
    Current(CurrentSettings),
    WithAutoExecute(WithAutoExecuteSettings),
    WithDialContent(WithDialContentSettings),
    Legacy(LegacySettings),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentSettings {
    module_control: ModuleControl,
    /// Missing in files written before the mode existed: Test (the mode's
    /// own default is Live, so the fallback is spelled out).
    #[serde(default = "run_mode_test")]
    run_mode: RunMode,
    /// Missing in the first files that stored a run mode: their user has
    /// used the switch already, so the one-time confirmation is not owed.
    #[serde(default = "live_offered_default")]
    live_offered: bool,
    /// Missing unless the file was written while the editing hold was up.
    #[serde(default)]
    held_from: Option<RunMode>,
    /// Missing in files written before the setting existed: on.
    #[serde(default = "press_feedback_default")]
    press_feedback: bool,
}

/// The field default for files that predate `press_feedback`.
fn press_feedback_default() -> bool {
    true
}

/// The field default for files that predate `live_offered`.
fn live_offered_default() -> bool {
    true
}

/// The field default for files that predate the run mode.
fn run_mode_test() -> RunMode {
    RunMode::Test
}

/// `{"module_control":…,"auto_execute":…}`, written while "Run actions
/// automatically" was the setting. The toggle becomes the run mode: on is
/// Live, off is Test.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithAutoExecuteSettings {
    module_control: ModuleControl,
    #[serde(default)]
    auto_execute: bool,
}

/// `{"module_control":…,"dial_content":…,"auto_execute":…}`, written while
/// the dial screen content was a global setting. The choice is ignored: every
/// profile now carries its own (`Profile`'s `dial_display`).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithDialContentSettings {
    module_control: ModuleControl,
    /// Missing in files written before the setting existed: off.
    #[serde(default)]
    auto_execute: bool,
    /// Named only so the old file shape still parses; never read.
    #[allow(dead_code)]
    dial_content: IgnoredAny,
}

/// `{"enabled":…,"dial_content":…}`, written by the experimental toggle.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySettings {
    enabled: bool,
    /// Named only so the old file shape still parses; never read.
    #[allow(dead_code)]
    dial_content: IgnoredAny,
}

impl From<StoredSettings> for DisplaySettings {
    fn from(stored: StoredSettings) -> Self {
        let run_mode = |auto_execute: bool| {
            if auto_execute {
                RunMode::Live
            } else {
                RunMode::Test
            }
        };
        match stored {
            StoredSettings::Current(CurrentSettings {
                module_control,
                run_mode,
                live_offered,
                held_from,
                press_feedback,
            }) => Self {
                module_control,
                run_mode,
                live_offered,
                held_from,
                press_feedback,
            },
            StoredSettings::WithAutoExecute(WithAutoExecuteSettings {
                module_control,
                auto_execute,
            })
            | StoredSettings::WithDialContent(WithDialContentSettings {
                module_control,
                auto_execute,
                ..
            }) => Self {
                module_control,
                run_mode: run_mode(auto_execute),
                // Their user predates the one-time confirmation.
                live_offered: true,
                held_from: None,
                press_feedback: true,
            },
            StoredSettings::Legacy(LegacySettings { enabled, .. }) => Self {
                module_control: if enabled {
                    ModuleControl::App
                } else {
                    ModuleControl::BuiltInAnimation
                },
                run_mode: RunMode::Test,
                live_offered: true,
                held_from: None,
                press_feedback: true,
            },
        }
    }
}

/// Why an automatic session on a fresh install runs dry: the run mode
/// defaults to Live, but the one-time what-will-run confirmation has not
/// answered yet, so nothing acts until the frontend confirms (or the user's
/// first confirmed Run does).
pub const LIVE_UNCONFIRMED: &str =
    "Live mode is not confirmed yet; confirm it in OpenStreamHub to run actions";

/// The modules `ModuleControl::App` drives in software mode.
pub fn app_controls(module: ServiceModule) -> bool {
    matches!(module, ServiceModule::M4315 | ServiceModule::M131E)
}

/// The modules the session manager starts on its own under `App`: every
/// control module. The keypad and the dial get software mode and display
/// sync; the M003K knobs, the M005S faders and the M032LE rollers report
/// input in their own mode and get an input-only session: never software
/// mode, display flags or any other write (`software_mode`,
/// `spawn_display`). The base is not a control module and never starts one.
pub fn auto_takes_over(module: ServiceModule) -> bool {
    matches!(
        module,
        ServiceModule::M4315
            | ServiceModule::M131E
            | ServiceModule::M003K
            | ServiceModule::M005S
            | ServiceModule::M032LE
    )
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
    /// Degrees clockwise (0, 90, 180 or 270) the module sits turned on the base; what its screen draws is turned the
    /// other way (`--display-turn`).
    pub turn: u16,
}

/// True when `profile` binds anything on the M131E dial.
fn binds_dial(profile: &Profile) -> bool {
    profile.dial_functions.is_some()
        || profile
            .bindings
            .iter()
            .any(|binding| binding.module == ServiceModule::M131E)
}

/// What the dial shows for `profile`: the profile's own `dial_display`
/// choice, defaulting to the active function's name. With that default, a
/// profile that binds nothing on the dial (such as the blank startup profile)
/// has no function to name, so the dial shows the calm analog clock face
/// instead of a profile name; every other choice is kept as set.
fn dial_content(profile: &Profile) -> DialDisplayContent {
    let content = profile
        .dial_display
        .as_ref()
        .map_or(DialDisplayContent::Function, |display| display.content);
    if content == DialDisplayContent::Function && !binds_dial(profile) {
        DialDisplayContent::AnalogClock
    } else {
        content
    }
}

/// `--display-turn` for a module turned on the base; nothing for an upright one.
fn turn_args(turn: u16) -> Vec<OsString> {
    if turn == 0 {
        Vec::new()
    } else {
        vec!["--display-turn".into(), turn.to_string().into()]
    }
}

/// Extra `masterhub-service run` arguments for one spawn of a managed session.
///
/// Nothing unless module control is `App` and the run requests software mode,
/// which `--display-sync` requires (`session::Driver::start` requests it for
/// every M4315 and M131E session with `App`). An M4315 gets `--display-sync`
/// only for a schema-2 keypad profile (`spawn_display` first turns a flat
/// profile into one page), so a profile switch between flat and keypad
/// profiles follows the profile of each spawn; an M131E gets
/// `--display-sync --dial-content` for the profile's dial display choice,
/// unless that choice is off (see `dial_content` for the default). Every
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
            let mut args: Vec<OsString> = vec!["--display-sync".into()];
            if launch.settings.press_feedback {
                args.push("--press-feedback".into());
            }
            args.extend(turn_args(launch.turn));
            args
        }
        ServiceModule::M131E if dial_content(profile) != DialDisplayContent::Off => {
            let mut args: Vec<OsString> = vec![
                "--display-sync".into(),
                "--dial-content".into(),
                dial_arg(dial_content(profile)).into(),
            ];
            args.extend(turn_args(launch.turn));
            args
        }
        _ => Vec::new(),
    }
}

/// A flat (schema-1) profile as one keypad page, for a keypad spawn that syncs
/// its display. `Profile::into_v2` puts every M4315 binding on its own root
/// key (labelled "Key N") with its gesture, serial selector and action, and
/// keeps every other binding, so `--display-sync` can draw it and the keys
/// plan and run the same actions as the flat profile. Only under module
/// control `App` with software mode on an M4315; never for a schema-2 profile
/// or one the conversion or the app's validation refuses, which runs flat in
/// software mode without display sync, as before.
pub fn single_page(
    launch: Option<&DisplayLaunch>,
    software_mode: bool,
    profile: &Profile,
) -> Option<Profile> {
    let launch = launch?;
    if launch.module != ServiceModule::M4315
        || launch.settings.module_control != ModuleControl::App
        || !software_mode
        || profile.schema_version != 1
    {
        return None;
    }
    let page = profile.clone().into_v2().ok()?;
    crate::replay::validate_profile(&page).ok()?;
    Some(page)
}

/// What one spawn writes and passes for the module displays.
#[derive(Debug, Default, PartialEq)]
pub struct SpawnDisplay {
    /// The profile to write instead of the session's own (a flat keypad
    /// profile shown as one page), or `None` to write the session's profile.
    /// The session keeps its own profile for restarts, switches and the
    /// Resume dialog.
    pub profile: Option<Profile>,
    pub args: Vec<OsString>,
}

/// The display profile and arguments for one spawn: only a session-managed
/// process gets any, never an unmanaged run (`lib.rs` `run_service`).
pub fn spawn_display(managed: bool, request: &StartRequest) -> SpawnDisplay {
    // Only the keypad and the dial ever get a display profile or flags; the
    // knobs' automatic session is input-only, whatever else it carries.
    if !managed
        || !request
            .display
            .is_some_and(|launch| app_controls(launch.module))
    {
        return SpawnDisplay::default();
    }
    let launch = request.display.as_ref();
    let profile = single_page(launch, request.software_mode, &request.profile);
    let args = display_args(
        launch,
        request.software_mode,
        profile.as_ref().unwrap_or(&request.profile),
    );
    SpawnDisplay { profile, args }
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
                    "the Module control settings in {} have an unexpected shape (expected {{\"module_control\": \"app\" or \"built_in_animation\", \"run_mode\": \"live\" or \"test\", \"live_offered\": true or false}}); fix the file or delete it to go back to the defaults",
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
        run_mode: RunMode::Test,
        live_offered: true,
        held_from: None,
        press_feedback: true,
    };
    const BUILT_IN: DisplaySettings = DisplaySettings {
        module_control: ModuleControl::BuiltInAnimation,
        run_mode: RunMode::Test,
        live_offered: true,
        held_from: None,
        press_feedback: true,
    };
    const APP_LIVE: DisplaySettings = DisplaySettings {
        module_control: ModuleControl::App,
        run_mode: RunMode::Live,
        live_offered: true,
        held_from: None,
        press_feedback: true,
    };
    /// A fresh install: Live, with the one-time confirmation still owed.
    const FRESH: DisplaySettings = DisplaySettings {
        module_control: ModuleControl::App,
        run_mode: RunMode::Live,
        live_offered: false,
        held_from: None,
        press_feedback: true,
    };

    fn launch(module: ServiceModule, settings: DisplaySettings) -> DisplayLaunch {
        DisplayLaunch {
            settings,
            module,
            turn: 0,
        }
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
        // No file (a fresh install): Live, with the confirmation still owed.
        assert_eq!(DisplaySettings::default(), FRESH);
        let parsed = parse(r#"{"module_control":"app"}"#).unwrap();
        assert_eq!(
            parsed,
            DisplaySettings {
                module_control: ModuleControl::App,
                run_mode: RunMode::Test,
                live_offered: true,
                held_from: None,
                press_feedback: true,
            }
        );
        let parsed = parse(r#"{"module_control":"built_in_animation","run_mode":"live"}"#).unwrap();
        assert_eq!(
            parsed,
            DisplaySettings {
                module_control: ModuleControl::BuiltInAnimation,
                run_mode: RunMode::Live,
                live_offered: true,
                held_from: None,
                press_feedback: true,
            }
        );
        // Files that already stored a run mode predate the confirmation.
        assert_eq!(
            parse(r#"{"module_control":"app","run_mode":"live","live_offered":false}"#).unwrap(),
            DisplaySettings {
                module_control: ModuleControl::App,
                run_mode: RunMode::Live,
                live_offered: false,
                held_from: None,
                press_feedback: true,
            }
        );
        // Always written in the current shape, confirmation included.
        assert_eq!(
            serde_json::to_value(BUILT_IN).unwrap(),
            serde_json::json!({"module_control":"built_in_animation","run_mode":"test","live_offered":true,"press_feedback":true})
        );
        for bad in [
            r#"{"module_control":"app","extra":1}"#,
            r#"{"module_control":"App","run_mode":"test"}"#,
            r#"{"module_control":"builtin"}"#,
            r#"{"module_control":"app","run_mode":"yes"}"#,
            r#"{"module_control":"app","live_offered":"yes"}"#,
            r#"{"module_control":"app","enabled":true}"#,
            r#"{"enabled":true}"#,
            r#"{"enabled":"yes"}"#,
            r#"{"enabled":true,"extra":1}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_earlier_file_shapes_migrate_on_read() {
        // The experimental toggle: an explicit "off" keeps the module's own
        // mode; "on" is App control. Its dial content choice is dropped.
        assert_eq!(
            parse(r#"{"enabled":false,"dial_content":"function"}"#).unwrap(),
            BUILT_IN
        );
        assert_eq!(
            parse(r#"{"enabled":true,"dial_content":"clock"}"#).unwrap(),
            APP
        );
        // The three-field shape that stored a global dial content choice
        // beside module control: the choice is ignored, module control is
        // kept, no "Run actions automatically" reads as Test.
        assert_eq!(
            parse(r#"{"module_control":"app","dial_content":"analog_clock"}"#).unwrap(),
            APP
        );
        assert_eq!(
            parse(r#"{"module_control":"built_in_animation","dial_content":"cpu","auto_execute":true}"#)
                .unwrap(),
            DisplaySettings {
                module_control: ModuleControl::BuiltInAnimation,
                run_mode: RunMode::Live,
                live_offered: true,
                held_from: None,
                press_feedback: true,
            }
        );
        // Every earlier shape reads as offered: their user predates the
        // one-time confirmation, which must never nag them.
        assert!(
            parse(r#"{"module_control":"app","run_mode":"test"}"#)
                .unwrap()
                .live_offered
        );
        // The "Run actions automatically" shape: on was Live, off is Test.
        assert_eq!(
            parse(r#"{"module_control":"app","auto_execute":true}"#).unwrap(),
            APP_LIVE
        );
        assert_eq!(
            parse(r#"{"module_control":"app","auto_execute":false}"#).unwrap(),
            APP
        );
        assert_eq!(
            parse(r#"{"module_control":"built_in_animation"}"#).unwrap(),
            BUILT_IN
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
            serde_json::json!({"module_control":"app","run_mode":"test","live_offered":true,"press_feedback":true})
        );
    }

    #[test]
    fn the_unreadable_value_is_never_stored_or_parsed() {
        assert!(parse(r#"{"module_control":"unreadable"}"#).is_err());
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
        use cooltek_sdk_cli::dial_artwork::DialContent as ServiceArg;
        for content in cooltek_sdk_service::DialDisplayContent::ALL {
            let service = match content {
                DialDisplayContent::Off => ServiceArg::Off,
                DialDisplayContent::Function => ServiceArg::Function,
                DialDisplayContent::Clock => ServiceArg::Clock,
                DialDisplayContent::AnalogClock => ServiceArg::AnalogClock,
                DialDisplayContent::Cpu => ServiceArg::Cpu,
                DialDisplayContent::Image => ServiceArg::Image,
            };
            assert_eq!(dial_arg(content), service.as_str(), "{content:?}");
        }
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
            ["--display-sync", "--press-feedback"]
        );
        // A flat profile on the keypad: software mode only (the service
        // refuses --display-sync without a schema-2 keypad profile).
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, APP)), true, &flat()),
            none
        );
        // A profile with nothing on the dial falls back to the analog clock;
        // a dial binding keeps the default function name, and an explicit
        // clock choice is kept either way.
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M131E, APP)), true, &flat()),
            ["--display-sync", "--dial-content", "analog-clock"]
        );
        assert_eq!(
            display_args(
                Some(&launch(ServiceModule::M131E, APP)),
                true,
                &with_dial_binding(flat())
            ),
            ["--display-sync", "--dial-content", "function"]
        );
        assert_eq!(
            display_args(
                Some(&launch(ServiceModule::M131E, APP)),
                true,
                &with_dial_display(flat(), DialDisplayContent::Clock)
            ),
            ["--display-sync", "--dial-content", "clock"]
        );
        // A profile whose dial display is off: no display flags at all.
        assert_eq!(
            display_args(
                Some(&launch(ServiceModule::M131E, APP)),
                true,
                &with_dial_display(flat(), DialDisplayContent::Off)
            ),
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
            ["--display-sync", "--press-feedback"]
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
        assert_eq!(
            spawn_display(true, &request),
            SpawnDisplay {
                profile: None,
                args: vec!["--display-sync".into(), "--press-feedback".into()]
            }
        );
        assert_eq!(spawn_display(false, &request), SpawnDisplay::default());
        let unidentified = StartRequest {
            display: None,
            ..request.clone()
        };
        assert_eq!(spawn_display(true, &unidentified), SpawnDisplay::default());
        // Not even a flat profile is converted for an unmanaged run.
        let flat_unmanaged = StartRequest {
            profile: flat(),
            ..request
        };
        assert_eq!(
            spawn_display(false, &flat_unmanaged),
            SpawnDisplay::default()
        );
    }

    /// `profile` with one binding on the dial.
    fn with_dial_binding(mut profile: Profile) -> Profile {
        profile.bindings.push(cooltek_sdk_service::Binding {
            module: ServiceModule::M131E,
            control: 0,
            gesture: cooltek_sdk_service::Gesture::Right,
            serial_number: None,
            action: cooltek_sdk_service::Action::Volume { delta: 2 },
        });
        profile.validate().unwrap();
        profile
    }

    /// A flat profile with two keypad bindings on keys 1 and 15.
    fn flat_keys() -> Profile {
        let mut profile = flat();
        for (control, delta) in [(0, 3), (14, -3)] {
            profile.bindings.push(cooltek_sdk_service::Binding {
                module: ServiceModule::M4315,
                control,
                gesture: cooltek_sdk_service::Gesture::Pressed,
                serial_number: None,
                action: cooltek_sdk_service::Action::Volume { delta },
            });
        }
        profile.validate().unwrap();
        profile
    }

    #[test]
    fn press_feedback_is_a_keypad_flag_that_defaults_on_and_can_be_turned_off() {
        let keypad = test_profiles::folders();
        let launch = |press_feedback| DisplayLaunch {
            settings: DisplaySettings {
                press_feedback,
                ..APP
            },
            module: ServiceModule::M4315,
            turn: 0,
        };
        assert_eq!(
            display_args(Some(&launch(true)), true, &keypad),
            ["--display-sync", "--press-feedback"]
        );
        assert_eq!(
            display_args(Some(&launch(false)), true, &keypad),
            ["--display-sync"]
        );
        // The dial never gets it, whatever the setting says.
        let dial = DisplayLaunch {
            settings: APP,
            module: ServiceModule::M131E,
            turn: 0,
        };
        assert!(
            !display_args(Some(&dial), true, &keypad)
                .iter()
                .any(|arg| arg == "--press-feedback")
        );
        // Files from before the setting read as on, and an explicit off survives.
        assert!(parse(r#"{"module_control":"app"}"#).unwrap().press_feedback);
        assert!(
            !parse(r#"{"module_control":"app","press_feedback":false}"#)
                .unwrap()
                .press_feedback
        );
        assert!(DisplaySettings::default().press_feedback);
    }

    #[test]
    fn a_module_turned_on_the_base_gets_display_turn_and_an_upright_one_does_not() {
        let keypad = Profile::from_json(include_bytes!(
            "../../../../examples/profiles/keypad-folders.json"
        ))
        .unwrap();
        let launch = |module, turn| DisplayLaunch {
            settings: APP,
            module,
            turn,
        };
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, 0)), true, &keypad),
            ["--display-sync", "--press-feedback"]
        );
        assert_eq!(
            display_args(Some(&launch(ServiceModule::M4315, 90)), true, &keypad),
            ["--display-sync", "--press-feedback", "--display-turn", "90"]
        );
        let dial_profile = Profile::from_json(include_bytes!(
            "../../../../examples/profiles/dial-functions.json"
        ))
        .unwrap();
        let args = display_args(
            Some(&launch(ServiceModule::M131E, 270)),
            true,
            &dial_profile,
        );
        assert_eq!(&args[args.len() - 2..], ["--display-turn", "270"]);
        // The knobs never draw, whatever turn they have.
        assert!(display_args(Some(&launch(ServiceModule::M003K, 90)), true, &keypad).is_empty());
    }

    #[test]
    fn a_flat_keypad_profile_is_shown_as_one_page_under_app_control() {
        use cooltek_sdk_service::KeypadTile;
        let request = |profile: Profile, settings, module, software_mode| StartRequest {
            profile,
            path_hex: "41".into(),
            execute: false,
            software_mode,
            profile_id: None,
            source_profile_id: None,
            display: Some(launch(module, settings)),
            obs: None,
        };
        let shown = spawn_display(true, &request(flat_keys(), APP, ServiceModule::M4315, true));
        assert_eq!(shown.args, ["--display-sync", "--press-feedback"]);
        let page = shown.profile.unwrap();
        // One root page; each keypad binding on its own key with its action,
        // every other binding unchanged, the name kept.
        assert_eq!((page.schema_version, page.name.as_str()), (2, "Folders"));
        let pages = &page.keypad.as_ref().unwrap().pages;
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].id, "root");
        let bound: Vec<usize> = pages[0]
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.is_some())
            .map(|(index, _)| index)
            .collect();
        assert_eq!(bound, [0, 14]);
        let Some(KeypadTile::Action { bindings, .. }) = &pages[0].slots[14] else {
            panic!("key 15 is not an action tile");
        };
        assert_eq!(
            bindings[0].action,
            cooltek_sdk_service::Action::Volume { delta: -3 }
        );
        assert_eq!(
            page.bindings,
            flat()
                .bindings
                .into_iter()
                .filter(|binding| binding.module != ServiceModule::M4315)
                .collect::<Vec<_>>()
        );
        // The blank startup profile is one empty page.
        let blank = spawn_display(
            true,
            &request(
                crate::startup::blank_profile(),
                APP,
                ServiceModule::M4315,
                true,
            ),
        );
        assert_eq!(blank.args, ["--display-sync", "--press-feedback"]);
        let blank = blank.profile.unwrap();
        assert!(
            blank.keypad.unwrap().pages[0]
                .slots
                .iter()
                .all(Option::is_none)
        );
        // Never under built-in animation or unreadable settings, without
        // software mode, for a schema-2 profile or on another module.
        for (profile, settings, module, software_mode) in [
            (flat_keys(), BUILT_IN, ServiceModule::M4315, true),
            (
                flat_keys(),
                DisplaySettings::UNREADABLE,
                ServiceModule::M4315,
                true,
            ),
            (flat_keys(), APP, ServiceModule::M4315, false),
            (test_profiles::folders(), APP, ServiceModule::M4315, true),
            (flat_keys(), APP, ServiceModule::M131E, true),
            (flat_keys(), APP, ServiceModule::M003K, true),
        ] {
            let request = request(profile, settings, module, software_mode);
            assert_eq!(
                single_page(request.display.as_ref(), software_mode, &request.profile),
                None
            );
            assert!(spawn_display(true, &request).profile.is_none());
        }
        // A profile the conversion refuses is passed on flat, without display
        // sync, exactly as before (the service then reports its own error).
        let mut invalid = flat_keys();
        invalid.name = " ".into();
        let refused = request(invalid, APP, ServiceModule::M4315, true);
        assert_eq!(spawn_display(true, &refused), SpawnDisplay::default());
    }

    #[test]
    fn the_dial_shows_an_idle_face_for_a_profile_with_nothing_on_it() {
        let dial = launch(ServiceModule::M131E, APP);
        let blank = crate::startup::blank_profile();
        // Without the profile setting and nothing bound on the dial: the
        // analog clock instead of a name.
        for profile in [&blank, &flat(), &test_profiles::folders()] {
            assert_eq!(
                display_args(Some(&dial), true, profile),
                ["--display-sync", "--dial-content", "analog-clock"]
            );
        }
        // A dial binding or dial functions keep the default function name.
        assert_eq!(
            display_args(Some(&dial), true, &with_dial_binding(flat()))[2],
            "function"
        );
        // Dial functions count as binding the dial, so an explicit default
        // keeps the function name there too.
        let with_functions = {
            let mut json = serde_json::json!({
                "schema_version": 1, "name": "Fn", "bindings": [],
                "dial_functions": [{"name": "Volume", "bindings": []}]
            });
            json["dial_display"] = serde_json::json!({"content": "function"});
            Profile::from_json(serde_json::to_vec(&json).unwrap().as_slice()).unwrap()
        };
        assert_eq!(
            display_args(Some(&dial), true, &with_functions)[2],
            "function"
        );
        // Every explicit choice is kept, and off stays off.
        for content in [
            DialDisplayContent::Clock,
            DialDisplayContent::Cpu,
            DialDisplayContent::AnalogClock,
            DialDisplayContent::Image,
        ] {
            assert_eq!(
                display_args(
                    Some(&dial),
                    true,
                    &with_dial_display(blank.clone(), content)
                )[2],
                dial_arg(content)
            );
        }
        assert!(
            display_args(
                Some(&dial),
                true,
                &with_dial_display(blank, DialDisplayContent::Off)
            )
            .is_empty()
        );
    }

    /// `profile` with its dial display content set (no image).
    fn with_dial_display(mut profile: Profile, content: DialDisplayContent) -> Profile {
        profile.dial_display = Some(cooltek_sdk_service::DialDisplay {
            content,
            image: None,
            source: None,
            animation: None,
        });
        profile
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

    #[test]
    fn the_run_mode_is_live_for_a_fresh_install_and_test_without_a_stored_mode() {
        // No file (a fresh install): Live, with the confirmation still owed.
        assert_eq!(DisplaySettings::default().run_mode, RunMode::Live);
        assert!(!DisplaySettings::default().live_offered);
        // A damaged or unreadable file never acts, whatever the mode.
        assert_eq!(DisplaySettings::UNREADABLE.run_mode, RunMode::Test);
        const { assert!(DisplaySettings::UNREADABLE.live_offered) }
        assert_eq!(
            parse(r#"{"module_control":"app","dial_content":"clock"}"#)
                .unwrap()
                .run_mode,
            RunMode::Test
        );
        assert_eq!(
            parse(r#"{"enabled":true,"dial_content":"clock"}"#)
                .unwrap()
                .run_mode,
            RunMode::Test
        );
        // The "Run actions automatically" shapes: on was Live, off is Test.
        assert_eq!(
            parse(r#"{"module_control":"app","dial_content":"clock","auto_execute":true}"#)
                .unwrap()
                .run_mode,
            RunMode::Live
        );
        assert_eq!(
            parse(r#"{"module_control":"app","auto_execute":true}"#)
                .unwrap()
                .run_mode,
            RunMode::Live
        );
        assert_eq!(
            parse(r#"{"module_control":"app","auto_execute":false}"#)
                .unwrap()
                .run_mode,
            RunMode::Test
        );
        // Strict: no other spelling or type, never through an old shape.
        for bad in [
            r#"{"module_control":"app","dial_content":"clock","auto_execute":"yes"}"#,
            r#"{"module_control":"app","dial_content":"clock","auto_execute":1}"#,
            r#"{"module_control":"app","dial_content":"clock","run_mode":"Live"}"#,
            r#"{"module_control":"app","run_mode":"dry"}"#,
            r#"{"enabled":true,"dial_content":"clock","auto_execute":true}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        // The unconfirmed shape round-trips through the file.
        let directory = tempfile::tempdir().unwrap();
        let state = DisplayState::open(Ok(directory.path()));
        state.set(FRESH).unwrap();
        assert_eq!(state.get().unwrap(), FRESH);
        assert_eq!(state.for_new_session(), FRESH);
        state.set(APP_LIVE).unwrap();
        assert_eq!(state.get().unwrap(), APP_LIVE);
        // A damaged file never turns it on.
        std::fs::write(directory.path().join(FILE_NAME), b"{").unwrap();
        assert_eq!(state.for_new_session().run_mode, RunMode::Test);
        assert!(state.for_new_session().live_offered);
    }

    #[test]
    fn the_editing_hold_is_stored_beside_the_mode_and_restores_as_absent() {
        // The held shape: Test stored on the user's behalf, with the Live to
        // restore beside it.
        let held = parse(
            r#"{"module_control":"app","run_mode":"test","live_offered":true,"held_from":"live"}"#,
        )
        .unwrap();
        assert_eq!(
            held,
            DisplaySettings {
                module_control: ModuleControl::App,
                run_mode: RunMode::Test,
                live_offered: true,
                held_from: Some(RunMode::Live),
                press_feedback: true,
            }
        );
        // Written back exactly as parsed, and the no-hold shape stays
        // byte-identical to the shape before the field existed.
        assert_eq!(
            serde_json::to_value(held).unwrap(),
            serde_json::json!({
                "module_control": "app", "run_mode": "test",
                "live_offered": true, "held_from": "live", "press_feedback": true
            })
        );
        assert_eq!(
            serde_json::to_value(APP).unwrap(),
            serde_json::json!({"module_control":"app","run_mode":"test","live_offered":true,"press_feedback":true})
        );
        // Strict about the value.
        assert!(parse(
            r#"{"module_control":"app","run_mode":"test","live_offered":true,"held_from":"dry"}"#
        )
        .is_err());
        // The hold survives a save and a reload, and restoring clears it.
        let directory = tempfile::tempdir().unwrap();
        let state = DisplayState::open(Ok(directory.path()));
        state.set(held).unwrap();
        assert_eq!(state.get().unwrap(), held);
        assert_eq!(state.for_new_session(), held);
        let restored = DisplaySettings {
            run_mode: RunMode::Live,
            ..APP
        };
        assert_ne!(restored, held);
        state.set(restored).unwrap();
        assert_eq!(state.get().unwrap(), restored);
        assert_eq!(state.for_new_session().held_from, None);
    }

    #[test]
    fn the_knobs_faders_and_rollers_are_taken_over_input_only() {
        for module in [
            ServiceModule::M003K,
            ServiceModule::M005S,
            ServiceModule::M032LE,
            ServiceModule::M4315,
            ServiceModule::M131E,
        ] {
            assert!(auto_takes_over(module));
        }
        let live = DisplaySettings {
            run_mode: RunMode::Live,
            ..APP
        };
        for (settings, module) in [APP, live].into_iter().flat_map(|settings| {
            [
                ServiceModule::M003K,
                ServiceModule::M005S,
                ServiceModule::M032LE,
            ]
            .map(|module| (settings, module))
        }) {
            assert!(!software_mode(settings, module, true));
            // Even a request that claims software mode gets nothing.
            let request = StartRequest {
                profile: flat(),
                path_hex: "41".into(),
                execute: true,
                software_mode: true,
                profile_id: None,
                source_profile_id: None,
                display: Some(launch(module, settings)),
                obs: None,
            };
            assert_eq!(spawn_display(true, &request), SpawnDisplay::default());
        }
    }
}
