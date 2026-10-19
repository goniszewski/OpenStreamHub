//! Generated 240×240 M131E dial artwork for `DisplaySync`, offline and deterministic.
//!
//! [`DialArtwork`] is a `DialImageProvider` for four kinds of dial content
//! ([`DialContent`]): the active dial function's name, a digital `HH:MM` clock,
//! a simple analog clock, or the machine's CPU usage. Everything is drawn into a 240×240 RGB buffer
//! with a built-in rasteriser (a 5×7 bitmap font and seven-segment digits, so
//! no font files are read or downloaded), encoded through
//! [`crate::image_prep::prepare_image`] (the same encoder and validation as
//! `dial prepare-image`), and checked again with `dial::validate_jpeg`.
//!
//! Function and profile faces are prepared before a run starts. A clock face
//! is drawn when its minute is first requested and cached, so at most one image
//! is encoded per minute. Time comes from an injected [`Clock`], so tests are
//! deterministic. A CPU face is drawn when its whole percentage is first
//! requested and cached the same way. Usage comes from an injected
//! [`StatSource`] (by default [`crate::cpu_usage::MacCpuStat`]), sampled at most
//! once per [`CPU_SAMPLE_INTERVAL`] and only while building a request, which
//! the live runtime does on its display thread.
//!
//! The provider performs no device I/O. Requests are opaque strings built by
//! [`DialArtwork::request`]: `profile`, `function:<index>`, `clock:HH:MM`,
//! `analog:HH:MM`, `cpu:<0-100>`, or `cpu:--`; anything else returns `None`.
//! Hardware acceptance of the images is unverified; see
//! `docs/masterhub-display-sync.md`.

use std::{
    cell::RefCell,
    collections::VecDeque,
    f64::consts::PI,
    fmt,
    io::{Cursor, Write},
    time::{Duration, Instant},
};

use cooltek_sdk_masterhub::dial;
use cooltek_sdk_service::{
    Profile,
    display_sync::{DialImageProvider, DialImageRequest},
};
use image::{ImageFormat, Rgb, RgbImage};

use crate::{
    cpu_usage::MacCpuStat,
    image_prep::{PrepareOptions, prepare_image},
};

/// Dial image side in pixels.
const SIDE: u32 = dial::IMAGE_WIDTH as u32;
/// JPEG quality for dial faces: flat colors stay clean and images stay small,
/// so fewer reports are written per update.
const QUALITY: u8 = 85;
/// Characters per line of a function or profile name at the large scale.
const LINE_CHARS: usize = 11;
/// Most characters shown from a name, over at most two lines.
const NAME_CHARS: usize = 2 * LINE_CHARS;
/// Pixel centre of the dial image, shared by every round element.
const CENTRE: f64 = (SIDE as f64 - 1.0) / 2.0;
/// Most position dots drawn under a function name (`dial::MAX_FUNCTIONS`).
const MAX_DOTS: usize = 16;

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const DIM: [u8; 3] = [90, 98, 112];
const ACCENT: [u8; 3] = [40, 140, 220];
const HAND: [u8; 3] = [230, 120, 40];

/// Shortest time between two CPU usage samples.
pub const CPU_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// From this CPU usage up the gauge is drawn in the warning color.
const CPU_HIGH: u8 = 80;

/// What the dial screen shows behind `--display-sync`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, clap::ValueEnum)]
pub enum DialContent {
    /// Nothing is drawn on the dial.
    #[default]
    Off,
    /// The active dial function's name, or the profile name without functions.
    Function,
    /// A digital `HH:MM` clock in local time.
    Clock,
    /// A simple analog clock in local time.
    AnalogClock,
    /// Whole-machine CPU usage in percent with a ring gauge (macOS only;
    /// elsewhere, or while no sample is available, `--`).
    Cpu,
    /// The picture the profile carries in its `dial_display` setting.
    Image,
}

impl DialContent {
    /// Every value, in command-line order.
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::Function,
        Self::Clock,
        Self::AnalogClock,
        Self::Cpu,
        Self::Image,
    ];

    /// The command-line spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Function => "function",
            Self::Clock => "clock",
            Self::AnalogClock => "analog-clock",
            Self::Cpu => "cpu",
            Self::Image => "image",
        }
    }
}

impl fmt::Display for DialContent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A wall-clock time of day, minute resolution.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WallTime {
    /// Hour, 0 through 23.
    pub hour: u8,
    /// Minute, 0 through 59.
    pub minute: u8,
}

impl WallTime {
    /// A time of day, wrapping out-of-range values into 0–23 and 0–59.
    pub fn new(hour: u8, minute: u8) -> Self {
        Self {
            hour: hour % 24,
            minute: minute % 60,
        }
    }
}

/// Source of the current time of day; injected so tests are deterministic.
pub trait Clock: Send {
    /// The current local time of day.
    fn now(&self) -> WallTime;
}

/// The system clock in local time.
///
/// On Unix it reads local time with `localtime_r`. glibc does not re-read the
/// time zone inside `localtime_r`, so on Linux a time-zone change takes effect
/// only after the process restarts (macOS re-checks it). Elsewhere, or when the
/// conversion fails, it reports UTC.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> WallTime {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        local_time(seconds).unwrap_or_else(|| {
            let of_day = seconds % 86_400;
            WallTime::new((of_day / 3600) as u8, (of_day / 60 % 60) as u8)
        })
    }
}

#[cfg(unix)]
fn local_time(seconds: u64) -> Option<WallTime> {
    let time: libc::time_t = seconds.try_into().ok()?;
    // SAFETY: `tm` is plain old data; all-zero bytes (including a null
    // `tm_zone` pointer) are a valid value that `localtime_r` overwrites.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers refer to live, properly aligned locals for the
    // duration of the call; the reentrant variant keeps no shared state.
    let result = unsafe { libc::localtime_r(&time, &mut tm) };
    if result.is_null() {
        return None;
    }
    Some(WallTime::new(
        u8::try_from(tm.tm_hour).ok()?,
        u8::try_from(tm.tm_min).ok()?,
    ))
}

#[cfg(not(unix))]
fn local_time(_: u64) -> Option<WallTime> {
    None
}

/// A clock whose time the owner sets; for tests and replay.
#[derive(Clone, Debug)]
pub struct ManualClock(std::sync::Arc<std::sync::Mutex<WallTime>>);

impl ManualClock {
    /// Starts at `time`.
    pub fn new(time: WallTime) -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(time)))
    }

    /// Sets the time every clone reports.
    pub fn set(&self, time: WallTime) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = time;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> WallTime {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A system statistic for an optional dial widget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatKind {
    /// Processor usage across all cores.
    Cpu,
    /// Graphics processor usage.
    Gpu,
}

/// One statistic sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StatReading {
    /// What was measured.
    pub kind: StatKind,
    /// Usage from 0 through 100.
    pub percent: u8,
}

/// Source of CPU or GPU statistics for dial faces.
///
/// [`crate::cpu_usage::MacCpuStat`] reports CPU usage on macOS and nothing
/// elsewhere; [`DialContent::Cpu`] draws it. No GPU source ships: GPU usage and
/// temperatures need private IOKit or SMC interfaces and are out of scope.
/// [`FakeStatSource`] replays readings for tests.
pub trait StatSource {
    /// The latest sample, or `None` when it is unavailable.
    fn sample(&mut self) -> Option<StatReading>;
}

/// Replays queued readings, then reports nothing.
#[derive(Clone, Debug, Default)]
pub struct FakeStatSource {
    readings: VecDeque<StatReading>,
}

impl FakeStatSource {
    /// Replays `readings` in order; percentages above 100 are clamped.
    pub fn new(readings: impl IntoIterator<Item = StatReading>) -> Self {
        Self {
            readings: readings
                .into_iter()
                .map(|reading| StatReading {
                    percent: reading.percent.min(100),
                    ..reading
                })
                .collect(),
        }
    }
}

impl StatSource for FakeStatSource {
    fn sample(&mut self) -> Option<StatReading> {
        self.readings.pop_front()
    }
}

/// The CPU usage shown on the dial, sampled at most once per `interval`.
struct CpuGauge {
    source: Box<dyn StatSource + Send>,
    interval: Duration,
    last_sample: Option<Instant>,
    /// The newest sample's percentage; `None` shows `--`.
    percent: Option<u8>,
    samples: u64,
    /// Set once a missing sample after the baseline was reported.
    unavailable_logged: bool,
    /// Set when the source panicked; it is never called again.
    broken: bool,
}

