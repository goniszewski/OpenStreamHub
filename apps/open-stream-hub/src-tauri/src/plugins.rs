//! Installed plugins: their manifests, and the checks an invoke passes before it reaches a plugin.
//!
//! A plugin is a folder `<app data>/plugins/<id>/` holding `plugin.json` (see `docs/plugins-design.md`). The folder
//! name must be the manifest's id, so two folders can never claim one id. The folder and the manifest must belong to
//! this user and be writable by nobody else, since the manifest names the program the app starts.
//!
//! ```json
//! {
//!   "schema": 1, "id": "com.example.lights", "name": "Lights", "version": "1.2.0",
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
//!
//! `schema` is the manifest format's version. This app reads [`SCHEMA`] only; a change that would make an older app
//! misread a manifest gets a new number, so an older app refuses it with a clear reason instead of guessing.
//!
//! A plugin runs only once the user has enabled it, after a dialog that lists what its manifest declares. The
//! approval is the [`fingerprint`] of the exact `plugin.json` bytes they saw, kept in `<app data>/`[`APPROVALS_FILE`]
//! (outside the plugins folder, so a plugin cannot approve itself), and any change to the file needs a new approval.

use cooltek_sdk_service::{
    Action, PluginValue,
    plugin::{parse_version, validate_plugin_action_id, validate_plugin_id},
    validate_variable_name,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    sync::Mutex,
};

