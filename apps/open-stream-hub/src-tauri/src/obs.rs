//! The OBS WebSocket setting for the OBS actions (every `obs_*` action type).
//!
//! The endpoint (host and port, loopback only) and an on/off switch live in
//! `<app data>/obs-settings.json`, beside the profiles and never inside one,
//! so importing or sharing a profile can never redirect a scene switch. The
//! file uses the same bounded, symlink-safe read and atomic 0600 write as
//! `display-settings.json`. A missing file means off.
//!
//! The OBS WebSocket password is **never written anywhere**: not to that
//! file, a profile, a log, a diagnostics bundle, an environment variable or
//! an argument. It is entered in Settings, kept only in this process's memory
//! (so it is entered again after every app launch), and handed to each
//! `masterhub-service` process on its stdin pipe (`--obs-password-stdin`),
//! which the service reads once at start. `ps` shows only
//! `--obs-endpoint 127.0.0.1:4455 --obs-password-stdin`.
//!
//! Nothing is zeroed: plaintext copies of the password (the setting here, a
//! copy in each session's snapshot, the stdin buffer, the service's own copy)
//! stay in process memory for the life of the app and of each service, and a
//! core dump would contain them. Forgetting the password affects only later
//! Starts: a running session keeps its snapshot and sends it again on every
//! respawn until it is stopped.
//!
//! Like the display setting, it is read once when the user starts a managed
//! session (`session::Driver::start`) and kept with that session: wake,
//! reconnect, crash and profile-switch restarts reuse it, and changing it
//! never stops or restarts anything. See `docs/masterhub-actions.md`.

use crate::store;
use cooltek_sdk_service::obs::{ObsEndpoint, ObsPassword};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILE_BYTES: usize = 4 * 1024;
const FILE_NAME: &str = "obs-settings.json";

/// The stored setting. Off by default, at the obs-websocket default endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObsSettings {
    pub enabled: bool,
    /// `127.0.0.1`, `::1` or `localhost`.
    pub host: String,
    pub port: u16,
}

impl Default for ObsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: ObsEndpoint::DEFAULT_PORT,
        }
    }
}

impl ObsSettings {
    pub fn endpoint(&self) -> Result<ObsEndpoint, String> {
        ObsEndpoint::new(&self.host, self.port)
    }
}

/// What the frontend sees: the stored setting and whether a password is held
/// in memory. Never the password.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ObsSettingsView {
    #[serde(flatten)]
    pub settings: ObsSettings,
    pub password_set: bool,
}

/// The OBS connection a managed session started with. Set by the session
/// driver only; the frontend cannot send it (`StartRequest` skips it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObsLaunch {
    pub endpoint: ObsEndpoint,
    pub password: Option<ObsPassword>,
}

/// `masterhub-service run` arguments for a session's OBS connection: the
/// endpoint, and `--obs-password-stdin` when a password goes on stdin. Never
/// the password itself.
pub fn obs_args(launch: Option<&ObsLaunch>) -> Vec<OsString> {
    let Some(launch) = launch else {
        return Vec::new();
    };
    let mut args = vec!["--obs-endpoint".into(), launch.endpoint.to_string().into()];
    if launch.password.is_some() {
        args.push("--obs-password-stdin".into());
    }
    args
}

/// The bytes written to the service's stdin pipe: the password and a newline,
/// only when [`obs_args`] passes `--obs-password-stdin`.
pub fn password_stdin(launch: Option<&ObsLaunch>) -> Option<Vec<u8>> {
    Some(launch?.password.as_ref()?.stdin_line())
}