impl CpuGauge {
    fn new(source: Box<dyn StatSource + Send>) -> Self {
        Self {
            source,
            interval: CPU_SAMPLE_INTERVAL,
            last_sample: None,
            percent: None,
            samples: 0,
            unavailable_logged: false,
            broken: false,
        }
    }

    /// The percentage to show at `now`, sampling first once `interval` has
    /// passed since the last sample. The first sample is a baseline, so a
    /// missing value there is expected; the first missing value after it is
    /// reported once on stderr, and every missing value shows `--`. A source
    /// that panics counts as unavailable for the rest of the run.
    fn percent(&mut self, now: Instant) -> Option<u8> {
        if self
            .last_sample
            .is_some_and(|last| now.saturating_duration_since(last) < self.interval)
        {
            return self.percent;
        }
        self.last_sample = Some(now);
        self.samples += 1;
        let reading = if self.broken {
            None
        } else {
            let source = &mut self.source;
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| source.sample()))
                .unwrap_or_else(|_| {
                    self.broken = true;
                    None
                })
        };
        self.percent = reading
            .filter(|reading| reading.kind == StatKind::Cpu)
            .map(|reading| reading.percent.min(100));
        if self.percent.is_none() && (self.samples > 1 || self.broken) && !self.unavailable_logged {
            self.unavailable_logged = true;
            // stdout carries JSONL only; the dial's `display_sync` record shows
            // `cpu:--`. A failed stderr write (for example a closed pipe) is
            // ignored, so logging can never panic.
            let _ = writeln!(
                std::io::stderr(),
                "masterhub-service: CPU usage is unavailable; the dial shows --"
            );
        }
        self.percent
    }

    /// True while only the baseline was taken and it produced no value, so the
    /// first face waits for the next sample instead of showing `--`.
    fn awaiting_first_value(&self) -> bool {
        self.samples <= 1 && self.percent.is_none() && !self.broken
    }
}

/// Prepared function and profile faces, a clock for clock faces, and a CPU
/// usage sampler for the CPU face.
pub struct DialArtwork {
    content: DialContent,
    profile_name: String,
    /// Only for [`DialContent::Function`].
    profile_face: Option<Vec<u8>>,
    functions: Vec<Vec<u8>>,
    /// Only for [`DialContent::Image`]: the profile's validated dial picture.
    image: Option<Vec<u8>>,
    /// Only for [`DialContent::Image`] with an animation in the profile.
    /// Boxed for the same reason as `cpu`.
    animation: Option<Box<Animation>>,
    clock: Box<dyn Clock>,
    /// Only for [`DialContent::Cpu`].
    /// Boxed: inline, it makes `DialArtwork` large enough to trip clippy's
    /// `large_enum_variant` on the runtime's `DisplayContent`.
    cpu: Option<Box<RefCell<CpuGauge>>>,
    /// The last clock or CPU face drawn: its request key and bytes.
    cached: RefCell<Option<(String, Vec<u8>)>>,
    /// Quarter turns clockwise applied to every face served, so a dial sitting turned on the base reads upright.
    quarter_turns: u8,
}

/// The frames of an animated dial picture and the clock they play against.
struct Animation {
    frames: Vec<Vec<u8>>,
    delays_ms: Vec<u32>,
    loop_count: u32,
    /// When playback began: frame 0 is up at this moment.
    started: Instant,
}

/// The frame showing `elapsed` after playback began. A positive `loop_count`
/// is the number of plays; after the last the final frame stays up.
fn animation_frame(delays_ms: &[u32], loop_count: u32, elapsed: Duration) -> usize {
    let cycle: u64 = delays_ms.iter().map(|&delay| u64::from(delay)).sum();
    let mut at = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if cycle == 0 {
        return 0;
    }
    if loop_count > 0 && at >= cycle * u64::from(loop_count) {
        return delays_ms.len() - 1;
    }
    at %= cycle;
    let mut index = 0;
    for (frame, &delay) in delays_ms.iter().enumerate() {
        index = frame;
        if at < u64::from(delay) {
            break;
        }
        at -= u64::from(delay);
    }
    index
}

/// How long until the frame showing `elapsed` after playback began gives way
/// to the next, or `None` once a finite animation has played its last time.
fn frame_remaining(delays_ms: &[u32], loop_count: u32, elapsed: Duration) -> Option<Duration> {
    let cycle: u64 = delays_ms.iter().map(|&delay| u64::from(delay)).sum();
    let at = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    if cycle == 0 || (loop_count > 0 && at >= cycle * u64::from(loop_count)) {
        return None;
    }
    let mut left = at % cycle;
    for &delay in delays_ms {
        let delay = u64::from(delay);
        if left < delay {
            return Some(Duration::from_millis(delay - left));
        }
        left -= delay;
    }
    None
}

impl fmt::Debug for DialArtwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DialArtwork")
            .field("content", &self.content)
            .field("profile_name", &self.profile_name)
            .field("functions", &self.functions.len())
            .finish_non_exhaustive()
    }
}

impl DialArtwork {
    /// Draws, encodes, and validates every function and profile face for
    /// `profile`, or one clock or CPU face, so a broken encoder fails before a
    /// run. CPU content reads [`crate::cpu_usage::MacCpuStat`] unless
    /// [`Self::with_stat_source`] replaces it; nothing is sampled here.
    ///
    /// # Errors
    /// Fails for [`DialContent::Off`] or when encoding or dial validation fails.
    pub fn prepare(
        content: DialContent,
        profile: &Profile,
        clock: Box<dyn Clock>,
    ) -> Result<Self, String> {
        if content == DialContent::Off {
            return Err("dial artwork needs dial content other than off".into());
        }
        let image = if content == DialContent::Image {
            Some(decode_profile_dial_image(profile)?)
        } else {
            None
        };
        let animation = match profile
            .dial_display
            .as_ref()
            .and_then(|d| d.animation.as_ref())
        {
            Some(animation) if content == DialContent::Image => Some(Box::new(Animation {
                frames: animation.jpegs()?,
                delays_ms: animation
                    .frames
                    .iter()
                    .map(|frame| frame.delay_ms)
                    .collect(),
                loop_count: animation.loop_count,
                started: Instant::now(),
            })),
            _ => None,
        };
        let names: Vec<&str> = profile
            .dial_functions
            .iter()
            .flatten()
            .map(|function| function.name.as_str())
            .collect();
        let count = names.len();
        let functions = if content == DialContent::Function {
            names
                .iter()
                .enumerate()
                .map(|(index, name)| encode(&draw_function(name, index, count)))
                .collect::<Result<_, _>>()?
        } else {
            Vec::new()
        };
        let artwork = Self {
            content,
            profile_name: profile.name.clone(),
            profile_face: (content == DialContent::Function)
                .then(|| encode(&draw_profile(&profile.name)))
                .transpose()?,
            functions,
            image,
            animation,
            clock,
            cpu: (content == DialContent::Cpu)
                .then(|| Box::new(RefCell::new(CpuGauge::new(Box::new(MacCpuStat::new()))))),
            cached: RefCell::new(None),
            quarter_turns: 0,
        };
        // A clock or CPU face cannot be prepared in advance; prove the path works.
        match content {
            DialContent::Off | DialContent::Function | DialContent::Image => {}
            DialContent::Cpu => {
                encode(&draw_cpu(Some(100)))?;
            }
            DialContent::Clock | DialContent::AnalogClock => {
                encode(&draw_clock(WallTime::new(0, 0), content))?;
            }
        }
        Ok(artwork)
    }

    /// Replaces the CPU usage source (CPU content only; ignored otherwise).
    pub fn with_stat_source(self, source: Box<dyn StatSource + Send>) -> Self {
        if let Some(gauge) = &self.cpu {
            gauge.borrow_mut().source = source;
        }
        self
    }

    /// Turns every face served by `quarter_turns` quarter turns clockwise (0 to 3).
    pub fn with_turn(mut self, quarter_turns: u8) -> Self {
        self.quarter_turns = quarter_turns % 4;
        self
    }

    /// The configured content.
    pub fn content(&self) -> DialContent {
        self.content
    }

