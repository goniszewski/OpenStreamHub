//! Installed plugins: their manifests, and the checks an invoke passes before it reaches a plugin.
//!
//! A plugin is a folder `<app data>/plugins/<id>/` holding `plugin.json` (see `docs/plugins-design.md`). The folder
//! name must be the manifest's id, so two folders can never claim one id. The folder and the manifest must belong to
//! this user and be writable by nobody else, since the manifest names the program the app starts.
//!
//! ```json
//! {
//!   "id": "com.example.lights", "name": "Lights", "version": "1.2.0",
//!   "command": ["bin/lights"],
//!   "actions": [{ "id": "set", "label": "Set a light", "params": [
//!     { "id": "entity", "label": "Light", "type": "text", "variables": true, "required": true },
//!     { "id": "level", "label": "Level", "type": "number", "min": 0, "max": 100 },
//!     { "id": "fade", "label": "Fade", "type": "bool" },
//!     { "id": "mode", "label": "Mode", "type": "choice", "choices": [{ "id": "warm", "label": "Warm" }], "default": "warm" }
//!   ] }],
//!   "permissions": { "network": ["homeassistant.local:8123"], "variables": ["room"], "programs": false }
//! }
//! ```
//!
//! `command[0]` is a path inside the folder (no `..`) or an absolute path such as `/usr/local/bin/node`; the rest are
//! its arguments, passed as they are. Permissions are what the plugin says it needs, shown to the user: a plugin runs
//! as the user, so they are consent information, not enforcement.

