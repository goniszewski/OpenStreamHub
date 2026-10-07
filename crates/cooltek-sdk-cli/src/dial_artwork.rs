//! Generated 240×240 M131E dial artwork for `DisplaySync`, offline and deterministic.
//!
//! [`DialArtwork`] is a `DialImageProvider` for three kinds of dial content
//! ([`DialContent`]): the active dial function's name, a digital `HH:MM` clock,
//! or a simple analog clock. Everything is drawn into a 240×240 RGB buffer
//! with a built-in rasteriser (a 5×7 bitmap font and seven-segment digits, so
//! no font files are read or downloaded), encoded through
//! [`crate::image_prep::prepare_image`] (the same encoder and validation as
//! `dial prepare-image`), and checked again with `dial::validate_jpeg`.
//!
//! Function and profile faces are prepared before a run starts. A clock face
//! is drawn when its minute is first requested and cached, so at most one image
//! is encoded per minute. Time comes from an injected [`Clock`], so tests are
//! deterministic.
//!
//! The provider performs no device I/O. Requests are opaque strings built by
//! [`DialArtwork::request`]: `profile`, `function:<index>`, `clock:HH:MM`, or
//! `analog:HH:MM`; anything else returns `None`. Hardware acceptance of the
//! images is unverified; see `docs/masterhub-display-sync.md`.

use std::{cell::RefCell, collections::VecDeque, f64::consts::PI, fmt, io::Cursor};

use cooltek_sdk_masterhub::dial;
use cooltek_sdk_service::{
    Profile,
    display_sync::{DialImageProvider, DialImageRequest},
};
use image::{ImageFormat, Rgb, RgbImage};

use crate::image_prep::{PrepareOptions, prepare_image};

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
}

impl DialContent {
    /// Every value, in command-line order.
    pub const ALL: [Self; 4] = [Self::Off, Self::Function, Self::Clock, Self::AnalogClock];

    /// The command-line spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Function => "function",
            Self::Clock => "clock",
            Self::AnalogClock => "analog-clock",
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

/// Source of CPU or GPU statistics for future dial widgets.
///
/// Abstraction only: no sensor implementation ships. On macOS, CPU usage needs
/// Mach `host_statistics` FFI that the existing dependencies do not expose
/// safely, and GPU usage needs IOKit, so sensors are pending. Nothing calls this
/// trait yet; [`FakeStatSource`] exercises its contract.
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

/// Prepared function and profile faces plus a clock for clock faces.
pub struct DialArtwork {
    content: DialContent,
    profile_name: String,
    /// Only for [`DialContent::Function`].
    profile_face: Option<Vec<u8>>,
    functions: Vec<Vec<u8>>,
    clock: Box<dyn Clock>,
    /// The last clock face drawn: its request key and bytes.
    cached: RefCell<Option<(String, Vec<u8>)>>,
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
    /// `profile`, and one clock face, so a broken encoder fails before a run.
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
            clock,
            cached: RefCell::new(None),
        };
        // A clock face cannot be prepared in advance; prove the path works.
        if content != DialContent::Function {
            encode(&draw_clock(WallTime::new(0, 0), content))?;
        }
        Ok(artwork)
    }

    /// The configured content.
    pub fn content(&self) -> DialContent {
        self.content
    }

    /// The request for what the dial should show now: the function at
    /// `dial_function` (or the profile face without functions), or the
    /// clock's current minute. Two requests are equal exactly when they
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

    /// Prepared bytes of the profile face.
    /// Prepared bytes of the profile face (function content only).
    pub fn profile_face(&self) -> Option<&[u8]> {
        self.profile_face.as_deref()
    }

    /// Prepared bytes of a function face.
    pub fn function_face(&self, index: usize) -> Option<&[u8]> {
        self.functions.get(index).map(Vec::as_slice)
    }

    fn clock_face(&self, key: &str, time: WallTime, content: DialContent) -> Option<Vec<u8>> {
        if let Some((cached, bytes)) = &*self.cached.borrow()
            && cached == key
        {
            return Some(bytes.clone());
        }
        let bytes = encode(&draw_clock(time, content)).ok()?;
        *self.cached.borrow_mut() = Some((key.to_owned(), bytes.clone()));
        Some(bytes)
    }
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
        let face = request.function.as_deref()?;
        if face == "profile" && self.content == DialContent::Function {
            return self.profile_face.clone();
        }
        if let Some(index) = face.strip_prefix("function:") {
            if index.starts_with('+') || (index.len() > 1 && index.starts_with('0')) {
                return None;
            }
            return self.function_face(index.parse().ok()?).map(<[u8]>::to_vec);
        }
        let (content, time) = match face.split_once(':')? {
            ("clock", time) if self.content == DialContent::Clock => (DialContent::Clock, time),
            ("analog", time) if self.content == DialContent::AnalogClock => {
                (DialContent::AnalogClock, time)
            }
            _ => return None,
        };
        self.clock_face(face, parse_time(time)?, content)
    }
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
    let mask = SEGMENTS[usize::from(digit % 10)];
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

/// 5×7 glyph rows, bit 4 is the leftmost column. Unknown characters map to `?`.
fn glyph(character: char) -> [u8; 7] {
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
    fn dial_content_parses_strictly() {
        use clap::ValueEnum;
        for content in DialContent::value_variants() {
            assert_eq!(DialContent::from_str(content.as_str(), false), Ok(*content));
        }
        for bad in ["", "Clock", "analog", "analog_clock", "on", " clock"] {
            assert!(DialContent::from_str(bad, false).is_err(), "{bad:?}");
        }
        assert_eq!(DialContent::default(), DialContent::Off);
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

    #[test]
    fn the_system_clock_reports_a_valid_time_of_day() {
        let now = SystemClock.now();
        assert!(now.hour < 24 && now.minute < 60);
    }
}