    /// The request for what the dial should show now: the function at
    /// `dial_function` (or the profile face without functions), the clock's
    /// current minute, or the whole CPU percentage (`cpu:--` while none is
    /// available). CPU content samples here, at most once per
    /// [`CPU_SAMPLE_INTERVAL`]. Two requests are equal exactly when they
    /// describe the same face.
    pub fn request(&self, dial_function: Option<usize>) -> DialImageRequest {
        let function = match self.content {
            DialContent::Off => None,
            DialContent::Function => {
                Some(match dial_function.filter(|&i| i < self.functions.len()) {
                    Some(index) => format!("function:{index}"),
                    None => "profile".into(),
                })
            }
            DialContent::Image => Some(match &self.animation {
                Some(animation) => format!(
                    "image:{}",
                    animation_frame(
                        &animation.delays_ms,
                        animation.loop_count,
                        animation.started.elapsed()
                    )
                ),
                None => "image".into(),
            }),
            DialContent::Cpu => Some(match self.cpu_percent(Instant::now()) {
                Some(percent) => format!("cpu:{percent}"),
                None => "cpu:--".into(),
            }),
            DialContent::Clock | DialContent::AnalogClock => {
                let time = self.clock.now();
                let prefix = if self.content == DialContent::Clock {
                    "clock"
                } else {
                    "analog"
                };
                Some(format!("{prefix}:{:02}:{:02}", time.hour, time.minute))
            }
        };
        DialImageRequest {
            profile: self.profile_name.clone(),
            function,
        }
    }

    /// How long until the animated picture's next frame is due, or `None` for
    /// a still picture and for an animation that has played its last time.
    pub fn next_frame_in(&self) -> Option<Duration> {
        let animation = self.animation.as_ref()?;
        frame_remaining(
            &animation.delays_ms,
            animation.loop_count,
            animation.started.elapsed(),
        )
    }

    /// True when the dial picture plays frames, so the face changes with time.
    pub fn is_animated(&self) -> bool {
        self.animation.is_some()
    }

    /// Prepared bytes of the profile face (function content only).
    pub fn profile_face(&self) -> Option<&[u8]> {
        self.profile_face.as_deref()
    }

    /// Prepared bytes of a function face.
    pub fn function_face(&self, index: usize) -> Option<&[u8]> {
        self.functions.get(index).map(Vec::as_slice)
    }

    #[cfg(test)]
    pub(crate) fn with_cpu_interval(self, interval: Duration) -> Self {
        if let Some(gauge) = &self.cpu {
            gauge.borrow_mut().interval = interval;
        }
        self
    }

    /// True for CPU content until a sample after the baseline was taken (or
    /// the baseline already had a value): drawing now would only show `--`.
    pub fn awaiting_first_sample(&self) -> bool {
        self.cpu
            .as_ref()
            .is_some_and(|gauge| gauge.borrow().awaiting_first_value())
    }

    fn cpu_percent(&self, now: Instant) -> Option<u8> {
        self.cpu.as_ref()?.borrow_mut().percent(now)
    }

    /// The clock or CPU face for `key`, drawn by `draw` unless it is cached.
    fn timed_face(&self, key: &str, draw: impl FnOnce() -> RgbImage) -> Option<Vec<u8>> {
        if let Some((cached, bytes)) = &*self.cached.borrow()
            && cached == key
        {
            return Some(bytes.clone());
        }
        let bytes = encode(&draw()).ok()?;
        *self.cached.borrow_mut() = Some((key.to_owned(), bytes.clone()));
        Some(bytes)
    }
}

/// Parses a CPU face value: `--`, or 0 through 100 without leading zeros.
fn parse_percent(text: &str) -> Option<Option<u8>> {
    if text == "--" {
        return Some(None);
    }
    if text.is_empty()
        || text.len() > 3
        || !text.bytes().all(|byte| byte.is_ascii_digit())
        || (text.len() > 1 && text.starts_with('0'))
    {
        return None;
    }
    let percent: u8 = text.parse().ok()?;
    (percent <= 100).then_some(Some(percent))
}

/// Parses `HH:MM` with two digits each, in range.
fn parse_time(text: &str) -> Option<WallTime> {
    let (hour, minute) = text.split_once(':')?;
    if hour.len() != 2 || minute.len() != 2 {
        return None;
    }
    let (hour, minute): (u8, u8) = (hour.parse().ok()?, minute.parse().ok()?);
    (hour < 24 && minute < 60).then(|| WallTime::new(hour, minute))
}

impl DialImageProvider for DialArtwork {
    fn dial_image(&self, request: &DialImageRequest) -> Option<Vec<u8>> {
        let face = self.face(request)?;
        Some(turned(&face, self.quarter_turns).unwrap_or(face))
    }
}

/// `jpeg` turned `quarter_turns` quarter turns clockwise; `None` when it cannot be decoded or re-encoded.
fn turned(jpeg: &[u8], quarter_turns: u8) -> Option<Vec<u8>> {
    if quarter_turns.is_multiple_of(4) {
        return Some(jpeg.to_vec());
    }
    let image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .ok()?
        .to_rgb8();
    if image.dimensions() != (SIDE, SIDE) {
        return None;
    }
    let image = match quarter_turns % 4 {
        1 => image::imageops::rotate90(&image),
        2 => image::imageops::rotate180(&image),
        _ => image::imageops::rotate270(&image),
    };
    encode(&image).ok()
}

impl DialArtwork {
    /// The face for `request`, upright as drawn.
    fn face(&self, request: &DialImageRequest) -> Option<Vec<u8>> {
        let face = request.function.as_deref()?;
        if face == "image" && self.content == DialContent::Image {
            return self.image.clone();
        }
        if let (Some(index), Some(animation)) = (face.strip_prefix("image:"), &self.animation)
            && self.content == DialContent::Image
        {
            if index.starts_with('+') || (index.len() > 1 && index.starts_with('0')) {
                return None;
            }
            return animation.frames.get(index.parse::<usize>().ok()?).cloned();
        }
        if face == "profile" && self.content == DialContent::Function {
            return self.profile_face.clone();
        }
        if let Some(index) = face.strip_prefix("function:") {
            if index.starts_with('+') || (index.len() > 1 && index.starts_with('0')) {
                return None;
            }
            return self.function_face(index.parse().ok()?).map(<[u8]>::to_vec);
        }
        let (content, value) = match face.split_once(':')? {
            ("clock", time) if self.content == DialContent::Clock => (DialContent::Clock, time),
            ("analog", time) if self.content == DialContent::AnalogClock => {
                (DialContent::AnalogClock, time)
            }
            ("cpu", value) if self.content == DialContent::Cpu => (DialContent::Cpu, value),
            _ => return None,
        };
        if content == DialContent::Cpu {
            let percent = parse_percent(value)?;
            return self.timed_face(face, || draw_cpu(percent));
        }
        let time = parse_time(value)?;
        self.timed_face(face, || draw_clock(time, content))
    }
}

/// The profile's dial picture for [`DialContent::Image`], decoded and
/// re-checked: profile validation already ran, but a run still refuses to
/// start on anything the dial would reject.
fn decode_profile_dial_image(profile: &Profile) -> Result<Vec<u8>, String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let text = profile
        .dial_display
        .as_ref()
        .and_then(|display| display.image.as_deref())
        .ok_or("dial image content requires a profile dial_display image")?;
    let jpeg = STANDARD
        .decode(text)
        .map_err(|_| "the profile's dial image is not valid base64".to_string())?;
    dial::validate_jpeg(&jpeg)
        .map_err(|error| format!("the profile's dial image is not a valid dial image: {error}"))?;
    Ok(jpeg)
}

/// A profile whose dial plays `delays_ms.len()` solid-colour frames, for tests.
#[cfg(test)]
pub(crate) fn animated_test_profile(delays_ms: &[u32], loop_count: u32) -> Profile {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let frames: Vec<serde_json::Value> = delays_ms
        .iter()
        .enumerate()
        .map(|(index, delay)| {
            let shade = (40 + index * 30).min(255) as u8;
            let jpeg = encode(&RgbImage::from_pixel(SIDE, SIDE, Rgb([shade, 20, 200]))).unwrap();
            serde_json::json!({"jpeg_base64": STANDARD.encode(jpeg), "delay_ms": delay})
        })
        .collect();
    let json = serde_json::json!({
        "schema_version": 1, "name": "Moving", "bindings": [],
        "dial_display": {
            "content": "image",
            "image": frames[0]["jpeg_base64"],
            "animation": {"frames": frames, "loop_count": loop_count},
        }
    });
    Profile::from_json(serde_json::to_vec(&json).unwrap().as_slice()).unwrap()
}

