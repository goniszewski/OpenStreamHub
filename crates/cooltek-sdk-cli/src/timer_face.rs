//! The countdown a running `timer` shows on its key.
//!
//! The action worker owns the timers; the display thread owns the key images.
//! [`TimerFaces`] is the small shared list between them: the worker publishes
//! which keys have a countdown and when it ends, and the display thread draws
//! the remaining time over the bottom of each such key, in orange, once a
//! second. When a timer ends or is cancelled the list shrinks and the key's
//! own artwork is drawn back. Nothing here sends to the device.

use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use cooltek_sdk_masterhub::keypad;
use cooltek_sdk_service::{
    TimerTable,
    display_sync::{KeyContent, KeyImageProvider, KeyImageRequest},
    format_duration,
};
use image::{ImageFormat, Rgb};

use crate::display_artwork::encode;

/// Key image side in pixels.
const SIDE: u32 = keypad::IMAGE_WIDTH as u32;
/// The countdown's colour.
pub const ORANGE: [u8; 3] = [255, 138, 0];
/// Top of the band the countdown replaces: below an icon and above the
/// bottom-placed label of the app's key designs.
const BAND_TOP: u32 = 47;
/// Widest the text may be, leaving a margin on both sides.
const MAX_TEXT_WIDTH: u32 = SIDE - 4;
/// Wait past a second boundary so the redraw sees the new second.
const SLACK: Duration = Duration::from_millis(10);

/// 5×7 digits, one row per byte, bit 4 is the leftmost column.
const DIGITS: [[u8; 7]; 10] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ],
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ],
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ],
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ],
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ],
    [
        0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
    ],
];
/// The colon is one column wide: dots on rows 2 and 4.
const COLON: [u8; 7] = [0, 0, 1, 0, 1, 0, 0];

/// One running countdown on a key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Countdown {
    /// Wire index of the key.
    pub control: u8,
    /// Keypad page the key belongs to, if the profile has pages.
    pub page_id: Option<String>,
    /// The table's deadline, to tell one countdown from another.
    pub deadline_micros: u64,
    /// When it ends on this process's clock.
    pub ends: Instant,
}

#[derive(Default)]
struct Shared {
    list: Mutex<Vec<Countdown>>,
    version: AtomicU64,
}

/// The countdowns the display thread should draw. Cloning shares the list.
#[derive(Clone, Default)]
pub struct TimerFaces {
    shared: Arc<Shared>,
}

impl TimerFaces {
    /// Replaces the list with the table's running timers, `now_micros` being
    /// the table's clock now. Does nothing, and wakes no one, when nothing changed.
    pub fn publish(&self, timers: &TimerTable, now_micros: u64) {
        let now = Instant::now();
        let mut next: Vec<Countdown> = timers
            .running()
            .iter()
            .map(|timer| Countdown {
                control: timer.key.control,
                page_id: timer.key.page_id.clone(),
                deadline_micros: timer.deadline_micros,
                ends: now + Duration::from_micros(timer.deadline_micros.saturating_sub(now_micros)),
            })
            .collect();
        next.sort_by_key(|countdown| (countdown.control, countdown.deadline_micros));
        let mut list = self
            .shared
            .list
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let same = list.len() == next.len()
            && list.iter().zip(&next).all(|(a, b)| {
                (a.control, &a.page_id, a.deadline_micros)
                    == (b.control, &b.page_id, b.deadline_micros)
            });
        if same {
            return;
        }
        *list = next;
        self.shared.version.fetch_add(1, Ordering::SeqCst);
    }

    /// Changes whenever the list does.
    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::SeqCst)
    }

    /// The countdowns now.
    pub fn snapshot(&self) -> Vec<Countdown> {
        self.shared
            .list
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Time until the shown second of the nearest countdown changes.
    pub fn next_change_in(&self, now: Instant) -> Option<Duration> {
        next_change_in(&self.snapshot(), now)
    }
}

/// Whole seconds left, counting a started second as left.
fn seconds_left(ends: Instant, now: Instant) -> u32 {
    let left = ends.saturating_duration_since(now);
    let whole = left.as_secs();
    u32::try_from(whole + u64::from(left.subsec_nanos() > 0)).unwrap_or(u32::MAX)
}

