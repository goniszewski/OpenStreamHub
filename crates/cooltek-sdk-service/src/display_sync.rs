//! Offline keypad and dial display synchronization.
//!
//! This module decides *which* images a MasterHUB display needs and applies
//! those decisions to a transport-agnostic [`DisplaySink`]. It performs no HID
//! I/O and renders no artwork or fonts. The CLI crate supplies a hardware-only
//! `HidDisplaySink` (`display_sink.rs`) and key and dial artwork providers,
//! wired behind the opt-in, experimental `masterhub-service run --display-sync`
//! flag (`--dial-content` for the dial); the desktop app passes it only with its
//! experimental, off-by-default display setting. Physical key order, orientation, pacing, and firmware
//! image acceptance are unverified; see `docs/masterhub-display-sync.md`.
//!
//! The flow is: a caller-owned [`KeyImageProvider`] turns each
//! [`KeyImageRequest`] into prepared 72×72 baseline JFIF bytes,
//! [`DisplaySync::plan_page`] diffs them against the last drawn state, and
//! [`DisplaySync::apply`] writes only changed keys in ascending wire order.
//! A failed or partial write leaves that key dirty, so a fresh plan redraws it.
//! Every change to the drawn state (a write, a failure, an invalidation) makes
//! earlier plans stale: `apply` rejects them, and plans made by another
//! [`DisplaySync`], with [`DisplayError::StalePlan`]. [`DisplaySync::invalidate`]
//! (reconnect) and [`DisplaySync::resume`] (wake) forget everything drawn and
//! force a full redraw.

use crate::{Action, KeypadLayout, KeypadTile, MediaCommand, ProfileSession, SystemCommand};
use cooltek_sdk_masterhub::{dial, keypad};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Source of per-[`DisplaySync`] owner IDs; 0 is never handed out.
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// Number of M4315 keys addressed by wire index 0 through 14.
pub const KEY_COUNT: usize = KeypadLayout::KEY_COUNT;

/// One display surface addressed by a sink.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "type", content = "index", rename_all = "snake_case")]
pub enum DisplayTarget {
    /// An M4315 key by wire index, 0 through 14; physical placement is unverified.
    Key(u8),
    /// The M131E dial's 240×240 display.
    Dial,
}

/// Display surfaces a sink can write.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DisplayCapabilities {
    /// Addressable keys starting at index 0; 0 for no key displays, at most 15.
    pub key_count: u8,
    /// Whether the dial display can be written.
    pub dial: bool,
}

/// A failed image transfer reported by a [`DisplaySink`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SinkError {
    /// The transport rejected the transfer; the device may hold a partial image.
    Transport(String),
    /// A report was written only partially. Never retried, following the SDK's
    /// "short writes stop without retry" policy; the device may hold a partial image.
    ShortWrite {
        /// Bytes the transport accepted.
        written: usize,
        /// Bytes in the complete report.
        expected: usize,
    },
    /// The sink cannot write this target.
    Unsupported(DisplayTarget),
}

impl fmt::Display for SinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(message) => write!(f, "display transport failed: {message}"),
            Self::ShortWrite { written, expected } => write!(
                f,
                "display write returned {written} bytes for a {expected}-byte report"
            ),
            Self::Unsupported(target) => write!(f, "display target {target:?} is unsupported"),
        }
    }
}

impl std::error::Error for SinkError {}

/// Writes complete prepared images to physical or simulated displays.
///
/// Real hardware sinks are expected to wrap the SDK's descriptor-sized packet
/// writers (`keypad::JpegPackets`, `dial::display_jpeg_with_pacing`), forward
/// `between_chunks` as the inter-report pacing hook, and stop on the first
/// error or short write without retrying. Images given to a sink have already
/// passed the SDK's JPEG validation. No hardware sink exists yet.
pub trait DisplaySink {
    /// Reports the surfaces this sink can write.
    fn capabilities(&self) -> DisplayCapabilities;

    /// Writes one complete 72×72 key image to wire index `key_index`.
    ///
    /// `between_chunks` receives the reports already written and runs between
    /// consecutive reports, matching `dial::display_jpeg_with_pacing`.
    fn write_key_image(
        &mut self,
        key_index: u8,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError>;

    /// Writes one complete 240×240 dial image.
    fn write_dial_image(
        &mut self,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError>;
}

/// The picture a tile without artwork of its own is drawn with, chosen from what its first binding does.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TileIcon {
    /// Any action without a picture of its own.
    #[default]
    Generic,
    /// Output or input volume.
    Volume,
    /// Mute toggle.
    Mute,
    /// Play or pause.
    Play,
    /// Next track.
    Next,
    /// Previous track.
    Previous,
    /// Stop playback.
    Stop,
    /// Screenshot.
    Screenshot,
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Timer.
    Timer,
    /// Open a web address.
    Web,
    /// Launch an application.
    Application,
    /// Type text.
    Text,
    /// Several actions in a row.
    Sequence,
    /// Switch profile.
    Profile,
    /// Any OBS action.
    Obs,
}

impl TileIcon {
    /// The picture for an action.
    #[must_use]
    pub fn of(action: &Action) -> Self {
        match action {
            Action::Volume { .. }
            | Action::SetVolume { .. }
            | Action::VolumeFromPosition
            | Action::SetMicInputVolume { .. }
            | Action::MicInputVolumeFromPosition => Self::Volume,
            Action::ToggleMute => Self::Mute,
            Action::Media { command } => match command {
                MediaCommand::PlayPause => Self::Play,
                MediaCommand::NextTrack => Self::Next,
                MediaCommand::PreviousTrack => Self::Previous,
                MediaCommand::Stop => Self::Stop,
            },
            Action::System { command } => match command {
                SystemCommand::Undo => Self::Undo,
                SystemCommand::Redo => Self::Redo,
                SystemCommand::Screenshot => Self::Screenshot,
            },
            Action::OpenUrl { .. } => Self::Web,
            Action::LaunchApp { .. } => Self::Application,
            Action::Text { .. } => Self::Text,
            Action::MultiAction { .. }
            | Action::Conditional { .. }
            | Action::SetVariable { .. }
            | Action::ToggleVariable { .. }
            | Action::AddToVariable { .. }
            | Action::Macro { .. } => Self::Sequence,
            Action::Profile { .. } => Self::Profile,
            Action::Timer { .. } => Self::Timer,
            Action::ObsScene { .. }
            | Action::ObsVirtualCamera { .. }
            | Action::ObsStream { .. }
            | Action::ObsRecord { .. }
            | Action::ObsReplayBuffer { .. }
            | Action::ObsStudioMode { .. }
            | Action::ObsTransition
            | Action::ObsScreenshot
            | Action::ObsFilter { .. }
            | Action::ObsSourceVisibility { .. }
            | Action::ObsSceneCollection { .. }
            | Action::ObsProfile { .. }
            | Action::ObsInputMute { .. }
            | Action::ObsAdjustInputVolume { .. }
            | Action::ObsMediaSource { .. }
            | Action::ObsSetInputVolume { .. }
            | Action::ObsInputVolumeFromPosition { .. } => Self::Obs,
            Action::Shortcut { .. } => Self::Generic,
        }
    }
}

/// What one key shows on the current page; the provider turns it into artwork.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeyContent {
    /// An empty slot. Providers should return a blank image so stale artwork
    /// from the previous page is cleared.
    Empty {
        /// Page containing the slot.
        page_id: String,
    },
    /// An action tile.
    Action {
        /// Page containing the tile.
        page_id: String,
        /// The tile's label.
        label: String,
        /// The picture for the tile's first binding.
        icon: TileIcon,
    },
    /// A folder tile opening a child page.
    Folder {
        /// Page containing the tile.
        page_id: String,
        /// Child page opened by the tile.
        target_page_id: String,
        /// Child page name, the folder's label.
        target_name: String,
    },
    /// The generated Back key in slot 0 of every child page.
    ///
    /// Its artwork is a left arrow with an `L{depth}` badge for the **current**
    /// depth; rendering it is the provider's concern.
    Back {
        /// Current child level, 1 through 8.
        depth: u8,
        /// Page shown after Back.
        parent_page_id: String,
    },
}

