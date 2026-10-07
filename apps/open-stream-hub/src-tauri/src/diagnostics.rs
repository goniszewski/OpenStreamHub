//! Local diagnostic bundle for hardware-verification sessions.
//!
//! `create_diagnostics` writes one pretty JSON file under
//! `<app data>/diagnostics/`. Nothing is uploaded or sent anywhere; the user
//! decides whether to attach the file to a report.
//!
//! Device access is read-only: enumeration, opening an interface, reading its
//! report descriptor, and (optionally) passive input reads. No feature or
//! output report is ever sent and software mode is never enabled. Interfaces
//! held by a running feed or service are not opened at all.

use crate::replay;
use cooltek_sdk_cli::{capture::encode_hex, device::parse_path};
use cooltek_sdk_masterhub::{
    DeviceRegistry, InputReader, ModuleIdentity, ReportTransport, descriptor, hidapi,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const SCHEMA_VERSION: u32 = 1;
/// Largest accepted request, measured as compact JSON.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;
/// Notes longer than this are truncated at a character boundary.
pub const MAX_NOTES_BYTES: usize = 2048;
/// Only the last entries of a longer activity list are kept.
pub const MAX_ACTIVITY: usize = 500;
pub const CAPTURE_SECONDS: std::ops::RangeInclusive<f64> = 1.0..=5.0;
pub const MAX_CAPTURE_REPORTS: usize = 256;
/// Raw input bytes kept per captured device.
pub const MAX_CAPTURE_BYTES: usize = 64 * 1024;
/// Newest bundles kept in the diagnostics folder.
pub const KEEP_BUNDLES: usize = 10;
pub const DIRECTORY: &str = "diagnostics";
/// Same read timeout as the live feed loop.
const READ_TIMEOUT_MS: i32 = 100;
const FILE_PREFIX: &str = "openstreamhub-diagnostics-";
const IN_USE: &str = "in use by feed/service";

/// Requires the key to be present even though its value may be null.
fn required<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsRequest {
    pub redact_serials: bool,
    pub include_capture: bool,
    pub capture_seconds: f64,
    #[serde(deserialize_with = "required")]
    pub include_profile: Option<Value>,
    pub client: ClientContext,
}

/// Recent UI activity; extra keys are kept as given.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientContext {
    pub activity: Vec<Value>,
    #[serde(deserialize_with = "required")]
    pub notes: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Summary {
    pub devices: usize,
    pub descriptors_read: usize,
    pub descriptor_errors: usize,
    pub capture_reports: usize,
    pub redacted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticsResult {
    pub path: String,
    pub bytes: u64,
    pub summary: Summary,
}

/// A request after limits are applied.
#[derive(Clone, Debug)]
pub struct Validated {
    pub redact: bool,
    pub capture: Option<Duration>,
    pub profile: Option<Value>,
    pub client: Value,
    pub notes_truncated: bool,
    pub activity_dropped: usize,
}

pub fn validate(request: DiagnosticsRequest) -> Result<Validated, String> {
    let size = serde_json::to_vec(&request)
        .map_err(|error| format!("could not measure the diagnostics request: {error}"))?
        .len();
    if size > MAX_REQUEST_BYTES {
        return Err(format!(
            "the diagnostics request is {size} bytes; the limit is {MAX_REQUEST_BYTES}"
        ));
    }
    let capture = if request.include_capture {
        let seconds = request.capture_seconds;
        if !seconds.is_finite() || !CAPTURE_SECONDS.contains(&seconds) {
            return Err("capture_seconds must be between 1 and 5".into());
        }
        Some(Duration::from_secs_f64(seconds))
    } else {
        None
    };
    if let Some(profile) = &request.include_profile
        && !profile.is_object()
    {
        return Err("include_profile must be a profile object or null".into());
    }
    let ClientContext {
        mut activity,
        notes,
        extra,
    } = request.client;
    let activity_dropped = activity.len().saturating_sub(MAX_ACTIVITY);
    activity.drain(..activity_dropped);
    let mut notes_truncated = false;
    let notes = notes.map(|mut notes| {
        if notes.len() > MAX_NOTES_BYTES {
            let mut end = MAX_NOTES_BYTES;
            while !notes.is_char_boundary(end) {
                end -= 1;
            }
            notes.truncate(end);
            notes_truncated = true;
        }
        notes
    });
    let mut client = extra;
    client.insert("activity".into(), Value::Array(activity));
    client.insert("notes".into(), notes.map_or(Value::Null, Value::String));
    Ok(Validated {
        redact: request.redact_serials,
        capture,
        profile: request.include_profile,
        client: Value::Object(client),
        notes_truncated,
        activity_dropped,
    })
}

// ---------------------------------------------------------------------------
// Redaction
// ---------------------------------------------------------------------------

/// `…` plus the last four characters, or just `…` when the serial is shorter
/// than five characters.
pub fn redact_serial(serial: &str) -> String {
    let chars: Vec<char> = serial.chars().collect();
    if chars.len() < 5 {
        return "…".into();
    }
    std::iter::once('…')
        .chain(chars[chars.len() - 4..].iter().copied())
        .collect()
}

/// Stable 32-bit FNV-1a of the raw path bytes, as `path-xxxxxxxx`.
pub fn path_label(path: &[u8]) -> String {
    let hash = path.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    format!("path-{hash:08x}")
}

/// Text replacements for one raw device path: its hex (both cases) and, when
/// long enough to be distinctive, its text form.
fn seed_path(path: &[u8], replacements: &mut Vec<(String, String)>) {
    let label = path_label(path);
    let hex = encode_hex(path);
    replacements.push((hex.to_uppercase(), label.clone()));
    replacements.push((hex, label.clone()));
    let text = String::from_utf8_lossy(path).into_owned();
    if text.chars().count() >= 6 {
        replacements.push((text, label));
    }
}

/// Text replacement for one raw serial, when long enough to be distinctive.
fn seed_serial(serial: &str, replacements: &mut Vec<(String, String)>) {
    if serial.chars().count() >= 5 {
        replacements.push((serial.to_owned(), redact_serial(serial)));
    }
}

/// `live:<hex>` / `service:<hex>` ids become `<kind>:path-xxxxxxxx`, or
/// `<kind>:<invalid>` when the hex is not a device path. Other text is None.
fn redact_runtime_id(id: &str, replacements: &mut Vec<(String, String)>) -> Option<String> {
    let (kind @ ("live" | "service"), hex) = id.split_once(':')? else {
        return None;
    };
    Some(match parse_path(hex) {
        Ok(path) => {
            seed_path(&path, replacements);
            format!("{kind}:{}", path_label(&path))
        }
        Err(_) => format!("{kind}:<invalid>"),
    })
}

/// Structural redaction of caller-supplied data, independent of which devices
/// are attached now: `serial_number`/`serial` strings are shortened,
/// `path_hex` becomes `path_label`, and whole-string runtime ids are relabeled.
/// Every raw value found is added to `replacements` for the text scrub.
fn redact_fields(value: &mut Value, replacements: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (key, mut item) in std::mem::take(map) {
                if is_data_key(&key) {
                    map.insert(key, item);
                    continue;
                }
                match (key.as_str(), &mut item) {
                    ("serial_number" | "serial", Value::String(serial)) => {
                        seed_serial(serial, replacements);
                        *serial = redact_serial(serial);
                    }
                    ("path_hex", _) => {
                        let label = match &item {
                            Value::Null => Value::Null,
                            Value::String(hex) => match parse_path(hex) {
                                Ok(path) => {
                                    seed_path(&path, replacements);
                                    Value::String(path_label(&path))
                                }
                                Err(_) => Value::String("<invalid>".into()),
                            },
                            _ => Value::String("<invalid>".into()),
                        };
                        map.insert("path_label".into(), label);
                        continue;
                    }
                    (_, Value::String(text)) => {
                        if let Some(id) = redact_runtime_id(text, replacements) {
                            *text = id;
                        }
                    }
                    _ => redact_fields(&mut item, replacements),
                }
                map.insert(key, item);
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact_fields(item, replacements)),
        Value::String(text) => {
            if let Some(id) = redact_runtime_id(text, replacements) {
                *text = id;
            }
        }
        _ => {}
    }
}

/// Shortest snippet or URL that is also scrubbed from free text (error
/// messages, stderr tails). Shorter values are too likely to match unrelated
/// text, so only their structural field is replaced.
const MIN_SCRUBBED_PAYLOAD_CHARS: usize = 8;

/// `scheme://host[:port]` of a URL, dropping any user information, path,
/// query and fragment; `<url>` when the value has no `scheme://` prefix.
fn url_origin(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "<url>".into();
    };
    // Client JSON is not validated: only a real scheme (RFC 3986 `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`)
    // is kept, so a malformed value cannot leak whatever text precedes "://".
    let mut chars = scheme.chars();
    if !chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return "<url>".into();
    }
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority.rsplit('@').next().unwrap_or_default();
    format!("{scheme}://{host}")
}