/// The manifest's file name inside a plugin folder.
pub const MANIFEST_FILE: &str = "plugin.json";
/// The manifest schema this app reads.
pub const SCHEMA: u64 = 1;
/// The approvals file in the app data folder: `{ "<plugin id>": "<fingerprint>" }`.
pub const APPROVALS_FILE: &str = "plugin-approvals.json";
/// The largest approvals file read.
const MAX_APPROVALS_BYTES: usize = 256 * 1024;
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
    /// The manifest format's version; always [`SCHEMA`] once parsed.
    pub schema: u64,
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
    /// Text only: kept in the Keychain, never in the profile, and never logged.
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
        if self.secret && self.default.is_some() {
            return Err(at(
                "is secret, so it has no default: its value is set in Settings > Plugins",
            ));
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
                self.check_text(value)
                    .map_err(|()| at("default must be one line of at most 2048 bytes"))?;
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

    /// A text parameter whose value lives in the Keychain.
    pub fn is_secret(&self) -> bool {
        self.kind == ParamType::Text && self.secret
    }

    /// Whether `value` passes the rules every text parameter value follows (one line without control or invisible
    /// characters, at most 2048 bytes), by the service's own check.
    fn check_text(&self, value: &str) -> Result<(), ()> {
        let mut probe = BTreeMap::new();
        probe.insert(self.id.clone(), PluginValue::Text(value.to_string()));
        cooltek_sdk_service::plugin::validate_plugin_action(
            "com.example.probe",
            "0.0.0",
            "probe",
            &probe,
            &[],
        )
        .map(drop)
        .map_err(drop)
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
        check_schema(Some(self.schema))?;
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

    /// Each secret parameter once by id, as the first action that declares it names it. Every action with a secret
    /// of that id shares one stored value.
    pub fn secret_params(&self) -> Vec<&Param> {
        let mut seen = BTreeSet::new();
        self.actions
            .iter()
            .flat_map(|action| &action.params)
            .filter(|param| param.is_secret() && seen.insert(param.id.as_str()))
            .collect()
    }

    /// Checks a value to store for the secret parameter `param`: it must be one some action declares, and the value
    /// must follow the rules of every text parameter. The error never contains the value.
    pub fn check_secret(&self, param: &str, value: &str) -> Result<(), String> {
        let declared = self
            .secret_params()
            .into_iter()
            .find(|declared| declared.id == param)
            .ok_or_else(|| format!("{} has no secret parameter {param}", self.name))?;
        if value.is_empty() {
            return Err(format!(
                "enter a value for {}, or forget it instead",
                declared.label
            ));
        }
        declared.check_text(value).map_err(|()| {
            format!(
                "{} must be one line of at most 2048 bytes without control or invisible characters",
                declared.label
            )
        })
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

/// Refuses parameters from a profile that hold a value for one of `action`'s secrets: those come only from the
/// Keychain.
pub fn refuse_profile_secrets(
    manifest: &Manifest,
    action: &str,
    params: &BTreeMap<String, PluginValue>,
) -> Result<(), String> {
    let Some(declared) = manifest.action(action) else {
        return Ok(());
    };
    match declared
        .params
        .iter()
        .find(|param| param.is_secret() && params.contains_key(&param.id))
    {
        Some(param) => Err(format!(
            "{} is a secret and is kept in the Keychain, not the profile; open the key and save it again",
            param.label
        )),
        None => Ok(()),
    }
}

/// Removes from `profile` every value of a parameter the installed manifest marks secret (a profile saved before
/// secrets moved to the Keychain may hold one), so an export never carries one, and names what was removed. A plugin
/// that is not installed cannot be checked, so its parameters stay as they are.
pub fn strip_secrets(profile: &mut cooltek_sdk_service::Profile, app_data: &Path) -> Vec<String> {
    let mut manifests: BTreeMap<String, Option<Manifest>> = BTreeMap::new();
    let mut dropped = BTreeSet::new();
    each_action_mut(profile, &mut |action| {
        let Action::Plugin {
            plugin,
            action,
            params,
            variables,
            ..
        } = action
        else {
            return;
        };
        let manifest = manifests.entry(plugin.clone()).or_insert_with(|| {
            installed(app_data, plugin)
                .ok()
                .map(|(manifest, _)| manifest)
        });
        let Some(declared) = manifest
            .as_ref()
            .and_then(|manifest| manifest.action(action))
        else {
            return;
        };
        for param in declared.params.iter().filter(|param| param.is_secret()) {
            if params.remove(&param.id).is_some() {
                variables.retain(|name| name != &param.id);
                dropped.insert(format!("{} ({plugin})", param.label));
            }
        }
    });
    dropped.into_iter().collect()
}

/// Fills the stored secrets into an invoke's parameters, before [`check_invoke`]: for each secret parameter of
/// `action`, `stored` gives its value from the Keychain. A profile that carries a value for a secret is refused, so a
/// secret saved before secrets moved to the Keychain is not sent from the profile. An unknown action is left to
/// [`check_invoke`] to report.
pub fn fill_secrets(
    manifest: &Manifest,
    action: &str,
    params: &mut BTreeMap<String, PluginValue>,
    mut stored: impl FnMut(&Param) -> Result<Option<String>, String>,
) -> Result<(), String> {
    refuse_profile_secrets(manifest, action, params)?;
    let Some(declared) = manifest.action(action) else {
        return Ok(());
    };
    for param in declared.params.iter().filter(|param| param.is_secret()) {
        let value = stored(param).map_err(|error| {
            format!(
                "{} for {} could not be read: {error}",
                param.label, manifest.name
            )
        })?;
        match value {
            Some(value) => {
                params.insert(param.id.clone(), PluginValue::Text(value));
            }
            None if param.required => {
                return Err(format!(
                    "{} for {} is not set; set it in Settings > Plugins",
                    param.label, manifest.name
                ));
            }
            None => {}
        }
    }
    Ok(())
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
    /// The [`fingerprint`] of the manifest's bytes, with the manifest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
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

/// Refuses a schema this app does not read, with what to do about it.
fn check_schema(schema: Option<u64>) -> Result<(), String> {
    match schema {
        Some(SCHEMA) => Ok(()),
        None => Err(format!(
            "{MANIFEST_FILE} has no \"schema\"; add \"schema\": {SCHEMA}"
        )),
        Some(schema) if schema > SCHEMA => Err(format!(
            "{MANIFEST_FILE} is schema {schema}; this OpenStreamHub reads schema {SCHEMA}, update the app"
        )),
        Some(schema) => Err(format!(
            "{MANIFEST_FILE} has \"schema\" {schema}; it must be {SCHEMA}"
        )),
    }
}

/// Parses manifest bytes. The schema is read first, from the untyped JSON, so a newer schema with fields this app
/// does not know is reported as newer, not as an unknown field.
pub fn parse(bytes: &[u8]) -> Result<Manifest, String> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("{MANIFEST_FILE} is not valid JSON: {error}"))?;
    if !value.is_object() {
        return Err(format!("{MANIFEST_FILE} must hold one JSON object"));
    }
    match value.get("schema") {
        None => check_schema(None)?,
        Some(schema) => match schema.as_u64() {
            Some(number) if number > 0 => check_schema(Some(number))?,
            _ => {
                return Err(format!(
                    "{MANIFEST_FILE} has \"schema\" {schema}; it must be the whole number {SCHEMA}"
                ));
            }
        },
    }
    let manifest: Manifest = serde_json::from_value(value)
        .map_err(|error| format!("{MANIFEST_FILE} is not a valid manifest: {error}"))?;
    manifest.validate()?;
    Ok(manifest)
}

/// What an approval is of: the SHA-256 of the exact manifest bytes, in hex. Any change to the file changes it.
pub fn fingerprint(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Reads and checks the manifest of the plugin folder `folder`, whose name must be the manifest's id, with the
/// [`fingerprint`] of the bytes it was parsed from.
pub fn load(folder: &Path) -> Result<(Manifest, String), String> {
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
    let manifest = parse(&bytes)?;
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
    Ok((manifest, fingerprint(&bytes)))
}

/// The installed plugin `id` in `<app data>/plugins`, with its fingerprint, or why it cannot be used. The id is
/// checked first, so it can never name a path outside the plugins folder.
pub fn installed(app_data: &Path, id: &str) -> Result<(Manifest, String), String> {
    validate_plugin_id(id)?;
    load(&dir(app_data).join(id)).map_err(|error| format!("{id} cannot be used: {error}"))
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
            let (manifest, fingerprint, error) = match load(&path) {
                Ok((manifest, fingerprint)) => (Some(manifest), Some(fingerprint), None),
                Err(error) => (None, None, Some(error)),
            };
            Some(Installed {
                folder,
                manifest,
                fingerprint,
                error,
            })
        })
        .collect();
    found.sort_by(|a, b| a.folder.cmp(&b.folder));
    found
}

/// Whether the user has enabled a plugin: `enabled` when the approved fingerprint is the manifest's current one,
/// `changed` when `plugin.json` changed since, `not_enabled` when it was never enabled (or was disabled).
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    Enabled,
    Changed,
    NotEnabled,
}