use cooltek_sdk_service::{
    Action, PluginValue,
    plugin::{parse_version, validate_plugin_action_id, validate_plugin_id},
    validate_variable_name,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

/// The manifest's file name inside a plugin folder.
pub const MANIFEST_FILE: &str = "plugin.json";
/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// Most actions one plugin offers.
const MAX_ACTIONS: usize = 64;
/// Most items in the start command.
const MAX_COMMAND_ITEMS: usize = 16;
/// Most choices of one choice parameter.
const MAX_CHOICES: usize = 64;
/// Longest name, label or permission entry in bytes.
const MAX_LABEL_BYTES: usize = 64;
/// Longest command item in bytes.
const MAX_COMMAND_BYTES: usize = 512;
/// Most entries of one permission list.
const MAX_PERMISSION_ITEMS: usize = 32;
/// Longest network permission entry in bytes.
const MAX_HOST_BYTES: usize = 255;

/// The folder plugins live in: `<app data>/plugins`.
pub fn dir(app_data: &Path) -> PathBuf {
    app_data.join("plugins")
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub command: Vec<String>,
    pub actions: Vec<ManifestAction>,
    #[serde(default)]
    pub permissions: Permissions,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManifestAction {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub params: Vec<Param>,
}

/// One parameter of a plugin action. `type` decides which of the other fields apply: `variables` and `secret` only to
/// text, `min` and `max` only to a number, `choices` only (and always) to a choice.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Param {
    pub id: String,
    pub label: String,
    #[serde(rename = "type")]
    pub kind: ParamType,
    /// The press fails when a required parameter is missing.
    #[serde(default)]
    pub required: bool,
    /// Text only: `{{name}}` references may be filled in when the key is pressed.
    #[serde(default)]
    pub variables: bool,
    /// Text only: shown masked in the editor and never logged.
    #[serde(default)]
    pub secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Choice>>,
    /// The value the editor starts with, of the parameter's type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PluginValue>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    Text,
    Number,
    Bool,
    Choice,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    /// Hosts it says it connects to, such as `homeassistant.local:8123`.
    #[serde(default)]
    pub network: Vec<String>,
    /// Variables it says it sets.
    #[serde(default)]
    pub variables: Vec<String>,
    /// Whether it says it starts other programs.
    #[serde(default)]
    pub programs: bool,
}

fn check_label(what: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty()
        || text.len() > MAX_LABEL_BYTES
        || text
            .chars()
            .any(|c| c.is_control() || cooltek_sdk_service::is_invisible_text_character(c))
    {
        return Err(format!(
            "{what} must be 1 to {MAX_LABEL_BYTES} bytes of one line without control or invisible characters"
        ));
    }
    Ok(())
}

fn finite(what: &str, value: Option<f64>) -> Result<(), String> {
    match value {
        Some(number) if !number.is_finite() => Err(format!("{what} must be a finite number")),
        _ => Ok(()),
    }
}

impl Param {
    fn validate(&self) -> Result<(), String> {
        let at = |what: &str| format!("parameter {} {what}", self.id);
        validate_variable_name(&self.id).map_err(|_| {
            format!(
                "parameter id {:?} must be 1 to 32 of a-z, 0-9 and _, starting with a letter",
                self.id
            )
        })?;
        check_label(&at("label"), &self.label)?;
        let text = self.kind == ParamType::Text;
        if (self.variables || self.secret) && !text {
            return Err(at("can take variables or be secret only as text"));
        }
        let number = self.kind == ParamType::Number;
        if (self.min.is_some() || self.max.is_some()) && !number {
            return Err(at("can have min and max only as a number"));
        }
        if self.choices.is_some() != (self.kind == ParamType::Choice) {
            return Err(at("has choices exactly when its type is choice"));
        }
        finite(&at("min"), self.min)?;
        finite(&at("max"), self.max)?;
        if let (Some(min), Some(max)) = (self.min, self.max)
            && min > max
        {
            return Err(at("min is above max"));
        }
        if let Some(choices) = &self.choices {
            if choices.is_empty() || choices.len() > MAX_CHOICES {
                return Err(at(&format!("must offer 1 to {MAX_CHOICES} choices")));
            }
            let mut seen = BTreeSet::new();
            for choice in choices {
                validate_plugin_action_id(&choice.id)
                    .map_err(|_| at(&format!("choice {:?} has an invalid id", choice.id)))?;
                check_label(&at("choice label"), &choice.label)?;
                if !seen.insert(&choice.id) {
                    return Err(at(&format!("offers choice {} twice", choice.id)));
                }
            }
        }
        if let Some(default) = &self.default {
            if let PluginValue::Text(value) = default
                && text
            {
                let mut probe = BTreeMap::new();
                probe.insert(self.id.clone(), PluginValue::Text(value.clone()));
                cooltek_sdk_service::plugin::validate_plugin_action(
                    "com.example.probe",
                    "0.0.0",
                    "probe",
                    &probe,
                    &[],
                )
                .map_err(|_| at("default must be one line of at most 2048 bytes"))?;
            } else if !self.accepts(default) {
                return Err(at("default is not a value of its type, range or choices"));
            }
        }
        Ok(())
    }

    /// Whether `value` is of this parameter's type, within its range or among its choices.
    fn accepts(&self, value: &PluginValue) -> bool {
        match (self.kind, value) {
            (ParamType::Text, PluginValue::Text(_)) | (ParamType::Bool, PluginValue::Bool(_)) => {
                true
            }
            (ParamType::Number, PluginValue::Number(number)) => number
                .as_f64()
                .is_some_and(|number| in_range(number, self.min, self.max)),
            (ParamType::Choice, PluginValue::Text(id)) => {
                self.choices.iter().flatten().any(|choice| &choice.id == id)
            }
            _ => false,
        }
    }

    fn takes_variables(&self) -> bool {
        self.kind == ParamType::Text && self.variables
    }
}

fn in_range(number: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.is_none_or(|min| number >= min) && max.is_none_or(|max| number <= max)
}

/// A command item that is a path inside the plugin folder: relative, without `..`.
fn inside_folder(path: &str) -> bool {
    let path = Path::new(path);
    path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        validate_plugin_id(&self.id)?;
        check_label("name", &self.name)?;
        parse_version(&self.version).ok_or("version must be MAJOR.MINOR.PATCH, such as 1.0.0")?;
        if self.command.is_empty() || self.command.len() > MAX_COMMAND_ITEMS {
            return Err(format!(
                "command must have 1 to {MAX_COMMAND_ITEMS} items: the program, then its arguments"
            ));
        }
        for item in &self.command {
            if item.len() > MAX_COMMAND_BYTES || item.chars().any(char::is_control) {
                return Err(format!(
                    "every command item must be one line of at most {MAX_COMMAND_BYTES} bytes"
                ));
            }
        }
        let program = &self.command[0];
        if program.is_empty() || !(program.starts_with('/') || inside_folder(program)) {
            return Err(
                "command must start with a program inside the plugin folder (no ..) or an absolute path".into(),
            );
        }
        if self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
            return Err(format!("a plugin offers 1 to {MAX_ACTIONS} actions"));
        }
        let mut action_ids = BTreeSet::new();
        for action in &self.actions {
            validate_plugin_action_id(&action.id)
                .map_err(|error| format!("action {:?}: {error}", action.id))?;
            if !action_ids.insert(&action.id) {
                return Err(format!("action {} is listed twice", action.id));
            }
            check_label(&format!("action {} label", action.id), &action.label)?;
            if action.params.len() > cooltek_sdk_service::plugin::MAX_PLUGIN_PARAMS {
                return Err(format!(
                    "action {} has more than {} parameters",
                    action.id,
                    cooltek_sdk_service::plugin::MAX_PLUGIN_PARAMS
                ));
            }
            let mut param_ids = BTreeSet::new();
            for param in &action.params {
                param
                    .validate()
                    .map_err(|error| format!("action {}: {error}", action.id))?;
                if !param_ids.insert(&param.id) {
                    return Err(format!(
                        "action {}: parameter {} is listed twice",
                        action.id, param.id
                    ));
                }
            }
        }
        let permissions = &self.permissions;
        if permissions.network.len() > MAX_PERMISSION_ITEMS
            || permissions.variables.len() > MAX_PERMISSION_ITEMS
        {
            return Err(format!(
                "each permission list has at most {MAX_PERMISSION_ITEMS} entries"
            ));
        }
        for host in &permissions.network {
            if host.is_empty()
                || host.len() > MAX_HOST_BYTES
                || !host.chars().all(|c| c.is_ascii_graphic())
            {
                return Err(format!(
                    "network permission {host:?} must be a host such as example.com or example.com:8123"
                ));
            }
        }
        for name in &permissions.variables {
            validate_variable_name(name)
                .map_err(|_| format!("variables permission {name:?} is not a variable name"))?;
        }
        Ok(())
    }

    pub fn action(&self, id: &str) -> Option<&ManifestAction> {
        self.actions.iter().find(|action| action.id == id)
    }

    /// The program and arguments to start, with a folder-relative program made absolute.
    pub fn command_in(&self, folder: &Path) -> (PathBuf, Vec<String>) {
        let program = &self.command[0];
        let program = if program.starts_with('/') {
            PathBuf::from(program)
        } else {
            folder.join(program)
        };
        (program, self.command[1..].to_vec())
    }

    /// Whether this installed version satisfies a profile's `min_version`.
    pub fn satisfies(&self, min_version: &str) -> bool {
        matches!(
            (parse_version(&self.version), parse_version(min_version)),
            (Some(installed), Some(wanted)) if installed >= wanted
        )
    }
}

