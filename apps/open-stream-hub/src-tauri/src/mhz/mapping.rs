//! Maps a parsed official profile onto profile schema 2.
//!
//! Only the actions in the mapping table of
//! `docs/research/masterhub-mhz-format.md` are produced. Everything else
//! becomes an unbound slot with a report row, and nothing is guessed: no
//! `launch_app` is ever produced, no serial number is stored, and values in
//! the report are redacted (basename for paths, `scheme://host` for URLs,
//! `<N chars>` for text).

use super::json::Node;
use cooltek_sdk_service::{
    Action, Binding, DialBinding, DialFunction, Gesture, KeypadBinding, KeypadLayout, KeypadPage,
    KeypadTile, MediaCommand, Modifier, Profile, ProfileDirection, ServiceModule, SystemCommand,
    is_invisible_text_character,
};
use serde::Serialize;
use std::collections::BTreeMap;

/// The official file stores no step size; every imported volume step uses this.
pub const VOLUME_STEP: i16 = 5;
const MAX_LABEL_BYTES: usize = 128;
/// Report rows kept at most; the counts still include every control.
pub const MAX_REPORT_ROWS: usize = 1000;
/// Distinct serial-like identifiers a file may carry. More is refused, so
/// scrubbing stays bounded (at most this many substring checks per value).
pub const MAX_SERIALS: usize = 64;
/// Bytes of pasted text and URLs one import may bind; later ones are left
/// unbound so the profile stays within `Profile::MAX_JSON_BYTES`.
pub const MAX_PAYLOAD_BYTES: usize = 400 * 1024;
/// Largest serialized report; rows past it are counted but not listed.
pub const MAX_REPORT_BYTES: usize = 512 * 1024;
/// File extensions shown in a redacted program path.
const PROGRAM_EXTENSIONS: &[&str] = &[
    "exe",
    "lnk",
    "bat",
    "cmd",
    "app",
    "msi",
    "com",
    "ps1",
    "sh",
    "jar",
    "url",
    "scr",
    "vbs",
    "appref-ms",
];
const FALLBACK_NAME: &str = "Imported profile";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Imported,
    ImportedLossy,
    UnboundUnsupported,
    UnboundDangerous,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Keypad,
    Knobs,
    Dial,
    Rollers,
    Faders,
}

/// One source control (or side) and what became of it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReportRow {
    pub section: Section,
    /// Position only, for example `Keypad · page 1 · key 5`. Never a serial.
    pub path: String,
    /// The official action id, or `unknown` for an id outside the catalogue.
    pub official_action: String,
    pub outcome: Outcome,
    pub reason: String,
    /// A redacted value: a basename, `scheme://host`, or `<N chars>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub high_risk: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub imported: usize,
    pub imported_lossy: usize,
    pub unbound_unsupported: usize,
    pub unbound_dangerous: usize,
}

impl Counts {
    fn sub(&mut self, outcome: Outcome) {
        let count = match outcome {
            Outcome::Imported => &mut self.imported,
            Outcome::ImportedLossy => &mut self.imported_lossy,
            Outcome::UnboundUnsupported => &mut self.unbound_unsupported,
            Outcome::UnboundDangerous => &mut self.unbound_dangerous,
        };
        *count = count.saturating_sub(1);
    }

    fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Imported => self.imported += 1,
            Outcome::ImportedLossy => self.imported_lossy += 1,
            Outcome::UnboundUnsupported => self.unbound_unsupported += 1,
            Outcome::UnboundDangerous => self.unbound_dangerous += 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CategoryCounts {
    pub category: String,
    #[serde(flatten)]
    pub counts: Counts,
}

/// What an import did. Holds no file path and no serial number.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ImportReport {
    /// The name the new profile was saved under.
    pub profile_name: String,
    /// The official profile name, cleaned.
    pub source_name: String,
    pub totals: Counts,
    /// Counts per module section.
    pub categories: Vec<CategoryCounts>,
    /// Counts per official action id.
    pub actions: Vec<CategoryCounts>,
    /// Rows flagged high risk (text with Return or a line break, web pages,
    /// screenshots, sequences that mix typing or pages with shortcuts).
    pub high_risk: usize,
    pub warnings: Vec<String>,
    pub rows: Vec<ReportRow>,
}

enum Mapping {
    Mapped {
        action: Action,
        lossy: bool,
        reason: String,
        high_risk: bool,
        detail: Option<String>,
    },
    Unbound {
        dangerous: bool,
        reason: String,
        detail: Option<String>,
    },
    Empty,
}

fn unsupported(reason: impl Into<String>) -> Mapping {
    Mapping::Unbound {
        dangerous: false,
        reason: reason.into(),
        detail: None,
    }
}

fn mapped(action: Action, lossy: bool, reason: impl Into<String>) -> Mapping {
    Mapping::Mapped {
        action,
        lossy,
        reason: reason.into(),
        high_risk: false,
        detail: None,
    }
}

const CORE: &[&str] = &[
    "multi-tasking",
    "lighting-effect",
    "profile",
    "brightness",
    "timer",
    "mh-zoom-in-out",
    "new-folder",
    "create-new-page",
    "previous-page",
    "next-page",
    "undo-redo",
    "switch-desktop",
    "media-fast-back-forward",
    "media-track-control",
    "volume-control",
    "play-pause",
    "screen-capture",
    "zoom-in-out",
    "scroll-up-down",
    "web-url",
    "app-files",
    "text",
    "hot-key",
    "clock",
];
const OBS: &[&str] = &[
    "record-start-stop",
    "record-continue-pause",
    "stream",
    "virtual-camera",
    "replay-buffer",
    "replay-buffer-save",
    "scene-collection",
    "scenes",
    "source-visibility",
    "obs-profile",
    "audio-mixer",
    "media-source",
    "studio-mode",
    "transition",
    "filter",
    "screenshot",
    "scene-transition",
];
const TWITCH: &[&str] = &[
    "chat-message",
    "clear-chat",
    "followers-chat",
    "sub-chat",
    "emote-chat",
    "slow-chat",
    "shield-mode",
    "play-ad",
    "stream-game-title",
    "create-stream-marker",
    "create-clip",
    "open-last-clip",
];
const MASTER_CTRL: &[&str] = &[
    "ctrl-scenes",
    "ctrl-lighting-profile",
    "ctrl-cooling-profile",
];
/// Presets known to run a fixed Photoshop script instead of a shortcut.
const SCRIPT_PRESETS: &[&str] = &["ps-flatten-image", "ps-tool-auto-masking-tool"];

