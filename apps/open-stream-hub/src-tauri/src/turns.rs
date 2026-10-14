//! How far each module sits turned on the base, for what its screen draws.
//!
//! A module the base's connector (or the user) turns a quarter or half shows its pictures sideways or upside down.
//! `masterhub-service --display-turn` turns what it draws the other way, so pictures read upright. The turn is the
//! base's rotation plus the turn of the connector the module sits on (read fresh from the inventory, so replugging a
//! module into another connector is picked up at once), or the rotation the user chose for that module instead. The
//! frontend, which holds the base rotation and the per-module choices, reports them here; they are kept in
//! `<app data>/module-turns.json` (`{"base":90,"overrides":{"serial:<serial number>":180}}`, a module without a
//! serial number is named `path:<hex path>`) so a run started as the app opens uses them before the webview loads.
//! The turn is read for each new run: a change reaches a module the next time its run starts, like the module
//! control settings.

use crate::feed::LiveDevice;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE_NAME: &str = "module-turns.json";
const MAX_FILE_BYTES: u64 = 64 * 1024;
const MAX_ENTRIES: usize = 256;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    /// The base's rotation in degrees.
    #[serde(default)]
    pub base: u16,
    /// Rotations the user chose for single modules, instead of following the base and the connector.
    #[serde(default)]
    pub overrides: BTreeMap<String, u16>,
}

pub struct ModuleTurns {
    path: Option<PathBuf>,
    preferences: Mutex<Preferences>,
}

/// The key a module's own rotation choice is stored under.
pub fn key(device: &LiveDevice) -> String {
    match &device.serial_number {
        Some(serial) => format!("serial:{serial}"),
        None => format!("path:{}", device.path_hex),
    }
}

fn valid(degrees: u16) -> bool {
    matches!(degrees, 0 | 90 | 180 | 270)
}

fn check(preferences: &Preferences) -> Result<(), String> {
    if preferences.overrides.len() > MAX_ENTRIES {
        return Err("too many modules".into());
    }
    if !valid(preferences.base) {
        return Err(format!("{} is not 0, 90, 180 or 270", preferences.base));
    }
    match preferences
        .overrides
        .iter()
        .find(|(_, degrees)| !valid(**degrees))
    {
        Some((name, degrees)) => Err(format!("{name}: {degrees} is not 0, 90, 180 or 270")),
        None => Ok(()),
    }
}

fn read(path: &Path) -> Preferences {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Preferences::default();
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Preferences::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Preferences>(&text).ok())
        .filter(|preferences| check(preferences).is_ok())
        .unwrap_or_default()
}

impl ModuleTurns {
    pub fn open(app_data: Option<&Path>) -> Self {
        let path = app_data.map(|dir| dir.join(FILE_NAME));
        let preferences = path.as_deref().map(read).unwrap_or_default();
        Self {
            path,
            preferences: Mutex::new(preferences),
        }
    }

    /// Degrees clockwise from upright: the user's choice for the module, else the base's rotation plus the turn of
    /// the connector it sits on.
    pub fn turn_of(&self, device: &LiveDevice) -> u16 {
        let Ok(preferences) = self.preferences.lock() else {
            return device.base_turn.unwrap_or(0);
        };
        preferences
            .overrides
            .get(&key(device))
            .copied()
            .unwrap_or((preferences.base + device.base_turn.unwrap_or(0)) % 360)
    }

    /// Replaces the stored preferences with the frontend's current ones.
    pub fn set(&self, preferences: Preferences) -> Result<(), String> {
        check(&preferences)?;
        let mut stored = self
            .preferences
            .lock()
            .map_err(|_| "module turns are unavailable")?;
        if *stored == preferences {
            return Ok(());
        }
        if let Some(path) = &self.path {
            let json = serde_json::to_string(&preferences).map_err(|error| error.to_string())?;
            let temporary = path.with_extension("json.tmp");
            std::fs::write(&temporary, format!("{json}\n"))
                .and_then(|()| std::fs::rename(&temporary, path))
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        }
        *stored = preferences;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::ServiceModule;

    fn device(serial: Option<&str>, base_turn: Option<u16>) -> LiveDevice {
        LiveDevice {
            path_hex: "ab".into(),
            module: ServiceModule::M4315,
            serial_number: serial.map(str::to_owned),
            base_cell: None,
            base_turn,
            port_chain: None,
        }
    }

    fn preferences(base: u16, overrides: &[(&str, u16)]) -> Preferences {
        Preferences {
            base,
            overrides: overrides
                .iter()
                .map(|(name, degrees)| ((*name).to_owned(), *degrees))
                .collect(),
        }
    }

    #[test]
    fn a_module_follows_the_base_and_its_connector_unless_the_user_chose_a_turn() {
        let dir = tempfile::tempdir().unwrap();
        let turns = ModuleTurns::open(Some(dir.path()));
        assert_eq!(turns.turn_of(&device(Some("S1"), Some(90))), 90);
        assert_eq!(turns.turn_of(&device(None, None)), 0);
        turns
            .set(preferences(90, &[("serial:S1", 270), ("path:ab", 0)]))
            .unwrap();
        // The module's own choice wins, even when it is upright.
        assert_eq!(turns.turn_of(&device(Some("S1"), Some(90))), 270);
        assert_eq!(turns.turn_of(&device(None, Some(90))), 0);
        // Everything else: the base plus the connector, read from the device as it is now.
        assert_eq!(turns.turn_of(&device(Some("S2"), Some(90))), 180);
        assert_eq!(turns.turn_of(&device(Some("S2"), Some(270))), 0);
        assert_eq!(turns.turn_of(&device(Some("S2"), None)), 90);
        // Kept across a restart of the app.
        let reopened = ModuleTurns::open(Some(dir.path()));
        assert_eq!(reopened.turn_of(&device(Some("S1"), None)), 270);
        assert_eq!(reopened.turn_of(&device(Some("S2"), Some(90))), 180);
    }

    #[test]
    fn only_quarter_turns_are_accepted_and_a_damaged_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let turns = ModuleTurns::open(Some(dir.path()));
        assert!(turns.set(preferences(45, &[])).is_err());
        assert!(turns.set(preferences(0, &[("serial:S1", 360)])).is_err());
        std::fs::write(dir.path().join(FILE_NAME), "{ nope").unwrap();
        assert_eq!(
            ModuleTurns::open(Some(dir.path())).turn_of(&device(Some("S1"), Some(90))),
            90
        );
        std::fs::write(dir.path().join(FILE_NAME), r#"{"base":45}"#).unwrap();
        assert_eq!(
            ModuleTurns::open(Some(dir.path())).turn_of(&device(Some("S1"), None)),
            0
        );
    }
}
