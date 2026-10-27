//! The Streamlabs Desktop setting for the `streamlabs_*` actions, built like
//! the OBS setting (`obs.rs`).
//!
//! The endpoint (host and port, loopback only) and an on/off switch live in
//! `<app data>/streamlabs-settings.json`, beside the profiles and never inside
//! one, with the same bounded, symlink-safe read and atomic 0600 write. A
//! missing file means off.
//!
//! The Remote Control API token is **never written to a file**: not to that
//! file, a profile, a log, a diagnostics bundle, an environment variable or
//! an argument, and it is never returned to the webview. It is entered in
//! Settings and kept in the macOS Keychain (a separate item from the OBS
//! password, [`crate::keychain`]), read back the first time it is needed in a
//! launch, and handed to each `masterhub-service` process on its stdin pipe
//! (`--streamlabs-token-stdin`, the line after the OBS password when both are
//! set). `ps` shows only `--streamlabs-endpoint 127.0.0.1:59650
//! --streamlabs-token-stdin`. Nothing is zeroed, as for the OBS password.
//!
//! Streamlabs always asks for the token, so a session gets Streamlabs only
//! when the switch is on **and** a token is held. Like the OBS setting it is
//! read once when the user starts a managed session and kept with it through
//! every restart. The transport is unverified against a real Streamlabs
//! Desktop (see `docs/masterhub-actions.md`).

use crate::{keychain::PasswordVault, store};
use cooltek_sdk_service::streamlabs::{StreamlabsEndpoint, StreamlabsStatus, StreamlabsToken};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_FILE_BYTES: usize = 4 * 1024;
const FILE_NAME: &str = "streamlabs-settings.json";

/// The stored setting. Off by default, at Streamlabs' default endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamlabsSettings {
    pub enabled: bool,
    /// `127.0.0.1`, `::1` or `localhost`.
    pub host: String,
    pub port: u16,
}

impl Default for StreamlabsSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            host: "127.0.0.1".into(),
            port: StreamlabsEndpoint::DEFAULT_PORT,
        }
    }
}

impl StreamlabsSettings {
    pub fn endpoint(&self) -> Result<StreamlabsEndpoint, String> {
        StreamlabsEndpoint::new(&self.host, self.port)
    }
}

/// What the frontend sees: the stored setting and whether a token is held.
/// Never the token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamlabsSettingsView {
    #[serde(flatten)]
    pub settings: StreamlabsSettings,
    pub token_set: bool,
}

/// The Streamlabs connection a managed session started with. Set by the
/// session driver only; the frontend cannot send it (`StartRequest` skips it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamlabsLaunch {
    pub endpoint: StreamlabsEndpoint,
    pub token: StreamlabsToken,
}

/// `masterhub-service run` arguments for a session's Streamlabs connection:
/// the endpoint and `--streamlabs-token-stdin`. Never the token itself.
pub fn streamlabs_args(launch: Option<&StreamlabsLaunch>) -> Vec<OsString> {
    let Some(launch) = launch else {
        return Vec::new();
    };
    vec![
        "--streamlabs-endpoint".into(),
        launch.endpoint.to_string().into(),
        "--streamlabs-token-stdin".into(),
    ]
}

/// The token line written to the service's stdin pipe, after the OBS password
/// line when there is one.
pub fn token_stdin(launch: Option<&StreamlabsLaunch>) -> Option<Vec<u8>> {
    Some(launch?.token.stdin_line())
}

/// `streamlabs-settings.json` inside the app data folder.
struct SettingsFile {
    directory: PathBuf,
}

impl SettingsFile {
    fn path(&self) -> PathBuf {
        self.directory.join(FILE_NAME)
    }

    /// A missing file means off. A damaged file (including a non-loopback
    /// host) is an error and is never overwritten.
    fn load(&self) -> Result<StreamlabsSettings, String> {
        let Some((bytes, _)) =
            store::read_private_file(&self.path(), MAX_FILE_BYTES, "Streamlabs settings file")?
        else {
            return Ok(StreamlabsSettings::default());
        };
        let unreadable = |error: String| {
            format!(
                "Streamlabs settings in {} are unreadable ({error}); fix or remove the file",
                self.path().display()
            )
        };
        let settings: StreamlabsSettings =
            serde_json::from_slice(&bytes).map_err(|error| unreadable(error.to_string()))?;
        settings.endpoint().map_err(unreadable)?;
        Ok(settings)
    }

