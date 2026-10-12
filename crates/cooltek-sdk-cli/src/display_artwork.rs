//! Prepared, cached M4315 key artwork for `DisplaySync`.
//!
//! Optional page artwork overrides each key independently; keys without it
//! get deterministic generated placeholders per tile kind. Every image is drawn into a 72×72 RGB
//! buffer, passed through [`crate::image_prep::prepare_keypad_image`] (the same
//! encoder and validation as `keypad prepare-image`), checked again with
//! `keypad::validate_jpeg`, and cached before a run starts. Serving an image
//! clones cached bytes; nothing is drawn or encoded per input event. Digits
//! come from a built-in 5×7 bitmap font, so no font files are read.

use std::{collections::BTreeMap, io::Cursor, time::Duration};

use cooltek_sdk_masterhub::keypad;
use cooltek_sdk_service::display_sync::{KeyContent, KeyImageProvider, KeyImageRequest};
use image::{ImageFormat, Rgb, RgbImage};

use crate::image_prep::{PrepareOptions, prepare_keypad_image};

/// Key image side in pixels.
const SIDE: u32 = keypad::IMAGE_WIDTH as u32;
/// Deepest child level that shows a Back key.
pub const MAX_BACK_DEPTH: u8 = 8;

const BLACK: [u8; 3] = [0, 0, 0];
const WHITE: [u8; 3] = [255, 255, 255];
const ACTION: [u8; 3] = [36, 52, 78];
const FOLDER_BACKGROUND: [u8; 3] = [24, 24, 24];
const FOLDER: [u8; 3] = [214, 150, 40];
const BADGE: [u8; 3] = [40, 110, 200];

/// 5×7 glyphs, one row per byte, bit 4 is the leftmost column.
const GLYPH_L: [u8; 7] = [
    0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
];
const DIGITS: [[u8; 7]; 9] = [
    [
        0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
    ], // 0
    [
        0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
    ], // 1
    [
        0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
    ], // 2
    [
        0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
    ], // 3
    [
        0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
    ], // 4
    [
        0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
    ], // 5
    [
        0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
    ], // 6
    [
        0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
    ], // 7
    [
        0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
    ], // 8
];
/// Glyph scale: each font pixel becomes a 3×3 block.
const SCALE: u32 = 3;

/// Every key image a keypad run can need, encoded and validated up front.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedKeyArtwork {
    blank: Vec<u8>,
    action: Vec<u8>,
    folder: Vec<u8>,
    back: Vec<Vec<u8>>,
    custom: BTreeMap<(String, u8), KeySequence>,
    page: Option<String>,
    page_started_ms: u64,
    elapsed_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct KeySequence {
    frames: Vec<Vec<u8>>,
    delays: Vec<u64>,
    cycle_ms: u64,
    loop_count: u32,
}

impl KeySequence {
    fn frame(&self, elapsed_ms: u64) -> (usize, Option<Duration>) {
        if self.frames.len() == 1 {
            return (0, None);
        }
        if self.loop_count > 0 && elapsed_ms >= self.cycle_ms * u64::from(self.loop_count) {
            return (self.frames.len() - 1, None);
        }
        let mut within = elapsed_ms % self.cycle_ms;
        for (index, delay) in self.delays.iter().enumerate() {
            if within < *delay {
                return (index, Some(Duration::from_millis(delay - within)));
            }
            within -= delay;
        }
        unreachable!("validated delays sum to the cycle duration")
    }
}

impl PreparedKeyArtwork {
    /// Draws, encodes, and validates every image once. Output is deterministic.
    ///
    /// # Errors
    /// Fails if encoding or keypad JPEG validation fails for any image.
    pub fn prepare() -> Result<Self, String> {
        Ok(Self {
            blank: encode(&RgbImage::from_pixel(SIDE, SIDE, Rgb(BLACK)))?,
            action: encode(&RgbImage::from_pixel(SIDE, SIDE, Rgb(ACTION)))?,
            folder: encode(&draw_folder())?,
            back: (1..=MAX_BACK_DEPTH)
                .map(|depth| encode(&draw_back(depth)))
                .collect::<Result<_, _>>()?,
            custom: BTreeMap::new(),
            page: None,
            page_started_ms: 0,
            elapsed_ms: 0,
        })
    }