impl Approval {
    pub fn of(approved: &Approved, id: &str, fingerprint: &str) -> Self {
        match approved.get(id) {
            Some(approved) if approved == fingerprint => Self::Enabled,
            Some(_) => Self::Changed,
            None => Self::NotEnabled,
        }
    }

    /// Why a plugin in this state does not run, or `None` when it may.
    pub fn refusal(self) -> Option<&'static str> {
        match self {
            Self::Enabled => None,
            Self::Changed => {
                Some("plugin.json changed since it was enabled: review it in Settings > Plugins")
            }
            Self::NotEnabled => Some("not enabled: enable it in Settings > Plugins"),
        }
    }
}

/// The approved fingerprint of each enabled plugin, by id.
pub type Approved = BTreeMap<String, String>;

/// The approvals file, `<app data>/`[`APPROVALS_FILE`], written atomically and 0600. The lock serializes
/// read-modify-write.
pub struct Approvals {
    directory: PathBuf,
    lock: Mutex<()>,
}

impl Approvals {
    pub fn new(app_data: &Path) -> Self {
        Self {
            directory: app_data.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join(APPROVALS_FILE)
    }

    fn load(&self) -> Result<Approved, String> {
        crate::store::read_private_file(&self.path(), MAX_APPROVALS_BYTES, "plugin approvals file")
            .and_then(|found| match found {
                None => Ok(Approved::new()),
                Some((bytes, _)) => serde_json::from_slice::<Approved>(&bytes)
                    .map_err(|error| format!("the plugin approvals file is damaged: {error}")),
            })
    }

    /// What is approved. A missing file approves nothing; so does a damaged one, which is logged and set aside at the
    /// next change, so a broken file can never enable a plugin.
    pub fn read(&self) -> Approved {
        self.load().unwrap_or_else(|error| {
            eprintln!("{error}; no plugin is enabled until one is enabled again");
            Approved::new()
        })
    }

    /// Approves `fingerprint` for `id`, or with `None` removes its approval, then calls `then` with what is approved
    /// now, still under the lock, so whoever acts on approvals (the plugin host) sees changes in the order the file
    /// does. A damaged file is renamed to `plugin-approvals.json.damaged` first and a new one is written.
    pub fn set(
        &self,
        id: &str,
        fingerprint: Option<&str>,
        then: impl FnOnce(&Approved),
    ) -> Result<Approved, String> {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut approved = match self.load() {
            Ok(approved) => approved,
            Err(error) => {
                let aside = self.directory.join(format!("{APPROVALS_FILE}.damaged"));
                match fs::rename(self.path(), &aside) {
                    Ok(()) => eprintln!("{error}; kept it as {}", aside.display()),
                    Err(rename) => eprintln!("{error}; could not set it aside: {rename}"),
                }
                Approved::new()
            }
        };
        match fingerprint {
            Some(fingerprint) => approved.insert(id.to_string(), fingerprint.to_string()),
            None => approved.remove(id),
        };
        let bytes = serde_json::to_vec_pretty(&approved)
            .map_err(|error| format!("could not encode the plugin approvals: {error}"))?;
        crate::store::write_private_file(
            &self.directory,
            &self.path(),
            &bytes,
            "plugin approvals file",
        )?;
        then(&approved);
        Ok(approved)
    }
}

/// Enables the installed plugin `id` for `fingerprint`, the one the first-enable dialog showed. When `plugin.json`
/// changed since, nothing is enabled, so the user approves exactly what they saw. `notify` gets the approved
/// fingerprint under the approvals lock.
pub fn enable(
    app_data: &Path,
    approvals: &Approvals,
    id: &str,
    fingerprint: &str,
    notify: impl FnOnce(String),
) -> Result<(), String> {
    let (_, current) = installed(app_data, id)?;
    if current != fingerprint {
        return Err("plugin.json changed since you reviewed it; review it again".into());
    }
    approvals.set(id, Some(&current), |_| notify(current.clone()))?;
    Ok(())
}

/// Removes the approval of `id`; `notify` runs under the approvals lock.
pub fn disable(approvals: &Approvals, id: &str, notify: impl FnOnce()) -> Result<(), String> {
    validate_plugin_id(id)?;
    approvals.set(id, None, |_| notify())?;
    Ok(())
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
    /// With a usable manifest: whether it is enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<Approval>,
    /// Its secret parameters and whether each holds a value; never the value.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<SecretStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<crate::plugin_host::Status>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SecretStatus {
    pub id: String,
    pub label: String,
    /// `None` when the Keychain could not tell.
    pub set: Option<bool>,
}

/// The catalog of `<app data>/plugins`, each entry with its approval, its secrets (`is_set` tells whether one holds a
/// value, `None` when that cannot be told) and its running state when the host has one. A plugin a running profile wants but whose folder is missing
/// is listed too, with the host's error.
pub fn listing(
    app_data: &Path,
    statuses: Vec<crate::plugin_host::Status>,
    approved: &Approved,
    is_set: &dyn Fn(&str, &str) -> Option<bool>,
) -> Listing {
    let folder = dir(app_data);
    let mut statuses: BTreeMap<String, crate::plugin_host::Status> = statuses
        .into_iter()
        .map(|status| (status.id.clone(), status))
        .collect();
    let mut plugins: Vec<Listed> = catalog(&folder)
        .into_iter()
        .map(|installed| {
            let status = statuses.remove(&installed.folder);
            let (approval, secrets) = match (&installed.manifest, &installed.fingerprint) {
                (Some(manifest), Some(fingerprint)) => (
                    Some(Approval::of(approved, &manifest.id, fingerprint)),
                    manifest
                        .secret_params()
                        .into_iter()
                        .map(|param| SecretStatus {
                            id: param.id.clone(),
                            label: param.label.clone(),
                            set: is_set(&manifest.id, &param.id),
                        })
                        .collect(),
                ),
                _ => (None, Vec::new()),
            };
            Listed {
                installed,
                approval,
                secrets,
                status,
            }
        })
        .collect();
    plugins.extend(statuses.into_values().map(|status| Listed {
        installed: Installed {
            folder: status.id.clone(),
            manifest: None,
            fingerprint: None,
            error: status.error.clone(),
        },
        approval: None,
        secrets: Vec::new(),
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

/// Calls `each` on every action of `profile` and every step inside one, able to change it.
pub fn each_action_mut(
    profile: &mut cooltek_sdk_service::Profile,
    each: &mut impl FnMut(&mut Action),
) {
    fn walk(action: &mut Action, each: &mut impl FnMut(&mut Action)) {
        each(action);
        match action {
            Action::Conditional {
                then, otherwise, ..
            } => then
                .iter_mut()
                .chain(otherwise.iter_mut())
                .for_each(|step| walk(step, each)),
            Action::MultiAction { actions } => actions.iter_mut().for_each(|step| walk(step, each)),
            _ => {}
        }
    }
    for binding in &mut profile.bindings {
        walk(&mut binding.action, each);
    }
    if let Some(keypad) = &mut profile.keypad {
        for page in &mut keypad.pages {
            for tile in page.slots.iter_mut().flatten() {
                if let cooltek_sdk_service::KeypadTile::Action { bindings, .. } = tile {
                    for binding in bindings {
                        walk(&mut binding.action, each);
                    }
                }
            }
        }
    }
    for function in profile.dial_functions.iter_mut().flatten() {
        for binding in &mut function.bindings {
            walk(&mut binding.action, each);
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
            "schema": 1, "id": "com.example.lights", "name": "Lights", "version": "1.2.0",
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
        assert!(with(|m| m["actions"][1]["params"][1]["default"] = json!("x")).is_err());
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
    fn the_schema_is_checked_before_the_fields() {
        let parsed = |change: &dyn Fn(&mut serde_json::Value)| {
            let mut value = manifest_json();
            change(&mut value);
            parse(value.to_string().as_bytes())
        };
        assert_eq!(parsed(&|_| {}).unwrap().schema, 1);
        assert_eq!(
            parsed(&|m| {
                m.as_object_mut().unwrap().remove("schema");
            })
            .unwrap_err(),
            "plugin.json has no \"schema\"; add \"schema\": 1"
        );
        // A newer schema with a field this app does not know is reported as newer, not as an unknown field.
        assert_eq!(
            parsed(&|m| {
                m["schema"] = json!(2);
                m["background"] = json!(true);
            })
            .unwrap_err(),
            "plugin.json is schema 2; this OpenStreamHub reads schema 1, update the app"
        );
        for bad in [json!(0), json!("1"), json!(1.5), json!(-1)] {
            let error = parsed(&|m| m["schema"] = bad.clone()).unwrap_err();
            assert!(error.contains("must be the whole number 1"), "{error}");
        }
        assert!(
            parse(b"[1]")
                .unwrap_err()
                .contains("must hold one JSON object")
        );
        assert!(with(|m| m["schema"] = json!(3)).is_err());
    }

    #[test]
    fn any_change_to_the_manifest_bytes_changes_its_fingerprint() {
        let bytes = manifest_json().to_string().into_bytes();
        let fingerprint = fingerprint(&bytes);
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.chars().all(|c| c.is_ascii_hexdigit()));
        let mut spaced = bytes.clone();
        spaced.push(b' ');
        assert_ne!(super::fingerprint(&spaced), fingerprint);
        let mut flipped = bytes.clone();
        flipped[10] ^= 1;
        assert_ne!(super::fingerprint(&flipped), fingerprint);
        assert_eq!(super::fingerprint(&bytes), fingerprint);
        let approved: Approved = [("com.example.lights".to_string(), fingerprint.clone())].into();
        assert_eq!(
            Approval::of(&approved, "com.example.lights", &fingerprint),
            Approval::Enabled
        );
        assert_eq!(
            Approval::of(
                &approved,
                "com.example.lights",
                &super::fingerprint(&spaced)
            ),
            Approval::Changed
        );
        assert_eq!(
            Approval::of(&approved, "com.example.other", &fingerprint),
            Approval::NotEnabled
        );
    }

    #[test]
    fn secrets_are_filled_in_and_never_taken_from_the_profile() {
        let mut value = manifest_json();
        value["actions"][1]["params"][1]["required"] = json!(true);
        value["actions"][0]["params"] =
            json!([{ "id": "token", "label": "Other label", "type": "text", "secret": true }]);
        let manifest: Manifest = serde_json::from_value(value).unwrap();
        manifest.validate().unwrap();
        let ids: Vec<(&str, &str)> = manifest
            .secret_params()
            .iter()
            .map(|param| (param.id.as_str(), param.label.as_str()))
            .collect();
        assert_eq!(ids, [("token", "Other label")]);

        let mut params: BTreeMap<String, PluginValue> =
            serde_json::from_value(json!({"entity":"a"})).unwrap();
        fill_secrets(&manifest, "set", &mut params, |_| Ok(Some("s3cret".into()))).unwrap();
        assert_eq!(params["token"], PluginValue::Text("s3cret".into()));
        // The required secret is in once filled in, so check_invoke does not call it missing.
        check_invoke(&manifest, "set", &params, &[]).unwrap();

        let mut params: BTreeMap<String, PluginValue> =
            serde_json::from_value(json!({"entity":"a"})).unwrap();
        assert_eq!(
            fill_secrets(&manifest, "set", &mut params, |_| Ok(None)).unwrap_err(),
            "Token for Lights is not set; set it in Settings > Plugins"
        );
        let mut params: BTreeMap<String, PluginValue> =
            serde_json::from_value(json!({"entity":"a","token":"old"})).unwrap();
        let error =
            fill_secrets(&manifest, "set", &mut params, |_| Ok(Some("s3cret".into()))).unwrap_err();
        assert!(
            error.contains("kept in the Keychain, not the profile"),
            "{error}"
        );
        assert!(!error.contains("old") && !error.contains("s3cret"));
        let error = fill_secrets(&manifest, "set", &mut BTreeMap::new(), |_| {
            Err("could not read it".into())
        })
        .unwrap_err();
        assert_eq!(
            error,
            "Token for Lights could not be read: could not read it"
        );

        manifest.check_secret("token", "abc").unwrap();
        assert!(
            manifest
                .check_secret("entity", "abc")
                .unwrap_err()
                .contains("no secret parameter entity")
        );
        assert!(
            manifest
                .check_secret("token", "")
                .unwrap_err()
                .contains("enter a value")
        );
        let error = manifest.check_secret("token", "a\nhidden").unwrap_err();
        assert!(
            error.contains("one line") && !error.contains("hidden"),
            "{error}"
        );
        assert!(manifest.check_secret("token", &"x".repeat(2049)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn approvals_are_kept_privately_and_a_damaged_file_approves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let approvals = Approvals::new(root.path());
        assert!(approvals.read().is_empty());
        approvals.set("com.example.a", Some("aa"), |_| {}).unwrap();
        let mut seen = None;
        approvals
            .set("com.example.b", Some("bb"), |now| seen = Some(now.len()))
            .unwrap();
        assert_eq!(seen, Some(2));
        let path = root.path().join(APPROVALS_FILE);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            approvals.set("com.example.a", None, |_| {}).unwrap(),
            [("com.example.b".to_string(), "bb".to_string())].into()
        );
        assert_eq!(approvals.read().len(), 1);
        fs::write(&path, "{ not json").unwrap();
        assert!(approvals.read().is_empty());
        approvals.set("com.example.c", Some("cc"), |_| {}).unwrap();
        assert_eq!(approvals.read().len(), 1);
        // The damaged file is set aside, not lost.
        assert_eq!(
            fs::read_to_string(root.path().join(format!("{APPROVALS_FILE}.damaged"))).unwrap(),
            "{ not json"
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
        assert_eq!(
            found[0].fingerprint.as_deref(),
            Some(fingerprint(manifest_json().to_string().as_bytes()).as_str())
        );
        assert!(found[1].fingerprint.is_none());
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

    #[cfg(unix)]
    #[test]
    fn enabling_needs_the_fingerprint_the_user_reviewed() {
        let root = tempfile::tempdir().unwrap();
        let folder = dir(root.path()).join("com.example.lights");
        fs::create_dir_all(&folder).unwrap();
        let reviewed = manifest_json().to_string();
        fs::write(folder.join(MANIFEST_FILE), &reviewed).unwrap();
        let approvals = Approvals::new(root.path());
        let id = "com.example.lights";
        // plugin.json changes after the dialog opened.
        fs::write(folder.join(MANIFEST_FILE), format!("{reviewed} ")).unwrap();
        let mut told = None;
        let error = enable(
            root.path(),
            &approvals,
            id,
            &fingerprint(reviewed.as_bytes()),
            |fingerprint| told = Some(fingerprint),
        )
        .unwrap_err();
        assert_eq!(
            error,
            "plugin.json changed since you reviewed it; review it again"
        );
        assert!(told.is_none());
        assert!(approvals.read().is_empty());
        let current = fingerprint(format!("{reviewed} ").as_bytes());
        enable(root.path(), &approvals, id, &current, |fingerprint| {
            told = Some(fingerprint)
        })
        .unwrap();
        assert_eq!(told.as_deref(), Some(current.as_str()));
        assert_eq!(approvals.read()[id], current);
        let mut disabled = false;
        disable(&approvals, id, || disabled = true).unwrap();
        assert!(disabled && approvals.read().is_empty());
        assert!(enable(root.path(), &approvals, "../x", &current, |_| {}).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_export_drops_the_secrets_an_installed_manifest_declares() {
        let root = tempfile::tempdir().unwrap();
        let folder = dir(root.path()).join("com.example.lights");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join(MANIFEST_FILE), manifest_json().to_string()).unwrap();
        let mut profile: cooltek_sdk_service::Profile = serde_json::from_value(json!({
            "schema_version": 5,
            "name": "p",
            "bindings": [
                { "module":"m003k", "control":0, "gesture":"pressed", "action": {
                    "type":"multi_action", "actions":[
                        {"type":"plugin","plugin":"com.example.lights","min_version":"1.0.0","action":"set",
                         "params":{"entity":"a","token":"old"}}
                    ]
                }},
                { "module":"m003k", "control":1, "gesture":"pressed", "action":
                    {"type":"plugin","plugin":"com.example.gone","min_version":"1.0.0","action":"set",
                     "params":{"token":"kept"}}
                }
            ],
        }))
        .unwrap();
        assert_eq!(
            strip_secrets(&mut profile, root.path()),
            ["Token (com.example.lights)"]
        );
        let text = serde_json::to_string(&profile).unwrap();
        assert!(!text.contains("\"old\""), "{text}");
        assert!(text.contains("\"entity\":\"a\""));
        // A plugin that is not installed cannot be checked.
        assert!(text.contains("\"kept\""));
    }

    #[cfg(unix)]
    #[test]
    fn the_listing_says_whether_each_plugin_is_enabled_and_which_secrets_are_set() {
        let root = tempfile::tempdir().unwrap();
        let folder = dir(root.path()).join("com.example.lights");
        fs::create_dir_all(&folder).unwrap();
        let bytes = manifest_json().to_string();
        fs::write(folder.join(MANIFEST_FILE), &bytes).unwrap();
        let listed = |approved: Approved| {
            let listing = listing(root.path(), Vec::new(), &approved, &|plugin, param| {
                (plugin == "com.example.lights" && param == "token").then_some(true)
            });
            let plugin = listing.plugins.into_iter().next().unwrap();
            (plugin.approval, plugin.secrets)
        };
        let (approval, secrets) = listed(Approved::new());
        assert_eq!(approval, Some(Approval::NotEnabled));
        assert_eq!(
            secrets,
            [SecretStatus {
                id: "token".into(),
                label: "Token".into(),
                set: Some(true)
            }]
        );
        // A Keychain that cannot tell is reported as unknown, not as not set.
        let unknown = listing(root.path(), Vec::new(), &Approved::new(), &|_, _| None);
        assert_eq!(unknown.plugins[0].secrets[0].set, None);
        assert!(
            serde_json::to_string(&unknown)
                .unwrap()
                .contains("\"set\":null")
        );
        let id = "com.example.lights".to_string();
        assert_eq!(
            listed([(id.clone(), fingerprint(bytes.as_bytes()))].into()).0,
            Some(Approval::Enabled)
        );
        assert_eq!(
            listed([(id, "old".into())].into()).0,
            Some(Approval::Changed)
        );
        let (manifest, current) = installed(root.path(), "com.example.lights").unwrap();
        assert_eq!(
            (manifest.id.as_str(), current),
            ("com.example.lights", fingerprint(bytes.as_bytes()))
        );
        assert!(installed(root.path(), "../escape").is_err());
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