/// A request for one key's prepared image on the current page.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct KeyImageRequest {
    /// Wire key index, 0 through 14.
    pub key_index: u8,
    /// What the key represents.
    pub content: KeyContent,
}

/// Supplies prepared key artwork; implementations must not perform device I/O.
pub trait KeyImageProvider {
    /// Returns 72×72 baseline JFIF bytes, or `None` when no artwork is available.
    ///
    /// A `None` key is reported in [`DisplayPlan::missing`] and is not written,
    /// so it may keep showing a previous page's image.
    fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>>;
    /// Advance a visible page's animation using monotonic run time. No I/O.
    fn advance(&mut self, _page_id: &str, _elapsed: Duration) {}
    /// Time until the next visible frame; static or completed providers do not wake.
    fn next_frame_in(&self) -> Option<Duration> {
        None
    }
}

/// A request for the dial's prepared image.
///
/// The caller maps its current profile and dial function onto these opaque
/// identifiers; this module attaches no meaning to them.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
pub struct DialImageRequest {
    /// Active profile name.
    pub profile: String,
    /// Active dial function, or `None` for the profile's base image.
    pub function: Option<String>,
}

/// Supplies prepared dial artwork; implementations must not perform device I/O.
pub trait DialImageProvider {
    /// Returns 240×240 baseline JFIF bytes, or `None` when no artwork is available.
    fn dial_image(&self, request: &DialImageRequest) -> Option<Vec<u8>>;
}

/// A planning or apply failure in which nothing was written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DisplayError {
    /// The session's profile has no schema-2 keypad page tree.
    NotKeypadProfile,
    /// The session's current page is missing from its layout.
    MissingPage(String),
    /// A provider returned an image the SDK validator rejects.
    InvalidImage {
        /// Image destination.
        target: DisplayTarget,
        /// Validator diagnostic.
        reason: String,
    },
    /// The plan was made before the last invalidation and may skip keys.
    StalePlan,
    /// The sink cannot write a target in the plan.
    Unsupported(DisplayTarget),
}

impl fmt::Display for DisplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotKeypadProfile => write!(f, "profile has no schema-2 keypad pages"),
            Self::MissingPage(id) => write!(f, "keypad page {id:?} is missing"),
            Self::InvalidImage { target, reason } => {
                write!(f, "invalid image for {target:?}: {reason}")
            }
            Self::StalePlan => write!(f, "display plan predates the last invalidation"),
            Self::Unsupported(target) => write!(f, "display target {target:?} is unsupported"),
        }
    }
}

impl std::error::Error for DisplayError {}

/// The images last written completely, per surface.
///
/// `None` means unknown or dirty: never drawn, invalidated, or failed. Diffing
/// compares exact bytes, so unchanged artwork is never resent and changed
/// artwork behind an identical request is still redrawn.
///
/// Separate key and dial generations change on every update to that surface's
/// state, so a plan is accepted only against the exact state it was made from.
#[derive(Clone, Debug, Default)]
pub struct DrawnState {
    keys: [Option<Vec<u8>>; KEY_COUNT],
    dial: Option<Vec<u8>>,
    owner: u64,
    key_generation: u64,
    dial_generation: u64,
}

impl DrawnState {
    /// Returns the image last drawn completely on a key, if known.
    pub fn key(&self, key_index: u8) -> Option<&[u8]> {
        self.keys.get(usize::from(key_index))?.as_deref()
    }

    /// Returns the image last drawn completely on the dial, if known.
    pub fn dial(&self) -> Option<&[u8]> {
        self.dial.as_deref()
    }

    fn set_key(&mut self, key_index: u8, jpeg: Option<Vec<u8>>) {
        self.keys[usize::from(key_index)] = jpeg;
        self.key_generation += 1;
    }

    fn set_dial(&mut self, jpeg: Option<Vec<u8>>) {
        self.dial = jpeg;
        self.dial_generation += 1;
    }

    fn invalidate(&mut self) {
        self.keys = Default::default();
        self.dial = None;
        self.key_generation += 1;
        self.dial_generation += 1;
    }
}

/// One planned key write.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyWrite {
    /// The request that produced the image.
    pub request: KeyImageRequest,
    /// Validated 72×72 baseline JFIF bytes.
    pub jpeg: Vec<u8>,
}

/// The validated, minimal ordered key writes that redraw one page.
#[derive(Clone, Debug)]
pub struct DisplayPlan {
    writes: Vec<KeyWrite>,
    missing: Vec<KeyImageRequest>,
    owner: u64,
    generation: u64,
}