    /// Validate/cache portable profile artwork before any device is opened.
    pub fn prepare_profile(profile: &cooltek_sdk_service::Profile) -> Result<Self, String> {
        profile.validate()?;
        let mut prepared = Self::prepare()?;
        if let Some(layout) = &profile.keypad {
            for page in &layout.pages {
                for (key, artwork) in &page.artwork {
                    let delays: Vec<u64> = artwork
                        .frames
                        .iter()
                        .map(|frame| u64::from(frame.delay_ms))
                        .collect();
                    prepared.custom.insert(
                        (
                            page.id.clone(),
                            key.parse().map_err(|_| "invalid artwork key")?,
                        ),
                        KeySequence {
                            frames: artwork
                                .frames
                                .iter()
                                .map(|frame| frame.jpeg())
                                .collect::<Result<_, _>>()?,
                            cycle_ms: delays.iter().sum(),
                            delays,
                            loop_count: artwork.loop_count,
                        },
                    );
                }
            }
        }
        Ok(prepared)
    }

    /// The blank image shown on empty slots.
    pub fn blank(&self) -> &[u8] {
        &self.blank
    }

    /// The placeholder for action tiles.
    pub fn action(&self) -> &[u8] {
        &self.action
    }

    /// The placeholder for folder tiles.
    pub fn folder(&self) -> &[u8] {
        &self.folder
    }

    /// The Back key with the `L{depth}` badge, for depths 1 through 8.
    pub fn back(&self, depth: u8) -> Option<&[u8]> {
        let index = usize::from(depth.checked_sub(1)?);
        self.back.get(index).map(Vec::as_slice)
    }
}

impl KeyImageProvider for PreparedKeyArtwork {
    fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>> {
        let page = match &request.content {
            KeyContent::Empty { page_id }
            | KeyContent::Action { page_id, .. }
            | KeyContent::Folder { page_id, .. } => Some(page_id),
            KeyContent::Back { .. } => None,
        };
        if let Some(sequence) =
            page.and_then(|page| self.custom.get(&(page.clone(), request.key_index)))
        {
            return Some(sequence.frames[sequence.frame(self.elapsed_ms).0].clone());
        }
        let image = match &request.content {
            KeyContent::Empty { .. } => self.blank(),
            KeyContent::Action { .. } => self.action(),
            KeyContent::Folder { .. } => self.folder(),
            KeyContent::Back { depth, .. } => self.back(*depth)?,
        };
        Some(image.to_vec())
    }

    fn advance(&mut self, page_id: &str, elapsed: Duration) {
        let now = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        if self.page.as_deref() != Some(page_id) {
            self.page = Some(page_id.into());
            self.page_started_ms = now;
        }
        self.elapsed_ms = now.saturating_sub(self.page_started_ms);
    }

    fn next_frame_in(&self) -> Option<Duration> {
        self.custom
            .iter()
            .filter(|((page, _), _)| self.page.as_ref() == Some(page))
            .filter_map(|(_, sequence)| sequence.frame(self.elapsed_ms).1)
            .min()
    }
}

/// Brightness a pressed key keeps, in percent.
const PRESSED_PERCENT: u16 = 55;