/// `obs-settings.json` inside the app data folder.
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

    /// A missing file means off. A damaged file (including a non-loopback
    /// host) is an error and is never overwritten.
    pub fn load(&self) -> Result<ObsSettings, String> {
        let Some((bytes, _)) =
            store::read_private_file(&self.path(), MAX_FILE_BYTES, "OBS settings file")?
        else {
            return Ok(ObsSettings::default());
        };
        let unreadable = |error: String| {
            format!(
                "OBS settings in {} are unreadable ({error}); fix or remove the file",
                self.path().display()
            )
        };
        let settings: ObsSettings =
            serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
        settings.endpoint().map_err(unreadable)?;
        Ok(settings)
    }

    pub fn save(&self, settings: &ObsSettings) -> Result<(), String> {
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
            .map_err(|error| format!("could not encode OBS settings: {error}"))?;
        store::write_private_file(&self.directory, &self.path(), &bytes, "OBS settings file")
    }
}

/// Settings storage and the in-memory password; the lock serializes
/// read-modify-write.
pub struct ObsState {
    file: Result<SettingsFile, String>,
    lock: Mutex<()>,
    password: Mutex<Option<ObsPassword>>,
}

impl ObsState {
    pub fn open(app_data: Result<&Path, String>) -> Self {
        Self {
            file: app_data.map(SettingsFile::new),
            lock: Mutex::new(()),
            password: Mutex::new(None),
        }
    }

    fn password_set(&self) -> bool {
        self.password
            .lock()
            .map(|password| password.is_some())
            .unwrap_or(false)
    }