    fn save(&self, settings: &StreamlabsSettings) -> Result<(), String> {
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
            .map_err(|error| format!("could not encode Streamlabs settings: {error}"))?;
        store::write_private_file(
            &self.directory,
            &self.path(),
            &bytes,
            "Streamlabs settings file",
        )
    }
}

/// Settings storage and the held token; the lock serializes read-modify-write.
pub struct StreamlabsState {
    file: Result<SettingsFile, String>,
    lock: Mutex<()>,
    token: Mutex<Option<StreamlabsToken>>,
    vault: Box<dyn PasswordVault>,
    /// Set once the vault has been read (or has failed to be), so the
    /// Keychain is asked at most once per launch.
    vault_read: Mutex<bool>,
}

impl StreamlabsState {
    pub fn open(app_data: Result<&Path, String>) -> Self {
        #[cfg(all(target_os = "macos", not(test), not(debug_assertions)))]
        let vault: Box<dyn PasswordVault> =
            Box::new(crate::keychain::Keychain(crate::keychain::STREAMLABS_TOKEN));
        #[cfg(all(target_os = "macos", not(test), debug_assertions))]
        let vault: Box<dyn PasswordVault> = Box::new(crate::keychain::DevKeychain(
            crate::keychain::STREAMLABS_TOKEN,
        ));
        #[cfg(not(all(target_os = "macos", not(test))))]
        let vault: Box<dyn PasswordVault> = Box::new(crate::keychain::MemoryOnly);
        Self::open_with_vault(app_data, vault)
    }

    pub fn open_with_vault(app_data: Result<&Path, String>, vault: Box<dyn PasswordVault>) -> Self {
        Self {
            file: app_data.map(|directory| SettingsFile {
                directory: directory.to_path_buf(),
            }),
            lock: Mutex::new(()),
            token: Mutex::new(None),
            vault,
            vault_read: Mutex::new(false),
        }
    }

    /// The held token, reading the vault the first time. A vault that fails
    /// or holds something unusable leaves the token unset.
    fn held_token(&self) -> Option<StreamlabsToken> {
        if let Ok(mut read) = self.vault_read.lock()
            && !*read
        {
            *read = true;
            if let Ok(Some(stored)) = self.vault.load()
                && let Ok(token) = StreamlabsToken::new(stored)
                && let Ok(mut held) = self.token.lock()
                && held.is_none()
            {
                *held = Some(token);
            }
        }
        self.token.lock().ok()?.clone()
    }

    fn load(&self) -> Result<StreamlabsSettings, String> {
        let file = self.file.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "Streamlabs settings lock is unavailable".to_string())?;
        file.load()
    }

    pub fn get(&self) -> Result<StreamlabsSettingsView, String> {
        Ok(StreamlabsSettingsView {
            settings: self.load()?,
            token_set: self.held_token().is_some(),
        })
    }

    /// Saves `settings` unless they are already stored. The endpoint must be
    /// loopback even while Streamlabs is off. A damaged file is left alone.
    pub fn set(&self, settings: StreamlabsSettings) -> Result<StreamlabsSettingsView, String> {
        settings.endpoint()?;
        let file = self.file.as_ref().map_err(Clone::clone)?;
        {
            let _guard = self
                .lock
                .lock()
                .map_err(|_| "Streamlabs settings lock is unavailable".to_string())?;
            if file.load()? != settings {
                file.save(&settings)?;
            }
        }
        Ok(StreamlabsSettingsView {
            settings,
            token_set: self.held_token().is_some(),
        })
    }

    /// Holds `token` for later Starts and keeps it in the vault, or forgets
    /// it everywhere with `None`, like [`crate::obs::ObsState::set_password`].
    /// Errors never contain the token; a refusing vault still leaves it held
    /// for this launch and says so.
    pub fn set_token(&self, token: Option<String>) -> Result<StreamlabsSettingsView, String> {
        let token = token.map(StreamlabsToken::new).transpose()?;
        *self
            .vault_read
            .lock()
            .map_err(|_| "Streamlabs token lock is unavailable".to_string())? = true;
        let stored = match &token {
            Some(token) => {
                let line = token.stdin_line();
                let text = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                self.vault.save(&text)
            }
            None => self.vault.forget(),
        };
        *self
            .token
            .lock()
            .map_err(|_| "Streamlabs token lock is unavailable".to_string())? = token;
        stored.map_err(|error| format!("{error}; the token is kept for this launch only"))?;
        self.get()
    }

    /// What a new session starts with: the endpoint and token when the switch
    /// is on and a token is held, otherwise `None` (a damaged file never
    /// connects anywhere).
    pub fn for_new_session(&self) -> Option<StreamlabsLaunch> {
        let settings = self.load().ok().filter(|settings| settings.enabled)?;
        Some(StreamlabsLaunch {
            endpoint: settings.endpoint().ok()?,
            token: self.held_token()?,
        })
    }

    /// "Test connection": opens one connection to the saved endpoint with the
    /// held token and names what it found. Nothing is tried while the switch
    /// is off or no token is held; a damaged settings file is an error.
    pub fn probe(&self) -> Result<StreamlabsConnection, String> {
        let settings = self.load()?;
        let port = settings.port;
        if !settings.enabled {
            return Ok(StreamlabsConnection::describe(Probe::Off, false, port));
        }
        let endpoint = settings.endpoint()?;
        let Some(token) = self.held_token() else {
            return Ok(StreamlabsConnection::describe(Probe::NoToken, false, port));
        };
        let status =
            cooltek_sdk_service::streamlabs::StreamlabsClient::new(endpoint, token).probe();
        Ok(StreamlabsConnection::describe(
            Probe::Done(status),
            status == StreamlabsStatus::Unreachable && streamlabs_process_running(),
            port,
        ))
    }
}