impl DisplayPlan {
    /// Plans the current page of a validated schema-2 session against `drawn`.
    ///
    /// Crate-private so that public planning goes through
    /// [`DisplaySync::plan_page`], which binds the plan to its owner and state.
    ///
    /// Requests all 15 keys in ascending wire order, validates every returned
    /// image, and keeps only keys whose bytes differ from the last complete
    /// write. Child pages request [`KeyContent::Back`] for slot 0.
    ///
    /// # Errors
    /// Returns an error, planning nothing, for a schema-1 profile, a missing
    /// page, or any image rejected by `keypad::validate_jpeg`.
    pub(crate) fn for_page(
        session: &ProfileSession,
        provider: &dyn KeyImageProvider,
        drawn: &DrawnState,
    ) -> Result<Self, DisplayError> {
        let layout = session
            .profile()
            .keypad
            .as_ref()
            .ok_or(DisplayError::NotKeypadProfile)?;
        let path = session.path();
        let page_id = path.last().ok_or(DisplayError::NotKeypadProfile)?;
        let page = layout
            .page(page_id)
            .ok_or_else(|| DisplayError::MissingPage(page_id.clone()))?;
        let depth = session.depth();
        let mut writes = Vec::new();
        let mut missing = Vec::new();
        for key_index in 0..KEY_COUNT as u8 {
            let slot = usize::from(key_index);
            let content = if slot == 0 && depth > 0 {
                KeyContent::Back {
                    depth: depth as u8,
                    parent_page_id: path[path.len() - 2].clone(),
                }
            } else {
                match page.slots.get(slot).and_then(Option::as_ref) {
                    None => KeyContent::Empty {
                        page_id: page.id.clone(),
                    },
                    Some(KeypadTile::Action { label, bindings }) => KeyContent::Action {
                        page_id: page.id.clone(),
                        label: label.clone(),
                        icon: bindings
                            .first()
                            .map_or(TileIcon::Generic, |binding| TileIcon::of(&binding.action)),
                    },
                    Some(KeypadTile::Folder { page_id: target }) => KeyContent::Folder {
                        page_id: page.id.clone(),
                        target_page_id: target.clone(),
                        target_name: layout
                            .page(target)
                            .ok_or_else(|| DisplayError::MissingPage(target.clone()))?
                            .name
                            .clone(),
                    },
                }
            };
            let request = KeyImageRequest { key_index, content };
            let Some(jpeg) = provider.key_image(&request) else {
                missing.push(request);
                continue;
            };
            keypad::validate_jpeg(&jpeg).map_err(|error| DisplayError::InvalidImage {
                target: DisplayTarget::Key(key_index),
                reason: error.to_string(),
            })?;
            if drawn.key(key_index) != Some(jpeg.as_slice()) {
                writes.push(KeyWrite { request, jpeg });
            }
        }
        Ok(Self {
            writes,
            missing,
            owner: drawn.owner,
            generation: drawn.key_generation,
        })
    }

    /// Changed keys to write, in ascending wire order.
    pub fn writes(&self) -> &[KeyWrite] {
        &self.writes
    }

    /// Keys whose provider returned no artwork; they are not written.
    pub fn missing(&self) -> &[KeyImageRequest] {
        &self.missing
    }

    /// Returns true when no key needs a write.
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// The validated dial image to write, or nothing when it is unchanged.
#[derive(Clone, Debug)]
pub struct DialPlan {
    write: Option<(DialImageRequest, Vec<u8>)>,
    missing: bool,
    owner: u64,
    generation: u64,
}

impl DialPlan {
    /// Plans a dial redraw for `request`, writing only when the bytes changed.
    ///
    /// Crate-private; public planning goes through [`DisplaySync::plan_dial`].
    ///
    /// # Errors
    /// Returns an error, planning nothing, when `dial::validate_jpeg` rejects
    /// the provider's image.
    pub(crate) fn new(
        request: DialImageRequest,
        provider: &dyn DialImageProvider,
        drawn: &DrawnState,
    ) -> Result<Self, DisplayError> {
        let image = provider.dial_image(&request);
        if let Some(jpeg) = &image {
            dial::validate_jpeg(jpeg).map_err(|error| DisplayError::InvalidImage {
                target: DisplayTarget::Dial,
                reason: error.to_string(),
            })?;
        }
        Ok(Self {
            missing: image.is_none(),
            write: image
                .filter(|jpeg| drawn.dial() != Some(jpeg.as_slice()))
                .map(|jpeg| (request, jpeg)),
            owner: drawn.owner,
            generation: drawn.dial_generation,
        })
    }

    /// The changed request and image, if any.
    pub fn write(&self) -> Option<(&DialImageRequest, &[u8])> {
        self.write
            .as_ref()
            .map(|(request, jpeg)| (request, jpeg.as_slice()))
    }

    /// Returns true when the provider returned no artwork.
    pub fn is_missing(&self) -> bool {
        self.missing
    }
}

/// Retry behaviour for transport errors; none by default.
///
/// Short writes and unsupported targets are never retried.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RetryPolicy {
    /// Extra attempts after a [`SinkError::Transport`] failure, so at most
    /// `transport_retries + 1` attempts. Retries run back to back: there is no
    /// pacing or backoff between attempts.
    pub transport_retries: u8,
}

/// One target that did not receive its complete image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteFailure {
    /// Target left dirty.
    pub target: DisplayTarget,
    /// The final attempt's error.
    pub error: SinkError,
    /// Attempts made, including retries; at most 256.
    pub attempts: u16,
}

/// Outcome of applying a plan; every unwritten target remains dirty.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ApplyReport {
    /// Targets written completely, in write order.
    pub written: Vec<DisplayTarget>,
    /// Accumulated failures, in attempt order.
    pub failures: Vec<WriteFailure>,
    /// Targets skipped after a short write stopped the plan.
    pub not_attempted: Vec<DisplayTarget>,
}

impl ApplyReport {
    /// Returns true when every planned write completed.
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty() && self.not_attempted.is_empty()
    }
}

/// Official-app resume steps after the 2 s settle delay, in order.
///
/// The session manager performs the device steps itself; this module performs
/// no HID I/O. Sequence from static analysis of MasterHUB 1.0.0.113, unverified.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeStep {
    /// Re-enter software mode.
    EnterSoftwareMode,
    /// Query firmware information.
    QueryFirmware,
    /// Redraw every display; [`DisplaySync::resume`] already forced it.
    RefreshDisplay,
}

/// What the caller must do after [`DisplaySync::resume`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ResumeAction {
    /// Steps to perform in order.
    pub steps: [ResumeStep; 3],
}

type Hook = Box<dyn FnMut(usize)>;

/// Applies display plans to one sink and tracks what it last drew.
///
/// Writes are sequential and never retried after a short write. A transport
/// failure is retried only as [`RetryPolicy`] allows (not at all by default),
/// recorded, and followed by the remaining writes; a short write stops the
/// plan. Every target that did not receive a complete image becomes dirty,
/// because the device may hold a partial image.
pub struct DisplaySync<S: DisplaySink> {
    sink: S,
    drawn: DrawnState,
    retry: RetryPolicy,
    between_chunks: Option<Hook>,
    between_images: Option<Hook>,
}