/// Encodes through the shared preparation path, then validates explicitly.
fn encode(image: &RgbImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|error| format!("could not stage generated dial artwork: {error}"))?;
    let jpeg = prepare_image(
        &png,
        PrepareOptions {
            quality: QUALITY,
            ..PrepareOptions::default()
        },
    )?;
    dial::validate_jpeg(&jpeg)
        .map_err(|error| format!("generated dial artwork is not a valid dial image: {error}"))?;
    Ok(jpeg)
}

fn canvas() -> RgbImage {
    RgbImage::from_pixel(SIDE, SIDE, Rgb(BLACK))
}

fn fill(image: &mut RgbImage, x: i32, y: i32, width: i32, height: i32, color: [u8; 3]) {
    let side = SIDE as i32;
    for py in y.max(0)..(y + height).min(side) {
        for px in x.max(0)..(x + width).min(side) {
            image.put_pixel(px as u32, py as u32, Rgb(color));
        }
    }
}

/// A filled disc of `radius` around (`cx`, `cy`).
fn disc(image: &mut RgbImage, cx: f64, cy: f64, radius: f64, color: [u8; 3]) {
    let side = SIDE as i32;
    let (x0, x1) = ((cx - radius).floor() as i32, (cx + radius).ceil() as i32);
    let (y0, y1) = ((cy - radius).floor() as i32, (cy + radius).ceil() as i32);
    for py in y0.max(0)..=y1.min(side - 1) {
        for px in x0.max(0)..=x1.min(side - 1) {
            let (dx, dy) = (f64::from(px) - cx, f64::from(py) - cy);
            if dx * dx + dy * dy <= radius * radius {
                image.put_pixel(px as u32, py as u32, Rgb(color));
            }
        }
    }
}

/// A thick line from the centre at `degrees` clockwise from 12 o'clock,
/// between radii `from` and `to`.
fn spoke(image: &mut RgbImage, degrees: f64, from: f64, to: f64, half: f64, color: [u8; 3]) {
    let (sin, cos) = (degrees * PI / 180.0).sin_cos();
    let steps = ((to - from) * 2.0).ceil().max(1.0) as u32;
    for step in 0..=steps {
        let radius = from + (to - from) * f64::from(step) / f64::from(steps);
        disc(
            image,
            CENTRE + sin * radius,
            CENTRE - cos * radius,
            half,
            color,
        );
    }
}

/// A ring between radii `inner` and `outer` around the centre.
fn ring(image: &mut RgbImage, inner: f64, outer: f64, color: [u8; 3]) {
    for py in 0..SIDE {
        for px in 0..SIDE {
            let (dx, dy) = (f64::from(px) - CENTRE, f64::from(py) - CENTRE);
            let distance = (dx * dx + dy * dy).sqrt();
            if (inner..=outer).contains(&distance) {
                image.put_pixel(px, py, Rgb(color));
            }
        }
    }
}

/// Seven-segment masks, bit 0 = a (top) through bit 6 = g (middle).
const SEGMENTS: [u8; 10] = [
    0b011_1111, // 0
    0b000_0110, // 1
    0b101_1011, // 2
    0b100_1111, // 3
    0b110_0110, // 4
    0b110_1101, // 5
    0b111_1101, // 6
    0b000_0111, // 7
    0b111_1111, // 8
    0b110_1111, // 9
];
const DIGIT_WIDTH: i32 = 36;
const DIGIT_HEIGHT: i32 = 70;
const STROKE: i32 = 8;

/// One seven-segment digit with its top-left corner at (`x`, `y`).
fn segment_digit(image: &mut RgbImage, x: i32, y: i32, digit: u8, color: [u8; 3]) {
    segment_mask(image, x, y, SEGMENTS[usize::from(digit % 10)], color);
}

/// The segments set in `mask` of one seven-segment cell at (`x`, `y`).
fn segment_mask(image: &mut RgbImage, x: i32, y: i32, mask: u8, color: [u8; 3]) {
    let (w, h, t) = (DIGIT_WIDTH, DIGIT_HEIGHT, STROKE);
    let middle = y + (h - t) / 2;
    let rects = [
        (x, y, w, t),                         // a
        (x + w - t, y, t, h / 2),             // b
        (x + w - t, y + h / 2, t, h - h / 2), // c
        (x, y + h - t, w, t),                 // d
        (x, y + h / 2, t, h - h / 2),         // e
        (x, y, t, h / 2),                     // f
        (x, middle, w, t),                    // g
    ];
    for (bit, (rx, ry, rw, rh)) in rects.into_iter().enumerate() {
        if mask & (1 << bit) != 0 {
            fill(image, rx, ry, rw, rh, color);
        }
    }
}

/// Left edges of the four `HH:MM` digits, centred on the dial.
const CLOCK_DIGITS_X: [i32; 4] = [25, 67, 137, 179];
/// Top edge of the `HH:MM` digits.
const CLOCK_DIGITS_Y: i32 = 85;

fn draw_clock(time: WallTime, content: DialContent) -> RgbImage {
    let mut image = canvas();
    if content == DialContent::AnalogClock {
        ring(&mut image, 112.0, 116.0, DIM);
        for hour in 0..12 {
            let long = hour % 3 == 0;
            let inner = if long { 92.0 } else { 100.0 };
            spoke(
                &mut image,
                f64::from(hour) * 30.0,
                inner,
                108.0,
                if long { 3.0 } else { 2.0 },
                WHITE,
            );
        }
        let minutes = f64::from(time.minute);
        let hours = f64::from(time.hour % 12) + minutes / 60.0;
        spoke(&mut image, hours * 30.0, 0.0, 55.0, 5.0, WHITE);
        spoke(&mut image, minutes * 6.0, 0.0, 85.0, 3.0, HAND);
        disc(&mut image, CENTRE, CENTRE, 7.0, HAND);
        return image;
    }
    let digits = [
        time.hour / 10,
        time.hour % 10,
        time.minute / 10,
        time.minute % 10,
    ];
    for (x, digit) in CLOCK_DIGITS_X.into_iter().zip(digits) {
        segment_digit(&mut image, x, CLOCK_DIGITS_Y, digit, WHITE);
    }
    // Colon between the hour and minute pairs.
    fill(&mut image, 116, 103, STROKE, STROKE, ACCENT);
    fill(&mut image, 116, 129, STROKE, STROKE, ACCENT);
    image
}

/// Inner and outer radius of the CPU gauge ring.
const CPU_RING: (f64, f64) = (102.0, 114.0);
/// Horizontal distance between seven-segment cells on the CPU face.
const CPU_DIGIT_PITCH: i32 = DIGIT_WIDTH + 6;
/// The middle segment only: a dash.
const DASH: u8 = 0b100_0000;

/// Left edges of `cells` seven-segment cells, centred on the dial.
fn cpu_digits_x(cells: i32) -> impl Iterator<Item = i32> {
    let left = (SIDE as i32 - (cells * CPU_DIGIT_PITCH - (CPU_DIGIT_PITCH - DIGIT_WIDTH))) / 2;
    (0..cells).map(move |cell| left + cell * CPU_DIGIT_PITCH)
}

/// CPU usage: `CPU` above, the percentage in seven-segment digits (`--` when
/// unknown), `%` below, and a ring filled clockwise from 12 o'clock.
fn draw_cpu(percent: Option<u8>) -> RgbImage {
    let mut image = canvas();
    let percent = percent.map(|percent| percent.min(100));
    let (inner, outer) = CPU_RING;
    let color = match percent {
        Some(percent) if percent >= CPU_HIGH => HAND,
        _ => ACCENT,
    };
    // Degrees of the ring that are lit.
    let lit = f64::from(percent.unwrap_or(0)) * 3.6;
    for py in 0..SIDE {
        for px in 0..SIDE {
            let (dx, dy) = (f64::from(px) - CENTRE, f64::from(py) - CENTRE);
            if !(inner..=outer).contains(&(dx * dx + dy * dy).sqrt()) {
                continue;
            }
            // Clockwise from 12 o'clock, in [0, 360).
            let angle = dx.atan2(-dy).to_degrees().rem_euclid(360.0);
            let pixel = if angle < lit { color } else { DIM };
            image.put_pixel(px, py, Rgb(pixel));
        }
    }
    text(&mut image, 52, "CPU", 2, DIM);
    match percent {
        Some(percent) => {
            let digits: Vec<u8> = percent.to_string().bytes().map(|b| b - b'0').collect();
            for (x, digit) in cpu_digits_x(digits.len() as i32).zip(digits) {
                segment_digit(&mut image, x, CLOCK_DIGITS_Y, digit, WHITE);
            }
        }
        None => {
            for x in cpu_digits_x(2) {
                segment_mask(&mut image, x, CLOCK_DIGITS_Y, DASH, DIM);
            }
        }
    }
    text(&mut image, 166, "%", 3, DIM);
    image
}