/// What "Test connection" could do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The switch is off: nothing was tried.
    Off,
    /// No token is held: nothing was tried.
    NoToken,
    Done(StreamlabsStatus),
}

/// The result of "Test connection": a state the Settings chip names and one
/// line saying what to do. Never carries text from Streamlabs or the token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StreamlabsConnection {
    pub state: &'static str,
    pub message: String,
}

impl StreamlabsConnection {
    /// `running` tells a closed Streamlabs Desktop from one whose Remote
    /// Control server does not answer.
    pub fn describe(probe: Probe, running: bool, port: u16) -> Self {
        const WHERE: &str = "Streamlabs Desktop › Settings › Remote Control";
        let (state, message) = match probe {
            Probe::Off => ("off", "Streamlabs is off in OpenStreamHub Settings. Turn it on to use the Streamlabs actions.".to_string()),
            Probe::NoToken => ("token_needed", format!("Streamlabs asks for an API token. Copy it from {WHERE} (Show details reveals it).")),
            Probe::Done(StreamlabsStatus::Connected) => ("connected", "Connected to Streamlabs Desktop.".to_string()),
            Probe::Done(StreamlabsStatus::Unreachable) if running => (
                "remote_control_off",
                format!("Streamlabs Desktop is running but nothing answers on port {port}. Check the port in {WHERE}."),
            ),
            Probe::Done(StreamlabsStatus::Unreachable) => (
                "not_running",
                "Streamlabs Desktop is not running. Start it, then test again.".to_string(),
            ),
            Probe::Done(StreamlabsStatus::NotStreamlabs) => (
                "wrong_port",
                format!("Something other than Streamlabs Desktop's Remote Control answers on port {port}. Use the port shown in {WHERE} ({} by default).", StreamlabsEndpoint::DEFAULT_PORT),
            ),
            Probe::Done(StreamlabsStatus::TokenRefused) => (
                "wrong_token",
                format!("Streamlabs refused the API token. Copy it again from {WHERE}."),
            ),
            Probe::Done(StreamlabsStatus::Failed) => (
                "failed",
                "Streamlabs Desktop answered but the connection did not complete. This connection is not yet verified against every Streamlabs Desktop version.".to_string(),
            ),
        };
        Self { state, message }
    }
}