impl<S: DisplaySink> DisplaySync<S> {
    /// Starts with every surface dirty, no retries, and no pacing.
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            drawn: DrawnState {
                owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
                ..DrawnState::default()
            },
            retry: RetryPolicy::default(),
            between_chunks: None,
            between_images: None,
        }
    }

    /// Replaces the transport retry policy.
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Sets the hook forwarded to the sink between reports of one image.
    pub fn with_chunk_pacing(mut self, hook: impl FnMut(usize) + 'static) -> Self {
        self.between_chunks = Some(Box::new(hook));
        self
    }

    /// Sets the hook run between consecutive key image writes in one plan.
    ///
    /// It receives the number of keys already attempted in that plan, including
    /// failed ones. It does not run between retry attempts or for [`Self::apply_dial`].
    pub fn with_image_pacing(mut self, hook: impl FnMut(usize) + 'static) -> Self {
        self.between_images = Some(Box::new(hook));
        self
    }

    /// Returns the last completely drawn state.
    pub fn drawn(&self) -> &DrawnState {
        &self.drawn
    }

    /// Returns the sink.
    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// Returns the sink mutably, for example to inject test faults.
    ///
    /// Anything written directly through the sink bypasses validation and
    /// tracking, so this invalidates all drawn state: earlier plans become
    /// stale and the next plans redraw everything.
    pub fn sink_mut(&mut self) -> &mut S {
        self.invalidate();
        &mut self.sink
    }

    /// Returns the sink, discarding drawn state.
    pub fn into_sink(self) -> S {
        self.sink
    }

    /// Plans the session's current page against the drawn state.
    ///
    /// Requests all 15 keys in ascending wire order, validates every returned
    /// image with `keypad::validate_jpeg`, and keeps only keys whose bytes
    /// differ from the last complete write. Child pages request
    /// [`KeyContent::Back`] for slot 0.
    ///
    /// # Errors
    /// Returns an error, planning nothing, for a schema-1 profile, a missing
    /// page, or any invalid image.
    pub fn plan_page(
        &self,
        session: &ProfileSession,
        provider: &dyn KeyImageProvider,
    ) -> Result<DisplayPlan, DisplayError> {
        DisplayPlan::for_page(session, provider, &self.drawn)
    }

    /// Plans the dial image for `request` against the drawn state.
    ///
    /// # Errors
    /// Returns an error, planning nothing, when `dial::validate_jpeg` rejects
    /// the provider's image.
    pub fn plan_dial(
        &self,
        request: DialImageRequest,
        provider: &dyn DialImageProvider,
    ) -> Result<DialPlan, DisplayError> {
        DialPlan::new(request, provider, &self.drawn)
    }

    /// Plans and applies the session's current page.
    ///
    /// # Errors
    /// See [`Self::plan_page`] and [`Self::apply`]; nothing is written on error.
    pub fn sync_page(
        &mut self,
        session: &ProfileSession,
        provider: &dyn KeyImageProvider,
    ) -> Result<ApplyReport, DisplayError> {
        let plan = self.plan_page(session, provider)?;
        self.apply(plan)
    }

    /// Writes a key plan in order.
    ///
    /// # Errors
    /// Returns an error before any write for a plan made by another instance
    /// or before any later change to the key state (a write, a failure, or an
    /// invalidation), or for a key the sink cannot address.
    pub fn apply(&mut self, plan: DisplayPlan) -> Result<ApplyReport, DisplayError> {
        if plan.owner != self.drawn.owner || plan.generation != self.drawn.key_generation {
            return Err(DisplayError::StalePlan);
        }
        let key_count = self.sink.capabilities().key_count.min(KEY_COUNT as u8);
        if let Some(write) = plan
            .writes
            .iter()
            .find(|write| write.request.key_index >= key_count)
        {
            return Err(DisplayError::Unsupported(DisplayTarget::Key(
                write.request.key_index,
            )));
        }
        let mut report = ApplyReport::default();
        let mut writes = plan.writes.into_iter();
        for (attempted, write) in writes.by_ref().enumerate() {
            let index = write.request.key_index;
            let target = DisplayTarget::Key(index);
            if attempted > 0
                && let Some(hook) = &mut self.between_images
            {
                hook(attempted);
            }
            match self.write(target, &write.jpeg) {
                Ok(()) => {
                    self.drawn.set_key(index, Some(write.jpeg));
                    report.written.push(target);
                }
                Err(failure) => {
                    self.drawn.set_key(index, None);
                    let stop = matches!(failure.error, SinkError::ShortWrite { .. });
                    report.failures.push(failure);
                    if stop {
                        break;
                    }
                }
            }
        }
        report.not_attempted = writes
            .map(|write| DisplayTarget::Key(write.request.key_index))
            .collect();
        Ok(report)
    }

    /// Writes a dial plan.
    ///
    /// # Errors
    /// Returns an error before writing for a stale or foreign plan (see
    /// [`Self::apply`]) or a sink without a dial.
    pub fn apply_dial(&mut self, plan: DialPlan) -> Result<ApplyReport, DisplayError> {
        if plan.owner != self.drawn.owner || plan.generation != self.drawn.dial_generation {
            return Err(DisplayError::StalePlan);
        }
        let mut report = ApplyReport::default();
        let Some((_, jpeg)) = plan.write else {
            return Ok(report);
        };
        if !self.sink.capabilities().dial {
            return Err(DisplayError::Unsupported(DisplayTarget::Dial));
        }
        match self.write(DisplayTarget::Dial, &jpeg) {
            Ok(()) => {
                self.drawn.set_dial(Some(jpeg));
                report.written.push(DisplayTarget::Dial);
            }
            Err(failure) => {
                self.drawn.set_dial(None);
                report.failures.push(failure);
            }
        }
        Ok(report)
    }

    /// Forgets all drawn state, for example after a reconnect or profile load.
    ///
    /// Plans made earlier are rejected as stale; the next plans redraw everything.
    pub fn invalidate(&mut self) {
        self.drawn.invalidate();
    }

    /// Marks the displays unknown before sleep; the device returns to hardware mode.
    pub fn suspend(&mut self) {
        self.invalidate();
    }

    /// Forces a full redraw after wake and returns the official resume sequence.
    ///
    /// The caller then performs the device steps and calls [`Self::sync_page`]
    /// (and dial planning) for [`ResumeStep::RefreshDisplay`].
    pub fn resume(&mut self) -> ResumeAction {
        self.invalidate();
        ResumeAction {
            steps: [
                ResumeStep::EnterSoftwareMode,
                ResumeStep::QueryFirmware,
                ResumeStep::RefreshDisplay,
            ],
        }
    }

    fn write(&mut self, target: DisplayTarget, jpeg: &[u8]) -> Result<(), WriteFailure> {
        let mut noop = |_: usize| {};
        let max_attempts = u16::from(self.retry.transport_retries) + 1;
        let mut attempts: u16 = 0;
        loop {
            attempts += 1;
            let pacing: &mut dyn FnMut(usize) = match &mut self.between_chunks {
                Some(hook) => hook,
                None => &mut noop,
            };
            let result = match target {
                DisplayTarget::Key(index) => self.sink.write_key_image(index, jpeg, pacing),
                DisplayTarget::Dial => self.sink.write_dial_image(jpeg, pacing),
            };
            match result {
                Ok(()) => return Ok(()),
                Err(SinkError::Transport(_)) if attempts < max_attempts => {}
                Err(error) => {
                    return Err(WriteFailure {
                        target,
                        error,
                        attempts,
                    });
                }
            }
        }
    }
}