/// Replaces user-authored action payloads wherever an action object appears
/// (bindings, `multi_action` steps, keypad tiles, dial functions, client
/// activity): `text` snippets, `timer` labels and `obs_scene` scene names
/// become `<N chars>`, and `open_url` URLs keep only their origin. Long
/// enough raw values are also queued for the text scrub; shorter scene names
/// go to `short_scenes` for the whole-word scrub of profile and client text.
fn redact_action_payloads(
    value: &mut Value,
    replacements: &mut Vec<(String, String)>,
    short_scenes: &mut Vec<(String, String)>,
) {
    match value {
        Value::Object(map) => {
            let field = match map.get("type").and_then(Value::as_str) {
                Some("text") => Some("text"),
                Some("open_url") => Some("url"),
                Some("timer") => Some("label"),
                Some("obs_scene") => Some("scene"),
                _ => None,
            };
            if let Some(field) = field
                && let Some(Value::String(raw)) = map.get_mut(field)
            {
                let redacted = if field == "url" {
                    url_origin(raw)
                } else {
                    format!("<{} chars>", raw.chars().count())
                };
                if raw.chars().count() >= MIN_SCRUBBED_PAYLOAD_CHARS && *raw != redacted {
                    replacements.push((raw.clone(), redacted.clone()));
                } else if field == "scene" && !raw.is_empty() {
                    short_scenes.push((raw.clone(), redacted.clone()));
                }
                *raw = redacted;
            }
            for (key, item) in map.iter_mut() {
                if !is_data_key(key) {
                    redact_action_payloads(item, replacements, short_scenes);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact_action_payloads(item, replacements, short_scenes)),
        _ => {}
    }
}

/// Raw report bytes are never rewritten by text scrubbing.
fn is_data_key(key: &str) -> bool {
    matches!(key, "hex" | "report_descriptor_hex")
}

/// Replaces each `(from, to)` in every string value except raw data, but only
/// where `from` is a whole word (not next to a letter or digit), so a short
/// scene name such as `Live` never rewrites `Lively`.
fn scrub_words(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (from, to) in replacements {
                let mut out = String::with_capacity(text.len());
                let mut rest = text.as_str();
                while let Some(at) = rest.find(from.as_str()) {
                    let end = at + from.len();
                    let before = rest[..at].chars().next_back();
                    let after = rest[end..].chars().next();
                    out.push_str(&rest[..at]);
                    if before.is_some_and(char::is_alphanumeric)
                        || after.is_some_and(char::is_alphanumeric)
                    {
                        out.push_str(from);
                    } else {
                        out.push_str(to);
                    }
                    rest = &rest[end..];
                }
                out.push_str(rest);
                *text = out;
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if !is_data_key(key) {
                    scrub_words(item, replacements);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| scrub_words(item, replacements)),
        _ => {}
    }
}

/// Applies `(from, to)` replacements to every string value except raw data.
fn scrub(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            for (from, to) in replacements {
                if text.contains(from.as_str()) {
                    *text = text.replace(from.as_str(), to);
                }
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if !is_data_key(key) {
                    scrub(item, replacements);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| scrub(item, replacements)),
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Device probing
// ---------------------------------------------------------------------------

/// Why a capture ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureEnd {
    Deadline,
    ReportCap,
    ByteCap,
    ReadError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureOutcome {
    /// `(microseconds since the capture started, raw bytes)`.
    pub reports: Vec<(u64, Vec<u8>)>,
    pub bytes: usize,
    pub ended: CaptureEnd,
    pub error: Option<String>,
}

/// Read-only access to HID interfaces. Implementations must never send
/// feature/output reports or change the device mode.
pub trait DeviceProbe {
    fn enumerate(&mut self) -> Result<Vec<ModuleIdentity>, String>;
    fn descriptor(&mut self, device: &ModuleIdentity) -> Result<Vec<u8>, String>;
    fn capture(
        &mut self,
        device: &ModuleIdentity,
        duration: Duration,
    ) -> Result<CaptureOutcome, String>;
}

/// Reads the report descriptor; the only transport call is the descriptor read.
pub fn read_descriptor<T: ReportTransport + ?Sized>(transport: &T) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0; hidapi::MAX_REPORT_DESCRIPTOR_SIZE];
    let length = transport
        .get_report_descriptor(&mut buffer)
        .map_err(|error| format!("could not read the report descriptor: {error}"))?;
    if length == 0 || length > buffer.len() {
        return Err("the report descriptor read returned an invalid length".into());
    }
    buffer.truncate(length);
    Ok(buffer)
}

/// Passive input capture, the same read loop as the live feed: only
/// `read_timeout` is called. Ends at `duration`, `MAX_CAPTURE_REPORTS`, a
/// report that would exceed `MAX_CAPTURE_BYTES`, or a read error.
pub fn capture_reports<T: ReportTransport>(
    reader: &mut InputReader<T>,
    duration: Duration,
    mut elapsed: impl FnMut() -> Duration,
) -> CaptureOutcome {
    let mut reports = Vec::new();
    let mut bytes = 0;
    let micros = |elapsed: Duration| u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let (ended, error) = loop {
        if elapsed() >= duration {
            break (CaptureEnd::Deadline, None);
        }
        if reports.len() >= MAX_CAPTURE_REPORTS {
            break (CaptureEnd::ReportCap, None);
        }
        match reader.read_input(READ_TIMEOUT_MS) {
            Err(error) => {
                break (
                    CaptureEnd::ReadError,
                    Some(format!("input read failed: {error}")),
                );
            }
            Ok(None) => {}
            Ok(Some(raw)) => {
                if bytes + raw.len() > MAX_CAPTURE_BYTES {
                    break (CaptureEnd::ByteCap, None);
                }
                bytes += raw.len();
                let raw = raw.to_vec();
                reports.push((micros(elapsed()), raw));
            }
        }
    };
    CaptureOutcome {
        reports,
        bytes,
        ended,
        error,
    }
}

/// The real probe; its registry is created on the calling (worker) thread.
#[derive(Default)]
pub struct HidProbe {
    registry: Option<DeviceRegistry>,
}

impl HidProbe {
    fn open(&self, device: &ModuleIdentity) -> Result<hidapi::HidDevice, String> {
        let registry = self
            .registry
            .as_ref()
            .ok_or("HID devices were not enumerated")?;
        registry
            .open(device)
            .map_err(|error| format!("could not open the interface: {error}"))
    }
}

impl DeviceProbe for HidProbe {
    fn enumerate(&mut self) -> Result<Vec<ModuleIdentity>, String> {
        let registry = DeviceRegistry::new()
            .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
        let devices = registry.devices();
        self.registry = Some(registry);
        Ok(devices)
    }

    fn descriptor(&mut self, device: &ModuleIdentity) -> Result<Vec<u8>, String> {
        read_descriptor(&self.open(device)?)
    }

    fn capture(
        &mut self,
        device: &ModuleIdentity,
        duration: Duration,
    ) -> Result<CaptureOutcome, String> {
        let mut reader = InputReader::new(self.open(device)?);
        let origin = Instant::now();
        Ok(capture_reports(&mut reader, duration, || origin.elapsed()))
    }
}

/// Output and feature report lengths for every report ID the descriptor
/// declares, keyed `0xNN`, plus the first structural parse error.
fn descriptor_lengths(raw: &[u8]) -> Value {
    let mut parse_error = None;
    let mut lengths = |parse: fn(&[u8], u8) -> hidapi::HidResult<usize>| {
        let mut found = BTreeMap::new();
        for id in 0..=u8::MAX {
            match parse(raw, id) {
                Ok(length) => {
                    found.insert(format!("{id:#04x}"), length);
                }
                Err(error) => {
                    let error = error.to_string();
                    if !error.contains("is absent") {
                        parse_error.get_or_insert(error);
                        break;
                    }
                }
            }
        }
        found
    };
    let output = lengths(descriptor::output_report_length);
    let feature = lengths(descriptor::feature_report_length);
    json!({
        "output_report_lengths": output,
        "feature_report_lengths": feature,
        "parse_error": parse_error,
    })
}

// ---------------------------------------------------------------------------
// Bundle
// ---------------------------------------------------------------------------

/// Ids of running feeds and services when the bundle was requested.
#[derive(Clone, Debug, Default)]
pub struct Running {
    pub feeds: Vec<String>,
    pub services: Vec<String>,
}

pub struct BundleInputs<'a> {
    pub request: &'a Validated,
    /// Seconds since the Unix epoch.
    pub generated_at: u64,
    pub app: Value,
    pub environment: Value,
    /// Replaced with `~` in every stored string.
    pub home: Option<String>,
    pub running: &'a Running,
    /// True when a running feed or service holds the raw path.
    pub in_use: &'a dyn Fn(&[u8]) -> bool,
}

pub struct Bundle {
    pub value: Value,
    pub summary: Summary,
}

/// Formats Unix seconds as `(yyyy-mm-ddThh:mm:ssZ, yyyymmdd-hhmmss)`.
pub fn utc_stamps(seconds: u64) -> (String, String) {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    let (hour, minute, second) = (rest / 3600, rest % 3600 / 60, rest % 60);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
        format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}"),
    )
}