/// Whether a Streamlabs Desktop process is running for this user. A failed
/// lookup counts as not running. The process name is not verified on every
/// Streamlabs Desktop version, so this only picks the hint.
pub fn streamlabs_process_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-i", "streamlabs"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "s3cret-streamlabs-token";

    fn on() -> StreamlabsSettings {
        StreamlabsSettings {
            enabled: true,
            host: "127.0.0.1".into(),
            port: 59650,
        }
    }

    fn launch() -> StreamlabsLaunch {
        StreamlabsLaunch {
            endpoint: StreamlabsEndpoint::parse("127.0.0.1:59650").unwrap(),
            token: StreamlabsToken::new(TOKEN.into()).unwrap(),
        }
    }

    /// A vault that records what is stored, and can be told to fail.
    #[derive(Clone, Default)]
    struct FakeVault {
        stored: std::sync::Arc<Mutex<Option<String>>>,
        loads: std::sync::Arc<Mutex<u32>>,
        refuse: bool,
    }

    impl PasswordVault for FakeVault {
        fn load(&self) -> Result<Option<String>, String> {
            *self.loads.lock().unwrap() += 1;
            if self.refuse {
                return Err("locked".into());
            }
            Ok(self.stored.lock().unwrap().clone())
        }
        fn save(&self, token: &str) -> Result<(), String> {
            if self.refuse {
                return Err("could not save the Streamlabs token in the Keychain".into());
            }
            *self.stored.lock().unwrap() = Some(token.into());
            Ok(())
        }
        fn forget(&self) -> Result<(), String> {
            if self.refuse {
                return Err("could not remove the Streamlabs token in the Keychain".into());
            }
            *self.stored.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn each_probe_has_its_own_state_and_a_fix_hint() {
        let describe = |probe, running| StreamlabsConnection::describe(probe, running, 59650);
        let states: Vec<_> = [
            describe(Probe::Off, false),
            describe(Probe::NoToken, false),
            describe(Probe::Done(StreamlabsStatus::Connected), false),
            describe(Probe::Done(StreamlabsStatus::Unreachable), false),
            describe(Probe::Done(StreamlabsStatus::Unreachable), true),
            describe(Probe::Done(StreamlabsStatus::NotStreamlabs), false),
            describe(Probe::Done(StreamlabsStatus::TokenRefused), false),
            describe(Probe::Done(StreamlabsStatus::Failed), false),
        ]
        .into_iter()
        .map(|connection| {
            assert!(!connection.message.is_empty() && !connection.message.contains('\n'));
            connection.state
        })
        .collect();
        assert_eq!(
            states,
            [
                "off",
                "token_needed",
                "connected",
                "not_running",
                "remote_control_off",
                "wrong_port",
                "wrong_token",
                "failed"
            ]
        );
        assert!(
            describe(Probe::NoToken, false)
                .message
                .contains("Settings › Remote Control")
        );
    }

    #[test]
    fn settings_default_to_off_and_parse_strictly() {
        assert_eq!(
            StreamlabsSettings::default(),
            StreamlabsSettings {
                enabled: false,
                host: "127.0.0.1".into(),
                port: 59650
            }
        );
        let parsed: StreamlabsSettings =
            serde_json::from_str(r#"{"enabled":true,"host":"::1","port":59651}"#).unwrap();
        assert_eq!(parsed.endpoint().unwrap().to_string(), "[::1]:59651");
        for bad in [
            r#"{"enabled":true}"#,
            r#"{"enabled":true,"host":"127.0.0.1","port":70000}"#,
            r#"{"enabled":true,"host":"127.0.0.1","port":59650,"token":"x"}"#,
        ] {
            assert!(
                serde_json::from_str::<StreamlabsSettings>(bad).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn arguments_carry_the_endpoint_and_never_the_token() {
        assert!(streamlabs_args(None).is_empty());
        assert_eq!(token_stdin(None), None);
        let with = launch();
        let args = streamlabs_args(Some(&with));
        assert_eq!(
            args,
            [
                "--streamlabs-endpoint",
                "127.0.0.1:59650",
                "--streamlabs-token-stdin"
            ]
        );
        assert!(
            args.iter()
                .all(|arg| !arg.to_string_lossy().contains("s3cret"))
        );
        let line = token_stdin(Some(&with)).unwrap();
        assert_eq!(line, format!("{TOKEN}\n").into_bytes());
        assert_eq!(
            StreamlabsToken::read_line(std::io::Cursor::new(line)).unwrap(),
            with.token
        );
        assert!(!format!("{with:?}").contains("s3cret"));
    }

    #[test]
    fn the_token_goes_to_the_vault_never_to_the_file_and_comes_back_after_a_relaunch() {
        let directory = tempfile::tempdir().unwrap();
        let vault = FakeVault::default();
        let state = StreamlabsState::open_with_vault(Ok(directory.path()), Box::new(vault.clone()));
        assert_eq!(state.for_new_session(), None);
        state.set(on()).unwrap();
        // On, but without a token: Streamlabs is not configured for a run.
        assert_eq!(state.for_new_session(), None);
        let view = state.set_token(Some(TOKEN.into())).unwrap();
        assert!(view.token_set);
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            r#"{"enabled":true,"host":"127.0.0.1","port":59650,"token_set":true}"#
        );
        assert_eq!(vault.stored.lock().unwrap().as_deref(), Some(TOKEN));
        assert_eq!(state.for_new_session(), Some(launch()));
        let path = directory.path().join(FILE_NAME);
        assert!(!std::fs::read_to_string(&path).unwrap().contains("s3cret"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let loads = *vault.loads.lock().unwrap();
        let relaunched =
            StreamlabsState::open_with_vault(Ok(directory.path()), Box::new(vault.clone()));
        assert_eq!(*vault.loads.lock().unwrap(), loads, "not read at launch");
        assert!(relaunched.get().unwrap().token_set);
        relaunched.get().unwrap();
        assert_eq!(
            *vault.loads.lock().unwrap(),
            loads + 1,
            "read once per launch"
        );
        assert!(!relaunched.set_token(None).unwrap().token_set);
        assert_eq!(*vault.stored.lock().unwrap(), None);
        assert_eq!(relaunched.for_new_session(), None);
    }

    #[test]
    fn a_refusing_vault_keeps_the_token_for_this_launch_and_bad_tokens_are_not_echoed() {
        let directory = tempfile::tempdir().unwrap();
        let vault = FakeVault {
            refuse: true,
            ..FakeVault::default()
        };
        let state = StreamlabsState::open_with_vault(Ok(directory.path()), Box::new(vault));
        assert!(!state.get().unwrap().token_set);
        let error = state.set_token(Some(TOKEN.into())).unwrap_err();
        assert!(error.contains("this launch only"), "{error}");
        assert!(!error.contains("s3cret"), "{error}");
        assert!(state.get().unwrap().token_set, "still held in memory");

        let state = StreamlabsState::open(Ok(directory.path()));
        for bad in [
            String::new(),
            "s3cret\ntoken".into(),
            format!("s3cret{}", "x".repeat(300)),
        ] {
            let error = state.set_token(Some(bad)).unwrap_err();
            assert!(!error.contains("s3cret"), "{error}");
        }
        assert!(!state.get().unwrap().token_set);
    }

    #[test]
    fn non_loopback_hosts_and_damaged_files_are_refused_and_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let state = StreamlabsState::open(Ok(directory.path()));
        for (host, port) in [
            ("192.168.1.10", 59650),
            ("127.0.0.2", 59650),
            ("127.0.0.1", 0),
        ] {
            let error = state
                .set(StreamlabsSettings {
                    enabled: false,
                    host: host.into(),
                    port,
                })
                .unwrap_err();
            assert!(error.contains("Streamlabs"), "{error}");
        }
        let path = directory.path().join(FILE_NAME);
        assert!(!path.exists());
        state.set_token(Some(TOKEN.into())).unwrap();
        for bytes in [
            b"not json".to_vec(),
            br#"{"enabled":true,"host":"10.0.0.5","port":59650}"#.to_vec(),
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(state.get().is_err());
            assert!(state.set(on()).is_err());
            assert!(state.probe().is_err());
            assert_eq!(state.for_new_session(), None);
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
        }
        assert!(
            StreamlabsState::open(Err("no app data".into()))
                .get()
                .is_err()
        );
    }

    #[test]
    fn a_probe_never_connects_while_off_or_without_a_token() {
        let directory = tempfile::tempdir().unwrap();
        let state = StreamlabsState::open(Ok(directory.path()));
        assert_eq!(state.probe().unwrap().state, "off");
        state.set(on()).unwrap();
        assert_eq!(state.probe().unwrap().state, "token_needed");
        // Nothing listens on this port.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        state.set(StreamlabsSettings { port, ..on() }).unwrap();
        state.set_token(Some(TOKEN.into())).unwrap();
        let connection = state.probe().unwrap();
        assert!(
            ["not_running", "remote_control_off"].contains(&connection.state),
            "{connection:?}"
        );
        assert!(!format!("{connection:?}").contains("s3cret"));
    }
}