fn next_change_in(timers: &[Countdown], now: Instant) -> Option<Duration> {
    timers
        .iter()
        .map(|countdown| {
            let left = countdown.ends.saturating_duration_since(now);
            let fraction = Duration::from_nanos(u64::from(left.subsec_nanos()));
            let wait = if fraction.is_zero() {
                Duration::from_secs(1)
            } else {
                fraction
            };
            wait + SLACK
        })
        .min()
}

fn glyph_rows(character: char) -> Option<&'static [u8; 7]> {
    match character {
        ':' => Some(&COLON),
        digit => digit.to_digit(10).map(|value| &DIGITS[value as usize]),
    }
}

fn glyph_width(character: char) -> u32 {
    if character == ':' { 1 } else { 5 }
}

/// Width of `text` in font pixels, one blank column between glyphs.
fn text_width(text: &str) -> u32 {
    let count = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    text.chars().map(glyph_width).sum::<u32>() + count.saturating_sub(1)
}

/// The key image with its bottom band replaced by `text` in orange on the
/// image's own background colour. `None` when the image cannot be decoded or
/// re-encoded; the caller then keeps the image as it was.
pub fn with_countdown(jpeg: &[u8], text: &str) -> Option<Vec<u8>> {
    let mut image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .ok()?
        .to_rgb8();
    if image.dimensions() != (SIDE, SIDE) {
        return None;
    }
    // The tile's background, read from its bottom-left corner.
    let background = *image.get_pixel(1, SIDE - 2);
    for y in BAND_TOP..SIDE {
        for x in 0..SIDE {
            image.put_pixel(x, y, background);
        }
    }
    let width = text_width(text);
    let scale = (1..=2u32)
        .rev()
        .find(|scale| width * scale <= MAX_TEXT_WIDTH)?;
    let left = (SIDE - width * scale) / 2;
    let top = BAND_TOP + (SIDE - BAND_TOP).saturating_sub(7 * scale) / 2;
    let mut x = left;
    for character in text.chars() {
        let rows = glyph_rows(character)?;
        for (row, bits) in (0u32..).zip(rows) {
            for column in 0..glyph_width(character) {
                let lit = if character == ':' {
                    *bits != 0
                } else {
                    bits & (0b10000 >> column) != 0
                };
                if lit {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            image.put_pixel(
                                x + column * scale + dx,
                                top + row * scale + dy,
                                Rgb(ORANGE),
                            );
                        }
                    }
                }
            }
        }
        x += (glyph_width(character) + 1) * scale;
    }
    encode(&image).ok()
}

/// Wraps a provider so each key with a running countdown shows the time left;
/// every other key comes from the inner provider unchanged.
pub struct CountdownKey<'a> {
    pub inner: &'a dyn KeyImageProvider,
    pub timers: &'a [Countdown],
    pub now: Instant,
}