/// Builds the bundle. Device errors are recorded and never stop the build.
pub fn build_bundle(inputs: &BundleInputs, probe: &mut dyn DeviceProbe) -> Bundle {
    let redact = inputs.request.redact;
    let mut summary = Summary {
        redacted: redact,
        ..Summary::default()
    };
    let path_field = |path: &[u8]| -> (&'static str, String) {
        if redact {
            ("path_label", path_label(path))
        } else {
            ("path_hex", encode_hex(path))
        }
    };
    let serial_field = |serial: &Option<String>| -> Value {
        match serial {
            Some(serial) if redact => Value::String(redact_serial(serial)),
            Some(serial) => Value::String(serial.clone()),
            None => Value::Null,
        }
    };

    let (mut devices, enumeration_error) = match probe.enumerate() {
        Ok(devices) => (devices, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    devices.sort_by(|a, b| {
        (a.path.as_bytes(), a.interface_number, a.usage_page, a.usage).cmp(&(
            b.path.as_bytes(),
            b.interface_number,
            b.usage_page,
            b.usage,
        ))
    });
    let picked: Vec<String> = crate::feed::live_devices(&devices)
        .into_iter()
        .map(|device| device.path_hex)
        .collect();
    let is_picked = |device: &ModuleIdentity| picked.contains(&encode_hex(device.path.as_bytes()));

    let mut entries = Vec::new();
    for device in &devices {
        summary.devices += 1;
        let path = device.path.as_bytes();
        let descriptor = if (inputs.in_use)(path) {
            json!({ "status": "skipped", "skipped": IN_USE })
        } else {
            match probe.descriptor(device) {
                Ok(raw) => {
                    summary.descriptors_read += 1;
                    let mut read = json!({
                        "status": "read",
                        "length": raw.len(),
                        "report_descriptor_hex": encode_hex(&raw),
                    });
                    if let (Value::Object(read), Value::Object(lengths)) =
                        (&mut read, descriptor_lengths(&raw))
                    {
                        read.extend(lengths);
                    }
                    read
                }
                Err(error) => {
                    summary.descriptor_errors += 1;
                    json!({ "status": "error", "error": error })
                }
            }
        };
        let (path_key, path_value) = path_field(path);
        entries.push(json!({
            path_key: path_value,
            "vendor_id": device.vendor_id,
            "product_id": device.product_id,
            "usb_id": format!("{:04x}:{:04x}", device.vendor_id, device.product_id),
            "module": device.module.map(|module| module.name()),
            "input_supported": replay::service_module(device.module).is_some(),
            "serial_number": serial_field(&device.serial_number),
            "manufacturer_string": device.manufacturer_string,
            "product_string": device.product_string,
            "release_number": device.release_number,
            "interface_number": device.interface_number,
            "usage_page": device.usage_page,
            "usage": device.usage,
            "picked_for_live": is_picked(device),
            "descriptor": descriptor,
        }));
    }

    let capture = inputs.request.capture.map(|duration| {
        let mut captured = Vec::new();
        let mut seen: Vec<&[u8]> = Vec::new();
        for device in devices.iter().filter(|device| is_picked(device)) {
            let path = device.path.as_bytes();
            if seen.contains(&path) {
                continue;
            }
            seen.push(path);
            let (path_key, path_value) = path_field(path);
            let mut entry = json!({
                path_key: path_value,
                "module": device.module.map(|module| module.name()),
                "serial_number": serial_field(&device.serial_number),
            });
            let details = if (inputs.in_use)(path) {
                json!({ "status": "skipped", "skipped": IN_USE })
            } else {
                match probe.capture(device, duration) {
                    Ok(outcome) => {
                        summary.capture_reports += outcome.reports.len();
                        json!({
                            "status": "captured",
                            "ended": outcome.ended,
                            "error": outcome.error,
                            "report_count": outcome.reports.len(),
                            "bytes": outcome.bytes,
                            "reports": outcome.reports.iter().map(|(timestamp_us, raw)| json!({
                                "timestamp_us": timestamp_us,
                                "hex": encode_hex(raw),
                            })).collect::<Vec<_>>(),
                        })
                    }
                    Err(error) => json!({ "status": "error", "error": error }),
                }
            };
            if let (Value::Object(entry), Value::Object(details)) = (&mut entry, details) {
                entry.extend(details);
            }
            captured.push(entry);
        }
        json!({
            "seconds": duration.as_secs_f64(),
            "read_timeout_ms": READ_TIMEOUT_MS,
            "max_reports": MAX_CAPTURE_REPORTS,
            "max_bytes": MAX_CAPTURE_BYTES,
            "devices": captured,
        })
    });

    // Serial and path text that leaked into error messages, stderr tails or
    // ids is scrubbed with these, seeded from both the enumerated devices and
    // every raw value found structurally in the caller-supplied data.
    let mut replacements: Vec<(String, String)> = Vec::new();
    let mut runtime_id = |id: &str| -> String {
        if redact {
            redact_runtime_id(id, &mut replacements).unwrap_or_else(|| id.to_owned())
        } else {
            id.to_owned()
        }
    };
    let feeds: Vec<String> = inputs
        .running
        .feeds
        .iter()
        .map(|id| runtime_id(id))
        .collect();
    let services: Vec<String> = inputs
        .running
        .services
        .iter()
        .map(|id| runtime_id(id))
        .collect();
    let mut profile = inputs.request.profile.clone();
    let mut client = inputs.request.client.clone();
    if redact {
        // Scene names shorter than the free-text minimum are still removed
        // from profile and client text (activity lines, errors), as whole words.
        let mut short_scenes = Vec::new();
        if let Some(profile) = &mut profile {
            redact_fields(profile, &mut replacements);
            redact_action_payloads(profile, &mut replacements, &mut short_scenes);
        }
        redact_fields(&mut client, &mut replacements);
        redact_action_payloads(&mut client, &mut replacements, &mut short_scenes);
        short_scenes.sort_by_key(|(from, _)| (std::cmp::Reverse(from.len()), from.clone()));
        short_scenes.dedup();
        if let Some(profile) = &mut profile {
            scrub_words(profile, &short_scenes);
        }
        scrub_words(&mut client, &short_scenes);
        for device in &devices {
            seed_path(device.path.as_bytes(), &mut replacements);
            if let Some(serial) = &device.serial_number {
                seed_serial(serial, &mut replacements);
            }
        }
    }
    let (generated_at, _) = utc_stamps(inputs.generated_at);
    let mut value = json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "openstreamhub_diagnostics",
        "generated_at": generated_at,
        "redacted": redact,
        "uploaded": false,
        "app": inputs.app,
        "environment": inputs.environment,
        "enumeration_error": enumeration_error,
        "devices": entries,
        "capture": capture,
        "profile": profile,
        "runtime": {
            "feeds": feeds,
            "services": services,
            "client": client,
            "client_activity_dropped": inputs.request.activity_dropped,
            "client_notes_truncated": inputs.request.notes_truncated,
        },
    });

    // Longest first so overlapping values stay whole.
    if let Some(home) = inputs
        .home
        .as_deref()
        .map(|home| home.trim_end_matches(['/', '\\']))
        && home.len() > 1
    {
        replacements.push((home.to_owned(), "~".into()));
    }
    replacements.sort_by_key(|(from, _)| (std::cmp::Reverse(from.len()), from.clone()));
    replacements.dedup();
    scrub(&mut value, &replacements);
    Bundle { value, summary }
}