/// One write attempt recorded by [`FakeDisplaySink`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedWrite {
    /// Target written.
    pub target: DisplayTarget,
    /// Complete image passed to the sink.
    pub jpeg: Vec<u8>,
    /// Injected fault returned for this attempt, if any.
    pub fault: Option<SinkError>,
}

/// An in-memory [`DisplaySink`] for offline tests; it touches no device.
///
/// Every attempt is recorded. Faults queued per target are returned by that
/// target's next attempts in order. Each image is treated as `chunks` reports,
/// so the pacing hook runs `chunks - 1` times on success.
#[derive(Clone, Debug)]
pub struct FakeDisplaySink {
    /// Reported capabilities.
    pub capabilities: DisplayCapabilities,
    /// Simulated reports per image, at least 1.
    pub chunks: usize,
    /// Every attempt in order.
    pub writes: Vec<RecordedWrite>,
    faults: HashMap<DisplayTarget, VecDeque<Option<SinkError>>>,
}

impl Default for FakeDisplaySink {
    fn default() -> Self {
        Self {
            capabilities: DisplayCapabilities {
                key_count: KEY_COUNT as u8,
                dial: true,
            },
            chunks: 1,
            writes: Vec::new(),
            faults: HashMap::new(),
        }
    }
}

impl FakeDisplaySink {
    /// Queues a fault for the next attempt on `target`.
    pub fn fail_next(&mut self, target: DisplayTarget, error: SinkError) {
        self.faults
            .entry(target)
            .or_default()
            .push_back(Some(error));
    }

    /// Queues `successes` successful attempts on `target`, then a fault.
    pub fn fail_after(&mut self, target: DisplayTarget, successes: usize, error: SinkError) {
        let queue = self.faults.entry(target).or_default();
        queue.extend(std::iter::repeat_n(None, successes));
        queue.push_back(Some(error));
    }

    /// Successful writes in order.
    pub fn successful(&self) -> Vec<DisplayTarget> {
        self.writes
            .iter()
            .filter(|write| write.fault.is_none())
            .map(|write| write.target)
            .collect()
    }

    fn record(
        &mut self,
        target: DisplayTarget,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        let fault = self
            .faults
            .get_mut(&target)
            .and_then(VecDeque::pop_front)
            .flatten();
        self.writes.push(RecordedWrite {
            target,
            jpeg: jpeg.to_vec(),
            fault: fault.clone(),
        });
        match fault {
            Some(error) => Err(error),
            None => {
                (1..self.chunks).for_each(&mut *between_chunks);
                Ok(())
            }
        }
    }
}

impl DisplaySink for FakeDisplaySink {
    fn capabilities(&self) -> DisplayCapabilities {
        self.capabilities
    }

    fn write_key_image(
        &mut self,
        key_index: u8,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        let target = DisplayTarget::Key(key_index);
        if key_index >= self.capabilities.key_count {
            return Err(SinkError::Unsupported(target));
        }
        self.record(target, jpeg, between_chunks)
    }

    fn write_dial_image(
        &mut self,
        jpeg: &[u8],
        between_chunks: &mut dyn FnMut(usize),
    ) -> Result<(), SinkError> {
        if !self.capabilities.dial {
            return Err(SinkError::Unsupported(DisplayTarget::Dial));
        }
        self.record(DisplayTarget::Dial, jpeg, between_chunks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceContext, Gesture, InputEvent, KeypadResolution, Profile, ServiceModule};
    use std::cell::RefCell;
    use std::rc::Rc;

    // Original 72×72 black baseline JFIF artwork (see the SDK fixture README).
    const KEY_JPEG: &[u8] =
        include_bytes!("../../cooltek-sdk-masterhub/tests/fixtures/keypad-72x72-baseline.jpg");

    /// A framing-valid variant: one extra entropy byte makes distinct bytes.
    /// Pixels are not claimed to decode; the SDK validator checks framing only.
    fn variant(base: &[u8], tag: u8) -> Vec<u8> {
        assert_ne!(tag, 0xff);
        let mut jpeg = base[..base.len() - 2].to_vec();
        jpeg.extend_from_slice(&[tag, 0xff, 0xd9]);
        jpeg
    }

    /// The key fixture with its frame dimensions rewritten to 240×240. Framing
    /// only; entropy data is not valid for the new size.
    fn dial_jpeg(tag: u8) -> Vec<u8> {
        let mut jpeg = KEY_JPEG.to_vec();
        let frame = jpeg.windows(2).position(|b| b == [0xff, 0xc0]).unwrap();
        jpeg[frame + 5..frame + 9].copy_from_slice(&[0, 240, 0, 240]);
        variant(&jpeg, tag)
    }

    /// Distinct artwork per request, derived from a stable hash of the request.
    struct Art {
        overrides: RefCell<HashMap<KeyImageRequest, Option<Vec<u8>>>>,
        calls: RefCell<Vec<KeyImageRequest>>,
    }

    impl Art {
        fn new() -> Self {
            Self {
                overrides: RefCell::default(),
                calls: RefCell::default(),
            }
        }
        fn tag(request: &KeyImageRequest) -> u8 {
            let text = serde_json::to_string(request).unwrap();
            let sum = text
                .bytes()
                .fold(7u32, |acc, b| acc.wrapping_mul(31) ^ u32::from(b));
            (sum % 250) as u8
        }
        fn image(request: &KeyImageRequest) -> Vec<u8> {
            match request.content {
                // Every blank key shares one image so blanks diff as unchanged.
                KeyContent::Empty { .. } => KEY_JPEG.to_vec(),
                _ => variant(KEY_JPEG, Self::tag(request)),
            }
        }
    }

    impl KeyImageProvider for Art {
        fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>> {
            self.calls.borrow_mut().push(request.clone());
            match self.overrides.borrow().get(request) {
                Some(value) => value.clone(),
                None => Some(Self::image(request)),
            }
        }
    }

    fn session() -> ProfileSession {
        ProfileSession::new(
            Profile::from_json(include_bytes!(
                "../../../examples/profiles/keypad-folders.json"
            ))
            .unwrap(),
        )
        .unwrap()
    }

    fn press(session: &mut ProfileSession, control: u8) {
        let device = DeviceContext {
            module: ServiceModule::M4315,
            serial_number: None,
        };
        for gesture in [Gesture::Pressed, Gesture::Released] {
            let event = InputEvent {
                timestamp_micros: 0,
                control,
                gesture,
                value: None,
            };
            let resolution = session.resolve(&device, &event).unwrap();
            if gesture == Gesture::Pressed {
                assert!(matches!(resolution, KeypadResolution::Navigation { .. }));
            }
        }
    }

    fn keys(indexes: impl IntoIterator<Item = u8>) -> Vec<DisplayTarget> {
        indexes.into_iter().map(DisplayTarget::Key).collect()
    }

    fn sync() -> DisplaySync<FakeDisplaySink> {
        DisplaySync::new(FakeDisplaySink::default())
    }

    #[test]
    fn first_draw_writes_all_fifteen_keys_in_wire_order() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        let report = sync.sync_page(&session, &art).unwrap();
        assert!(report.is_complete());
        assert_eq!(report.written, keys(0..15));
        assert_eq!(sync.sink().successful(), keys(0..15));
        let requested: Vec<u8> = art.calls.borrow().iter().map(|r| r.key_index).collect();
        assert_eq!(requested, (0..15).collect::<Vec<_>>());
        assert_eq!(sync.drawn().key(14), Some(KEY_JPEG));
        assert_eq!(sync.drawn().key(15), None);
    }