/// Checks an invoke against the manifest: the action exists, every parameter is one it declares and of its type
/// (and in range or among its choices), every required one is given, and only parameters that allow it take
/// variables. `variables` are the parameter names the profile fills in.
pub fn check_invoke(
    manifest: &Manifest,
    action: &str,
    params: &BTreeMap<String, PluginValue>,
    variables: &[String],
) -> Result<(), String> {
    let declared = manifest.action(action).ok_or_else(|| {
        format!(
            "{} {} has no action {action}",
            manifest.name, manifest.version
        )
    })?;
    for (name, value) in params {
        let param = declared
            .params
            .iter()
            .find(|param| &param.id == name)
            .ok_or_else(|| format!("{action} has no parameter {name}"))?;
        let fits = param.accepts(value);
        if !fits {
            return Err(format!(
                "parameter {name} of {action} is not a value the plugin accepts"
            ));
        }
    }
    if let Some(missing) = declared
        .params
        .iter()
        .find(|param| param.required && !params.contains_key(&param.id))
    {
        return Err(format!("{action} needs parameter {}", missing.id));
    }
    for name in variables {
        let allowed = declared
            .params
            .iter()
            .any(|param| &param.id == name && param.takes_variables());
        if !allowed {
            return Err(format!(
                "parameter {name} of {action} does not take variables"
            ));
        }
    }
    Ok(())
}