// ---------------------------------------------------------------------------
// App and environment facts
// ---------------------------------------------------------------------------

pub fn app_info(service_binary: Result<PathBuf, String>) -> Value {
    let service_binary = match service_binary {
        Ok(path) => json!({ "path": path.display().to_string() }),
        Err(error) => json!({ "error": error }),
    };
    json!({
        "name": env!("CARGO_PKG_NAME"),
        "version": env!("CARGO_PKG_VERSION"),
        "build": if cfg!(debug_assertions) { "debug" } else { "release" },
        "target_os": std::env::consts::OS,
        "target_arch": std::env::consts::ARCH,
        "tauri_version": tauri::VERSION,
        "service_binary": service_binary,
    })
}

pub fn environment() -> Value {
    json!({
        "os": std::env::consts::OS,
        "os_family": std::env::consts::FAMILY,
        "arch": std::env::consts::ARCH,
        "os_version": os_version(),
    })
}

/// Reads a small, bounded system file; never runs a program.
fn read_small(path: &str) -> Option<String> {
    use io::Read;
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn os_version() -> Option<String> {
    if cfg!(target_os = "macos") {
        read_small("/System/Library/CoreServices/SystemVersion.plist")
            .and_then(|plist| plist_string(&plist, "ProductVersion"))
            .map(|version| format!("macOS {version}"))
    } else if cfg!(target_os = "linux") {
        read_small("/etc/os-release").and_then(|text| os_release_name(&text))
    } else {
        None
    }
}

fn plist_string(plist: &str, key: &str) -> Option<String> {
    let after = &plist[plist.find(&format!("<key>{key}</key>"))?..];
    let start = after.find("<string>")? + "<string>".len();
    let end = after[start..].find("</string>")? + start;
    Some(after[start..end].trim().chars().take(64).collect())
}

fn os_release_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|name| name.trim().trim_matches('"').chars().take(64).collect())
}

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

pub fn file_name(stamp: &str) -> String {
    format!("{FILE_PREFIX}{stamp}.json")
}