    #[test]
    fn unchanged_page_is_not_resent() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        sync.sync_page(&session, &art).unwrap();
        let plan = sync.plan_page(&session, &art).unwrap();
        assert!(plan.is_empty());
        assert!(sync.apply(plan).unwrap().written.is_empty());
        assert_eq!(sync.sink().writes.len(), 15);
    }

    #[test]
    fn navigation_redraws_only_changed_keys_and_requests_back_badge() {
        let (mut session, art, mut sync) = (session(), Art::new(), sync());
        sync.sync_page(&session, &art).unwrap();
        let root = sync.drawn().clone();
        press(&mut session, 1);
        let plan = sync.plan_page(&session, &art).unwrap();
        let changed: Vec<u8> = plan.writes().iter().map(|w| w.request.key_index).collect();
        // Only keys whose bytes differ from root are written; shared blanks are not.
        let expected: Vec<u8> = (0..15u8)
            .filter(|&i| {
                let request = &art.calls.borrow()[15 + usize::from(i)];
                root.key(i) != Some(Art::image(request).as_slice())
            })
            .collect();
        assert_eq!(changed, expected);
        assert!(changed.len() < 15 && changed.contains(&0));
        assert_eq!(
            plan.writes()[0].request.content,
            KeyContent::Back {
                depth: 1,
                parent_page_id: "root".into()
            }
        );
        let report = sync.apply(plan).unwrap();
        assert_eq!(report.written, keys(changed.iter().copied()));
        assert_eq!(sync.sink().writes.len(), 15 + changed.len());

        // Second level shows the L2 badge; returning to root restores slot 0.
        press(&mut session, 1);
        let plan = sync.plan_page(&session, &art).unwrap();
        assert!(matches!(
            plan.writes()[0].request.content,
            KeyContent::Back { depth: 2, .. }
        ));
        sync.apply(plan).unwrap();
        session.reset();
        let plan = sync.plan_page(&session, &art).unwrap();
        assert!(matches!(
            plan.writes()[0].request.content,
            KeyContent::Action { ref label, .. } if label == "Mute"
        ));
        assert!(matches!(
            &plan.writes().iter().find(|w| w.request.key_index == 1).unwrap().request.content,
            KeyContent::Folder { target_page_id, .. } if target_page_id == "development"
        ));
    }

    #[test]
    fn invalid_jpeg_is_rejected_before_any_write() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        let request = KeyImageRequest {
            key_index: 14,
            content: KeyContent::Empty {
                page_id: "root".into(),
            },
        };
        let mut wrong = KEY_JPEG.to_vec();
        wrong.push(0);
        art.overrides.borrow_mut().insert(request, Some(wrong));
        let error = sync.sync_page(&session, &art).unwrap_err();
        assert!(matches!(
            error,
            DisplayError::InvalidImage {
                target: DisplayTarget::Key(14),
                ..
            }
        ));
        assert!(sync.sink().writes.is_empty());
        assert_eq!(sync.drawn().key(0), None);
    }

    #[test]
    fn missing_artwork_is_reported_and_not_written() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        let request = KeyImageRequest {
            key_index: 3,
            content: KeyContent::Empty {
                page_id: "root".into(),
            },
        };
        art.overrides.borrow_mut().insert(request.clone(), None);
        let plan = sync.plan_page(&session, &art).unwrap();
        assert_eq!(plan.missing(), &[request]);
        let report = sync.apply(plan).unwrap();
        assert!(!report.written.contains(&DisplayTarget::Key(3)));
        assert_eq!(report.written.len(), 14);
    }

    #[test]
    fn transport_failure_mid_page_accumulates_and_leaves_key_dirty() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        sync.sync_page(&session, &art).unwrap();
        // Same bytes, but a failed rewrite must make the key unknown.
        sync.invalidate();
        let failure = SinkError::Transport("disconnected".into());
        sync.sink_mut()
            .fail_next(DisplayTarget::Key(4), failure.clone());
        sync.sink_mut()
            .fail_next(DisplayTarget::Key(9), failure.clone());
        let report = sync.sync_page(&session, &art).unwrap();
        assert!(!report.is_complete());
        assert_eq!(
            report.written,
            keys((0..15).filter(|i| ![4, 9].contains(i)))
        );
        assert_eq!(
            report.failures,
            vec![
                WriteFailure {
                    target: DisplayTarget::Key(4),
                    error: failure.clone(),
                    attempts: 1
                },
                WriteFailure {
                    target: DisplayTarget::Key(9),
                    error: failure,
                    attempts: 1
                },
            ]
        );
        assert_eq!(sync.drawn().key(4), None);
        assert!(sync.drawn().key(5).is_some());
        let retry = sync.plan_page(&session, &art).unwrap();
        let dirty: Vec<u8> = retry.writes().iter().map(|w| w.request.key_index).collect();
        assert_eq!(dirty, [4, 9]);
    }

    #[test]
    fn short_write_is_never_retried_and_stops_the_plan() {
        let (session, art) = (session(), Art::new());
        let mut sync = sync().with_retry(RetryPolicy {
            transport_retries: 3,
        });
        let short = SinkError::ShortWrite {
            written: 10,
            expected: 65,
        };
        sync.sink_mut()
            .fail_next(DisplayTarget::Key(2), short.clone());
        let report = sync.sync_page(&session, &art).unwrap();
        assert_eq!(report.written, keys([0, 1]));
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].error, short);
        assert_eq!(report.failures[0].attempts, 1);
        assert_eq!(report.not_attempted, keys(3..15));
        let attempts = sync.sink().writes.iter();
        assert_eq!(
            attempts
                .filter(|w| w.target == DisplayTarget::Key(2))
                .count(),
            1
        );
        assert_eq!(sync.drawn().key(2), None);
        assert_eq!(sync.drawn().key(3), None);
        let resumed = sync.sync_page(&session, &art).unwrap();
        assert_eq!(resumed.written, keys(2..15));
    }

    #[test]
    fn transport_retries_are_opt_in_and_bounded() {
        let (session, art) = (session(), Art::new());
        let failure = SinkError::Transport("busy".into());
        let mut none = sync();
        none.sink_mut()
            .fail_next(DisplayTarget::Key(0), failure.clone());
        none.sync_page(&session, &art).unwrap();
        assert_eq!(none.drawn().key(0), None);

        let mut bounded = sync().with_retry(RetryPolicy {
            transport_retries: 1,
        });
        for _ in 0..2 {
            bounded
                .sink_mut()
                .fail_next(DisplayTarget::Key(0), failure.clone());
        }
        bounded
            .sink_mut()
            .fail_next(DisplayTarget::Key(1), failure.clone());
        let report = bounded.sync_page(&session, &art).unwrap();
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].target, DisplayTarget::Key(0));
        assert_eq!(report.failures[0].attempts, 2);
        assert!(report.written.contains(&DisplayTarget::Key(1)));
        let key1 = bounded.sink().writes.iter();
        assert_eq!(
            key1.filter(|w| w.target == DisplayTarget::Key(1)).count(),
            2
        );
    }

    #[test]
    fn invalidate_and_resume_force_full_redraw_and_reject_stale_plans() {
        let (mut session, art, mut sync) = (session(), Art::new(), sync());
        sync.sync_page(&session, &art).unwrap();
        press(&mut session, 1);
        let stale = sync.plan_page(&session, &art).unwrap();
        sync.invalidate();
        assert_eq!(sync.apply(stale).unwrap_err(), DisplayError::StalePlan);
        assert_eq!(sync.sync_page(&session, &art).unwrap().written, keys(0..15));

        sync.suspend();
        let action = sync.resume();
        assert_eq!(
            action.steps,
            [
                ResumeStep::EnterSoftwareMode,
                ResumeStep::QueryFirmware,
                ResumeStep::RefreshDisplay
            ]
        );
        assert!(sync.drawn().key(0).is_none() && sync.drawn().dial().is_none());
        assert_eq!(sync.sync_page(&session, &art).unwrap().written, keys(0..15));
    }

    #[test]
    fn ordering_is_deterministic_across_runs() {
        let run = || {
            let (mut session, art, mut sync) = (session(), Art::new(), sync());
            sync.sync_page(&session, &art).unwrap();
            press(&mut session, 1);
            sync.sync_page(&session, &art).unwrap();
            sync.into_sink().writes
        };
        let first = run();
        assert_eq!(first, run());
        let indexes =
            |range: &[RecordedWrite]| range.windows(2).all(|pair| pair[0].target < pair[1].target);
        assert!(indexes(&first[..15]) && indexes(&first[15..]));
    }

    #[test]
    fn sink_bounds_are_checked_before_any_write() {
        let (session, art) = (session(), Art::new());
        let mut sync = DisplaySync::new(FakeDisplaySink {
            capabilities: DisplayCapabilities {
                key_count: 14,
                dial: false,
            },
            ..FakeDisplaySink::default()
        });
        assert_eq!(
            sync.sync_page(&session, &art).unwrap_err(),
            DisplayError::Unsupported(DisplayTarget::Key(14))
        );
        assert!(sync.sink().writes.is_empty());
        let mut fake = FakeDisplaySink::default();
        assert_eq!(
            fake.write_key_image(15, KEY_JPEG, &mut |_| {}),
            Err(SinkError::Unsupported(DisplayTarget::Key(15)))
        );
        assert!(fake.writes.is_empty());
    }

    #[test]
    fn flat_profiles_cannot_be_planned() {
        let mut profile = session().profile().clone();
        profile.schema_version = 1;
        profile.keypad = None;
        let session = ProfileSession::new(profile).unwrap();
        assert_eq!(
            sync().plan_page(&session, &Art::new()).unwrap_err(),
            DisplayError::NotKeypadProfile
        );
    }

    #[test]
    fn pacing_hooks_run_between_chunks_and_images() {
        let (session, art) = (session(), Art::new());
        let chunks = Rc::new(RefCell::new(Vec::new()));
        let images = Rc::new(RefCell::new(Vec::new()));
        let (c, i) = (chunks.clone(), images.clone());
        let mut sync = DisplaySync::new(FakeDisplaySink {
            chunks: 3,
            ..FakeDisplaySink::default()
        })
        .with_chunk_pacing(move |n| c.borrow_mut().push(n))
        .with_image_pacing(move |n| i.borrow_mut().push(n));
        sync.sync_page(&session, &art).unwrap();
        assert_eq!(chunks.borrow().len(), 30);
        assert_eq!(&chunks.borrow()[..2], &[1, 2]);
        assert_eq!(*images.borrow(), (1..15).collect::<Vec<_>>());
    }

    struct DialArt;

    impl DialImageProvider for DialArt {
        fn dial_image(&self, request: &DialImageRequest) -> Option<Vec<u8>> {
            match request.function.as_deref() {
                None => Some(dial_jpeg(1)),
                Some("volume") => Some(dial_jpeg(2)),
                Some("bad") => Some(KEY_JPEG.to_vec()),
                Some(_) => None,
            }
        }
    }

    fn dial_request(function: Option<&str>) -> DialImageRequest {
        DialImageRequest {
            profile: "Desk".into(),
            function: function.map(Into::into),
        }
    }

    #[test]
    fn dial_writes_only_when_its_image_changes() {
        let mut sync = sync();
        let plan = |sync: &DisplaySync<_>, function| {
            sync.plan_dial(dial_request(function), &DialArt).unwrap()
        };
        let first = plan(&sync, None);
        assert_eq!(first.write().unwrap().0, &dial_request(None));
        assert_eq!(
            sync.apply_dial(first).unwrap().written,
            [DisplayTarget::Dial]
        );
        assert!(plan(&sync, None).write().is_none());
        sync.apply_dial(plan(&sync, None)).unwrap();
        sync.apply_dial(plan(&sync, Some("volume"))).unwrap();
        let unknown = plan(&sync, Some("other"));
        assert!(unknown.is_missing() && unknown.write().is_none());
        assert_eq!(sync.sink().successful(), [DisplayTarget::Dial; 2]);
        assert_eq!(sync.drawn().dial(), Some(dial_jpeg(2).as_slice()));

        sync.invalidate();
        assert!(plan(&sync, Some("volume")).write().is_some());
    }

    #[test]
    fn dial_rejects_key_sized_images_and_failures_leave_it_dirty() {
        let mut sync = sync();
        assert!(matches!(
            sync.plan_dial(dial_request(Some("bad")), &DialArt)
                .unwrap_err(),
            DisplayError::InvalidImage {
                target: DisplayTarget::Dial,
                ..
            }
        ));
        let plan = sync.plan_dial(dial_request(None), &DialArt).unwrap();
        sync.apply_dial(plan).unwrap();
        sync.invalidate();
        sync.sink_mut().fail_next(
            DisplayTarget::Dial,
            SinkError::ShortWrite {
                written: 1,
                expected: 65,
            },
        );
        let plan = sync.plan_dial(dial_request(None), &DialArt).unwrap();
        let report = sync.apply_dial(plan).unwrap();
        assert_eq!(report.failures[0].attempts, 1);
        assert_eq!(sync.drawn().dial(), None);

        let mut no_dial = DisplaySync::new(FakeDisplaySink {
            capabilities: DisplayCapabilities {
                key_count: 15,
                dial: false,
            },
            ..FakeDisplaySink::default()
        });
        let plan = no_dial.plan_dial(dial_request(None), &DialArt).unwrap();
        assert_eq!(
            no_dial.apply_dial(plan).unwrap_err(),
            DisplayError::Unsupported(DisplayTarget::Dial)
        );
        assert!(no_dial.sink().writes.is_empty());
    }

    /// Every key gets the same image chosen by `tag`; blanks included.
    struct Uniform(u8);

    impl KeyImageProvider for Uniform {
        fn key_image(&self, _: &KeyImageRequest) -> Option<Vec<u8>> {
            Some(variant(KEY_JPEG, self.0))
        }
    }

    #[test]
    fn plan_made_before_a_failed_apply_is_rejected() {
        let session = session();
        // Queued up front so `sink_mut` (which invalidates) is not involved:
        // key 4's first write succeeds, its second is a short write.
        let mut sink = FakeDisplaySink::default();
        sink.fail_after(
            DisplayTarget::Key(4),
            1,
            SinkError::ShortWrite {
                written: 3,
                expected: 65,
            },
        );
        let mut sync = DisplaySync::new(sink);
        assert!(sync.sync_page(&session, &Uniform(1)).unwrap().is_complete());
        let held = sync.plan_page(&session, &Uniform(1)).unwrap();
        assert!(held.is_empty());
        let report = sync.sync_page(&session, &Uniform(2)).unwrap();
        assert_eq!(report.written, keys(0..4));
        assert_eq!(report.not_attempted, keys(5..15));
        assert_eq!(sync.drawn().key(4), None);
        assert_eq!(sync.apply(held).unwrap_err(), DisplayError::StalePlan);

        // A plan made before a successful apply is stale too.
        let before = sync.plan_page(&session, &Uniform(2)).unwrap();
        assert_eq!(before.writes().len(), 11);
        sync.sync_page(&session, &Uniform(2)).unwrap();
        assert_eq!(sync.apply(before).unwrap_err(), DisplayError::StalePlan);

        // Dial: a plan made before a failed dial write is rejected.
        let mut sink = FakeDisplaySink::default();
        sink.fail_after(DisplayTarget::Dial, 1, SinkError::Transport("gone".into()));
        let mut sync = DisplaySync::new(sink);
        let first = sync.plan_dial(dial_request(None), &DialArt).unwrap();
        sync.apply_dial(first).unwrap();
        let held = sync.plan_dial(dial_request(None), &DialArt).unwrap();
        assert!(held.write().is_none());
        let volume = sync
            .plan_dial(dial_request(Some("volume")), &DialArt)
            .unwrap();
        assert_eq!(sync.apply_dial(volume).unwrap().failures.len(), 1);
        assert_eq!(sync.apply_dial(held).unwrap_err(), DisplayError::StalePlan);
    }

    #[test]
    fn retry_count_does_not_overflow_at_the_u8_boundary() {
        let session = session();
        let mut sync = sync().with_retry(RetryPolicy {
            transport_retries: u8::MAX,
        });
        for _ in 0..300 {
            sync.sink_mut()
                .fail_next(DisplayTarget::Key(0), SinkError::Transport("busy".into()));
        }
        let report = sync.sync_page(&session, &Art::new()).unwrap();
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].attempts, 256);
        let key0 = sync.sink().writes.iter();
        assert_eq!(
            key0.filter(|w| w.target == DisplayTarget::Key(0)).count(),
            256
        );
    }

    #[test]
    fn plans_from_another_instance_are_rejected() {
        let (session, art) = (session(), Art::new());
        let (mut a, mut b) = (sync(), sync());
        let from_b = b.plan_page(&session, &art).unwrap();
        assert_eq!(a.apply(from_b).unwrap_err(), DisplayError::StalePlan);
        let dial_from_b = b.plan_dial(dial_request(None), &DialArt).unwrap();
        assert_eq!(
            a.apply_dial(dial_from_b).unwrap_err(),
            DisplayError::StalePlan
        );
        assert!(a.sink().writes.is_empty());
        assert_eq!(b.sync_page(&session, &art).unwrap().written.len(), 15);
    }

    #[test]
    fn sink_mut_invalidates_drawn_state_and_outstanding_plans() {
        let (session, art, mut sync) = (session(), Art::new(), sync());
        sync.sync_page(&session, &art).unwrap();
        let held = sync.plan_page(&session, &art).unwrap();
        let held_dial = sync.plan_dial(dial_request(None), &DialArt).unwrap();
        sync.sink_mut()
            .write_key_image(3, KEY_JPEG, &mut |_| {})
            .unwrap();
        assert_eq!(sync.drawn().key(3), None);
        assert_eq!(sync.apply(held).unwrap_err(), DisplayError::StalePlan);
        assert_eq!(
            sync.apply_dial(held_dial).unwrap_err(),
            DisplayError::StalePlan
        );
        assert_eq!(sync.sync_page(&session, &art).unwrap().written.len(), 15);
    }
}