/// One plugin folder as the app sees it: its manifest, or why it cannot be used.
#[derive(Clone, Debug, Serialize)]
pub struct Installed {
    /// The folder name.
    pub folder: String,
    pub manifest: Option<Manifest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Refuses a path that another user owns or that others may write.
#[cfg(unix)]
fn check_owner(path: &Path, what: &str) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let metadata =
        fs::metadata(path).map_err(|error| format!("could not read the {what}: {error}"))?;
    // SAFETY: getuid has no preconditions and cannot fail.
    if metadata.uid() != unsafe { libc::getuid() } {
        return Err(format!("the {what} belongs to another user"));
    }
    if metadata.mode() & 0o022 != 0 {
        return Err(format!(
            "the {what} can be changed by other users (chmod go-w)"
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_owner(_: &Path, _: &str) -> Result<(), String> {
    Ok(())
}

/// Reads and checks the manifest of the plugin folder `folder`, whose name must be the manifest's id.
pub fn load(folder: &Path) -> Result<Manifest, String> {
    check_owner(folder, "plugin folder")?;
    let file = folder.join(MANIFEST_FILE);
    check_owner(&file, MANIFEST_FILE)?;
    let size = fs::metadata(&file)
        .map_err(|error| format!("could not read {MANIFEST_FILE}: {error}"))?
        .len();
    if size > MAX_MANIFEST_BYTES {
        return Err(format!("{MANIFEST_FILE} is larger than 64 KiB"));
    }
    let bytes =
        fs::read(&file).map_err(|error| format!("could not read {MANIFEST_FILE}: {error}"))?;
    let manifest: Manifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{MANIFEST_FILE} is not a valid manifest: {error}"))?;
    manifest.validate()?;
    let name = folder
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    if name != manifest.id {
        return Err(format!(
            "the folder is named {name}, but the manifest's id is {}; rename the folder to its id",
            manifest.id
        ));
    }
    Ok(manifest)
}

/// Every folder in the plugins folder, sorted by name, each with its manifest or the reason it is unusable. A missing
/// plugins folder is an empty list. Hidden entries and plain files are skipped.
pub fn catalog(plugins: &Path) -> Vec<Installed> {
    let Ok(entries) = fs::read_dir(plugins) else {
        return Vec::new();
    };
    let mut found: Vec<Installed> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let folder = entry.file_name().to_str()?.to_string();
            if folder.starts_with('.') {
                return None;
            }
            let path = entry.path();
            let (manifest, error) = match load(&path) {
                Ok(manifest) => (Some(manifest), None),
                Err(error) => (None, Some(error)),
            };
            Some(Installed {
                folder,
                manifest,
                error,
            })
        })
        .collect();
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// What Settings lists: the plugins folder and every plugin in it, with the host's view of the ones that run.
#[derive(Debug, Serialize)]
pub struct Listing {
    pub folder: String,
    pub plugins: Vec<Listed>,
}

#[derive(Debug, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub installed: Installed,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::plugin_host::Status>,
}

/// The catalog of `<app data>/plugins`, each entry with its running state when the host has one. A plugin a running
/// profile wants but whose folder is missing is listed too, with the host's error.
pub fn listing(app_data: &Path, statuses: Vec<crate::plugin_host::Status>) -> Listing {
    let folder = dir(app_data);
    let mut statuses: BTreeMap<String, crate::plugin_host::Status> = statuses
        .into_iter()
        .map(|status| (status.id.clone(), status))
        .collect();
    let mut plugins: Vec<Listed> = catalog(&folder)
        .into_iter()
        .map(|installed| {
            let status = statuses.remove(&installed.folder);
            Listed { installed, status }
        })
        .collect();
    plugins.extend(statuses.into_values().map(|status| Listed {
        installed: Installed {
            folder: status.id.clone(),
            manifest: None,
            error: status.error.clone(),
        },
        status: Some(status),
    }));
    Listing {
        folder: folder.display().to_string(),
        plugins,
    }
}

/// Opens `<app data>/plugins/` in the Finder, creating it (0700) when missing.
#[cfg(target_os = "macos")]
pub fn open(app_data: &Path) -> Result<(), String> {
    let folder = dir(app_data);
    crate::ingress::prepare_private_dir(&folder, "plugins", "plugins cannot run")?;
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-a", "Finder"])
        .arg(&folder)
        .status()
        .map_err(|_| "Could not open the plugins folder.".to_string())?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| "Could not open the plugins folder.".to_string())
}

#[cfg(not(target_os = "macos"))]
pub fn open(_: &Path) -> Result<(), String> {
    Err("Opening the plugins folder works only on macOS.".into())
}

/// Calls `each` on every action of `profile` and every step inside one.
pub fn each_action<'a>(
    profile: &'a cooltek_sdk_service::Profile,
    each: &mut impl FnMut(&'a Action),
) {
    fn walk<'a>(action: &'a Action, each: &mut impl FnMut(&'a Action)) {
        each(action);
        match action {
            Action::Conditional {
                then, otherwise, ..
            } => then
                .iter()
                .chain(otherwise)
                .for_each(|step| walk(step, each)),
            Action::MultiAction { actions } => actions.iter().for_each(|step| walk(step, each)),
            _ => {}
        }
    }
    for binding in &profile.bindings {
        walk(&binding.action, each);
    }
    if let Some(keypad) = &profile.keypad {
        for page in &keypad.pages {
            for tile in page.slots.iter().flatten() {
                if let cooltek_sdk_service::KeypadTile::Action { bindings, .. } = tile {
                    for binding in bindings {
                        walk(&binding.action, each);
                    }
                }
            }
        }
    }
    for function in profile.dial_functions.iter().flatten() {
        for binding in &function.bindings {
            walk(&binding.action, each);
        }
    }
}

/// The ids of the plugins `profile` uses anywhere.
pub fn plugins_used(profile: &cooltek_sdk_service::Profile) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    each_action(profile, &mut |action| {
        if let Action::Plugin { plugin, .. } = action {
            used.insert(plugin.clone());
        }
    });
    used
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn manifest_json() -> serde_json::Value {
        json!({
            "id": "com.example.lights", "name": "Lights", "version": "1.2.0",
            "command": ["bin/lights", "--quiet"],
            "actions": [
                { "id": "toggle", "label": "Toggle" },
                { "id": "set", "label": "Set a light", "params": [
                    { "id": "entity", "label": "Light", "type": "text", "variables": true, "required": true },
                    { "id": "token", "label": "Token", "type": "text", "secret": true },
                    { "id": "level", "label": "Level", "type": "number", "min": 0, "max": 100, "default": 50 },
                    { "id": "fade", "label": "Fade", "type": "bool" },
                    { "id": "mode", "label": "Mode", "type": "choice", "choices": [
                        { "id": "warm", "label": "Warm" }, { "id": "cold", "label": "Cold" }
                    ], "default": "warm" }
                ] }
            ],
            "permissions": { "network": ["homeassistant.local:8123"], "variables": ["room"] }
        })
    }