/// 5×7 glyph rows, bit 4 is the leftmost column. Unknown characters map to `?`.
pub(crate) fn glyph(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 17, 25, 21, 19, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 17, 10, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        ' ' => [0; 7],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 12, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        '/' => [0, 1, 2, 4, 8, 16, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '&' => [12, 18, 20, 8, 21, 18, 13],
        '\'' => [12, 4, 8, 0, 0, 0, 0],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '#' => [10, 10, 31, 10, 31, 10, 10],
        '!' => [4, 4, 4, 4, 4, 0, 4],
        _ => [14, 17, 1, 2, 4, 0, 4],
    }
}

/// Draws `text` centred on `y` (top edge) at `scale` pixels per font pixel.
fn text(image: &mut RgbImage, y: i32, line: &str, scale: i32, color: [u8; 3]) {
    let advance = 6 * scale;
    let count = line.chars().count() as i32;
    let width = count * advance - scale;
    let mut x = (SIDE as i32 - width) / 2;
    for character in line.chars() {
        for (row, bits) in (0i32..).zip(glyph(character)) {
            for column in 0..5 {
                if bits & (0b10000 >> column) != 0 {
                    fill(
                        image,
                        x + column * scale,
                        y + row * scale,
                        scale,
                        scale,
                        color,
                    );
                }
            }
        }
        x += advance;
    }
}

/// Splits a name into at most two lines of `LINE_CHARS`, preferring a space;
/// a longer name is cut and ends in `.`.
fn wrap(name: &str) -> Vec<String> {
    let mut chars: Vec<char> = name.trim().chars().collect();
    if chars.len() > NAME_CHARS {
        chars.truncate(NAME_CHARS);
        chars[NAME_CHARS - 1] = '.';
    }
    if chars.len() <= LINE_CHARS {
        return vec![chars.into_iter().collect()];
    }
    let split = chars[..=LINE_CHARS]
        .iter()
        .rposition(|&c| c == ' ')
        .filter(|&at| at > 0 && chars.len() - at - 1 <= LINE_CHARS)
        .unwrap_or(LINE_CHARS);
    let first: String = chars[..split].iter().collect();
    let rest: String = chars[split..].iter().collect::<String>().trim().to_owned();
    let mut rest: Vec<char> = rest.chars().collect();
    if rest.len() > LINE_CHARS {
        rest.truncate(LINE_CHARS);
        rest[LINE_CHARS - 1] = '.';
    }
    vec![first.trim_end().to_owned(), rest.into_iter().collect()]
}

/// A name in large text, one or two lines centred around the middle.
fn name_lines(image: &mut RgbImage, name: &str) {
    let lines = wrap(name);
    let top = if lines.len() == 1 { 99 } else { 81 };
    for (index, line) in (0i32..).zip(&lines) {
        text(image, top + index * 36, line, 3, WHITE);
    }
}

/// The function name with `n/N` above it and one dot per function below.
fn draw_function(name: &str, index: usize, count: usize) -> RgbImage {
    let mut image = canvas();
    ring(&mut image, 114.0, 117.0, ACCENT);
    text(&mut image, 44, &format!("{}/{}", index + 1, count), 2, DIM);
    name_lines(&mut image, name);
    if count <= MAX_DOTS {
        let pitch = 12;
        let start = (SIDE as i32 - (count as i32 - 1) * pitch) / 2;
        for dot in 0..count {
            let color = if dot == index { ACCENT } else { DIM };
            let x = f64::from(start + dot as i32 * pitch);
            disc(&mut image, x, 178.0, 3.5, color);
        }
    }
    image
}