fn id_like(func: &str, prefix: &str) -> bool {
    func.len() > prefix.len()
        && func.len() <= 64
        && func.starts_with(prefix)
        && func
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_preset(func: &str) -> bool {
    ["ps-", "ai-", "pr-"]
        .iter()
        .any(|prefix| id_like(func, prefix))
}

/// The id shown in the report: a closed catalogue id, a prefix bucket such
/// as `ps-*` or `discord-*` (the free-form suffix is never echoed), or
/// `unknown`. This also bounds the per-action counts.
fn official_id(func: &str) -> String {
    if CORE.contains(&func)
        || OBS.contains(&func)
        || TWITCH.contains(&func)
        || MASTER_CTRL.contains(&func)
        || SCRIPT_PRESETS.contains(&func)
    {
        return func.to_owned();
    }
    for prefix in ["ps-", "ai-", "pr-", "streamlabs-", "discord-"] {
        if id_like(func, prefix) {
            return format!("{prefix}*");
        }
    }
    "unknown".to_owned()
}

/// A serial or text reduced to its letters and digits, compatibility forms
/// folded (NFKC), lowercased: `MHBS-01AA 1234`, `mhbs01aa1234` and
/// `ＭＨＢＳ０１ＡＡ１２３４` compare equal.
fn normalize_serial(text: &str) -> String {
    if text.is_ascii() {
        return text
            .bytes()
            .filter(u8::is_ascii_alphanumeric)
            .map(|byte| byte.to_ascii_lowercase() as char)
            .collect();
    }
    // NFKC folds compatibility forms (fullwidth, mathematical alphanumerics,
    // circled letters, …) to their plain letters and digits.
    icu_normalizer::ComposingNormalizerBorrowed::new_nfkc()
        .normalize(text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// `%XX` escapes decoded (lossily), for serial checks on URLs.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| bytes.get(at).and_then(|b| (*b as char).to_digit(16));
        match (bytes[index], hex(index + 1), hex(index + 2)) {
            (b'%', Some(high), Some(low)) => {
                out.push((high * 16 + low) as u8);
                index += 3;
            }
            (byte, _, _) => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Bytes a string takes in the profile JSON (escaped, with quotes).
fn json_len(text: &str) -> usize {
    serde_json::to_string(text).map_or(text.len(), |encoded| encoded.len())
}

fn class_reason(func: &str) -> String {
    let reason = match func {
        "chat-message" => {
            "Twitch chat messages are not imported: the text would be posted to a network service"
        }
        f if TWITCH.contains(&f) => "Twitch integration is not supported",
        f if OBS.contains(&f) => "OBS integration is not supported",
        f if MASTER_CTRL.contains(&f) => "Master CTRL integration is not supported",
        f if id_like(f, "streamlabs-") => "Streamlabs integration is not supported",
        f if id_like(f, "discord-") => "Discord integration is not supported",
        "switch-desktop"
        | "zoom-in-out"
        | "scroll-up-down"
        | "mh-zoom-in-out"
        | "media-fast-back-forward" => "the stored Windows shortcut has no macOS equivalent",
        "play-pause" => "plays a local audio file by path; not supported",
        "timer" => "timers are not supported",
        "clock" => "clock tiles are not supported",
        "brightness" | "lighting-effect" => "lighting and brightness are not supported yet",
        "create-new-page" => "page creation keys are not imported",
        "previous-page" => "dropped: the folder Back key covers it",
        "new-folder" | "next-page" => "navigation is only imported on keypad pages",
        f if CORE.contains(&f) => "this action is only imported on keys",
        _ => "unknown official action",
    };
    reason.to_owned()
}

/// Strips control and invisible characters, trims, and clamps to
/// `max_bytes` on a character boundary.
pub fn clean(text: &str, max_bytes: usize) -> String {
    let filtered: String = text
        .chars()
        .filter(|c| !c.is_control() && !is_invisible_text_character(*c))
        .collect();
    let mut end = 0;
    for (index, c) in filtered.trim().char_indices() {
        if index + c.len_utf8() > max_bytes {
            break;
        }
        end = index + c.len_utf8();
    }
    filtered.trim()[..end].trim_end().to_owned()
}

fn modifier(name: &str) -> Option<Modifier> {
    Some(match name.to_ascii_lowercase().as_str() {
        "control" | "ctrl" | "leftcontrol" | "rightcontrol" | "lctrl" | "rctrl" => {
            Modifier::Control
        }
        "alt" | "leftalt" | "rightalt" | "option" => Modifier::Option,
        "shift" | "leftshift" | "rightshift" => Modifier::Shift,
        "win" | "leftwin" | "rightwin" | "super" | "leftsuper" | "rightsuper" | "meta"
        | "command" | "cmd" => Modifier::Command,
        _ => return None,
    })
}

/// The closed table of official key names that have one of our keys.
fn key_name(name: &str) -> Option<String> {
    if name.len() == 1 && name.as_bytes()[0].is_ascii_alphanumeric() {
        return Some(name.to_ascii_lowercase());
    }
    let lower = name.to_ascii_lowercase();
    let named = match lower.as_str() {
        " " | "space" => "space",
        "enter" | "return" => "return",
        "tab" => "tab",
        "escape" | "esc" => "escape",
        "backspace" => "delete",
        "delete" | "del" => "forward_delete",
        "home" => "home",
        "end" => "end",
        "pageup" => "page_up",
        "pagedown" => "page_down",
        "left" | "arrowleft" => "arrow_left",
        "right" | "arrowright" => "arrow_right",
        "up" | "arrowup" => "arrow_up",
        "down" | "arrowdown" => "arrow_down",
        other => {
            let number: u8 = other.strip_prefix('f')?.parse().ok()?;
            return (1..=20)
                .contains(&number)
                .then(|| format!("f{number}"))
                .filter(|_| !other[1..].starts_with('0'));
        }
    };
    Some(named.to_owned())
}

/// A short, printable key name for a report reason, or a neutral word.
fn shown_key(name: &str, serial: bool) -> String {
    if !serial
        && !name.is_empty()
        && name.len() <= 16
        && name.bytes().all(|b| b.is_ascii_graphic())
        && !name.contains('"')
    {
        format!("\"{name}\"")
    } else {
        "a key".to_owned()
    }
}

fn redact_url(url: &str) -> String {
    let lower = url.to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if lower.starts_with(scheme) {
            let rest = &url[scheme.len()..];
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let host = authority.rsplit('@').next().unwrap_or("");
            let host: String = host
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
                .take(253)
                .collect();
            return format!("{scheme}{host}");
        }
    }
    format!("<{} chars>", url.chars().count())
}

fn looks_dangerous_url(url: &str) -> bool {
    if url
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || "\"`|<>\\^{}".contains(c))
        || url.contains("$(")
        || url.chars().any(is_invisible_text_character)
    {
        return true;
    }
    // Any scheme other than http or https (file:, javascript:, smb:, …).
    let scheme_end = url.find(':');
    scheme_end.is_some_and(|end| {
        let scheme = &url[..end];
        !scheme.is_empty()
            && scheme.as_bytes()[0].is_ascii_alphabetic()
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-'))
            && !scheme.eq_ignore_ascii_case("http")
            && !scheme.eq_ignore_ascii_case("https")
    })
}

/// The file name of a program path, only when it looks like one: a name
/// with a short extension (`tool.exe`), no shell characters, and not a
/// folder such as a user's home (`C:\\Users\\bob`). Anything else is
/// `<N chars>`. Serials are checked by the caller (`Mapper::row_with`).
fn basename(path: &str) -> String {
    let parts: Vec<&str> = path
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let name = clean(parts.last().copied().unwrap_or(""), 64);
    let extension = name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty() && PROGRAM_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
    });
    let in_home = parts.len() >= 2
        && matches!(
            parts[parts.len() - 2].to_ascii_lowercase().as_str(),
            "users" | "home" | "documents and settings"
        );
    if path.trim().is_empty() {
        "<no file>".to_owned()
    } else if !extension || in_home || name.chars().any(|c| "\"'`&|;<>$%^!*?()[]{}=~".contains(c)) {
        format!("<{} chars>", path.chars().count())
    } else {
        name
    }
}

/// The value of a `Buffer`-shaped image field, or the field itself.
fn has_image(node: &Node) -> bool {
    [
        "imageBuffer",
        "bigImageBuffer",
        "originalImageBuffer",
        "imageIcon",
        "frames",
        "smallFrames",
        "gifPath",
        "gifFileHash",
    ]
    .iter()
    .any(|key| match node.get(key) {
        Node::Object(_) => node.get(key).get("data").is_present(),
        other => other.is_present(),
    })
}

/// The `btnFunc` of a tile or function: "" when absent, and a marker that
/// maps to `unknown` when it is not a string.
fn func_of<'a>(node: &'a Node, key: &str) -> &'a str {
    match node.get(key) {
        Node::String(func) => func.trim_matches(|c: char| c.is_whitespace()),
        Node::Null => "",
        _ => "\u{0}",
    }
}

#[derive(Clone, Copy)]
enum Volume {
    Up,
    Down,
    Mute,
}

fn volume_from_action(node: &Node) -> Option<Volume> {
    match node.str()? {
        "volume-up" => Some(Volume::Up),
        "volume-down" => Some(Volume::Down),
        "volume-mute" | "mute" | "volume-mute-toggle" | "mute-toggle" | "volume-unmute" => {
            Some(Volume::Mute)
        }
        _ => None,
    }
}

fn volume_action(volume: Volume) -> Action {
    match volume {
        Volume::Up => Action::Volume { delta: VOLUME_STEP },
        Volume::Down => Action::Volume {
            delta: -VOLUME_STEP,
        },
        Volume::Mute => Action::ToggleMute,
    }
}

/// Where a binding lives in the profile, so the final size fit can remove it.
#[derive(Clone, Debug, PartialEq)]
enum Location {
    Key {
        page: String,
        slot: usize,
    },
    Flat {
        module: ServiceModule,
        control: u8,
        gesture: Gesture,
    },
    Dial {
        function: usize,
        gesture: Gesture,
    },
}

/// One bound action and its report row.
struct Bound {
    location: Location,
    row: Option<usize>,
    section: Section,
    official: String,
    outcome: Outcome,
    high_risk: bool,
    /// Carries text or URLs (removed first by the final fit).
    payload: bool,
    /// Estimated serialized size.
    bytes: usize,
}