    fn manifest() -> Manifest {
        serde_json::from_value(manifest_json()).unwrap()
    }

    fn with(change: impl FnOnce(&mut serde_json::Value)) -> Result<(), String> {
        let mut value = manifest_json();
        change(&mut value);
        serde_json::from_value::<Manifest>(value)
            .map_err(|error| error.to_string())?
            .validate()
    }

    #[test]
    fn a_manifest_is_checked_field_by_field() {
        manifest().validate().unwrap();
        assert!(with(|m| m["id"] = json!("lights")).is_err());
        assert!(with(|m| m["version"] = json!("1.2")).is_err());
        assert!(with(|m| m["command"] = json!([])).is_err());
        assert!(with(|m| m["command"] = json!(["../escape"])).is_err());
        assert!(with(|m| m["command"] = json!(["bin/../../escape"])).is_err());
        assert!(with(|m| m["command"] = json!(["/usr/local/bin/node", "index.js"])).is_ok());
        assert!(with(|m| m["actions"] = json!([])).is_err());
        assert!(with(|m| m["actions"][1]["id"] = json!("toggle")).is_err());
        assert!(with(|m| m["actions"][1]["params"][1]["id"] = json!("entity")).is_err());
        assert!(with(|m| m["actions"][1]["params"][2]["min"] = json!(200)).is_err());
        assert!(with(|m| m["actions"][1]["params"][2]["default"] = json!(150)).is_err());
        assert!(with(|m| m["actions"][1]["params"][4]["default"] = json!("hot")).is_err());
        assert!(with(|m| m["actions"][1]["params"][4]["choices"] = json!([])).is_err());
        assert!(with(|m| m["actions"][1]["params"][3]["variables"] = json!(true)).is_err());
        assert!(with(|m| m["actions"][1]["params"][0]["min"] = json!(1)).is_err());
        assert!(
            with(|m| m["actions"][1]["params"][3]["choices"] = json!([{"id":"a","label":"A"}]))
                .is_err()
        );
        assert!(with(|m| m["actions"][1]["params"][3]["default"] = json!("yes")).is_err());
        assert!(with(|m| m["actions"][1]["params"][3]["varaibles"] = json!(true)).is_err());
        assert!(with(|m| m["actions"][0]["label"] = json!("a\nb")).is_err());
        assert!(with(|m| m["permissions"]["network"] = json!(["has space"])).is_err());
        assert!(with(|m| m["permissions"]["variables"] = json!(["Bad"])).is_err());
        assert!(with(|m| m["unknown"] = json!(1)).is_err());
    }