/// True only for `openstreamhub-diagnostics-yyyymmdd-hhmmss.json`.
pub fn is_bundle_name(name: &str) -> bool {
    let Some(stamp) = name
        .strip_prefix(FILE_PREFIX)
        .and_then(|rest| rest.strip_suffix(".json"))
    else {
        return false;
    };
    let bytes = stamp.as_bytes();
    bytes.len() == 15
        && bytes.iter().enumerate().all(|(index, byte)| {
            if index == 8 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

/// Writes the bundle as a new private file without ever replacing one; see
/// `export::write_new`.
pub fn write_bundle(
    directory: &Path,
    bundle: &Value,
    stamp: &str,
) -> Result<(PathBuf, u64), String> {
    let mut bytes = serde_json::to_vec_pretty(bundle)
        .map_err(|error| format!("could not encode the diagnostics bundle: {error}"))?;
    bytes.push(b'\n');
    let path =
        crate::export::write_new(directory, &file_name(stamp), &bytes, "diagnostics bundle")?;
    Ok((path, bytes.len() as u64))
}

/// Deletes all but the newest `keep` bundle files; other files are ignored.
pub fn prune(directory: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut bundles: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            is_bundle_name(&name).then(|| (name, entry.path()))
        })
        .collect();
    // The fixed-width UTC stamp sorts chronologically.
    bundles.sort();
    let excess = bundles.len().saturating_sub(keep);
    for (_, path) in bundles.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

/// The command without Tauri: validates, probes read-only, writes, prunes.
pub fn create(
    request: DiagnosticsRequest,
    app_data: &Path,
    running: &Running,
    in_use: &dyn Fn(&[u8]) -> bool,
    service_binary: Result<PathBuf, String>,
    probe: &mut dyn DeviceProbe,
) -> Result<DiagnosticsResult, String> {
    let request = validate(request)?;
    let generated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "the system clock is before 1970".to_string())?
        .as_secs();
    let home = std::env::home_dir().map(|home| home.to_string_lossy().into_owned());
    let bundle = build_bundle(
        &BundleInputs {
            request: &request,
            generated_at,
            app: app_info(service_binary),
            environment: environment(),
            home,
            running,
            in_use,
        },
        probe,
    );
    let directory = app_data.join(DIRECTORY);
    let (_, stamp) = utc_stamps(generated_at);
    let (path, bytes) = write_bundle(&directory, &bundle.value, &stamp)?;
    prune(&directory, KEEP_BUNDLES);
    Ok(DiagnosticsResult {
        path: path.to_string_lossy().into_owned(),
        bytes,
        summary: bundle.summary,
    })
}

#[cfg(test)]
pub mod fake {
    //! A scripted probe over a fake transport that records every command.

    use super::*;
    use cooltek_sdk_masterhub::{ModuleKind, hidapi::HidResult};
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        ffi::CString,
        rc::Rc,
    };

    pub struct FakeTransport {
        descriptor: Result<Vec<u8>, String>,
        reads: RefCell<VecDeque<Vec<u8>>>,
        clock: Rc<Cell<u64>>,
        pub commands: RefCell<Vec<&'static str>>,
    }

    impl ReportTransport for FakeTransport {
        fn read_timeout(&self, buffer: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            match self.reads.borrow_mut().pop_front() {
                Some(bytes) => {
                    self.clock.set(self.clock.get() + 1_000);
                    buffer[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                None => {
                    self.clock.set(self.clock.get() + 100_000);
                    Ok(0)
                }
            }
        }

        fn get_feature_report(&self, _buffer: &mut [u8]) -> HidResult<usize> {
            self.commands.borrow_mut().push("get_feature_report");
            Ok(0)
        }

        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            self.commands.borrow_mut().push("send_feature_report");
            Ok(())
        }

        fn write_output(&self, _data: &[u8]) -> HidResult<usize> {
            self.commands.borrow_mut().push("write_output");
            Ok(0)
        }

        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            match &self.descriptor {
                Ok(raw) => {
                    buffer[..raw.len()].copy_from_slice(raw);
                    Ok(raw.len())
                }
                Err(message) => Err(hidapi::HidError::HidApiError {
                    message: message.clone(),
                }),
            }
        }
    }

    pub struct FakeDevice {
        pub identity: ModuleIdentity,
        pub open_error: Option<String>,
        pub descriptor: Result<Vec<u8>, String>,
        pub reads: Vec<Vec<u8>>,
    }

    #[derive(Default)]
    pub struct FakeProbe {
        pub devices: Vec<FakeDevice>,
        pub enumerate_error: Option<String>,
        /// Paths opened, in order.
        pub opened: Vec<Vec<u8>>,
        /// Every non-read transport call across all opens.
        pub commands: Vec<&'static str>,
    }

    impl FakeProbe {
        fn open(&mut self, identity: &ModuleIdentity) -> Result<FakeTransport, String> {
            self.opened.push(identity.path.as_bytes().to_vec());
            let device = self
                .devices
                .iter()
                .find(|device| device.identity == *identity)
                .ok_or("not enumerated")?;
            if let Some(error) = &device.open_error {
                return Err(format!("could not open the interface: {error}"));
            }
            Ok(FakeTransport {
                descriptor: device.descriptor.clone(),
                reads: RefCell::new(device.reads.iter().cloned().collect()),
                clock: Rc::default(),
                commands: RefCell::default(),
            })
        }
    }

    impl DeviceProbe for FakeProbe {
        fn enumerate(&mut self) -> Result<Vec<ModuleIdentity>, String> {
            match &self.enumerate_error {
                Some(error) => Err(error.clone()),
                None => Ok(self.devices.iter().map(|d| d.identity.clone()).collect()),
            }
        }

        fn descriptor(&mut self, device: &ModuleIdentity) -> Result<Vec<u8>, String> {
            let transport = self.open(device)?;
            let result = read_descriptor(&transport);
            self.commands.extend(transport.commands.take());
            result
        }

        fn capture(
            &mut self,
            device: &ModuleIdentity,
            duration: Duration,
        ) -> Result<CaptureOutcome, String> {
            let transport = self.open(device)?;
            let clock = Rc::clone(&transport.clock);
            let mut reader = InputReader::new(transport);
            let outcome =
                capture_reports(&mut reader, duration, || Duration::from_micros(clock.get()));
            self.commands
                .extend(reader.into_transport().commands.take());
            Ok(outcome)
        }
    }

    pub fn identity(
        path: &str,
        product_id: u16,
        serial: Option<&str>,
        usage_page: u16,
    ) -> ModuleIdentity {
        ModuleIdentity {
            path: CString::new(path).unwrap(),
            vendor_id: 0x2516,
            product_id,
            serial_number: serial.map(str::to_owned),
            manufacturer_string: Some("Cooler Master".into()),
            product_string: Some("MasterHUB module".into()),
            release_number: 0x0100,
            interface_number: 0,
            usage_page,
            usage: 1,
            module: ModuleKind::from_ids(0x2516, product_id),
        }
    }

    /// Vendor collection with input report 1 (64 bytes), output report 2
    /// (64 bytes) and feature report 3 (31 bytes).
    pub fn descriptor() -> Vec<u8> {
        vec![
            0x06, 0x00, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x75, 0x08, 0x85, 0x01, 0x95, 0x3f, 0x09,
            0x01, 0x81, 0x02, 0x85, 0x02, 0x95, 0x3f, 0x09, 0x01, 0x91, 0x02, 0x85, 0x03, 0x95,
            0x1f, 0x09, 0x01, 0xb1, 0x02, 0xc0,
        ]
    }

    /// One fader module, one knob module with two collections, the hub base,
    /// and an unknown Cooler Master product.
    pub fn rig() -> FakeProbe {
        let fader = |positions: [u8; 5]| {
            let mut raw = vec![0x01, 0x2a, 0x07, 0x00, 0x02, 0x05];
            raw.extend_from_slice(&positions);
            raw
        };
        FakeProbe {
            devices: vec![
                FakeDevice {
                    identity: identity(
                        r"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042",
                        0x0200,
                        Some("FADER-SERIAL-0042"),
                        0xff00,
                    ),
                    open_error: None,
                    descriptor: Ok(descriptor()),
                    reads: vec![fader([0, 25, 50, 75, 100]), fader([60, 25, 50, 75, 100])],
                },
                FakeDevice {
                    identity: identity("DevSrvsID:4294970001", 0x0202, Some("KNOB7"), 0x000c),
                    open_error: None,
                    descriptor: Ok(vec![0x05, 0x0c, 0x09, 0x01, 0xa1]),
                    reads: vec![],
                },
                FakeDevice {
                    identity: identity("DevSrvsID:4294970002", 0x0202, Some("KNOB7"), 0xff00),
                    open_error: Some("exclusive access denied for KNOB7".into()),
                    descriptor: Ok(descriptor()),
                    reads: vec![],
                },
                FakeDevice {
                    identity: identity("DevSrvsID:4294970003", 0x01f6, None, 0xff00),
                    open_error: None,
                    descriptor: Err("descriptor unavailable".into()),
                    reads: vec![],
                },
                FakeDevice {
                    identity: identity("DevSrvsID:4294970004", 0x1234, Some("X1"), 0xff00),
                    open_error: None,
                    descriptor: Ok(descriptor()),
                    reads: vec![],
                },
            ],
            ..FakeProbe::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    fn request(redact: bool, capture: bool, seconds: f64) -> DiagnosticsRequest {
        DiagnosticsRequest {
            redact_serials: redact,
            include_capture: capture,
            capture_seconds: seconds,
            include_profile: None,
            client: ClientContext {
                activity: vec![],
                notes: None,
                extra: Map::new(),
            },
        }
    }

    fn build(
        validated: &Validated,
        probe: &mut FakeProbe,
        in_use: &dyn Fn(&[u8]) -> bool,
        running: &Running,
    ) -> Bundle {
        build_bundle(
            &BundleInputs {
                request: validated,
                generated_at: 1_791_201_600,
                app: json!({ "name": "test" }),
                environment: json!({ "os": "test" }),
                home: Some("/Users/tester".into()),
                running,
                in_use,
            },
            probe,
        )
    }

    fn device<'a>(bundle: &'a Value, key: &str, value: &str) -> &'a Value {
        bundle["devices"]
            .as_array()
            .unwrap()
            .iter()
            .find(|device| device[key] == value)
            .unwrap()
    }

    #[test]
    fn serials_and_paths_are_redacted_everywhere() {
        let validated = validate(DiagnosticsRequest {
            include_profile: Some(json!({
                "name": "rig",
                "modules": [{ "serial_number": "FADER-SERIAL-0042" }, { "serial_number": "K1" }],
            })),
            client: ClientContext {
                activity: vec![json!({
                    "service_id": format!("service:{}", encode_hex(br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042")),
                    "serial": "ABCDEFGH",
                    "nested": { "serial_number": "QWERTY12" },
                    "stderr_tail": "opened FADER-SERIAL-0042 at /Users/tester/x",
                })],
                notes: Some("rig at home".into()),
                extra: Map::new(),
            },
            ..request(true, true, 1.0)
        })
        .unwrap();
        let fader_hex = encode_hex(br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042");
        let running = Running {
            feeds: vec![
                "demo:faders".into(),
                "live:2f6465762f6869647261773939".into(),
            ],
            services: vec![format!("service:{fader_hex}")],
        };
        let mut probe = rig();
        let bundle = build(&validated, &mut probe, &|_| false, &running);
        let text = bundle.value.to_string();
        for secret in [
            "FADER-SERIAL-0042",
            "KNOB7",
            "ABCDEFGH",
            "QWERTY12",
            "DevSrvsID",
            "HID#VID",
            fader_hex.as_str(),
            "/Users/tester",
            "path_hex",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        assert!(bundle.summary.redacted);
        let label = path_label(br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042");
        let fader = device(&bundle.value, "path_label", &label);
        assert_eq!(fader["serial_number"], "…0042");
        assert_eq!(
            device(&bundle.value, "usb_id", "2516:1234")["serial_number"],
            "…"
        );
        assert_eq!(
            bundle.value["profile"]["modules"][0]["serial_number"],
            "…0042"
        );
        assert_eq!(bundle.value["profile"]["modules"][1]["serial_number"], "…");
        let client = &bundle.value["runtime"]["client"]["activity"][0];
        assert_eq!(client["serial"], "…EFGH");
        assert_eq!(client["nested"]["serial_number"], "…TY12");
        assert_eq!(client["service_id"], format!("service:{label}"));
        assert_eq!(client["stderr_tail"], "opened …0042 at ~/x");
        assert_eq!(
            bundle.value["runtime"]["services"],
            json!([format!("service:{label}")])
        );
        assert_eq!(
            bundle.value["runtime"]["feeds"],
            json!([
                "demo:faders",
                format!("live:{}", path_label(b"/dev/hidraw99"))
            ])
        );
        // The open error embedding a serial is scrubbed too.
        let knob_vendor = device(
            &bundle.value,
            "path_label",
            &path_label(b"DevSrvsID:4294970002"),
        );
        assert!(
            knob_vendor["descriptor"]["error"]
                .as_str()
                .unwrap()
                .contains('…')
        );
        // Labels are stable and 8 hex digits.
        assert_eq!(
            label,
            path_label(br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042")
        );
        assert_eq!(label.len(), "path-".len() + 8);
        assert_eq!(path_label(b""), "path-811c9dc5");
    }

    #[test]
    fn client_data_naming_unenumerated_devices_is_redacted_structurally() {
        // A device that unplugged before the bundle was built: it is named
        // only by the caller-supplied activity and profile.
        let gone = b"/dev/hidraw7-GONE";
        let gone_hex = encode_hex(gone);
        let validated = validate(DiagnosticsRequest {
            include_profile: Some(json!({ "modules": [{ "serial_number": "PROFILE-SN-777" }] })),
            client: ClientContext {
                activity: vec![
                    json!({
                        "source": "device_feed",
                        "devices": [{ "path_hex": gone_hex, "module": "M005S", "serial_number": "GONE-SERIAL-9" }],
                        "feeds": [{ "feed_id": format!("live:{}", gone_hex.to_uppercase()), "error": format!("{gone_hex} vanished") }],
                    }),
                    json!({
                        "source": "service_status",
                        "service_id": format!("service:{gone_hex}"),
                        "stderr_tail": "open /dev/hidraw7-GONE failed for GONE-SERIAL-9 (PROFILE-SN-777)",
                        "counters": { "hex": "00ff" },
                    }),
                    json!(format!("live:{gone_hex}")),
                ],
                notes: Some(format!("unplugged {} at 12:00", gone_hex.to_uppercase())),
                extra: Map::new(),
            },
            ..request(true, false, 0.0)
        })
        .unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        let text = bundle.value.to_string();
        for secret in [
            "/dev/hidraw7-GONE",
            gone_hex.as_str(),
            gone_hex.to_uppercase().as_str(),
            "GONE-SERIAL-9",
            "PROFILE-SN-777",
            "path_hex",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        let label = path_label(gone);
        let activity = &bundle.value["runtime"]["client"]["activity"];
        let feed = &activity[0];
        assert_eq!(feed["devices"][0]["path_label"], label);
        assert_eq!(feed["devices"][0]["serial_number"], "…AL-9");
        assert_eq!(feed["feeds"][0]["feed_id"], format!("live:{label}"));
        assert_eq!(feed["feeds"][0]["error"], format!("{label} vanished"));
        assert_eq!(activity[1]["service_id"], format!("service:{label}"));
        assert_eq!(
            activity[1]["stderr_tail"],
            format!("open {label} failed for …AL-9 (…-777)")
        );
        // Raw data keys are never rewritten.
        assert_eq!(activity[1]["counters"]["hex"], "00ff");
        assert_eq!(activity[2], format!("live:{label}"));
        assert_eq!(
            bundle.value["runtime"]["client"]["notes"],
            format!("unplugged {label} at 12:00")
        );
        assert_eq!(
            bundle.value["profile"]["modules"][0]["serial_number"],
            "…-777"
        );

        // Without redaction the caller's data is kept as given.
        let mut plain = validated.clone();
        plain.redact = false;
        let bundle = build(&plain, &mut rig(), &|_| false, &Running::default());
        assert_eq!(
            bundle.value["runtime"]["client"]["activity"][0]["devices"][0]["path_hex"],
            gone_hex
        );
    }

    fn action_payload_profile() -> Value {
        json!({
            "bindings": [
                { "action": { "type": "text", "text": "my secret password\n", "press_enter": true } },
                { "action": { "type": "open_url", "url": "https://docs.example.com:8443/private/token?id=42#x" } },
                { "action": { "type": "multi_action", "actions": [
                    { "type": "open_url", "url": "http://user:pw@intranet.local/reset" },
                    { "type": "text", "text": "żółć 🎉", "press_enter": false },
                    { "type": "obs_scene", "scene": "Interview guest cam" },
                ] } },
                { "action": { "type": "obs_scene", "scene": "Private call scene" } },
            ],
            "keypad": { "pages": [{ "slots": [
                { "type": "action", "action": { "type": "text", "text": "keypad tile snippet", "press_enter": false } },
                { "type": "action", "action": { "type": "timer", "seconds": 300, "label": "call the doctor", "sound": "glass" } },
            ] }] },
            "dial_functions": [{ "name": "Links", "bindings": [
                { "gesture": "right", "action": { "type": "open_url", "url": "https://dial.example.org/a/b" } },
            ] }],
        })
    }

    #[test]
    fn text_and_url_action_payloads_are_redacted_everywhere() {
        let validated = validate(DiagnosticsRequest {
            include_profile: Some(action_payload_profile()),
            client: ClientContext {
                activity: vec![
                    json!({ "kind": "service_record", "record": { "type": "action_failed",
                        "action": { "type": "text", "text": "activity snippet text", "press_enter": false },
                        "error": "pasting \"activity snippet text\" failed" } }),
                    json!({ "kind": "binding", "action": { "type": "open_url", "url": "https://activity.example.net/secret-path" } }),
                    json!({ "type": "open_url", "url": "not a url" }),
                ],
                notes: None,
                extra: Map::new(),
            },
            ..request(true, false, 0.0)
        })
        .unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        let text = bundle.value.to_string();
        for secret in [
            "my secret password",
            "private/token",
            "id=42",
            "user:pw",
            "/reset",
            "żółć",
            "keypad tile snippet",
            "/a/b",
            "activity snippet text",
            "secret-path",
            "not a url",
            "call the doctor",
            "Private call",
            "Interview guest",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
        let timer = &bundle.value["profile"]["keypad"]["pages"][0]["slots"][1]["action"];
        assert_eq!(
            timer,
            &json!({ "type": "timer", "seconds": 300, "label": "<15 chars>", "sound": "glass" })
        );
        let profile = &bundle.value["profile"];
        assert_eq!(profile["bindings"][0]["action"]["text"], "<19 chars>");
        assert_eq!(profile["bindings"][0]["action"]["press_enter"], true);
        assert_eq!(
            profile["bindings"][1]["action"]["url"],
            "https://docs.example.com:8443"
        );
        let steps = &profile["bindings"][2]["action"]["actions"];
        assert_eq!(steps[0]["url"], "http://intranet.local");
        assert_eq!(steps[1]["text"], "<6 chars>");
        assert_eq!(steps[2]["scene"], "<19 chars>");
        assert_eq!(profile["bindings"][3]["action"]["scene"], "<18 chars>");
        assert_eq!(
            profile["keypad"]["pages"][0]["slots"][0]["action"]["text"],
            "<19 chars>"
        );
        assert_eq!(
            profile["dial_functions"][0]["bindings"][0]["action"]["url"],
            "https://dial.example.org"
        );
        let activity = &bundle.value["runtime"]["client"]["activity"];
        assert_eq!(activity[0]["record"]["action"]["text"], "<21 chars>");
        // Long raw snippets are scrubbed from free text too.
        assert_eq!(
            activity[0]["record"]["error"],
            "pasting \"<21 chars>\" failed"
        );
        assert_eq!(activity[1]["action"]["url"], "https://activity.example.net");
        assert_eq!(activity[2]["url"], "<url>");
    }

    fn activity_text(text: &str) -> Value {
        json!({ "source": "service", "service_id": "service:x", "kind": "action", "text": text, "at": 1 })
    }

    #[test]
    fn obs_scene_names_of_any_length_are_scrubbed_from_activity_text() {
        let validated = validate(DiagnosticsRequest {
            include_profile: Some(json!({ "bindings": [
                { "action": { "type": "obs_scene", "scene": "Live" } },
                { "action": { "type": "obs_scene", "scene": "BRB" } },
                { "action": { "type": "multi_action", "actions": [
                    { "type": "toggle_mute" }, { "type": "obs_scene", "scene": "Interview guest cam" },
                ] } },
            ] })),
            client: ClientContext {
                activity: vec![
                    // Lines in the old format, which printed the name.
                    activity_text("Executed OBS scene \"Live\" (m4315 control 1 pressed)."),
                    activity_text(
                        "OBS scene \"BRB\" failed: OBS at 127.0.0.1:4455: no scene named BRB",
                    ),
                    activity_text("multi-action [toggle mute, OBS scene \"Interview guest cam\"]"),
                    // A short name inside a longer word is left alone.
                    activity_text("Lively BRBs and Live."),
                    activity_text("Executed OBS scene (4 characters)."),
                ],
                notes: None,
                extra: Map::new(),
            },
            ..request(true, false, 0.0)
        })
        .unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        let texts: Vec<&str> = bundle.value["runtime"]["client"]["activity"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["text"].as_str().unwrap())
            .collect();
        assert_eq!(
            texts,
            [
                "Executed OBS scene \"<4 chars>\" (m4315 control 1 pressed).",
                "OBS scene \"<3 chars>\" failed: OBS at 127.0.0.1:4455: no scene named <3 chars>",
                "multi-action [toggle mute, OBS scene \"<19 chars>\"]",
                "Lively BRBs and <4 chars>.",
                "Executed OBS scene (4 characters).",
            ]
        );
        let profile = &bundle.value["profile"]["bindings"];
        assert_eq!(profile[0]["action"]["scene"], "<4 chars>");
        assert_eq!(profile[1]["action"]["scene"], "<3 chars>");
        assert_eq!(profile[2]["action"]["actions"][1]["scene"], "<19 chars>");
        assert!(!bundle.value.to_string().contains("Interview guest"));
    }

    #[test]
    fn without_the_profile_obs_scene_names_in_client_records_are_still_scrubbed() {
        let validated = validate(DiagnosticsRequest {
            client: ClientContext {
                activity: vec![
                    json!({ "kind": "service_record", "record": { "type": "action_failed",
                        "action": { "type": "obs_scene", "scene": "BRB" },
                        "error": "OBS at 127.0.0.1:4455: no scene named BRB" } }),
                    activity_text("Executed OBS scene (4 characters)."),
                ],
                notes: None,
                extra: Map::new(),
            },
            ..request(true, false, 0.0)
        })
        .unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        let activity = &bundle.value["runtime"]["client"]["activity"];
        assert_eq!(activity[0]["record"]["action"]["scene"], "<3 chars>");
        assert_eq!(
            activity[0]["record"]["error"],
            "OBS at 127.0.0.1:4455: no scene named <3 chars>"
        );
        assert_eq!(activity[1]["text"], "Executed OBS scene (4 characters).");
        assert_eq!(bundle.value["profile"], Value::Null);
        assert!(!bundle.value.to_string().contains("BRB"));

        // Without redaction nothing is rewritten.
        let plain = validate(DiagnosticsRequest {
            client: ClientContext {
                activity: vec![activity_text("Executed OBS scene \"Live\".")],
                notes: None,
                extra: Map::new(),
            },
            include_profile: Some(json!({ "bindings": [
                { "action": { "type": "obs_scene", "scene": "Live" } },
            ] })),
            ..request(false, false, 0.0)
        })
        .unwrap();
        let bundle = build(&plain, &mut rig(), &|_| false, &Running::default());
        assert_eq!(
            bundle.value["runtime"]["client"]["activity"][0]["text"],
            "Executed OBS scene \"Live\"."
        );
    }

    #[test]
    fn unredacted_bundles_keep_text_and_url_action_payloads() {
        let validated = validate(DiagnosticsRequest {
            include_profile: Some(action_payload_profile()),
            ..request(false, false, 0.0)
        })
        .unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        assert!(!bundle.summary.redacted);
        assert_eq!(bundle.value["profile"], action_payload_profile());
    }

    #[test]
    fn url_origin_keeps_only_scheme_and_host() {
        for (url, origin) in [
            ("https://example.com", "https://example.com"),
            ("https://example.com/", "https://example.com"),
            ("https://example.com?q=1", "https://example.com"),
            ("https://example.com#frag", "https://example.com"),
            ("http://[::1]:8080/x", "http://[::1]:8080"),
            ("https://a:b@host.test/p", "https://host.test"),
            ("mailto:someone@example.com", "<url>"),
            ("", "<url>"),
            ("HTTPS://Example.com/x", "https://Example.com"),
            ("my secret token https://x", "<url>"),
            ("1http://x", "<url>"),
            ("://x", "<url>"),
            ("ht tp://x", "<url>"),
        ] {
            assert_eq!(url_origin(url), origin, "{url}");
        }
    }

    #[test]
    fn unredacted_bundles_keep_real_paths_and_serials() {
        let validated = validate(request(false, false, 0.0)).unwrap();
        let bundle = build(&validated, &mut rig(), &|_| false, &Running::default());
        let hex = encode_hex(b"DevSrvsID:4294970001");
        let knob = device(&bundle.value, "path_hex", &hex);
        assert_eq!(knob["serial_number"], "KNOB7");
        assert_eq!(knob["module"], "M003K");
        assert_eq!(knob["picked_for_live"], false);
        assert_eq!(
            device(
                &bundle.value,
                "path_hex",
                &encode_hex(b"DevSrvsID:4294970002")
            )["picked_for_live"],
            true
        );
        // Unsupported Cooler Master products are listed with a null module.
        assert_eq!(
            device(&bundle.value, "usb_id", "2516:1234")["module"],
            Value::Null
        );
        assert_eq!(bundle.value["capture"], Value::Null);
        assert_eq!(bundle.value["profile"], Value::Null);
        assert!(!bundle.summary.redacted);
    }

    #[test]
    fn in_use_interfaces_are_never_opened() {
        let validated = validate(request(false, true, 1.0)).unwrap();
        let mut probe = rig();
        let held = br"\\?\HID#VID_2516&PID_0200#FADER-SERIAL-0042".to_vec();
        let in_use = |path: &[u8]| path == held.as_slice();
        let bundle = build(&validated, &mut probe, &in_use, &Running::default());
        assert!(!probe.opened.contains(&held));
        let fader = device(&bundle.value, "path_hex", &encode_hex(&held));
        assert_eq!(fader["descriptor"]["skipped"], "in use by feed/service");
        let captured = bundle.value["capture"]["devices"].as_array().unwrap();
        let fader = captured
            .iter()
            .find(|entry| entry["path_hex"] == encode_hex(&held))
            .unwrap();
        assert_eq!(fader["skipped"], "in use by feed/service");
        assert_eq!(bundle.summary.capture_reports, 0);
    }

    #[test]
    fn descriptor_errors_are_recorded_and_the_rest_continue() {
        let validated = validate(request(false, false, 0.0)).unwrap();
        let mut probe = rig();
        let bundle = build(&validated, &mut probe, &|_| false, &Running::default());
        assert_eq!(
            bundle.summary,
            Summary {
                devices: 5,
                descriptors_read: 3,
                descriptor_errors: 2,
                capture_reports: 0,
                redacted: false,
            }
        );
        let hub = device(&bundle.value, "usb_id", "2516:01f6");
        assert_eq!(hub["descriptor"]["status"], "error");
        assert!(
            hub["descriptor"]["error"]
                .as_str()
                .unwrap()
                .contains("descriptor unavailable")
        );
        let unknown = device(&bundle.value, "usb_id", "2516:1234");
        assert_eq!(
            unknown["descriptor"]["output_report_lengths"],
            json!({ "0x02": 64 })
        );
        assert_eq!(
            unknown["descriptor"]["feature_report_lengths"],
            json!({ "0x03": 32 })
        );
        assert_eq!(unknown["descriptor"]["parse_error"], Value::Null);
        // A truncated consumer descriptor is kept with its parse error.
        let consumer = device(
            &bundle.value,
            "path_hex",
            &encode_hex(b"DevSrvsID:4294970001"),
        );
        assert_eq!(
            consumer["descriptor"]["report_descriptor_hex"],
            "050c0901a1"
        );
        assert!(
            consumer["descriptor"]["parse_error"]
                .as_str()
                .unwrap()
                .contains("truncated")
        );
        assert_eq!(probe.opened.len(), 5);

        let mut probe = FakeProbe {
            enumerate_error: Some("no hid".into()),
            ..FakeProbe::default()
        };
        let bundle = build(&validated, &mut probe, &|_| false, &Running::default());
        assert_eq!(bundle.value["enumeration_error"], "no hid");
        assert_eq!(bundle.value["devices"], json!([]));
    }

    #[test]
    fn capture_reads_picked_devices_and_never_writes() {
        let validated = validate(request(false, true, 2.0)).unwrap();
        let mut probe = rig();
        let bundle = build(&validated, &mut probe, &|_| false, &Running::default());
        let captured = bundle.value["capture"]["devices"].as_array().unwrap();
        // The fader and the knob vendor interface; no hub, no consumer page.
        assert_eq!(captured.len(), 2);
        let fader = &captured[1];
        assert_eq!(fader["status"], "captured");
        assert_eq!(fader["ended"], "deadline");
        assert_eq!(fader["report_count"], 2);
        assert_eq!(
            fader["reports"][0],
            json!({ "timestamp_us": 1000, "hex": "012a070002050019324b64" })
        );
        assert_eq!(captured[0]["status"], "error");
        assert_eq!(bundle.summary.capture_reports, 2);
        assert!(probe.commands.is_empty(), "{:?}", probe.commands);
    }

    fn capture(reads: Vec<Vec<u8>>, seconds: u64) -> (CaptureOutcome, Vec<&'static str>) {
        let mut probe = FakeProbe {
            devices: vec![FakeDevice {
                identity: identity("p", 0x0200, None, 0xff00),
                open_error: None,
                descriptor: Ok(descriptor()),
                reads,
            }],
            ..FakeProbe::default()
        };
        let identity = probe.devices[0].identity.clone();
        let outcome = probe
            .capture(&identity, Duration::from_secs(seconds))
            .unwrap();
        (outcome, probe.commands)
    }

    #[test]
    fn capture_bounds() {
        for seconds in [0.0, 6.0, 0.5, f64::NAN, -1.0] {
            assert!(
                validate(request(false, true, seconds)).is_err(),
                "{seconds}"
            );
        }
        for seconds in [1.0, 5.0] {
            let validated = validate(request(false, true, seconds)).unwrap();
            assert_eq!(validated.capture, Some(Duration::from_secs_f64(seconds)));
        }
        // Ignored without a capture.
        assert_eq!(validate(request(false, false, 99.0)).unwrap().capture, None);

        let (outcome, commands) = capture(vec![vec![1; 8]; 300], 5);
        assert_eq!(outcome.ended, CaptureEnd::ReportCap);
        assert_eq!(outcome.reports.len(), MAX_CAPTURE_REPORTS);
        assert!(commands.is_empty());

        let (outcome, _) = capture(vec![vec![2; 4096]; 40], 5);
        assert_eq!(outcome.ended, CaptureEnd::ByteCap);
        assert_eq!(outcome.bytes, MAX_CAPTURE_BYTES);
        assert_eq!(outcome.reports.len(), 16);

        // Idle reads end at the deadline: ten 100 ms timeouts per second.
        let (outcome, _) = capture(vec![], 1);
        assert_eq!(outcome.ended, CaptureEnd::Deadline);
        assert!(outcome.reports.is_empty());
    }

    #[test]
    fn request_limits() {
        let mut big = request(false, false, 0.0);
        big.client.activity = vec![json!("x".repeat(1024)); 300];
        assert!(validate(big).unwrap_err().contains("limit"));

        let mut many = request(false, false, 0.0);
        many.client.activity = (0..600).map(|index| json!(index)).collect();
        many.client.notes = Some("é".repeat(2000));
        many.client.extra.insert("ui".into(), json!("devices"));
        let validated = validate(many).unwrap();
        let activity = validated.client["activity"].as_array().unwrap();
        assert_eq!(activity.len(), MAX_ACTIVITY);
        assert_eq!(activity[0], 100);
        assert_eq!(validated.activity_dropped, 100);
        let notes = validated.client["notes"].as_str().unwrap();
        assert!(notes.len() <= MAX_NOTES_BYTES && notes.len() > MAX_NOTES_BYTES - 2);
        assert!(validated.notes_truncated);
        assert_eq!(validated.client["ui"], "devices");

        let mut profile = request(false, false, 0.0);
        profile.include_profile = Some(json!([1]));
        assert!(validate(profile).is_err());

        // Every key is required; null is allowed only for profile and notes.
        let full = json!({
            "redact_serials": true, "include_capture": false, "capture_seconds": 0,
            "include_profile": null, "client": { "activity": [], "notes": null },
        });
        assert!(serde_json::from_value::<DiagnosticsRequest>(full.clone()).is_ok());
        for key in [
            "redact_serials",
            "include_capture",
            "capture_seconds",
            "include_profile",
            "client",
        ] {
            let mut missing = full.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                serde_json::from_value::<DiagnosticsRequest>(missing).is_err(),
                "{key}"
            );
        }
        let mut no_notes = full.clone();
        no_notes["client"].as_object_mut().unwrap().remove("notes");
        assert!(serde_json::from_value::<DiagnosticsRequest>(no_notes).is_err());
        let mut extra = full;
        extra["upload"] = json!(true);
        assert!(serde_json::from_value::<DiagnosticsRequest>(extra).is_err());
    }

    #[test]
    fn home_paths_become_tilde() {
        let mut validated = validate(request(false, false, 0.0)).unwrap();
        validated.client = json!({ "activity": ["/Users/tester/Library/x"], "notes": null });
        let bundle = build_bundle(
            &BundleInputs {
                request: &validated,
                generated_at: 0,
                app: app_info(Err("could not find masterhub-service. Checked: /Users/tester/bin/masterhub-service".into())),
                environment: environment(),
                home: Some("/Users/tester/".into()),
                running: &Running::default(),
                in_use: &|_| false,
            },
            &mut FakeProbe::default(),
        );
        assert_eq!(
            bundle.value["runtime"]["client"]["activity"][0],
            "~/Library/x"
        );
        assert_eq!(
            bundle.value["app"]["service_binary"]["error"],
            "could not find masterhub-service. Checked: ~/bin/masterhub-service"
        );
        assert_eq!(bundle.value["generated_at"], "1970-01-01T00:00:00Z");
        assert!(!bundle.value.to_string().contains("/Users/tester"));
    }

    #[test]
    fn utc_stamps_format_dates() {
        assert_eq!(
            utc_stamps(1_791_201_600),
            ("2026-10-05T12:00:00Z".into(), "20261005-120000".into())
        );
        assert_eq!(utc_stamps(951_782_400).1, "20000229-000000");
        assert!(is_bundle_name(
            "openstreamhub-diagnostics-20261005-120000.json"
        ));
        for name in [
            "openstreamhub-diagnostics-20261005-12000.json",
            "openstreamhub-diagnostics-20261005_120000.json",
            "openstreamhub-diagnostics-20261005-120000.json.bak",
            "notes.json",
        ] {
            assert!(!is_bundle_name(name), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn bundles_are_private_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("app/diagnostics");
        let (path, bytes) =
            write_bundle(&directory, &json!({ "a": 1 }), "20261005-120000").unwrap();
        assert_eq!(
            path,
            directory.join("openstreamhub-diagnostics-20261005-120000.json")
        );
        assert_eq!(bytes, fs::metadata(&path).unwrap().len());
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&directory), 0o700);
        let error = write_bundle(&directory, &json!({ "a": 2 }), "20261005-120000").unwrap_err();
        assert!(error.contains("already exists"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{\n  \"a\": 1\n}\n");
        // No temporary files are left behind.
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert!(
            write_bundle(&directory, &json!({ "a": 3 }), "20261005-120001")
                .unwrap()
                .0
                .exists()
        );
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
    }

    #[test]
    fn retention_keeps_the_newest_bundles_only() {
        let directory = tempfile::tempdir().unwrap();
        let directory = directory.path();
        for index in 0..13 {
            fs::write(
                directory.join(file_name(&format!("20261005-1200{index:02}"))),
                "{}",
            )
            .unwrap();
        }
        for unrelated in [
            "notes.json",
            "openstreamhub-diagnostics-old.json",
            ".openstreamhub-diagnostics-x.tmp",
        ] {
            fs::write(directory.join(unrelated), "").unwrap();
        }
        fs::create_dir(directory.join(file_name("20000101-000000"))).unwrap();
        prune(directory, KEEP_BUNDLES);
        let mut names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let kept = names.iter().filter(|name| name.contains("-1200")).count();
        assert_eq!(kept, KEEP_BUNDLES);
        assert!(!names.contains(&file_name("20261005-120002")));
        assert!(names.contains(&file_name("20261005-120003")));
        assert!(names.contains(&file_name("20261005-120012")));
        for kept in [
            "notes.json",
            "openstreamhub-diagnostics-old.json",
            ".openstreamhub-diagnostics-x.tmp",
        ] {
            assert!(names.iter().any(|name| name == kept), "{kept}");
        }
        assert!(directory.join(file_name("20000101-000000")).is_dir());
    }

    #[test]
    fn create_writes_a_bundle_with_the_fake_probe() {
        let root = tempfile::tempdir().unwrap();
        let result = create(
            request(true, false, 0.0),
            root.path(),
            &Running::default(),
            &|_| false,
            Ok(PathBuf::from("/opt/masterhub-service")),
            &mut rig(),
        )
        .unwrap();
        let written: Value = serde_json::from_slice(&fs::read(&result.path).unwrap()).unwrap();
        assert_eq!(written["schema_version"], 1);
        assert_eq!(
            written["app"]["service_binary"]["path"],
            "/opt/masterhub-service"
        );
        assert_eq!(result.summary.devices, 5);
        assert!(result.summary.redacted);
        assert!(Path::new(&result.path).starts_with(root.path().join(DIRECTORY)));
    }

    #[test]
    fn os_version_parsers() {
        let plist = "<dict><key>ProductName</key><string>macOS</string><key>ProductVersion</key>\n\t<string>27.0</string></dict>";
        assert_eq!(
            plist_string(plist, "ProductVersion").as_deref(),
            Some("27.0")
        );
        assert_eq!(
            os_release_name("NAME=x\nPRETTY_NAME=\"Ubuntu 26.04 LTS\"\n").as_deref(),
            Some("Ubuntu 26.04 LTS")
        );
    }
}