#[cfg(test)]
thread_local! {
    /// Bytes passed to `has_serial` (an operation count for tests).
    pub static SCANNED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Profile serializations made by the final fit.
    pub static FIT_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Mapper {
    bound: Vec<Bound>,
    /// Serial-like identifiers from the file, lowercased. Used only to keep
    /// them out of names, labels, URLs, text and the report; never stored.
    serials: Vec<String>,
    bindings: usize,
    /// Bytes of text and URLs bound so far (`MAX_PAYLOAD_BYTES`).
    payload: usize,
    rows: Vec<ReportRow>,
    /// Rows counted but not kept (`MAX_REPORT_ROWS`).
    omitted: usize,
    totals: Counts,
    categories: BTreeMap<Section, Counts>,
    actions: BTreeMap<String, Counts>,
    high_risk: usize,
    warnings: Vec<String>,
    shortcuts: usize,
    scrubbed: usize,
    images: usize,
}

/// Shortcuts in a bound action, multi-action steps included.
fn shortcut_count(action: &Action) -> usize {
    match action {
        Action::Shortcut { .. } => 1,
        Action::MultiAction { actions } => actions.iter().map(shortcut_count).sum(),
        _ => 0,
    }
}

/// Bytes of pasted text and URLs in a bound action.
fn payload_bytes(action: &Action) -> usize {
    match action {
        Action::Text { text, .. } => json_len(text),
        Action::OpenUrl { url } => json_len(url),
        Action::MultiAction { actions } => actions.iter().map(payload_bytes).sum(),
        _ => 0,
    }
}

impl Mapper {
    /// Whether `text` contains a serial of the file, ignoring case and
    /// separators. At most `MAX_SERIALS` linear substring checks.
    fn has_serial(&self, text: &str) -> bool {
        if self.serials.is_empty() {
            return false;
        }
        #[cfg(test)]
        SCANNED.with(|scanned| scanned.set(scanned.get() + text.len()));
        let normalized = normalize_serial(text);
        self.serials
            .iter()
            .any(|serial| normalized.contains(serial))
    }

    /// A cleaned, clamped name. The raw and the whole cleaned text are
    /// checked for serials before clamping, so a serial cut in half by the
    /// limit never leaves a fragment behind. Its escaped length counts
    /// against the payload budget.
    fn label(&mut self, title: &Node, max_bytes: usize, fallback: &str) -> String {
        let raw = title.str().unwrap_or("");
        // Only what can be stored, plus enough of what follows the cut to
        // hold 64 more letters or digits (the longest serial), is scanned:
        // the stored label is a prefix of `scanned`, so a serial cut by the
        // limit is seen whole however long a separator run splits it, and a
        // huge hostile title costs no more than that.
        let full = clean(raw, usize::MAX);
        let stored = clean(raw, max_bytes).len();
        let mut end = stored;
        let mut letters = 0;
        for (index, c) in full[stored..].char_indices() {
            if letters >= 64 {
                break;
            }
            if c.is_alphanumeric() {
                letters += 1;
            }
            end = stored + index + c.len_utf8();
        }
        let scanned = &full[..end];
        let label = if !scanned.is_empty() && self.has_serial(scanned) {
            self.scrubbed += 1;
            fallback.to_owned()
        } else {
            let cleaned = clean(raw, max_bytes);
            if cleaned.is_empty() {
                fallback.to_owned()
            } else {
                cleaned
            }
        };
        self.payload += json_len(&label);
        label
    }

    fn row(
        &mut self,
        section: Section,
        path: String,
        func: &str,
        outcome: Outcome,
        reason: String,
    ) {
        self.row_with(section, path, func, outcome, reason, (None, false));
    }

    fn texts(action: &Action) -> Vec<&str> {
        match action {
            Action::Text { text, .. } => vec![text.as_str()],
            Action::MultiAction { actions } => actions.iter().flat_map(Self::texts).collect(),
            _ => Vec::new(),
        }
    }

    /// A row with a redacted value and the high-risk mark.
    fn row_with(
        &mut self,
        section: Section,
        path: String,
        func: &str,
        outcome: Outcome,
        reason: String,
        (detail, high_risk): (Option<String>, bool),
    ) -> Option<usize> {
        let official_action = official_id(func);
        self.totals.add(outcome);
        self.categories.entry(section).or_default().add(outcome);
        self.actions
            .entry(official_action.clone())
            .or_default()
            .add(outcome);
        if high_risk {
            self.high_risk += 1;
        }
        if self.rows.len() >= MAX_REPORT_ROWS {
            self.omitted += 1;
            return None;
        }
        // Nothing in the report may carry a serial from the file.
        let detail = detail.map(|detail| {
            if self.has_serial(&detail) {
                format!("<{} chars>", detail.chars().count())
            } else {
                detail
            }
        });
        let reason = if self.has_serial(&reason) {
            "the value contains a device serial number".to_owned()
        } else {
            reason
        };
        self.rows.push(ReportRow {
            section,
            path,
            official_action,
            outcome,
            reason,
            detail,
            high_risk,
        });
        Some(self.rows.len() - 1)
    }

    /// Records `mapping` and returns its action when a binding may be added.
    /// The binding cap and the payload budget are checked before pasted
    /// text is scanned for serials, so text that would be left unbound
    /// anyway costs nothing.
    fn settle(
        &mut self,
        section: Section,
        path: String,
        func: &str,
        mapping: Mapping,
        location: Location,
    ) -> Option<Action> {
        match mapping {
            Mapping::Empty => None,
            Mapping::Unbound {
                dangerous,
                reason,
                detail,
            } => {
                let outcome = if dangerous {
                    Outcome::UnboundDangerous
                } else {
                    Outcome::UnboundUnsupported
                };
                self.row_with(section, path, func, outcome, reason, (detail, false));
                None
            }
            Mapping::Mapped { ref action, .. }
                if self.bindings < Profile::MAX_BINDINGS
                    && self.payload + payload_bytes(action) <= MAX_PAYLOAD_BYTES
                    && Self::texts(action).iter().any(|text| self.has_serial(text)) =>
            {
                let chars: usize = Self::texts(action).iter().map(|t| t.chars().count()).sum();
                self.row_with(
                    section,
                    path,
                    func,
                    Outcome::UnboundUnsupported,
                    "the text contains a device serial number".into(),
                    (Some(format!("<{chars} chars>")), false),
                );
                None
            }
            Mapping::Mapped { .. } if self.bindings >= Profile::MAX_BINDINGS => {
                let reason = format!(
                    "left unbound: the profile reached {} bindings",
                    Profile::MAX_BINDINGS
                );
                self.row(section, path, func, Outcome::UnboundUnsupported, reason);
                None
            }
            Mapping::Mapped { ref action, .. }
                if payload_bytes(action) > 0
                    && self.payload + payload_bytes(action) > MAX_PAYLOAD_BYTES =>
            {
                let reason = format!(
                    "left unbound: the imported text and web addresses reached {} KiB",
                    MAX_PAYLOAD_BYTES / 1024
                );
                self.row(section, path, func, Outcome::UnboundUnsupported, reason);
                None
            }
            Mapping::Mapped {
                action,
                lossy,
                reason,
                high_risk,
                detail,
            } => {
                let outcome = if lossy {
                    Outcome::ImportedLossy
                } else {
                    Outcome::Imported
                };
                self.bindings += 1;
                self.payload += payload_bytes(&action);
                // Counted from what is really bound, never from a sequence
                // that was dropped later.
                self.shortcuts += shortcut_count(&action);
                let official = official_id(func);
                let row = self.row_with(section, path, func, outcome, reason, (detail, high_risk));
                let payload = payload_bytes(&action);
                self.bound.push(Bound {
                    location,
                    row,
                    section,
                    official,
                    outcome,
                    high_risk,
                    payload: payload > 0,
                    // Pretty-printed, as the store writes it (the nesting
                    // inside the profile adds a little more).
                    bytes: serde_json::to_vec_pretty(&action).map_or(payload, |bytes| bytes.len())
                        + 64,
                });
                Some(action)
            }
        }
    }

    fn shortcut(&mut self, hotkey: &Node, label: &str) -> Mapping {
        let Node::Array { items, numbers } = hotkey.get("key") else {
            return unsupported("no shortcut is stored");
        };
        if items.is_empty() {
            return unsupported("no shortcut is stored");
        }
        if *numbers > 0 || items.len() > 8 {
            return unsupported("the stored shortcut is malformed");
        }
        let mut modifiers = Vec::new();
        let mut key = None;
        for item in items {
            let Some(name) = item.str() else {
                return unsupported("the stored shortcut is malformed");
            };
            if let Some(modifier) = modifier(name) {
                if !modifiers.contains(&modifier) {
                    modifiers.push(modifier);
                }
            } else if let Some(name) = key_name(name) {
                if key.is_some() {
                    return unsupported(
                        "shortcuts with more than one non-modifier key are not supported",
                    );
                }
                key = Some(name);
            } else {
                return unsupported(format!(
                    "the key {} has no supported macOS key (punctuation and keypad keys are not imported)",
                    shown_key(name, name.len() <= 16 && self.has_serial(name))
                ));
            }
        }
        let Some(key) = key else {
            return unsupported("the shortcut has only modifier keys");
        };
        let action = Action::Shortcut { key, modifiers };
        if let Err(error) = action.validate() {
            return unsupported(error);
        }
        mapped(
            action,
            true,
            format!(
                "{label}: Ctrl-based Windows shortcuts are usually Cmd-based on macOS; review it"
            ),
        )
    }

    fn url(&self, value: &Node) -> Mapping {
        let url = match value {
            Node::String(url) if !url.trim().is_empty() => url,
            Node::Skipped => return unsupported("the web address is too long"),
            _ => return unsupported("no web address is stored"),
        };
        // Longer than any URL we accept: refused before anything is scanned.
        if url.len() > Action::MAX_URL_BYTES {
            return Mapping::Unbound {
                dangerous: false,
                reason: "the web address is too long".into(),
                detail: Some(format!("<{} chars>", url.chars().count())),
            };
        }
        // Checked first, so no branch can show a serial in the detail
        // (a percent-encoded host included).
        let serial =
            self.has_serial(url) || (url.contains('%') && self.has_serial(&percent_decode(url)));
        let detail = Some(if serial {
            format!("<{} chars>", url.chars().count())
        } else {
            redact_url(url)
        });
        let action = Action::OpenUrl { url: url.clone() };
        match action.validate() {
            Ok(()) if serial => Mapping::Unbound {
                dangerous: false,
                reason: "the address contains a device serial number".into(),
                detail: Some(format!("<{} chars>", url.chars().count())),
            },
            Ok(()) => Mapping::Mapped {
                action,
                lossy: true,
                reason: "opens a web page; the official https:// prefixing is not reproduced"
                    .into(),
                high_risk: true,
                detail,
            },
            Err(_) if looks_dangerous_url(url) => Mapping::Unbound {
                dangerous: true,
                reason: "the address could run commands or open a local file; never imported"
                    .into(),
                detail,
            },
            Err(error) => Mapping::Unbound {
                dangerous: false,
                reason: format!("the address does not pass the URL rules: {error}"),
                detail,
            },
        }
    }

    fn text(&self, content: &Node, after_enter: &Node) -> Mapping {
        let text = match content {
            Node::String(text) if !text.is_empty() => text,
            Node::Skipped => {
                return unsupported(format!(
                    "the text is longer than {} bytes",
                    Action::MAX_TEXT_BYTES
                ));
            }
            _ => return unsupported("no text is stored"),
        };
        let detail = Some(format!("<{} chars>", text.chars().count()));
        // `afterEnter` must be a boolean (or absent); any other value is
        // ambiguous, so the text is left unbound rather than guessed.
        let press_enter = match after_enter {
            Node::Bool(value) => *value,
            Node::Null => false,
            _ => {
                return Mapping::Unbound {
                    dangerous: false,
                    reason: "the stored afterEnter value is not true or false; left unbound".into(),
                    detail,
                };
            }
        };
        let action = Action::Text {
            text: text.clone(),
            press_enter,
        };
        if let Err(error) = action.validate() {
            let hidden = text.chars().any(|c| {
                is_invisible_text_character(c) || (c.is_control() && !matches!(c, '\n' | '\t'))
            });
            return Mapping::Unbound {
                dangerous: hidden,
                reason: if hidden {
                    "the text contains invisible or control characters, so it could differ from what you review; never imported".into()
                } else {
                    error
                },
                detail,
            };
        }
        let high_risk = press_enter || text.contains('\n');
        let reason = match (press_enter, text.contains('\n')) {
            (true, _) => {
                "pastes text and presses Return: in a terminal it runs commands".to_owned()
            }
            (false, true) => {
                "pastes text with a line break: in a terminal it runs commands".to_owned()
            }
            (false, false) => "pastes text".to_owned(),
        };
        Mapping::Mapped {
            action,
            lossy: false,
            reason,
            high_risk,
            detail,
        }
    }

    fn multi(&mut self, list: &Node) -> Mapping {
        let steps = list.items();
        if list.numbers() > 0 {
            return unsupported("the multi-step action is malformed");
        }
        if steps.is_empty() {
            return unsupported("the multi-step action has no steps");
        }
        if steps.len() > Action::MAX_MULTI_ACTION_STEPS {
            return unsupported(format!(
                "more than {} steps are not supported",
                Action::MAX_MULTI_ACTION_STEPS
            ));
        }
        let mut actions = Vec::new();
        let mut high = false;
        let mut reasons = Vec::new();
        let mut details = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            let func = func_of(step, "action");
            match self.map_action(func, step, true) {
                Mapping::Mapped {
                    action,
                    high_risk,
                    reason,
                    detail,
                    ..
                } => {
                    reasons.push(format!("{}. {reason}", index + 1));
                    details.extend(detail);
                    if steps.len() > 1 && matches!(action, Action::Profile { .. }) {
                        return unsupported(format!(
                            "step {}: a profile switch cannot be a step; the whole sequence is left unbound",
                            index + 1
                        ));
                    }
                    high |= high_risk;
                    actions.push(action);
                }
                Mapping::Unbound {
                    dangerous,
                    reason,
                    detail,
                } => {
                    return Mapping::Unbound {
                        dangerous,
                        reason: format!(
                            "step {}: {reason}; the whole sequence is left unbound",
                            index + 1
                        ),
                        detail,
                    };
                }
                Mapping::Empty => {
                    return unsupported(format!(
                        "step {} has no action; the whole sequence is left unbound",
                        index + 1
                    ));
                }
            }
        }
        let detail = (!details.is_empty()).then(|| details.join(", "));
        if actions.len() == 1 {
            let action = actions.remove(0);
            return Mapping::Mapped {
                action,
                lossy: true,
                reason: format!(
                    "a one-step sequence, imported as a single action: {}",
                    reasons.join("")
                ),
                high_risk: high,
                detail,
            };
        }
        let types = |test: fn(&Action) -> bool| actions.iter().any(test);
        if types(|a| matches!(a, Action::OpenUrl { .. } | Action::Text { .. }))
            && types(|a| matches!(a, Action::Shortcut { .. }))
        {
            high = true;
        }
        let count = actions.len();
        let action = Action::MultiAction { actions };
        if let Err(error) = action.validate() {
            return unsupported(error);
        }
        Mapping::Mapped {
            action,
            lossy: true,
            reason: format!("runs {count} steps in order: {}", reasons.join("; ")),
            high_risk: high,
            detail,
        }
    }

    fn app_files(node: &Node) -> Mapping {
        let path = ["appFilePath", "app", "file"]
            .iter()
            .find_map(|key| node.get(key).str().filter(|p| !p.trim().is_empty()))
            .unwrap_or("");
        let detail = Some(basename(path));
        if node.get("actionAppMode").str() == Some("Close") {
            Mapping::Unbound {
                dangerous: false,
                reason: "closes a program by its Windows path; not supported".into(),
                detail,
            }
        } else {
            Mapping::Unbound {
                dangerous: true,
                reason: "launches a Windows program through a shell; never imported or converted"
                    .into(),
                detail,
            }
        }
    }

    fn preset(&mut self, func: &str, node: &Node) -> Mapping {
        if SCRIPT_PRESETS.contains(&func) {
            return unsupported("runs an application script; not supported");
        }
        if node.get("hotkey").get("key").items().is_empty() {
            return unsupported("the application preset has no stored shortcut");
        }
        self.shortcut(node.get("hotkey"), "application preset")
    }

    /// A key tile or multi-step step.
    fn map_action(&mut self, func: &str, node: &Node, step: bool) -> Mapping {
        match func {
            "" => Mapping::Empty,
            "hot-key" => {
                if node.get("hotKeySettingMode").str() == Some("macro") {
                    unsupported(
                        "recorded macros are not supported (replay could leave keys held down)",
                    )
                } else {
                    self.shortcut(node.get("hotkey"), "Windows shortcut")
                }
            }
            "web-url" => self.url(node.get("url")),
            "text" => self.text(node.get("content"), node.get("afterEnter")),
            "multi-tasking" if step => {
                unsupported("a multi-step action cannot contain another one")
            }
            "multi-tasking" => self.multi(node.get("actionList")),
            "profile" => {
                let direction = match node.get("dropdownOption").index() {
                    Some(0) => ProfileDirection::Next,
                    Some(1) => ProfileDirection::Previous,
                    _ => return unsupported("unknown profile option"),
                };
                let mut reason = match direction {
                    ProfileDirection::Next => "switches to the next saved profile".to_owned(),
                    ProfileDirection::Previous => {
                        "switches to the previous saved profile".to_owned()
                    }
                };
                if node.get("profileId").is_present() {
                    reason.push_str("; the official target profile id was dropped");
                }
                mapped(Action::Profile { direction }, true, reason)
            }
            "volume-control" => {
                if node.get("volumeControlProcessId").is_present() {
                    return unsupported("per-application volume is not supported");
                }
                let from_action = volume_from_action(node.get("volumeAction"));
                let from_option = match node.get("dropdownOption").index() {
                    Some(0) => Some(Volume::Up),
                    Some(1) => Some(Volume::Down),
                    Some(2 | 3) => Some(Volume::Mute),
                    _ => None,
                };
                let volume = match (from_action, from_option) {
                    (Some(a), Some(b))
                        if std::mem::discriminant(&a) != std::mem::discriminant(&b) =>
                    {
                        return unsupported("the stored volume settings disagree");
                    }
                    (Some(volume), _) | (None, Some(volume)) => volume,
                    (None, None) => return unsupported("unknown volume option"),
                };
                mapped(
                    volume_action(volume),
                    true,
                    format!("volume step set to {VOLUME_STEP}%"),
                )
            }
            "media-track-control" => {
                let command = match node.get("dropdownOption").index() {
                    Some(4) => MediaCommand::PlayPause,
                    Some(5) => MediaCommand::Stop,
                    Some(2) => MediaCommand::PreviousTrack,
                    Some(3) => MediaCommand::NextTrack,
                    _ => return unsupported("unknown media option"),
                };
                let reason = if command == MediaCommand::Stop {
                    "macOS has no stop media key: stops Apple Music or pauses Spotify, only when one is already running"
                } else {
                    "sent as a macOS media key to whichever app controls playback"
                };
                mapped(Action::Media { command }, false, reason)
            }
            "undo-redo" => {
                let command = match node.get("dropdownOption").index() {
                    Some(0) => SystemCommand::Undo,
                    Some(1) => SystemCommand::Redo,
                    _ => return unsupported("unknown undo/redo option"),
                };
                mapped(
                    Action::System { command },
                    false,
                    "mapped by intent; Windows pressed the stored shortcut",
                )
            }
            "screen-capture" => Mapping::Mapped {
                action: Action::System {
                    command: SystemCommand::Screenshot,
                },
                lossy: true,
                reason: "captures the whole screen to the clipboard (Windows used a region snip)"
                    .into(),
                high_risk: true,
                detail: None,
            },
            "app-files" => Self::app_files(node),
            "new-folder" | "next-page" | "previous-page" | "create-new-page" if step => {
                unsupported("navigation cannot be a step of a sequence")
            }
            func if is_preset(func) => self.preset(func, node),
            func => unsupported(class_reason(func)),
        }
    }

    /// One side (rotation direction) of a knob, roller or dial function.
    /// `owners` are the nodes that configure the side (the control, and for
    /// the dial also the function): per-application volume or a recorded
    /// macro on any of them leaves the side unbound.
    fn map_side(&mut self, func: &str, owners: &[&Node], side: &Node, up: bool) -> Mapping {
        let any = |key: &str, test: &dyn Fn(&Node) -> bool| {
            owners.iter().chain([&side]).any(|node| test(node.get(key)))
        };
        if (func == "hot-key" || is_preset(func))
            && any("hotKeySettingMode", &|mode| mode.str() == Some("macro"))
        {
            return unsupported(
                "recorded macros are not supported (replay could leave keys held down)",
            );
        }
        match func {
            "" => Mapping::Empty,
            "volume-control" => {
                if any("volumeControlProcessId", &Node::is_present) {
                    return unsupported("per-application volume is not supported");
                }
                let volume = match volume_from_action(side.get("volumeAction")) {
                    Some(Volume::Mute) => {
                        return unsupported(
                            "mute on a rotating control would toggle on every detent; not imported",
                        );
                    }
                    Some(volume) => volume,
                    None if up => Volume::Up,
                    None => Volume::Down,
                };
                mapped(
                    volume_action(volume),
                    true,
                    format!(
                        "volume step set to {VOLUME_STEP}%; the physical direction is unverified"
                    ),
                )
            }
            "hot-key" => self.shortcut(side.get("hotkey"), "Windows shortcut"),
            "app-files" => Self::app_files(side),
            func if is_preset(func) => self.preset(func, side),
            func => unsupported(class_reason(func)),
        }
    }

    /// The first section of `key`; others are reported as dropped.
    fn first<'a>(&mut self, root: &'a Node, key: &str, what: &str) -> Option<&'a Node> {
        let sections = root.get(key).entries();
        if sections.len() > 1 {
            self.warnings.push(format!(
                "The file has {} {what} sections; only the first was imported.",
                sections.len()
            ));
        }
        sections.first().map(|(_, section)| section)
    }

    fn rotations(
        &mut self,
        section: Section,
        module: ServiceModule,
        controls: &[(&str, u8, &str)],
        sides: [(&str, Gesture, bool, &str); 2],
        node: &Node,
        flat: &mut Vec<Binding>,
    ) {
        for (key, control, name) in controls {
            let control_node = node.get(key);
            let func = func_of(control_node, "btnFunc");
            if func.is_empty() {
                continue;
            }
            if has_image(control_node) {
                self.images += 1;
            }
            let functions = control_node.get("functionList").items().len();
            if section == Section::Knobs && functions > 1 {
                self.warnings.push(format!(
                    "{name} had {functions} functions; only the current one was imported."
                ));
            }
            for (side_key, gesture, up, label) in sides {
                let mapping = self.map_side(func, &[control_node], control_node.get(side_key), up);
                let path = format!("{name} · {label}");
                let location = Location::Flat {
                    module,
                    control: *control,
                    gesture,
                };
                if let Some(action) = self.settle(section, path, func, mapping, location) {
                    flat.push(Binding {
                        module,
                        control: *control,
                        gesture,
                        serial_number: None,
                        action,
                    });
                }
            }
        }
    }

    fn dial(&mut self, dial: &Node) -> Option<Vec<DialFunction>> {
        if dial.get("functionList").numbers() > 0 {
            self.row(
                Section::Dial,
                "Dial".into(),
                func_of(dial, "btnFunc"),
                Outcome::UnboundUnsupported,
                "the dial function list is malformed (it holds numbers among its functions)".into(),
            );
            return None;
        }
        let listed = dial.get("functionList").items();
        let sources: Vec<&Node> = if listed.is_empty() {
            vec![dial]
        } else {
            listed.iter().collect()
        };
        if has_image(dial) {
            self.images += 1;
        }
        let mut functions: Vec<DialFunction> = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            let func = func_of(source, "btnFunc");
            if index >= DialFunction::MAX_FUNCTIONS {
                if !func.is_empty() {
                    self.row(
                        Section::Dial,
                        format!("Dial · function {}", index + 1),
                        func,
                        Outcome::UnboundUnsupported,
                        format!(
                            "only {} dial functions are supported",
                            DialFunction::MAX_FUNCTIONS
                        ),
                    );
                }
                continue;
            }
            let fallback = format!("Function {}", index + 1);
            let base = self.label(
                source.get("btnName"),
                DialFunction::MAX_NAME_BYTES,
                &fallback,
            );
            let mut name = base.clone();
            let mut copy = 2;
            while functions.iter().any(|f| f.name == name) {
                let suffix = format!(" {copy}");
                name = clean(&base, DialFunction::MAX_NAME_BYTES - suffix.len()) + &suffix;
                copy += 1;
            }
            // The suffix can complete a serial (`…000` + ` 2`).
            if name != fallback && self.has_serial(&name) {
                self.scrubbed += 1;
                name = fallback.clone();
                let mut copy = 2;
                while functions.iter().any(|f| f.name == name) {
                    name = format!("{fallback} {copy}");
                    copy += 1;
                }
            }
            let mut bindings = Vec::new();
            for (side_key, gesture, up, label) in [
                ("right", Gesture::Right, true, "right"),
                ("left", Gesture::Left, false, "left"),
            ] {
                let mapping = self.map_side(func, &[dial, source], source.get(side_key), up);
                let path = format!("Dial · function {} · {label}", index + 1);
                let location = Location::Dial {
                    function: index,
                    gesture,
                };
                if let Some(action) = self.settle(Section::Dial, path, func, mapping, location) {
                    bindings.push(DialBinding {
                        gesture,
                        serial_number: None,
                        action,
                    });
                }
            }
            functions.push(DialFunction { name, bindings });
        }
        functions
            .iter()
            .any(|function| !function.bindings.is_empty())
            .then_some(functions)
    }

    fn faders(&mut self, faders: &Node, flat: &mut Vec<Binding>) {
        if faders.numbers() > 0 {
            // Dropped numbers would shift every later fader.
            self.row(
                Section::Faders,
                "Faders".into(),
                "",
                Outcome::UnboundUnsupported,
                "the fader list is malformed (it holds numbers among its faders)".into(),
            );
            return;
        }
        for control in 0..5u8 {
            let fader = match faders {
                Node::Array { items, .. } => items.get(control as usize).unwrap_or(&Node::Null),
                other => other.get(&control.to_string()),
            };
            let func = func_of(fader, "btnFunc");
            let path = format!("Fader {}", control + 1);
            let mapping = match func {
                "" => Mapping::Empty,
                "volume-control" if fader.get("volumeControlProcessId").is_present() => {
                    unsupported("per-application volume is not supported")
                }
                "volume-control" => mapped(
                    Action::VolumeFromPosition,
                    true,
                    "sets the output volume from the fader position; the official volume range is ignored",
                ),
                func => unsupported(class_reason(func)),
            };
            let location = Location::Flat {
                module: ServiceModule::M005S,
                control,
                gesture: Gesture::PositionChanged,
            };
            if let Some(action) = self.settle(Section::Faders, path, func, mapping, location) {
                flat.push(Binding {
                    module: ServiceModule::M005S,
                    control,
                    gesture: Gesture::PositionChanged,
                    serial_number: None,
                    action,
                });
            }
        }
    }

    /// Assigns each tile a slot: its `id` when valid and free, else its
    /// position, else the first free slot. Tiles past 15 are returned apart.
    fn slots(tiles: &[Node]) -> (Vec<(usize, &Node)>, Vec<&Node>) {
        let mut used = [false; KeypadLayout::KEY_COUNT];
        let mut placed = Vec::new();
        let mut overflow = Vec::new();
        for (position, tile) in tiles.iter().enumerate() {
            let id = tile
                .get("id")
                .index()
                .map(|id| id as usize)
                .filter(|id| *id < KeypadLayout::KEY_COUNT && !used[*id]);
            let slot = id
                .or(Some(position).filter(|p| *p < KeypadLayout::KEY_COUNT && !used[*p]))
                .or_else(|| used.iter().position(|taken| !taken));
            match slot {
                Some(slot) => {
                    used[slot] = true;
                    placed.push((slot, tile));
                }
                None => overflow.push(tile),
            }
        }
        placed.sort_by_key(|(slot, _)| *slot);
        (placed, overflow)
    }

    /// Reports every non-empty tile of a page that is not imported.
    /// Every non-empty tile counts: extra tiles past 15 and the children of
    /// folders on the dropped page too.
    fn drop_page(&mut self, tiles: &[Node], prefix: &str, reason: &str) {
        let (placed, overflow) = Self::slots(tiles);
        for (slot, tile) in placed {
            let func = func_of(tile, "btnFunc");
            if func.is_empty() {
                continue;
            }
            let path = format!("{prefix} · key {}", slot + 1);
            self.row(
                Section::Keypad,
                path.clone(),
                func,
                Outcome::UnboundUnsupported,
                reason.to_owned(),
            );
            if func == "new-folder" {
                self.drop_page(
                    tile.get("folder").items(),
                    &format!("{path} folder"),
                    reason,
                );
            }
        }
        for tile in overflow {
            let func = func_of(tile, "btnFunc");
            if !func.is_empty() {
                self.row(
                    Section::Keypad,
                    format!("{prefix} · extra key"),
                    func,
                    Outcome::UnboundUnsupported,
                    reason.to_owned(),
                );
            }
        }
    }

    /// Builds one keypad page and its folder pages. Returns the page and
    /// whether a `next-page` key linked the following page.
    #[allow(clippy::too_many_arguments)]
    fn page(
        &mut self,
        id: &str,
        name: String,
        depth: usize,
        tiles: &[Node],
        next_page: Option<&str>,
        folders: bool,
        prefix: &str,
        pages: &mut Vec<KeypadPage>,
        reserved: &mut usize,
    ) -> (KeypadPage, bool) {
        let mut slots: Vec<Option<KeypadTile>> = vec![None; KeypadLayout::KEY_COUNT];
        let mut linked = false;
        let (placed, overflow) = Self::slots(tiles);
        for tile in overflow {
            let func = func_of(tile, "btnFunc");
            if !func.is_empty() {
                self.row(
                    Section::Keypad,
                    format!("{prefix} · extra key"),
                    func,
                    Outcome::UnboundUnsupported,
                    "a page holds at most 15 keys".into(),
                );
            }
        }
        for (slot, tile) in placed {
            let func = func_of(tile, "btnFunc");
            if func.is_empty() {
                continue;
            }
            if has_image(tile) {
                self.images += 1;
            }
            let path = format!("{prefix} · key {}", slot + 1);
            if depth > 0 && slot == 0 {
                let icon = tile.get("iconPath").str().unwrap_or("");
                if !(func == "back" || func == "back-to-master" || icon.contains("back-to-master"))
                {
                    self.row(
                        Section::Keypad,
                        path,
                        func,
                        Outcome::UnboundUnsupported,
                        "key 1 of a folder page is reserved for Back".into(),
                    );
                }
                continue;
            }
            let label = self.label(
                tile.get("title"),
                MAX_LABEL_BYTES,
                &format!("Key {}", slot + 1),
            );
            let unbound = |label: String| {
                Some(KeypadTile::Action {
                    label,
                    bindings: Vec::new(),
                })
            };
            match func {
                "new-folder" if folders && tile.get("folder").numbers() > 0 => {
                    let children = tile.get("folder").items();
                    self.row(
                        Section::Keypad,
                        path.clone(),
                        func,
                        Outcome::UnboundUnsupported,
                        "the folder is malformed (it holds numbers among its keys)".into(),
                    );
                    self.drop_page(
                        children,
                        &format!("{path} folder"),
                        "inside a malformed folder",
                    );
                    slots[slot] = unbound(label);
                }
                "new-folder" if folders => {
                    let children = tile.get("folder").items();
                    if depth + 1 > KeypadLayout::MAX_DEPTH || *reserved >= KeypadLayout::MAX_PAGES {
                        let reason = format!(
                            "the folder would exceed {} levels or {} pages",
                            KeypadLayout::MAX_DEPTH,
                            KeypadLayout::MAX_PAGES
                        );
                        self.row(
                            Section::Keypad,
                            path.clone(),
                            func,
                            Outcome::UnboundUnsupported,
                            reason,
                        );
                        self.drop_page(
                            children,
                            &format!("{path} folder"),
                            "inside a folder that could not be imported",
                        );
                        slots[slot] = unbound(label);
                        continue;
                    }
                    *reserved += 1;
                    let child_id = format!("{id}_key{}", slot + 1);
                    self.row(
                        Section::Keypad,
                        path.clone(),
                        func,
                        Outcome::Imported,
                        "folder".into(),
                    );
                    let (child, _) = self.page(
                        &child_id,
                        label,
                        depth + 1,
                        children,
                        None,
                        false,
                        &format!("{path} folder"),
                        pages,
                        reserved,
                    );
                    pages.push(child);
                    slots[slot] = Some(KeypadTile::Folder { page_id: child_id });
                }
                "new-folder" => {
                    self.row(
                        Section::Keypad,
                        path.clone(),
                        func,
                        Outcome::UnboundUnsupported,
                        "folders inside folders are not imported (the official app supports one level)".into(),
                    );
                    self.drop_page(
                        tile.get("folder").items(),
                        &format!("{path} folder"),
                        "inside a nested folder that was not imported",
                    );
                    slots[slot] = unbound(label);
                }
                "next-page" => {
                    let reason = match next_page {
                        _ if linked => Some("only the first next-page key of a page is linked"),
                        None => Some("there is no following page"),
                        Some(_)
                            if depth + 1 > KeypadLayout::MAX_DEPTH
                                || *reserved >= KeypadLayout::MAX_PAGES =>
                        {
                            Some("the next page would exceed the folder limits")
                        }
                        Some(_) => None,
                    };
                    match (reason, next_page) {
                        (None, Some(next_id)) => {
                            linked = true;
                            *reserved += 1;
                            self.row(
                                Section::Keypad,
                                path,
                                func,
                                Outcome::ImportedLossy,
                                "the next page became a folder; its Back key returns here".into(),
                            );
                            slots[slot] = Some(KeypadTile::Folder {
                                page_id: next_id.to_owned(),
                            });
                        }
                        (reason, _) => {
                            let reason = reason.unwrap_or("there is no following page").to_owned();
                            self.row(
                                Section::Keypad,
                                path,
                                func,
                                Outcome::UnboundUnsupported,
                                reason,
                            );
                            slots[slot] = unbound(label);
                        }
                    }
                }
                "previous-page" | "create-new-page" => {
                    self.row(
                        Section::Keypad,
                        path,
                        func,
                        Outcome::UnboundUnsupported,
                        class_reason(func),
                    );
                    slots[slot] = unbound(label);
                }
                _ => {
                    let mapping = self.map_action(func, tile, false);
                    let bindings = self
                        .settle(
                            Section::Keypad,
                            path,
                            func,
                            mapping,
                            Location::Key {
                                page: id.to_owned(),
                                slot,
                            },
                        )
                        .map(|action| {
                            vec![KeypadBinding {
                                gesture: Gesture::Pressed,
                                serial_number: None,
                                action,
                            }]
                        })
                        .unwrap_or_default();
                    slots[slot] = Some(KeypadTile::Action { label, bindings });
                }
            }
        }
        (
            KeypadPage {
                id: id.to_owned(),
                name,
                slots,
                artwork: Default::default(),
            },
            linked,
        )
    }

    fn keypad(&mut self, root: &Node) -> KeypadLayout {
        let sections = root.get("keypad").entries();
        let non_empty = |pages: &Node| {
            pages.items().iter().any(|page| {
                page.items()
                    .iter()
                    .any(|tile| !func_of(tile, "btnFunc").is_empty())
            })
        };
        let chosen = sections
            .iter()
            .position(|(_, pages)| non_empty(pages))
            .unwrap_or(0);
        if sections.len() > 1 {
            self.warnings.push(format!(
                "The file has {} keypad sections; only one was imported.",
                sections.len()
            ));
        }
        let source: &[Node] = sections
            .get(chosen)
            .map(|(_, pages)| pages.items())
            .unwrap_or(&[]);
        if sections
            .get(chosen)
            .is_some_and(|(_, pages)| pages.numbers() > 0)
        {
            // Dropped numbers would shift every later page.
            for (index, tiles) in source.iter().enumerate() {
                self.drop_page(
                    tiles.items(),
                    &format!("Keypad · page {}", index + 1),
                    "the keypad page list is malformed (it holds numbers among its pages)",
                );
            }
            return KeypadLayout {
                pages: vec![KeypadPage {
                    id: KeypadLayout::ROOT_PAGE_ID.into(),
                    name: "Root".into(),
                    slots: vec![None; KeypadLayout::KEY_COUNT],
                    artwork: Default::default(),
                }],
            };
        }
        let mut pages = Vec::new();
        let mut reserved = 1;
        let mut linked = true;
        for (index, tiles) in source.iter().enumerate() {
            let malformed = tiles.numbers() > 0;
            let tiles = tiles.items();
            let prefix = format!("Keypad · page {}", index + 1);
            if malformed {
                // Dropped numbers would shift the position of every tile.
                self.drop_page(
                    tiles,
                    &prefix,
                    "this page is malformed (it holds numbers among its keys)",
                );
                linked = false;
                continue;
            }
            if !linked {
                self.drop_page(
                    tiles,
                    &prefix,
                    "this page is not reachable: the page before it has no linked next-page key",
                );
                continue;
            }
            let id = if index == 0 {
                KeypadLayout::ROOT_PAGE_ID.to_owned()
            } else {
                format!("page{}", index + 1)
            };
            let next_id = format!("page{}", index + 2);
            // A malformed following page is dropped, so it is never offered.
            let next = (index + 1 < source.len() && source[index + 1].numbers() == 0)
                .then_some(next_id.as_str());
            let name = if index == 0 {
                "Root".to_owned()
            } else {
                format!("Page {}", index + 1)
            };
            let (page, next_linked) = self.page(
                &id,
                name,
                index,
                tiles,
                next,
                true,
                &prefix,
                &mut pages,
                &mut reserved,
            );
            pages.push(page);
            linked = next_linked;
        }
        if !pages
            .iter()
            .any(|page| page.id == KeypadLayout::ROOT_PAGE_ID)
        {
            pages.push(KeypadPage {
                id: KeypadLayout::ROOT_PAGE_ID.into(),
                name: "Root".into(),
                slots: vec![None; KeypadLayout::KEY_COUNT],
                artwork: Default::default(),
            });
        }
        KeypadLayout { pages }
    }
}