/// The same key image dimmed, as shown while the key is held down. `None`
/// when the bytes cannot be decoded or re-encoded, or the image is already
/// black (a blank key): the caller then keeps the normal image.
pub fn dimmed(jpeg: &[u8]) -> Option<Vec<u8>> {
    let mut image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg)
        .ok()?
        .to_rgb8();
    if image.dimensions() != (SIDE, SIDE) {
        return None;
    }
    if image.as_raw().iter().all(|&channel| channel == 0) {
        return None;
    }
    for pixel in image.pixels_mut() {
        for channel in &mut pixel.0 {
            *channel = (u16::from(*channel) * PRESSED_PERCENT / 100) as u8;
        }
    }
    encode(&image).ok()
}

/// Wraps a provider so one held key shows dimmed; every other key, and the
/// held key's animation frames, come from the inner provider unchanged.
pub struct PressedKey<'a> {
    pub inner: &'a dyn KeyImageProvider,
    pub pressed: Option<u8>,
}

impl KeyImageProvider for PressedKey<'_> {
    fn key_image(&self, request: &KeyImageRequest) -> Option<Vec<u8>> {
        let image = self.inner.key_image(request)?;
        if self.pressed != Some(request.key_index) {
            return Some(image);
        }
        Some(dimmed(&image).unwrap_or(image))
    }
}

/// Encodes through the shared preparation path, then validates explicitly.
pub(crate) fn encode(image: &RgbImage) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|error| format!("could not stage generated key artwork: {error}"))?;
    let jpeg = prepare_keypad_image(&png, PrepareOptions::default())?;
    keypad::validate_jpeg(&jpeg)
        .map_err(|error| format!("generated key artwork is not a valid key image: {error}"))?;
    Ok(jpeg)
}

fn fill(image: &mut RgbImage, x: u32, y: u32, width: u32, height: u32, color: [u8; 3]) {
    for py in y..(y + height).min(SIDE) {
        for px in x..(x + width).min(SIDE) {
            image.put_pixel(px, py, Rgb(color));
        }
    }
}

fn glyph(image: &mut RgbImage, x: u32, y: u32, rows: &[u8; 7], color: [u8; 3]) {
    for (row, bits) in (0u32..).zip(rows) {
        for column in 0..5 {
            if bits & (0b10000 >> column) != 0 {
                fill(
                    image,
                    x + column * SCALE,
                    y + row * SCALE,
                    SCALE,
                    SCALE,
                    color,
                );
            }
        }
    }
}

/// A folder silhouette: a tab above a body.
fn draw_folder() -> RgbImage {
    let mut image = RgbImage::from_pixel(SIDE, SIDE, Rgb(FOLDER_BACKGROUND));
    fill(&mut image, 12, 18, 20, 8, FOLDER);
    fill(&mut image, 12, 24, 48, 32, FOLDER);
    image
}