    #[test]
    fn an_invoke_must_fit_the_manifest() {
        let manifest = manifest();
        let params = |value: serde_json::Value| -> BTreeMap<String, PluginValue> {
            serde_json::from_value(value).unwrap()
        };
        let ok = params(json!({"entity":"light.{{room}}","level":40,"fade":true,"mode":"cold"}));
        check_invoke(&manifest, "set", &ok, &["entity".into()]).unwrap();
        check_invoke(&manifest, "toggle", &BTreeMap::new(), &[]).unwrap();
        let refused = |action: &str, value: serde_json::Value, variables: &[&str]| {
            let variables: Vec<String> = variables.iter().map(|name| name.to_string()).collect();
            check_invoke(&manifest, action, &params(value), &variables).unwrap_err()
        };
        assert!(refused("dim", json!({}), &[]).contains("no action dim"));
        assert!(refused("set", json!({"level":4}), &[]).contains("needs parameter entity"));
        assert!(
            refused("set", json!({"entity":"a","bogus":1}), &[]).contains("no parameter bogus")
        );
        assert!(refused("set", json!({"entity":"a","level":101}), &[]).contains("level"));
        assert!(refused("set", json!({"entity":"a","level":"40"}), &[]).contains("level"));
        assert!(refused("set", json!({"entity":"a","mode":"hot"}), &[]).contains("mode"));
        assert!(refused("set", json!({"entity":true}), &[]).contains("entity"));
        assert!(
            refused("set", json!({"entity":"a","token":"{{x}}"}), &["token"])
                .contains("does not take variables")
        );
    }

    #[test]
    fn versions_compare_numerically() {
        let manifest = manifest();
        assert!(manifest.satisfies("1.2.0"));
        assert!(manifest.satisfies("1.1.9"));
        assert!(!manifest.satisfies("1.10.0"));
        assert!(!manifest.satisfies("bogus"));
    }

    #[cfg(unix)]
    #[test]
    fn the_catalog_lists_folders_with_their_manifest_or_their_problem() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let plugins = root.path();
        let good = plugins.join("com.example.lights");
        fs::create_dir(&good).unwrap();
        fs::write(good.join(MANIFEST_FILE), manifest_json().to_string()).unwrap();
        let misnamed = plugins.join("lights");
        fs::create_dir(&misnamed).unwrap();
        fs::write(misnamed.join(MANIFEST_FILE), manifest_json().to_string()).unwrap();
        let open = plugins.join("com.example.open");
        fs::create_dir(&open).unwrap();
        let mut value = manifest_json();
        value["id"] = json!("com.example.open");
        fs::write(open.join(MANIFEST_FILE), value.to_string()).unwrap();
        fs::set_permissions(open.join(MANIFEST_FILE), fs::Permissions::from_mode(0o666)).unwrap();
        fs::create_dir(plugins.join(".hidden")).unwrap();
        fs::write(plugins.join("stray.json"), "{}").unwrap();
        let found = catalog(plugins);
        let folders: Vec<&str> = found.iter().map(|item| item.folder.as_str()).collect();
        assert_eq!(
            folders,
            ["com.example.lights", "com.example.open", "lights"]
        );
        assert_eq!(found[0].manifest.as_ref().unwrap().id, "com.example.lights");
        assert!(found[1].error.as_ref().unwrap().contains("other users"));
        assert!(
            found[2]
                .error
                .as_ref()
                .unwrap()
                .contains("rename the folder")
        );
        assert!(catalog(&plugins.join("missing")).is_empty());
    }

    #[test]
    fn plugins_are_found_in_steps_and_on_every_surface() {
        let profile: cooltek_sdk_service::Profile = serde_json::from_value(json!({
            "schema_version": 5,
            "name": "p",
            "bindings": [{ "module":"m003k", "control":0, "gesture":"pressed", "action": {
                "type":"multi_action", "actions":[
                    {"type":"toggle_mute"},
                    {"type":"plugin","plugin":"com.example.a","min_version":"1.0.0","action":"x"}
                ]
            }}],
        }))
        .unwrap();
        assert_eq!(
            plugins_used(&profile).into_iter().collect::<Vec<_>>(),
            ["com.example.a"]
        );
    }
}
