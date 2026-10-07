//! Prepared, cached M4315 key artwork for `DisplaySync`.
//!
//! Schema-2 keypad tiles carry no image field, so tiles get deterministic
//! generated placeholders per tile kind. Every image is drawn into a 72×72 RGB
//! buffer, passed through [`crate::image_prep::prepare_keypad_image`] (the same
//! encoder and validation as `keypad prepare-image`), checked again with
//! `keypad::validate_jpeg`, and cached before a run starts. Serving an image
//! clones cached bytes; nothing is drawn or encoded per input event. Digits
//! come from a built-in 5×7 bitmap font, so no font files are read.

use std::io::Cursor;

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
        })
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
        let image = match &request.content {
            KeyContent::Empty { .. } => self.blank(),
            KeyContent::Action { .. } => self.action(),
            KeyContent::Folder { .. } => self.folder(),
            KeyContent::Back { depth, .. } => self.back(*depth)?,
        };
        Some(image.to_vec())
    }
}

/// Encodes through the shared preparation path, then validates explicitly.
fn encode(image: &RgbImage) -> Result<Vec<u8>, String> {
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
mod tests {
    use std::collections::HashSet;

    use cooltek_sdk_service::{
        Profile, ProfileSession,
        display_sync::{DisplaySync, DisplayTarget, FakeDisplaySink},
    };

    use super::*;

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
}