/// Every serial-like identifier in the file (the entry name's serial, every
/// module section key, and the value of any `…serialNumber` key anywhere), normalized
/// (`normalize_serial`) and kept when 8 to 64 ASCII letters and digits long
/// and not `undefined`, with at least one digit (every real serial shape
/// has digits, so setting names such as `brightness` never count). More than `MAX_SERIALS` rejects the file, so
/// the scrubbing work stays bounded; nothing is ever silently dropped.
fn collect_serials(root: &Node, entry_serial: &str) -> Result<Vec<String>, String> {
    let mut found = std::collections::BTreeSet::new();
    let mut add = |candidate: &str| {
        // No serial is anywhere near this long; skipping keeps hostile
        // values from costing a normalization.
        if candidate.len() > 256 {
            return true;
        }
        let candidate = normalize_serial(candidate.trim());
        if (8..=64).contains(&candidate.len())
            && candidate.bytes().all(|b| b.is_ascii_alphanumeric())
            && candidate.bytes().any(|b| b.is_ascii_digit())
            && candidate != "undefined"
        {
            found.insert(candidate);
        }
        found.len() <= MAX_SERIALS
    };
    // A serial stored as a JSON number is matched by its decimal digits.
    let text_of = |node: &Node| match node {
        Node::String(text) => Some(text.clone()),
        Node::Number(value) if value.fract() == 0.0 && value.abs() < 1e16 => {
            Some(format!("{}", *value as i64))
        }
        _ => None,
    };
    let too_many = || {
        format!(
            "the file names more than {MAX_SERIALS} device serial numbers; it is not a MasterHUB profile export"
        )
    };
    if !add(entry_serial) {
        return Err(too_many());
    }
    if let Some(serial) = text_of(root.get("serialNumber"))
        && !add(&serial)
    {
        return Err(too_many());
    }
    for key in [
        "knobsList",
        "keypad",
        "keypadBtnConfig",
        "rollerBlock",
        "dialEncoder",
        "sliders5List",
        "touch46s",
        "touch50s",
    ] {
        for (serial, _) in root.get(key).entries() {
            if !add(serial) {
                return Err(too_many());
            }
        }
    }
    // Lighting is keyed by module type, then by serial, except for the
    // whole-device settings, whose keys are setting names.
    for (kind, by_serial) in root.get("deviceLightingEffect").entries() {
        if matches!(kind.as_str(), "wholeDevice" | "currentLightingEffectDevice") {
            continue;
        }
        for (serial, _) in by_serial.entries() {
            if !add(serial) {
                return Err(too_many());
            }
        }
    }
    // Anywhere in the file (the device inventory, keypad tiles, folder
    // children, sequence steps, …): the value of every key that ends in
    // `serialNumber`, ignoring case.
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node {
            Node::Object(entries) => {
                for (key, value) in entries {
                    if key.to_ascii_lowercase().ends_with("serialnumber")
                        && let Some(serial) = text_of(value)
                        && !add(&serial)
                    {
                        return Err(too_many());
                    }
                    stack.push(value);
                }
            }
            Node::Array { items, .. } => stack.extend(items),
            _ => {}
        }
    }
    Ok(found.into_iter().collect())
}