/// The profile name, for function content without dial functions.
fn draw_profile(name: &str) -> RgbImage {
    let mut image = canvas();
    ring(&mut image, 114.0, 117.0, DIM);
    text(&mut image, 44, "PROFILE", 2, DIM);
    name_lines(&mut image, name);
    image
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use cooltek_sdk_service::display_sync::{
        DisplaySync, DisplayTarget, FakeDisplaySink, SinkError,
    };

    use super::*;

    fn functions_profile() -> Profile {
        Profile::from_json(include_bytes!(
            "../../../examples/profiles/dial-functions.json"
        ))
        .unwrap()
    }

    fn flat_profile() -> Profile {
        Profile::from_json(include_bytes!("../../../examples/profiles/macos.json")).unwrap()
    }

    fn clock(hour: u8, minute: u8) -> ManualClock {
        ManualClock::new(WallTime::new(hour, minute))
    }

    fn artwork(content: DialContent, clock: &ManualClock) -> DialArtwork {
        DialArtwork::prepare(content, &functions_profile(), Box::new(clock.clone())).unwrap()
    }

    fn decoded(jpeg: &[u8]) -> RgbImage {
        image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8()
    }

    #[test]
    fn a_turned_dial_serves_every_face_turned_clockwise() {
        let face = |turns: u8| {
            let art = artwork(DialContent::Function, &clock(9, 5)).with_turn(turns);
            let request = DialImageRequest {
                profile: "Pictures".into(),
                function: Some("function:0".into()),
            };
            art.dial_image(&request).unwrap()
        };
        let upright = decoded(&face(0));
        for (turns, expected) in [
            (1, image::imageops::rotate90(&upright)),
            (2, image::imageops::rotate180(&upright)),
            (3, image::imageops::rotate270(&upright)),
        ] {
            let served = decoded(&face(turns));
            // Re-encoding is lossy, so compare the average difference per channel.
            let total: u64 = served
                .as_raw()
                .iter()
                .zip(expected.as_raw())
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum();
            assert!(
                total / served.as_raw().len() as u64 <= 3,
                "{turns} quarter turns"
            );
        }
        // More than three turns wrap around.
        assert_eq!(face(4), face(0));
    }

    #[test]
    fn dial_content_parses_strictly() {
        use clap::ValueEnum;
        for content in DialContent::value_variants() {
            assert_eq!(DialContent::from_str(content.as_str(), false), Ok(*content));
        }
        assert_eq!(DialContent::from_str("cpu", false), Ok(DialContent::Cpu));
        for bad in [
            "",
            "Clock",
            "analog",
            "analog_clock",
            "on",
            " clock",
            "CPU",
            "cpu ",
            "gpu",
            "clock+cpu",
        ] {
            assert!(DialContent::from_str(bad, false).is_err(), "{bad:?}");
        }
        assert_eq!(DialContent::default(), DialContent::Off);
    }

    #[test]
    fn image_content_serves_the_profile_picture_and_requires_one() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        // A dial-valid JPEG, produced by the same path generated faces use.
        let source = image::RgbImage::from_pixel(240, 240, image::Rgb([90, 40, 200]));
        let mut png = Vec::new();
        source
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        let jpeg = prepare_image(
            &png,
            PrepareOptions {
                quality: QUALITY,
                ..PrepareOptions::default()
            },
        )
        .unwrap();
        let text = STANDARD.encode(&jpeg);
        let profile = |dial_display: serde_json::Value| {
            let mut json = serde_json::json!({
                "schema_version": 1, "name": "Pictures", "bindings": []
            });
            json["dial_display"] = dial_display;
            Profile::from_json(serde_json::to_vec(&json).unwrap().as_slice()).unwrap()
        };
        // The exact stored bytes are served under the static "image" key,
        // whichever function is current.
        let art = DialArtwork::prepare(
            DialContent::Image,
            &profile(serde_json::json!({"content": "image", "image": text})),
            Box::new(clock(9, 41)),
        )
        .unwrap();
        assert_eq!(art.request(Some(1)).function.as_deref(), Some("image"));
        for request in [art.request(None), art.request(Some(1))] {
            assert_eq!(art.dial_image(&request).as_deref(), Some(jpeg.as_slice()));
        }
        // Nothing else serves bytes, and the other accessors stay empty.
        assert_eq!(
            art.dial_image(&DialImageRequest {
                profile: "Pictures".into(),
                function: Some("clock:09:41".into())
            }),
            None
        );
        assert_eq!(art.profile_face(), None);
        // Image content without the profile picture refuses to start: one
        // profile with the setting but no picture, and one without the
        // setting at all. The struct bypasses profile validation on purpose —
        // prepare must still refuse what it is handed.
        let without_setting = profile(serde_json::Value::Null);
        let unpictured = cooltek_sdk_service::Profile {
            dial_display: Some(cooltek_sdk_service::DialDisplay {
                content: cooltek_sdk_service::DialDisplayContent::Image,
                image: None,
                source: None,
                animation: None,
            }),
            ..profile(serde_json::Value::Null)
        };
        for bad in [&without_setting, &unpictured] {
            assert!(DialArtwork::prepare(DialContent::Image, bad, Box::new(clock(0, 0))).is_err());
        }
        // The same profiles start fine with the content they declare.
        DialArtwork::prepare(DialContent::Clock, &without_setting, Box::new(clock(9, 41))).unwrap();
    }

    #[test]
    fn the_animation_frame_follows_the_elapsed_time() {
        let ms = Duration::from_millis;
        let delays = [200, 300, 500];
        for (elapsed, expected) in [
            (0, 0),
            (199, 0),
            (200, 1),
            (499, 1),
            (500, 2),
            (999, 2),
            // Looping forever wraps around.
            (1000, 0),
            (1200, 1),
            (12_000_000, 0),
        ] {
            assert_eq!(
                animation_frame(&delays, 0, ms(elapsed)),
                expected,
                "{elapsed} ms"
            );
        }
        // Two plays, then the last frame holds.
        assert_eq!(animation_frame(&delays, 2, ms(1999)), 2);
        assert_eq!(animation_frame(&delays, 2, ms(1499)), 1);
        assert_eq!(animation_frame(&delays, 2, ms(2000)), 2);
        assert_eq!(animation_frame(&delays, 2, ms(u64::MAX)), 2);
    }

    #[test]
    fn the_time_to_the_next_frame_counts_down_and_ends_with_a_finite_animation() {
        let ms = Duration::from_millis;
        let delays = [200, 300, 500];
        for (elapsed, expected) in [
            (0, Some(200)),
            (150, Some(50)),
            (200, Some(300)),
            (999, Some(1)),
            (1000, Some(200)),
            (1450, Some(50)),
        ] {
            assert_eq!(
                frame_remaining(&delays, 0, ms(elapsed)),
                expected.map(ms),
                "{elapsed} ms"
            );
        }
        assert_eq!(frame_remaining(&delays, 2, ms(1999)), Some(ms(1)));
        assert_eq!(frame_remaining(&delays, 2, ms(2000)), None);
        assert_eq!(frame_remaining(&[], 0, ms(5)), None);
    }

    #[test]
    fn an_animated_picture_serves_each_frame_under_its_own_face() {
        let profile = animated_test_profile(&[200, 200, 200], 0);
        let art =
            DialArtwork::prepare(DialContent::Image, &profile, Box::new(clock(9, 41))).unwrap();
        assert!(art.is_animated());
        // Just prepared: playback is at frame 0.
        assert_eq!(art.request(None).function.as_deref(), Some("image:0"));
        let frames: Vec<Vec<u8>> = (0..3)
            .map(|index| {
                art.dial_image(&DialImageRequest {
                    profile: "Moving".into(),
                    function: Some(format!("image:{index}")),
                })
                .unwrap()
            })
            .collect();
        assert_ne!(frames[0], frames[1]);
        assert_ne!(frames[1], frames[2]);
        for bad in ["image:3", "image:+1", "image:01", "image:", "image:x"] {
            let request = DialImageRequest {
                profile: "Moving".into(),
                function: Some(bad.into()),
            };
            assert_eq!(art.dial_image(&request), None, "{bad}");
        }
        // A still picture is not animated and keeps its single face.
        let still = Profile::from_json(
            serde_json::to_vec(&{
                let mut json = serde_json::to_value(&profile).unwrap();
                json["dial_display"]
                    .as_object_mut()
                    .unwrap()
                    .remove("animation");
                json
            })
            .unwrap()
            .as_slice(),
        )
        .unwrap();
        let art = DialArtwork::prepare(DialContent::Image, &still, Box::new(clock(9, 41))).unwrap();
        assert!(!art.is_animated());
        assert_eq!(art.request(None).function.as_deref(), Some("image"));
    }

    #[test]
    fn every_face_is_a_valid_240px_dial_image() {
        let time = clock(9, 41);
        for content in [
            DialContent::Function,
            DialContent::Clock,
            DialContent::AnalogClock,
        ] {
            let art = artwork(content, &time);
            let request = art.request(Some(1));
            let jpeg = art.dial_image(&request).unwrap();
            dial::validate_jpeg(&jpeg).unwrap();
            let image = decoded(&jpeg);
            assert_eq!((image.width(), image.height()), (240, 240));
        }
        let art = artwork(DialContent::Function, &time);
        dial::validate_jpeg(art.profile_face().unwrap()).unwrap();
        assert_eq!(artwork(DialContent::Clock, &time).profile_face(), None);
        for index in 0..3 {
            dial::validate_jpeg(art.function_face(index).unwrap()).unwrap();
        }
        assert_eq!(art.function_face(3), None);
    }

    #[test]
    fn preparation_is_deterministic_and_faces_are_distinct() {
        let time = clock(0, 0);
        let (a, b) = (
            artwork(DialContent::Function, &time),
            artwork(DialContent::Function, &time),
        );
        let mut seen = HashSet::new();
        assert!(seen.insert(a.profile_face().unwrap().to_vec()));
        for index in 0..3 {
            assert_eq!(a.function_face(index), b.function_face(index));
            assert!(seen.insert(a.function_face(index).unwrap().to_vec()));
        }
        assert!(
            DialArtwork::prepare(DialContent::Off, &functions_profile(), Box::new(time)).is_err()
        );
    }

    #[test]
    fn requests_name_the_face_and_follow_the_injected_clock() {
        let time = clock(7, 5);
        let digital = artwork(DialContent::Clock, &time);
        assert_eq!(
            digital.request(Some(2)).function.as_deref(),
            Some("clock:07:05")
        );
        assert_eq!(digital.request(None).profile, "Dial functions");
        time.set(WallTime::new(23, 59));
        assert_eq!(
            digital.request(None).function.as_deref(),
            Some("clock:23:59")
        );
        let analog = artwork(DialContent::AnalogClock, &time);
        assert_eq!(
            analog.request(None).function.as_deref(),
            Some("analog:23:59")
        );

        let function = artwork(DialContent::Function, &time);
        assert_eq!(
            function.request(Some(2)).function.as_deref(),
            Some("function:2")
        );
        // Out of range or without functions: the profile face.
        assert_eq!(
            function.request(Some(3)).function.as_deref(),
            Some("profile")
        );
        assert_eq!(function.request(None).function.as_deref(), Some("profile"));
        let flat =
            DialArtwork::prepare(DialContent::Function, &flat_profile(), Box::new(time)).unwrap();
        assert_eq!(flat.request(Some(0)).function.as_deref(), Some("profile"));
        assert_eq!(
            flat.dial_image(&flat.request(Some(0))).as_deref(),
            flat.profile_face()
        );
    }

    #[test]
    fn foreign_or_malformed_requests_have_no_artwork() {
        let time = clock(12, 0);
        let digital = artwork(DialContent::Clock, &time);
        let function = artwork(DialContent::Function, &time);
        let request = |face: Option<&str>| DialImageRequest {
            profile: "x".into(),
            function: face.map(Into::into),
        };
        for face in [
            None,
            Some(""),
            Some("clock:24:00"),
            Some("clock:12:60"),
            Some("clock:1:00"),
            Some("clock:12-00"),
            Some("analog:12:00"),
            Some("profile"),
            Some("function:0"),
        ] {
            assert_eq!(digital.dial_image(&request(face)), None, "{face:?}");
        }
        for face in [
            Some("function:3"),
            Some("function:01"),
            Some("function:+1"),
            Some("function:-1"),
            Some("clock:12:00"),
        ] {
            assert_eq!(function.dial_image(&request(face)), None, "{face:?}");
        }
    }

    #[test]
    fn digital_clock_digits_come_from_the_segment_renderer() {
        let image = draw_clock(WallTime::new(18, 47), DialContent::Clock);
        let lit = |digit: usize, segment: usize| {
            let (x, y) = (CLOCK_DIGITS_X[digit], CLOCK_DIGITS_Y);
            let (w, h, t) = (DIGIT_WIDTH, DIGIT_HEIGHT, STROKE);
            // A point inside each segment, away from shared corners.
            let (px, py) = match segment {
                0 => (x + w / 2, y + t / 2),
                1 => (x + w - t / 2, y + h / 4),
                2 => (x + w - t / 2, y + 3 * h / 4),
                3 => (x + w / 2, y + h - t / 2),
                4 => (x + t / 2, y + 3 * h / 4),
                5 => (x + t / 2, y + h / 4),
                _ => (x + w / 2, y + h / 2),
            };
            image.get_pixel(px as u32, py as u32).0 == WHITE
        };
        for (digit, value) in [1u8, 8, 4, 7].into_iter().enumerate() {
            for segment in 0..7 {
                let expected = SEGMENTS[usize::from(value)] & (1 << segment) != 0;
                assert_eq!(
                    lit(digit, segment),
                    expected,
                    "digit {digit} segment {segment}"
                );
            }
        }
        assert_eq!(image.get_pixel(119, 106).0, ACCENT);
    }

    #[test]
    fn analog_hands_point_at_the_time() {
        // 3:00: minute hand straight up, hour hand to the right.
        let image = draw_clock(WallTime::new(15, 0), DialContent::AnalogClock);
        assert_eq!(image.get_pixel(120, 50).0, HAND);
        assert_eq!(image.get_pixel(165, 120).0, WHITE);
        assert_eq!(image.get_pixel(75, 120).0, BLACK);
        assert_eq!(image.get_pixel(120, 190).0, BLACK);
        // 6:30: both hands point down; the minute hand reaches further.
        let image = draw_clock(WallTime::new(6, 30), DialContent::AnalogClock);
        assert_eq!(image.get_pixel(120, 195).0, HAND);
        assert_eq!(image.get_pixel(120, 50).0, BLACK);
    }

    #[test]
    fn function_faces_spell_the_name_and_mark_the_position() {
        let image = draw_function("Volume", 1, 3);
        // "VOLUME" at scale 3: 6 glyphs, 18 px advance, centred on 240.
        let left = (240 - (6 * 18 - 3)) / 2;
        for (index, character) in "VOLUME".chars().enumerate() {
            for (row, bits) in (0i32..).zip(glyph(character)) {
                for column in 0..5 {
                    let lit = bits & (0b10000 >> column) != 0;
                    let x = left + index as i32 * 18 + column * 3 + 1;
                    let pixel = image.get_pixel(x as u32, (99 + row * 3 + 1) as u32);
                    assert_eq!(pixel.0 == WHITE, lit, "{character} ({column}, {row})");
                }
            }
        }
        // Three dots: the second is highlighted.
        assert_eq!(image.get_pixel(120, 178).0, ACCENT);
        assert_eq!(image.get_pixel(108, 178).0, DIM);
        assert_eq!(image.get_pixel(132, 178).0, DIM);
        // Unknown characters render as '?', never panic.
        draw_function("Ünïcode ☃ name", 0, 1);
    }

    #[test]
    fn long_names_wrap_onto_two_lines_and_are_cut() {
        assert_eq!(wrap("Volume"), ["Volume"]);
        assert_eq!(wrap("Media keys control"), ["Media keys", "control"]);
        assert_eq!(wrap("Supercalifragilistic"), ["Supercalifr", "agilistic"]);
        let long = wrap(&"x".repeat(64));
        assert_eq!(long.len(), 2);
        assert!(long.iter().all(|line| line.chars().count() <= LINE_CHARS));
        assert!(long[1].ends_with('.'));
    }

    #[test]
    fn clock_faces_are_cached_per_minute() {
        let time = clock(10, 10);
        let art = artwork(DialContent::Clock, &time);
        let first = art.dial_image(&art.request(None)).unwrap();
        assert_eq!(art.cached.borrow().as_ref().unwrap().0, "clock:10:10");
        assert_eq!(art.dial_image(&art.request(None)).unwrap(), first);
        time.set(WallTime::new(10, 11));
        let next = art.dial_image(&art.request(None)).unwrap();
        assert_ne!(next, first);
        assert_eq!(art.cached.borrow().as_ref().unwrap().0, "clock:10:11");
    }

    #[test]
    fn display_sync_writes_the_dial_only_when_the_minute_changes() {
        let time = clock(8, 0);
        let art = artwork(DialContent::Clock, &time);
        let mut sync = DisplaySync::new(FakeDisplaySink::default());
        let plan = sync.plan_dial(art.request(None), &art).unwrap();
        assert_eq!(
            sync.apply_dial(plan).unwrap().written,
            [DisplayTarget::Dial]
        );
        // Same minute: nothing to write.
        let plan = sync.plan_dial(art.request(None), &art).unwrap();
        assert!(plan.write().is_none() && !plan.is_missing());
        sync.apply_dial(plan).unwrap();
        time.set(WallTime::new(8, 1));
        let plan = sync.plan_dial(art.request(None), &art).unwrap();
        sync.apply_dial(plan).unwrap();
        assert_eq!(sync.sink().successful(), [DisplayTarget::Dial; 2]);

        // A plan made before the minute's write is stale once that write lands.
        time.set(WallTime::new(8, 2));
        let held = sync.plan_dial(art.request(None), &art).unwrap();
        let fresh = sync.plan_dial(art.request(None), &art).unwrap();
        sync.apply_dial(fresh).unwrap();
        assert!(sync.apply_dial(held).is_err());
    }

    #[test]
    fn a_short_dial_write_is_never_retried_and_leaves_the_dial_dirty() {
        let time = clock(8, 0);
        let art = artwork(DialContent::Function, &time);
        let mut sink = FakeDisplaySink::default();
        sink.fail_next(
            DisplayTarget::Dial,
            SinkError::ShortWrite {
                written: 3,
                expected: 65,
            },
        );
        let mut sync =
            DisplaySync::new(sink).with_retry(cooltek_sdk_service::display_sync::RetryPolicy {
                transport_retries: 3,
            });
        let plan = sync.plan_dial(art.request(Some(0)), &art).unwrap();
        let report = sync.apply_dial(plan).unwrap();
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].attempts, 1);
        assert_eq!(sync.sink().writes.len(), 1);
        assert_eq!(sync.drawn().dial(), None);
    }

    #[test]
    fn the_fake_stat_source_replays_clamped_readings() {
        let mut source = FakeStatSource::new([
            StatReading {
                kind: StatKind::Cpu,
                percent: 37,
            },
            StatReading {
                kind: StatKind::Gpu,
                percent: 250,
            },
        ]);
        let source: &mut dyn StatSource = &mut source;
        assert_eq!(source.sample().unwrap().percent, 37);
        assert_eq!(
            source.sample(),
            Some(StatReading {
                kind: StatKind::Gpu,
                percent: 100
            })
        );
        assert_eq!(source.sample(), None);
    }

    fn cpu(percent: u8) -> StatReading {
        StatReading {
            kind: StatKind::Cpu,
            percent,
        }
    }

    /// Counts samples and reports `percent` (or nothing) each time.
    struct Counting {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        percent: Option<u8>,
    }
    impl StatSource for Counting {
        fn sample(&mut self) -> Option<StatReading> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.percent.map(cpu)
        }
    }

    fn cpu_artwork(source: impl StatSource + Send + 'static) -> DialArtwork {
        artwork(DialContent::Cpu, &clock(0, 0))
            .with_stat_source(Box::new(source))
            .with_cpu_interval(Duration::ZERO)
    }

    fn face(art: &DialArtwork) -> String {
        art.request(None).function.unwrap()
    }

    #[test]
    fn cpu_requests_name_the_percentage_or_dashes() {
        let art = cpu_artwork(FakeStatSource::new([
            cpu(0),
            cpu(37),
            cpu(250),
            StatReading {
                kind: StatKind::Gpu,
                percent: 5,
            },
        ]));
        let faces: Vec<String> = (0..5).map(|_| face(&art)).collect();
        // A foreign kind and an exhausted source both show dashes.
        assert_eq!(faces, ["cpu:0", "cpu:37", "cpu:100", "cpu:--", "cpu:--"]);
        assert_eq!(art.request(Some(1)).profile, "Dial functions");
        for face in faces {
            let request = DialImageRequest {
                profile: "x".into(),
                function: Some(face),
            };
            dial::validate_jpeg(&art.dial_image(&request).unwrap()).unwrap();
        }
    }

    #[test]
    fn cpu_is_sampled_at_most_once_per_interval() {
        assert_eq!(CPU_SAMPLE_INTERVAL, Duration::from_secs(1));
        let calls = std::sync::Arc::default();
        let mut gauge = CpuGauge::new(Box::new(Counting {
            calls: std::sync::Arc::clone(&calls),
            percent: Some(12),
        }));
        let count = || calls.load(std::sync::atomic::Ordering::SeqCst);
        let start = Instant::now();
        assert_eq!(gauge.percent(start), Some(12));
        for millis in [0, 1, 500, 999] {
            assert_eq!(
                gauge.percent(start + Duration::from_millis(millis)),
                Some(12)
            );
        }
        assert_eq!(count(), 1);
        gauge.percent(start + Duration::from_secs(1));
        assert_eq!(count(), 2);
        // A clock that steps back never samples early.
        gauge.percent(start);
        assert_eq!(count(), 2);

        // Preparing CPU artwork samples nothing; requests do.
        let calls = std::sync::Arc::default();
        let art = artwork(DialContent::Cpu, &clock(0, 0)).with_stat_source(Box::new(Counting {
            calls: std::sync::Arc::clone(&calls),
            percent: Some(1),
        }));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        for _ in 0..20 {
            art.request(None);
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Other content never samples, even with a source installed.
        let calls = std::sync::Arc::default();
        let clock_art =
            artwork(DialContent::Clock, &clock(0, 0)).with_stat_source(Box::new(Counting {
                calls: std::sync::Arc::clone(&calls),
                percent: Some(1),
            }));
        clock_art.request(None);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn an_unavailable_cpu_shows_dashes_and_is_reported_once() {
        let mut gauge = CpuGauge::new(Box::new(FakeStatSource::default()));
        gauge.interval = Duration::ZERO;
        let now = Instant::now();
        // The first sample is only a baseline: nothing to report yet.
        assert_eq!(gauge.percent(now), None);
        assert!(!gauge.unavailable_logged);
        for _ in 0..5 {
            assert_eq!(gauge.percent(now), None);
            assert!(gauge.unavailable_logged);
        }
        assert_eq!(gauge.samples, 6);
        let art = cpu_artwork(FakeStatSource::default());
        assert_eq!(face(&art), "cpu:--");
        let dashes = art.dial_image(&art.request(None)).unwrap();
        dial::validate_jpeg(&dashes).unwrap();
    }

    #[test]
    fn a_panicking_sampler_shows_dashes_and_is_never_called_again() {
        struct Panicking(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl StatSource for Panicking {
            fn sample(&mut self) -> Option<StatReading> {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                panic!("sampler failure")
            }
        }
        let calls = std::sync::Arc::default();
        let mut gauge = CpuGauge::new(Box::new(Panicking(std::sync::Arc::clone(&calls))));
        gauge.interval = Duration::ZERO;
        let now = Instant::now();
        for _ in 0..4 {
            assert_eq!(gauge.percent(now), None);
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(gauge.unavailable_logged);
    }

    #[test]
    fn cpu_faces_are_deterministic_distinct_and_cached() {
        let (a, b) = (
            cpu_artwork(FakeStatSource::new([cpu(42)])),
            cpu_artwork(FakeStatSource::new([cpu(42)])),
        );
        let (first, second) = (
            a.dial_image(&a.request(None)).unwrap(),
            b.dial_image(&b.request(None)).unwrap(),
        );
        assert_eq!(first, second);
        assert_eq!(a.cached.borrow().as_ref().unwrap().0, "cpu:42");
        let mut seen = HashSet::new();
        for percent in [Some(0), Some(42), Some(43), Some(100), None] {
            assert_eq!(draw_cpu(percent), draw_cpu(percent));
            assert!(seen.insert(encode(&draw_cpu(percent)).unwrap()));
        }
    }

    #[test]
    fn the_cpu_gauge_fills_clockwise_from_twelve() {
        // A point on the ring at `degrees` clockwise from 12 o'clock.
        let ring = |image: &RgbImage, degrees: f64| {
            let (sin, cos) = (degrees * PI / 180.0).sin_cos();
            let radius = (CPU_RING.0 + CPU_RING.1) / 2.0;
            let (x, y) = (CENTRE + sin * radius, CENTRE - cos * radius);
            image.get_pixel(x.round() as u32, y.round() as u32).0
        };
        let half = draw_cpu(Some(50));
        assert_eq!(ring(&half, 10.0), ACCENT);
        assert_eq!(ring(&half, 170.0), ACCENT);
        assert_eq!(ring(&half, 190.0), DIM);
        assert_eq!(ring(&half, 350.0), DIM);
        let empty = draw_cpu(Some(0));
        assert_eq!(ring(&empty, 10.0), DIM);
        // From the warning threshold up, the gauge changes color.
        let high = draw_cpu(Some(CPU_HIGH));
        assert_eq!(ring(&high, 10.0), HAND);
        let full = draw_cpu(Some(100));
        for degrees in [0.0, 90.0, 180.0, 270.0, 355.0] {
            assert_eq!(ring(&full, degrees), HAND);
        }
        // Unknown: a dim ring and two dashes.
        let unknown = draw_cpu(None);
        assert_eq!(ring(&unknown, 10.0), DIM);
        let middle = CLOCK_DIGITS_Y + (DIGIT_HEIGHT - STROKE) / 2 + STROKE / 2;
        for x in cpu_digits_x(2) {
            assert_eq!(
                unknown
                    .get_pixel((x + DIGIT_WIDTH / 2) as u32, middle as u32)
                    .0,
                DIM
            );
            // The top segment stays dark.
            assert_eq!(
                unknown
                    .get_pixel((x + DIGIT_WIDTH / 2) as u32, (CLOCK_DIGITS_Y + 2) as u32)
                    .0,
                BLACK
            );
        }
        // "7" lights segments a, b, c only, centred as a single cell.
        let seven = draw_cpu(Some(7));
        let x = cpu_digits_x(1).next().unwrap();
        assert_eq!(x, (240 - DIGIT_WIDTH) / 2);
        assert_eq!(
            seven
                .get_pixel((x + DIGIT_WIDTH / 2) as u32, (CLOCK_DIGITS_Y + 2) as u32)
                .0,
            WHITE
        );
        assert_eq!(
            seven
                .get_pixel((x + DIGIT_WIDTH / 2) as u32, middle as u32)
                .0,
            BLACK
        );
    }

    #[test]
    fn foreign_or_malformed_cpu_requests_have_no_artwork() {
        let art = cpu_artwork(FakeStatSource::default());
        let digital = artwork(DialContent::Clock, &clock(12, 0));
        let request = |face: &str| DialImageRequest {
            profile: "x".into(),
            function: Some(face.into()),
        };
        for face in [
            "cpu:",
            "cpu:101",
            "cpu:255",
            "cpu:007",
            "cpu:00",
            "cpu:+1",
            "cpu:-1",
            "cpu:-",
            "cpu:---",
            "cpu: 5",
            "cpu:1000",
            "cpu:5%",
            "clock:12:00",
            "profile",
            "function:0",
        ] {
            assert_eq!(art.dial_image(&request(face)), None, "{face:?}");
        }
        for face in ["cpu:0", "cpu:100", "cpu:--"] {
            assert!(art.dial_image(&request(face)).is_some(), "{face:?}");
            assert_eq!(digital.dial_image(&request(face)), None, "{face:?}");
        }
    }

    #[test]
    fn display_sync_writes_the_cpu_face_only_when_the_percentage_changes() {
        let art = cpu_artwork(FakeStatSource::new([5, 5, 5, 6, 6, 5].into_iter().map(cpu)));
        let mut sync = DisplaySync::new(FakeDisplaySink::default());
        let mut faces = Vec::new();
        for _ in 0..7 {
            let request = art.request(None);
            faces.push(request.function.clone().unwrap());
            let plan = sync.plan_dial(request, &art).unwrap();
            sync.apply_dial(plan).unwrap();
        }
        assert_eq!(
            faces,
            [
                "cpu:5", "cpu:5", "cpu:5", "cpu:6", "cpu:6", "cpu:5", "cpu:--"
            ]
        );
        // 5, 6, 5 and the dashes: unchanged percentages are never rewritten.
        assert_eq!(sync.sink().successful(), [DisplayTarget::Dial; 4]);
    }

    #[test]
    fn the_system_clock_reports_a_valid_time_of_day() {
        let now = SystemClock.now();
        assert!(now.hour < 24 && now.minute < 60);
    }
}