/// A left arrow above an `L{depth}` badge.
fn draw_back(depth: u8) -> RgbImage {
    let mut image = RgbImage::from_pixel(SIDE, SIDE, Rgb(BLACK));
    // Arrow head: tip at x = 12, base at x = 32, centred on y = 22.
    for x in 12..=32u32 {
        let half = (x - 12) * 12 / 20;
        fill(&mut image, x, 22 - half, 1, 2 * half + 1, WHITE);
    }
    // Shaft.
    fill(&mut image, 32, 18, 26, 9, WHITE);
    // Badge with "L" and one digit, each 15×21 after scaling.
    fill(&mut image, 16, 40, 41, 28, BADGE);
    glyph(&mut image, 20, 44, &GLYPH_L, WHITE);
    glyph(&mut image, 38, 44, &DIGITS[usize::from(depth)], WHITE);
    image
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashSet;

    use cooltek_sdk_service::{
        Profile, ProfileSession,
        display_sync::{DisplaySync, DisplayTarget, FakeDisplaySink},
    };

    use super::*;
    use base64::Engine;
    use cooltek_sdk_service::{KeyArtwork, KeyArtworkFrame};

    fn decoded_side(jpeg: &[u8]) -> (u32, u32) {
        let image = image::load_from_memory_with_format(jpeg, ImageFormat::Jpeg).unwrap();
        (image.width(), image.height())
    }

    #[test]
    fn every_back_depth_is_a_distinct_valid_72px_key_image() {
        let art = PreparedKeyArtwork::prepare().unwrap();
        let mut seen = HashSet::new();
        for depth in 1..=MAX_BACK_DEPTH {
            let jpeg = art.back(depth).unwrap();
            keypad::validate_jpeg(jpeg).unwrap();
            assert_eq!(decoded_side(jpeg), (72, 72));
            assert!(seen.insert(jpeg.to_vec()), "depth {depth} repeats bytes");
        }
        assert_eq!(art.back(0), None);
        assert_eq!(art.back(MAX_BACK_DEPTH + 1), None);
    }

    #[test]
    fn blank_and_placeholders_are_valid_and_distinct() {
        let art = PreparedKeyArtwork::prepare().unwrap();
        for jpeg in [art.blank(), art.action(), art.folder()] {
            keypad::validate_jpeg(jpeg).unwrap();
            assert_eq!(decoded_side(jpeg), (72, 72));
        }
        let blank = image::load_from_memory(art.blank()).unwrap().to_rgb8();
        assert!(blank.pixels().all(|pixel| pixel.0.iter().all(|&c| c < 8)));
        assert_ne!(art.blank(), art.action());
        assert_ne!(art.blank(), art.folder());
        assert_ne!(art.action(), art.folder());
    }

    #[test]
    fn preparation_is_deterministic() {
        assert_eq!(
            PreparedKeyArtwork::prepare().unwrap(),
            PreparedKeyArtwork::prepare().unwrap()
        );
    }

    #[test]
    fn badge_digits_are_drawn_from_the_bitmap_font() {
        for depth in 1..=MAX_BACK_DEPTH {
            let image = draw_back(depth);
            let digit = &DIGITS[usize::from(depth)];
            for (row, bits) in (0u32..).zip(digit) {
                for column in 0..5u32 {
                    let lit = bits & (0b10000 >> column) != 0;
                    let pixel = image.get_pixel(38 + column * SCALE + 1, 44 + row * SCALE + 1);
                    assert_eq!(pixel.0 == WHITE, lit, "depth {depth} ({column}, {row})");
                }
            }
        }
    }

    #[test]
    fn provider_maps_each_tile_kind_to_cached_artwork() {
        let art = PreparedKeyArtwork::prepare().unwrap();
        let request = |content| KeyImageRequest {
            key_index: 3,
            content,
        };
        let page_id = String::from("root");
        assert_eq!(
            art.key_image(&request(KeyContent::Empty {
                page_id: page_id.clone()
            })),
            Some(art.blank().to_vec())
        );
        assert_eq!(
            art.key_image(&request(KeyContent::Action {
                page_id: page_id.clone(),
                label: "Mute".into()
            })),
            Some(art.action().to_vec())
        );
        assert_eq!(
            art.key_image(&request(KeyContent::Folder {
                page_id: page_id.clone(),
                target_page_id: "child".into(),
                target_name: "Child".into()
            })),
            Some(art.folder().to_vec())
        );
        for depth in 1..=MAX_BACK_DEPTH {
            assert_eq!(
                art.key_image(&request(KeyContent::Back {
                    depth,
                    parent_page_id: page_id.clone()
                })),
                art.back(depth).map(<[u8]>::to_vec)
            );
        }
        assert_eq!(
            art.key_image(&request(KeyContent::Back {
                depth: 9,
                parent_page_id: page_id
            })),
            None
        );
    }

    #[test]
    fn display_sync_draws_a_page_and_the_back_key_from_prepared_artwork() {
        let art = PreparedKeyArtwork::prepare().unwrap();
        let mut session = ProfileSession::new(
            Profile::from_json(include_bytes!(
                "../../../examples/profiles/keypad-folders.json"
            ))
            .unwrap(),
        )
        .unwrap();
        let mut sync = DisplaySync::new(FakeDisplaySink::default());
        let report = sync.sync_page(&session, &art).unwrap();
        assert!(report.is_complete());
        assert_eq!(report.written.len(), 15);
        assert_eq!(sync.drawn().key(14), Some(art.blank()));

        session
            .set_path(&["root".into(), "development".into()])
            .unwrap();
        let plan = sync.plan_page(&session, &art).unwrap();
        assert!(plan.missing().is_empty());
        let report = sync.apply(plan).unwrap();
        assert!(report.written.contains(&DisplayTarget::Key(0)));
        assert_eq!(sync.drawn().key(0), art.back(1));
        // Unchanged blank keys are not rewritten.
        assert!(!report.written.contains(&DisplayTarget::Key(14)));
    }

    pub(crate) fn solid(shade: u8) -> Vec<u8> {
        encode(&RgbImage::from_pixel(
            SIDE,
            SIDE,
            Rgb([shade, 0, 255 - shade]),
        ))
        .unwrap()
    }

    pub(crate) fn key_artwork(shades_and_delays: &[(u8, u32)], loop_count: u32) -> KeyArtwork {
        KeyArtwork {
            frames: shades_and_delays
                .iter()
                .map(|&(shade, delay_ms)| KeyArtworkFrame {
                    jpeg_base64: base64::engine::general_purpose::STANDARD.encode(solid(shade)),
                    delay_ms,
                })
                .collect(),
            loop_count,
            design: None,
        }
    }

    /// The folders fixture with `artwork` added as (page index, key, artwork).
    pub(crate) fn profile_with(artwork: Vec<(usize, &str, KeyArtwork)>) -> Profile {
        let mut profile = Profile::from_json(include_bytes!(
            "../../../examples/profiles/keypad-folders.json"
        ))
        .unwrap();
        let pages = &mut profile.keypad.as_mut().unwrap().pages;
        for (page, key, value) in artwork {
            pages[page].artwork.insert(key.into(), value);
        }
        profile
    }

    fn image_on(art: &PreparedKeyArtwork, page: &str, key_index: u8) -> Vec<u8> {
        art.key_image(&KeyImageRequest {
            key_index,
            content: KeyContent::Empty {
                page_id: page.into(),
            },
        })
        .unwrap()
    }

    fn at(art: &mut PreparedKeyArtwork, page: &str, ms: u64) {
        art.advance(page, Duration::from_millis(ms));
    }

    #[test]
    fn key_sequences_pick_frames_by_elapsed_time_and_hold_after_finite_loops() {
        let sequence = |loop_count| KeySequence {
            frames: vec![vec![0], vec![1], vec![2]],
            delays: vec![100, 200, 300],
            cycle_ms: 600,
            loop_count,
        };
        let ms = Duration::from_millis;
        let forever = sequence(0);
        assert_eq!(forever.frame(0), (0, Some(ms(100))));
        assert_eq!(forever.frame(99), (0, Some(ms(1))));
        assert_eq!(forever.frame(100), (1, Some(ms(200))));
        // A late wake skips straight to the frame the clock selects.
        assert_eq!(forever.frame(350), (2, Some(ms(250))));
        assert_eq!(forever.frame(600), (0, Some(ms(100))));
        assert_eq!(forever.frame(600 * 1000 + 450), (2, Some(ms(150))));
        let twice = sequence(2);
        assert_eq!(twice.frame(1199), (2, Some(ms(1))));
        assert_eq!(twice.frame(1200), (2, None));
        assert_eq!(twice.frame(u64::MAX), (2, None));
        let still = KeySequence {
            frames: vec![vec![7]],
            delays: vec![100],
            cycle_ms: 100,
            loop_count: 0,
        };
        assert_eq!(still.frame(0), (0, None));
        assert_eq!(still.frame(12_345), (0, None));
    }

    #[test]
    fn profile_artwork_overrides_keys_and_restarts_on_page_entry() {
        let mut art = PreparedKeyArtwork::prepare_profile(&profile_with(vec![
            (0, "3", key_artwork(&[(10, 100), (120, 200)], 0)),
            (0, "5", key_artwork(&[(60, 100)], 1)),
            (1, "2", key_artwork(&[(200, 100), (250, 100)], 1)),
        ]))
        .unwrap();
        let ms = Duration::from_millis;
        at(&mut art, "root", 1_000);
        assert_eq!(image_on(&art, "root", 3), solid(10));
        assert_eq!(image_on(&art, "root", 5), solid(60));
        // Keys without artwork keep their generated tiles; artwork is per page.
        assert_eq!(image_on(&art, "root", 4), art.blank());
        assert_eq!(image_on(&art, "development", 3), art.blank());
        assert_eq!(art.next_frame_in(), Some(ms(100)));
        at(&mut art, "root", 1_250);
        assert_eq!(image_on(&art, "root", 3), solid(120));
        assert_eq!(art.next_frame_in(), Some(ms(50)));

        // Entering a page starts its timelines at its first frame.
        at(&mut art, "development", 1_260);
        assert_eq!(image_on(&art, "development", 2), solid(200));
        assert_eq!(art.next_frame_in(), Some(ms(100)));
        at(&mut art, "development", 1_360);
        assert_eq!(image_on(&art, "development", 2), solid(250));
        // One play, then the last frame holds and nothing wakes.
        at(&mut art, "development", 1_460);
        assert_eq!(image_on(&art, "development", 2), solid(250));
        assert_eq!(art.next_frame_in(), None);
        at(&mut art, "root", 1_470);
        assert_eq!(image_on(&art, "root", 3), solid(10));
        assert_eq!(art.next_frame_in(), Some(ms(100)));
        // The Back key of a child page is always generated.
        assert_eq!(
            art.key_image(&KeyImageRequest {
                key_index: 0,
                content: KeyContent::Back {
                    depth: 1,
                    parent_page_id: "root".into()
                },
            }),
            art.back(1).map(<[u8]>::to_vec)
        );
    }

    #[test]
    fn the_earliest_deadline_across_keys_wakes_and_static_artwork_never_does() {
        let mut art = PreparedKeyArtwork::prepare_profile(&profile_with(vec![
            (0, "1", key_artwork(&[(10, 100), (20, 100)], 0)),
            (0, "2", key_artwork(&[(30, 150), (40, 150)], 0)),
        ]))
        .unwrap();
        let ms = Duration::from_millis;
        at(&mut art, "root", 0);
        assert_eq!(art.next_frame_in(), Some(ms(100)));
        at(&mut art, "root", 100);
        assert_eq!(art.next_frame_in(), Some(ms(50)));
        // Deadlines closer than the display's minimum gap are drawn together on
        // the next wake, from the clock: both keys are on their second frame.
        at(&mut art, "root", 180);
        assert_eq!(image_on(&art, "root", 1), solid(20));
        assert_eq!(image_on(&art, "root", 2), solid(40));
        assert_eq!(art.next_frame_in(), Some(ms(20)));

        for profile in [
            profile_with(Vec::new()),
            profile_with(vec![
                (0, "1", key_artwork(&[(10, 100)], 0)),
                (1, "3", key_artwork(&[(20, 60_000)], 1)),
            ]),
        ] {
            let mut art = PreparedKeyArtwork::prepare_profile(&profile).unwrap();
            assert_eq!(art.next_frame_in(), None);
            for page in ["root", "development"] {
                at(&mut art, page, 5);
                assert_eq!(art.next_frame_in(), None);
            }
        }
        // Without a profile the provider is static too.
        let mut art = PreparedKeyArtwork::prepare().unwrap();
        at(&mut art, "root", 0);
        assert_eq!(art.next_frame_in(), None);
    }
}