    fn load(&self) -> Result<ObsSettings, String> {
        let file = self.file.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "OBS settings lock is unavailable".to_string())?;
        file.load()
    }

    pub fn get(&self) -> Result<ObsSettingsView, String> {
        Ok(ObsSettingsView {
            settings: self.load()?,
            password_set: self.password_set(),
        })
    }

    /// Saves `settings` unless they are already stored. The endpoint must be
    /// loopback even while OBS is off. A damaged file is reported and left alone.
    pub fn set(&self, settings: ObsSettings) -> Result<ObsSettingsView, String> {
        settings.endpoint()?;
        let file = self.file.as_ref().map_err(Clone::clone)?;
        {
            let _guard = self
                .lock
                .lock()
                .map_err(|_| "OBS settings lock is unavailable".to_string())?;
            if file.load()? != settings {
                file.save(&settings)?;
            }
        }
        Ok(ObsSettingsView {
            settings,
            password_set: self.password_set(),
        })
    }

    /// Holds `password` in memory for later Starts, or forgets it with
    /// `None`. Nothing is written. Errors never contain the password.
    /// Forgetting does not reach running sessions, which keep their snapshot
    /// until stopped, and the old value is not zeroed in memory.
    pub fn set_password(&self, password: Option<String>) -> Result<ObsSettingsView, String> {
        let password = password.map(ObsPassword::new).transpose()?;
        *self
            .password
            .lock()
            .map_err(|_| "OBS password lock is unavailable".to_string())? = password;
        self.get()
    }

    /// What a new session starts with: the endpoint and password when OBS is
    /// on, or `None` when it is off or the file cannot be read, so a damaged
    /// file never connects anywhere.
    pub fn for_new_session(&self) -> Option<ObsLaunch> {
        let settings = self.load().ok().filter(|settings| settings.enabled)?;
        Some(ObsLaunch {
            endpoint: settings.endpoint().ok()?,
            password: self.password.lock().ok()?.clone(),
        })
    }

    /// The connection for a picker listing (`obs_catalog`,
    /// `obs_source_filters`, `obs_scene_sources`). Unlike
    /// [`Self::for_new_session`], an off switch is an error the Settings
    /// panel can show rather than a silent `None`, and a damaged settings
    /// file is reported too.
    pub fn for_listing(&self) -> Result<ObsLaunch, String> {
        let settings = self.load()?;
        if !settings.enabled {
            return Err("OBS is off in OpenStreamHub Settings".into());
        }
        Ok(ObsLaunch {
            endpoint: settings.endpoint()?,
            password: self
                .password
                .lock()
                .map_err(|_| "OBS password lock is unavailable".to_string())?
                .clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const PASSWORD: &str = "hunter2-obs-secret";

    fn on() -> ObsSettings {
        ObsSettings {
            enabled: true,
            host: "127.0.0.1".into(),
            port: 4455,
        }
    }

    pub fn launch(password: Option<&str>) -> ObsLaunch {
        ObsLaunch {
            endpoint: ObsEndpoint::parse("127.0.0.1:4455").unwrap(),
            password: password.map(|password| ObsPassword::new(password.into()).unwrap()),
        }
    }

    #[test]
    fn settings_default_to_off_and_parse_strictly() {
        assert_eq!(
            ObsSettings::default(),
            ObsSettings {
                enabled: false,
                host: "127.0.0.1".into(),
                port: 4455
            }
        );
        let parsed: ObsSettings =
            serde_json::from_str(r#"{"enabled":true,"host":"::1","port":4456}"#).unwrap();
        assert_eq!(parsed.endpoint().unwrap().to_string(), "[::1]:4456");
        for bad in [
            r#"{"enabled":true}"#,
            r#"{"enabled":true,"host":"127.0.0.1"}"#,
            r#"{"enabled":true,"host":"127.0.0.1","port":70000}"#,
            r#"{"enabled":true,"host":"127.0.0.1","port":4455,"password":"x"}"#,
            r#"{"enabled":"yes","host":"127.0.0.1","port":4455}"#,
        ] {
            assert!(serde_json::from_str::<ObsSettings>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn arguments_carry_the_endpoint_and_never_the_password() {
        assert!(obs_args(None).is_empty());
        assert_eq!(password_stdin(None), None);
        assert_eq!(
            obs_args(Some(&launch(None))),
            ["--obs-endpoint", "127.0.0.1:4455"]
        );
        assert_eq!(password_stdin(Some(&launch(None))), None);
        let with = launch(Some(PASSWORD));
        let args = obs_args(Some(&with));
        assert_eq!(
            args,
            ["--obs-endpoint", "127.0.0.1:4455", "--obs-password-stdin"]
        );
        assert!(
            args.iter()
                .all(|arg| !arg.to_string_lossy().contains("hunter2"))
        );
        assert_eq!(
            password_stdin(Some(&with)).unwrap(),
            format!("{PASSWORD}\n").into_bytes()
        );
        // The service reads exactly what is written.
        let read =
            ObsPassword::read_line(std::io::Cursor::new(password_stdin(Some(&with)).unwrap()))
                .unwrap();
        assert_eq!(Some(read), with.password);
        assert!(!format!("{with:?}").contains("hunter2"));
    }

    #[test]
    fn the_settings_file_round_trips_privately_and_never_holds_the_password() {
        let directory = tempfile::tempdir().unwrap();
        let state = ObsState::open(Ok(directory.path()));
        assert_eq!(
            state.get().unwrap(),
            ObsSettingsView {
                settings: ObsSettings::default(),
                password_set: false
            }
        );
        assert!(!directory.path().join(FILE_NAME).exists());
        assert_eq!(state.for_new_session(), None);
        assert_eq!(state.set(on()).unwrap().settings, on());
        let view = state.set_password(Some(PASSWORD.into())).unwrap();
        assert!(view.password_set);
        let json = serde_json::to_string(&view).unwrap();
        assert_eq!(
            json,
            r#"{"enabled":true,"host":"127.0.0.1","port":4455,"password_set":true}"#
        );
        assert_eq!(state.for_new_session(), Some(launch(Some(PASSWORD))));
        // Re-saving the same settings keeps the password in memory only.
        state.set(on()).unwrap();
        let path = directory.path().join(FILE_NAME);
        let stored = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&stored).contains("hunter2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Nothing else is written to the app data folder.
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(FILE_NAME)]);
        // A fresh state (an app relaunch) has no password.
        let relaunched = ObsState::open(Ok(directory.path()));
        assert!(!relaunched.get().unwrap().password_set);
        assert_eq!(relaunched.for_new_session(), Some(launch(None)));
        // Clearing forgets it; off yields nothing.
        assert!(!state.set_password(None).unwrap().password_set);
        state
            .set(ObsSettings {
                enabled: false,
                ..on()
            })
            .unwrap();
        assert_eq!(state.for_new_session(), None);
    }

    #[test]
    fn listing_launches_need_the_switch_on_and_report_why() {
        let directory = tempfile::tempdir().unwrap();
        let state = ObsState::open(Ok(directory.path()));
        // Off (the default) and a damaged file are errors the Settings panel
        // can show, never a silent fallback.
        assert_eq!(
            state.for_listing().unwrap_err(),
            "OBS is off in OpenStreamHub Settings"
        );
        let path = directory.path().join(FILE_NAME);
        std::fs::write(&path, b"not json").unwrap();
        assert!(state.for_listing().unwrap_err().contains("unreadable"));
        std::fs::write(
            &path,
            br#"{"enabled":true,"host":"127.0.0.1","port":4455}"#,
        )
        .unwrap();
        let listed = state.for_listing().unwrap();
        assert_eq!(listed, launch(None));
        assert_eq!(
            obs_args(Some(&listed)),
            ["--obs-endpoint", "127.0.0.1:4455"]
        );
        state.set_password(Some(PASSWORD.into())).unwrap();
        assert_eq!(state.for_listing().unwrap(), launch(Some(PASSWORD)));
        state.set(ObsSettings {
            enabled: false,
            ..on()
        })
        .unwrap();
        assert_eq!(
            state.for_listing().unwrap_err(),
            "OBS is off in OpenStreamHub Settings"
        );
        // The non-descriptor paths are unchanged: off stays a silent `None`.
        assert_eq!(state.for_new_session(), None);
    }

    #[test]
    fn non_loopback_hosts_and_bad_passwords_are_refused_without_echo() {        let directory = tempfile::tempdir().unwrap();
        let state = ObsState::open(Ok(directory.path()));
        for (host, port) in [
            ("192.168.1.10", 4455),
            ("0.0.0.0", 4455),
            ("obs.local", 4455),
            ("127.0.0.2", 4455),
            ("127.0.0.1", 0),
        ] {
            let error = state
                .set(ObsSettings {
                    enabled: false,
                    host: host.into(),
                    port,
                })
                .unwrap_err();
            assert!(error.contains("OBS"), "{error}");
        }
        assert!(!directory.path().join(FILE_NAME).exists());
        for bad in [String::new(), "a\nb".into(), "x".repeat(2000)] {
            let error = state.set_password(Some(bad.clone())).unwrap_err();
            assert!(bad.is_empty() || !error.contains(&bad), "{error}");
        }
        let error = state
            .set_password(Some(format!("{PASSWORD}\u{7}")))
            .unwrap_err();
        assert!(!error.contains("hunter2"), "{error}");
        assert!(!state.get().unwrap().password_set);
    }

    #[test]
    fn a_damaged_file_is_preserved_and_new_sessions_start_without_obs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let state = ObsState::open(Ok(directory.path()));
        state.set_password(Some(PASSWORD.into())).ok();
        for bytes in [
            b"not json".to_vec(),
            br#"{"enabled":true,"host":"127.0.0.1","port":4455,"extra":1}"#.to_vec(),
            // A hand-edited remote host is refused like a damaged file.
            br#"{"enabled":true,"host":"10.0.0.5","port":4455}"#.to_vec(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(state.get().is_err());
            assert!(state.set(on()).is_err());
            assert_eq!(state.for_new_session(), None);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
        }
        assert!(ObsState::open(Err("no app data".into())).get().is_err());
        assert_eq!(
            ObsState::open(Err("no app data".into())).for_new_session(),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_settings_file_is_never_read_or_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(
            &outside,
            br#"{"enabled":true,"host":"127.0.0.1","port":4455}"#,
        )
        .unwrap();
        let app = directory.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::os::unix::fs::symlink(&outside, app.join(FILE_NAME)).unwrap();
        let state = ObsState::open(Ok(&app));
        assert!(state.get().unwrap_err().contains("symbolic link"));
        assert!(state.set(ObsSettings::default()).is_err());
        assert_eq!(state.for_new_session(), None);
    }
}