/// Maps one parsed official profile. `entry_serial` and every serial in the
/// file are used only to scrub stored strings.
pub fn map(root: &Node, entry_serial: &str) -> Result<(Profile, ImportReport), String> {
    map_with_limit(root, entry_serial, Profile::MAX_JSON_BYTES)
}

/// `map` with another store size limit (tests force the final fit).
pub fn map_with_limit(
    root: &Node,
    entry_serial: &str,
    limit: usize,
) -> Result<(Profile, ImportReport), String> {
    if !matches!(root, Node::Object(_)) {
        return Err("this file is not a MasterHUB profile: expected a JSON object".into());
    }
    let mut mapper = Mapper {
        serials: collect_serials(root, entry_serial)?,
        bound: Vec::new(),
        bindings: 0,
        payload: 0,
        rows: Vec::new(),
        omitted: 0,
        totals: Counts::default(),
        categories: BTreeMap::new(),
        actions: BTreeMap::new(),
        high_risk: 0,
        warnings: Vec::new(),
        shortcuts: 0,
        scrubbed: 0,
        images: 0,
    };
    let name_node = match root.get("profileName") {
        Node::Null => root.get("profilename"),
        node => node,
    };
    let source_name = mapper.label(name_node, MAX_LABEL_BYTES, FALLBACK_NAME);

    let keypad = mapper.keypad(root);
    let mut flat = Vec::new();
    if let Some(knobs) = mapper.first(root, "knobsList", "knob module") {
        mapper.rotations(
            Section::Knobs,
            ServiceModule::M003K,
            &[
                ("rotateR", 0, "Knob 1"),
                ("rotateG", 1, "Knob 2"),
                ("rotateB", 2, "Knob 3"),
            ],
            [
                ("right", Gesture::Clockwise, true, "clockwise"),
                (
                    "left",
                    Gesture::CounterClockwise,
                    false,
                    "counter-clockwise",
                ),
            ],
            knobs,
            &mut flat,
        );
    }
    let dial_functions = match mapper.first(root, "dialEncoder", "dial") {
        Some(dial) => mapper.dial(dial),
        None => None,
    };
    if let Some(rollers) = mapper.first(root, "rollerBlock", "roller module") {
        mapper.rotations(
            Section::Rollers,
            ServiceModule::M032LE,
            &[("left", 0, "Left roller"), ("right", 1, "Right roller")],
            [
                ("up", Gesture::Up, true, "up"),
                ("down", Gesture::Down, false, "down"),
            ],
            rollers,
            &mut flat,
        );
    }
    if let Some(faders) = mapper.first(root, "sliders5List", "fader module") {
        mapper.faders(faders, &mut flat);
    }

    if root.get("touch46s").is_present() || root.get("touch50s").is_present() {
        mapper
            .warnings
            .push("Touch module sections are not supported and were not imported.".into());
    }
    if root.get("deviceLightingEffect").is_present() {
        mapper
            .warnings
            .push("Lighting settings were not imported.".into());
    }
    if mapper.images > 0 {
        mapper.warnings.push(format!(
            "Controls with images or GIFs: {}. Images, key colours and fonts are not imported.",
            mapper.images
        ));
    }
    if mapper.shortcuts > 0 {
        mapper.warnings.push(format!(
            "Shortcuts recorded on Windows: {}. Ctrl-based shortcuts are usually Cmd-based on macOS; review them before allowing actions.",
            mapper.shortcuts
        ));
    }
    if mapper.scrubbed > 0 {
        mapper.warnings.push(format!(
            "Names that contained a device serial number were replaced: {}.",
            mapper.scrubbed
        ));
    }
    mapper.warnings.push(
        "Device serial numbers, module identifiers and local file paths from the file were not stored."
            .into(),
    );

    let mut profile = Profile {
        schema_version: 2,
        name: source_name.clone(),
        bindings: flat,
        keypad: Some(keypad),
        dial_functions,
        // The official file's dial display mode is not mapped; the imported
        // profile shows the default face until the user picks one.
        dial_display: None,
    };
    let trimmed = fit_store_limit(&mut profile, &mut mapper, limit)?;
    if trimmed > 0 {
        mapper.warnings.push(format!(
            "Bindings left unbound so the profile fits the 1 MiB limit (long text first): {trimmed}."
        ));
    }
    profile
        .validate()
        .map_err(|error| format!("the imported profile is invalid: {error}"))?;

    // Rows past the byte budget are counted but not listed.
    let mut used = 0;
    let keep = mapper
        .rows
        .iter()
        .take_while(|row| {
            used += serde_json::to_vec(row).map_or(0, |bytes| bytes.len() + 1);
            used <= MAX_REPORT_BYTES - 64 * 1024
        })
        .count();
    mapper.omitted += mapper.rows.len() - keep;
    mapper.rows.truncate(keep);
    if mapper.omitted > 0 {
        mapper.warnings.push(format!(
            "{} more controls are counted above but not listed.",
            mapper.omitted
        ));
    }
    let section_name = |section: Section| {
        serde_json::to_value(section)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    let report = ImportReport {
        profile_name: source_name.clone(),
        source_name,
        totals: mapper.totals,
        categories: mapper
            .categories
            .into_iter()
            .map(|(section, counts)| CategoryCounts {
                category: section_name(section),
                counts,
            })
            .collect(),
        actions: mapper
            .actions
            .into_iter()
            .map(|(category, counts)| CategoryCounts { category, counts })
            .collect(),
        high_risk: mapper.high_risk,
        warnings: mapper.warnings,
        rows: mapper.rows,
    };
    Ok((profile, report))
}

/// Unbinds bindings until the profile, as the store writes it
/// (pretty-printed schema 2), fits `limit` minus 1 KiB of slack for the
/// later ` (imported N)` rename, and marks their report rows and counts as
/// left unbound: the last text- or URL-carrying bindings first, then any
/// other binding from the end. The profile is serialized once per batch,
/// and each batch is sized from the overflow, so this stays linear. If even
/// that cannot make it fit, the import is refused with a clear error.
fn fit_store_limit(
    profile: &mut Profile,
    mapper: &mut Mapper,
    limit: usize,
) -> Result<usize, String> {
    let budget = limit.saturating_sub(1024);
    let size = |profile: &Profile| {
        #[cfg(test)]
        FIT_PASSES.with(|passes| passes.set(passes.get() + 1));
        serde_json::to_vec_pretty(profile).map_or(usize::MAX, |b| b.len())
    };
    let mut current = size(profile);
    let order: Vec<usize> = (0..mapper.bound.len())
        .rev()
        .filter(|index| mapper.bound[*index].payload)
        .chain(
            (0..mapper.bound.len())
                .rev()
                .filter(|index| !mapper.bound[*index].payload),
        )
        .collect();
    let mut next = order.into_iter();
    let mut removed = Vec::new();
    // The estimates come from each action alone; the real saving inside the
    // profile differs a little. The first batch covers half the overflow,
    // and the saving it really made calibrates the estimates of the next
    // batches, so the fit lands within about one binding of the limit.
    let mut scale = 1.0_f64;
    let mut first = true;
    while current > budget {
        let need = current - budget;
        let target = if first { need.div_ceil(2) } else { need } as f64;
        let before = removed.len();
        let mut covered = 0.0_f64;
        let mut estimated = 0usize;
        while covered < target {
            let Some(index) = next.next() else { break };
            remove_binding(profile, &mapper.bound[index].location);
            covered += mapper.bound[index].bytes as f64 * scale;
            estimated += mapper.bound[index].bytes;
            removed.push(index);
        }
        if removed.len() == before {
            return Err(format!(
                "the imported profile is too large to store (more than {} KiB even without any binding)",
                limit / 1024
            ));
        }
        let now = size(profile);
        if estimated > 0 && now < current {
            scale = (current - now) as f64 / estimated as f64;
        }
        current = now;
        first = false;
    }
    for &index in &removed {
        let bound = &mapper.bound[index];
        let (section, official, outcome, high_risk, row) = (
            bound.section,
            bound.official.clone(),
            bound.outcome,
            bound.high_risk,
            bound.row,
        );
        for counts in [
            &mut mapper.totals,
            mapper.categories.entry(section).or_default(),
            mapper.actions.entry(official).or_default(),
        ] {
            counts.sub(outcome);
            counts.add(Outcome::UnboundUnsupported);
        }
        if high_risk {
            mapper.high_risk = mapper.high_risk.saturating_sub(1);
        }
        if let Some(row) = row.and_then(|row| mapper.rows.get_mut(row)) {
            row.outcome = Outcome::UnboundUnsupported;
            row.reason = "left unbound so the profile fits 1 MiB".into();
            row.high_risk = false;
        }
    }
    Ok(removed.len())
}

fn remove_binding(profile: &mut Profile, location: &Location) {
    match location {
        Location::Key { page, slot } => {
            if let Some(Some(KeypadTile::Action { bindings, .. })) = profile
                .keypad
                .as_mut()
                .and_then(|keypad| keypad.pages.iter_mut().find(|p| &p.id == page))
                .and_then(|page| page.slots.get_mut(*slot))
            {
                bindings.clear();
            }
        }
        Location::Flat {
            module,
            control,
            gesture,
        } => profile.bindings.retain(|binding| {
            !(binding.module == *module
                && binding.control == *control
                && binding.gesture == *gesture)
        }),
        Location::Dial { function, gesture } => {
            if let Some(function) = profile
                .dial_functions
                .as_mut()
                .and_then(|functions| functions.get_mut(*function))
            {
                function
                    .bindings
                    .retain(|binding| binding.gesture != *gesture);
            }
        }
    }
}