impl KeyImageProvider for CountdownKey<'_> {
    fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>> {
        let image = self.inner.key_image(request)?;
        let page = match &request.content {
            KeyContent::Empty { page_id }
            | KeyContent::Action { page_id, .. }
            | KeyContent::Folder { page_id, .. } => Some(page_id.as_str()),
            KeyContent::Back { .. } => None,
        };
        let Some(countdown) = self.timers.iter().find(|countdown| {
            countdown.control == request.key_index
                && countdown
                    .page_id
                    .as_deref()
                    .is_none_or(|id| Some(id) == page)
        }) else {
            return Some(image);
        };
        let text = format_duration(seconds_left(countdown.ends, self.now));
        Some(with_countdown(&image, &text).unwrap_or(image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_service::Action;
    use cooltek_sdk_service::Gesture;
    use cooltek_sdk_service::TimerKey;
    use image::RgbImage;

    fn plain_key(colour: [u8; 3]) -> Vec<u8> {
        encode(&RgbImage::from_pixel(SIDE, SIDE, Rgb(colour))).unwrap()
    }

    fn orange_pixels(jpeg: &[u8]) -> usize {
        image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8()
            .pixels()
            .filter(|pixel| pixel.0[0] > 200 && (80..200).contains(&pixel.0[1]) && pixel.0[2] < 90)
            .count()
    }

    #[test]
    fn counts_a_started_second_as_left() {
        let now = Instant::now();
        assert_eq!(seconds_left(now + Duration::from_millis(2500), now), 3);
        assert_eq!(seconds_left(now + Duration::from_secs(2), now), 2);
        assert_eq!(seconds_left(now, now), 0);
    }

    #[test]
    fn the_next_change_is_the_next_whole_second() {
        let now = Instant::now();
        let timer = |ends| Countdown {
            control: 0,
            page_id: None,
            deadline_micros: 0,
            ends,
        };
        let wait = next_change_in(&[timer(now + Duration::from_millis(2300))], now).unwrap();
        assert_eq!(wait, Duration::from_millis(300) + SLACK);
        let whole = next_change_in(&[timer(now + Duration::from_secs(2))], now).unwrap();
        assert_eq!(whole, Duration::from_secs(1) + SLACK);
        let soonest = next_change_in(
            &[
                timer(now + Duration::from_millis(2300)),
                timer(now + Duration::from_millis(1100)),
            ],
            now,
        )
        .unwrap();
        assert_eq!(soonest, Duration::from_millis(100) + SLACK);
        assert_eq!(next_change_in(&[], now), None);
    }

    #[test]
    fn draws_orange_time_on_the_tile_background() {
        let base = plain_key([20, 30, 60]);
        assert_eq!(orange_pixels(&base), 0);
        for text in ["4:59", "0:09", "59:59", "23:59:59"] {
            let drawn = with_countdown(&base, text).unwrap();
            assert!(orange_pixels(&drawn) > 40, "{text}");
            keypad::validate_jpeg(&drawn).unwrap();
        }
        // Above the band the tile is as it was.
        let drawn = image::load_from_memory_with_format(
            &with_countdown(&base, "1:00").unwrap(),
            ImageFormat::Jpeg,
        )
        .unwrap()
        .to_rgb8();
        let top = drawn.get_pixel(36, 10).0;
        assert!(top[2] > 40 && top[0] < 60, "{top:?}");
        assert_eq!(with_countdown(b"not a jpeg", "1:00"), None);
    }

    #[test]
    fn a_text_too_wide_even_at_the_smallest_size_is_refused() {
        assert_eq!(
            with_countdown(&plain_key([0, 0, 0]), "99:99:99:99:99:99:99"),
            None
        );
    }

    struct Plain(Vec<u8>);
    impl KeyImageProvider for Plain {
        fn key_image(&self, _: &KeyImageRequest) -> Option<Vec<u8>> {
            Some(self.0.clone())
        }
    }

    fn request(key_index: u8, page: &str) -> KeyImageRequest {
        KeyImageRequest {
            key_index,
            content: KeyContent::Action {
                page_id: page.into(),
                label: "Timer".into(),
                icon: cooltek_sdk_service::display_sync::TileIcon::Timer,
            },
        }
    }

    #[test]
    fn only_the_keys_with_a_countdown_on_their_page_change() {
        let inner = Plain(plain_key([0, 0, 0]));
        let now = Instant::now();
        let timers = [Countdown {
            control: 3,
            page_id: Some("root".into()),
            deadline_micros: 0,
            ends: now + Duration::from_secs(90),
        }];
        let provider = CountdownKey {
            inner: &inner,
            timers: &timers,
            now,
        };
        assert!(orange_pixels(&provider.key_image(&request(3, "root")).unwrap()) > 40);
        assert_eq!(provider.key_image(&request(4, "root")).unwrap(), inner.0);
        assert_eq!(provider.key_image(&request(3, "other")).unwrap(), inner.0);
    }

    #[test]
    fn publishing_follows_the_table_and_wakes_only_on_a_change() {
        let faces = TimerFaces::default();
        let mut table = TimerTable::default();
        let action = Action::Timer {
            seconds: 300,
            label: None,
            sound: None,
        };
        let key = TimerKey {
            control: 2,
            gesture: Gesture::Pressed,
            page_id: None,
            dial_function: None,
        };
        faces.publish(&table, 0);
        assert_eq!(faces.version(), 0);
        table.toggle(key.clone(), &action, 1_000_000).unwrap();
        faces.publish(&table, 1_000_000);
        assert_eq!(faces.version(), 1);
        let shown = faces.snapshot();
        assert_eq!((shown.len(), shown[0].control), (1, 2));
        // The same timer later on the clock is not a change.
        faces.publish(&table, 5_000_000);
        assert_eq!(faces.version(), 1);
        table.toggle(key, &action, 6_000_000).unwrap();
        faces.publish(&table, 6_000_000);
        assert_eq!(faces.version(), 2);
        assert!(faces.snapshot().is_empty());
    }
}
