//! Portable pre-encoded M4315 image sequences, bounded before allocations.
use std::io::Cursor;

use base64::{Engine, engine::general_purpose::STANDARD};
use cooltek_sdk_masterhub::keypad;
use image::{ImageDecoder, codecs::jpeg::JpegDecoder};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// One portable, pre-encoded key frame and its display duration.
pub struct KeyArtworkFrame {
    /// Canonical standard base64 containing a complete 72×72 baseline JFIF.
    pub jpeg_base64: String,
    /// Display duration, bounded to 100 through 60000 milliseconds.
    pub delay_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Vertical placement of the label text inside one studio design.
pub enum KeyArtworkTextAlign {
    /// The label sits under the top padding, the icon below it.
    Top,
    /// The label block is centred vertically.
    Middle,
    /// The label sits above the bottom padding, the icon above it.
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// The icon-and-label design one key's artwork was prepared from. Pure
/// metadata: nothing drawn reads it, and the studio reopens with it. Absent
/// for artwork prepared from an uploaded file.
pub struct KeyArtworkDesign {
    /// Catalog icon id or an `app:<bundle id>` suggestion; absent for text only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_id: Option<String>,
    /// Label text of at most 128 bytes, possibly empty.
    pub text: String,
    /// Studio text size, 0 (small) through 3 (extra large).
    pub text_size: u8,
    /// Studio icon size, 0 (small) through 2 (large). Artwork saved before the
    /// setting existed reads as medium, the size it was drawn at; large fills the key.
    #[serde(default = "default_icon_size")]
    pub icon_size: u8,
    /// Where the label sits relative to the icon.
    pub align: KeyArtworkTextAlign,
    /// Label colour as typed: `#rgb`, `rgb`, `#rrggbb`, or `rrggbb`.
    pub text_colour: String,
    /// Icon colour in the same bounded hex forms.
    pub icon_colour: String,
    /// Background colour in the same bounded hex forms.
    pub background: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Independently timed artwork for one key on one page.
pub struct KeyArtwork {
    /// One static frame, or an ordered bounded animation sequence.
    pub frames: Vec<KeyArtworkFrame>,
    /// Zero loops forever; positive values count total plays, then hold the last frame.
    pub loop_count: u32,
    /// The studio design this artwork was prepared from, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design: Option<KeyArtworkDesign>,
}

impl KeyArtworkFrame {
    /// Decode and validate a complete 72×72 baseline JFIF, including its pixel data.
    pub fn jpeg(&self) -> Result<Vec<u8>, String> {
        if self.jpeg_base64.len() > KeyArtwork::MAX_FRAME_BYTES.div_ceil(3) * 4 {
            return Err("key artwork frame exceeds 32 KiB".into());
        }
        let bytes = STANDARD
            .decode(&self.jpeg_base64)
            .map_err(|_| "key artwork is not valid base64")?;
        if bytes.len() > KeyArtwork::MAX_FRAME_BYTES {
            return Err("key artwork frame exceeds 32 KiB".into());
        }
        if STANDARD.encode(&bytes) != self.jpeg_base64 {
            return Err("key artwork must use bounded canonical base64".into());
        }
        keypad::validate_jpeg(&bytes)
            .map_err(|error| format!("invalid key artwork JPEG: {error}"))?;
        let decoder = JpegDecoder::new(Cursor::new(&bytes))
            .map_err(|error| format!("invalid key artwork JPEG: {error}"))?;
        if decoder.dimensions() != (72, 72) || decoder.total_bytes() > 72 * 72 * 4 {
            return Err("key artwork must decode to 72×72 pixels".into());
        }
        image::DynamicImage::from_decoder(decoder)
            .map_err(|error| format!("invalid key artwork JPEG pixels: {error}"))?;
        Ok(bytes)
    }
}

fn default_icon_size() -> u8 {
    1
}

impl KeyArtworkDesign {
    /// Highest icon size index; the studio offers small, medium and large.
    pub const MAX_ICON_SIZE: u8 = 2;
    /// Longest stored icon id: a catalog id, or `app:` plus a bundle id.
    pub const MAX_ICON_ID_BYTES: usize = 128;
    /// Longest stored label text, matching the key label limit.
    pub const MAX_TEXT_BYTES: usize = 128;
    /// Highest text size index; the studio offers four sizes.
    pub const MAX_TEXT_SIZE: u8 = 3;

    /// Checks byte bounds, the closed text-size range, and the hex colour
    /// shapes the studio's parser accepts.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(icon_id) = &self.icon_id
            && (icon_id.is_empty()
                || icon_id.len() > Self::MAX_ICON_ID_BYTES
                || icon_id.chars().any(char::is_control))
        {
            return Err(
                "key artwork icon id must be 1 through 128 bytes without control characters".into(),
            );
        }
        if self.text.len() > Self::MAX_TEXT_BYTES || self.text.chars().any(char::is_control) {
            return Err(
                "key artwork design text must be at most 128 bytes without control characters"
                    .into(),
            );
        }
        if self.text_size > Self::MAX_TEXT_SIZE {
            return Err("key artwork text size must be 0 through 3".into());
        }
        if self.icon_size > Self::MAX_ICON_SIZE {
            return Err("key artwork icon size must be 0 through 2".into());
        }
        for (field, colour) in [
            ("text colour", &self.text_colour),
            ("icon colour", &self.icon_colour),
            ("background", &self.background),
        ] {
            validate_hex_colour(colour).map_err(|_| {
                format!("key artwork design {field} must be a hex colour such as #000000 or #fff")
            })?;
        }
        Ok(())
    }
}

/// `#rgb`, `rgb`, `#rrggbb`, or `rrggbb` — the shapes the studio stores.
fn validate_hex_colour(colour: &str) -> Result<(), ()> {
    let digits = colour.strip_prefix('#').unwrap_or(colour);
    if (digits.len() != 3 && digits.len() != 6)
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(());
    }
    Ok(())
}

impl KeyArtwork {
    /// Maximum frames in one artwork.
    pub const MAX_FRAMES: usize = 120;
    /// Maximum prepared JPEG bytes in one frame.
    pub const MAX_FRAME_BYTES: usize = 32 * 1024;
    /// Maximum prepared JPEG bytes in one artwork.
    pub const MAX_BYTES: usize = 512 * 1024;
    /// Maximum prepared JPEG bytes across one profile, below what the 1 MiB
    /// profile JSON limit can hold once base64 encoded.
    pub const MAX_PROFILE_BYTES: usize = 700 * 1024;
    /// Shortest permitted frame duration.
    pub const MIN_DELAY_MS: u32 = 100;
    /// Longest permitted frame duration.
    pub const MAX_DELAY_MS: u32 = 60_000;

    /// Returns the total decoded JPEG byte count after validating every frame.
    pub fn validate(&self) -> Result<usize, String> {
        if let Some(design) = &self.design {
            design.validate()?;
        }
        if self.frames.is_empty() || self.frames.len() > Self::MAX_FRAMES {
            return Err("key artwork requires 1 through 120 frames".into());
        }
        let mut bytes = 0usize;
        for frame in &self.frames {
            if !(Self::MIN_DELAY_MS..=Self::MAX_DELAY_MS).contains(&frame.delay_ms) {
                return Err("key artwork delay must be 100 through 60000 ms".into());
            }
            // Bound aggregate base64 before decoding the next frame.
            if frame.jpeg_base64.len() > (Self::MAX_BYTES - bytes).div_ceil(3) * 4 {
                return Err("key artwork exceeds 512 KiB".into());
            }
            bytes = bytes
                .checked_add(frame.jpeg()?.len())
                .ok_or("artwork byte count overflow")?;
            if bytes > Self::MAX_BYTES {
                return Err("key artwork exceeds 512 KiB".into());
            }
        }
        Ok(bytes)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use image::{ExtendedColorType, ImageEncoder, codecs::jpeg::JpegEncoder};
    use serde_json::json;

    /// A one-color square baseline JFIF.
    pub(crate) fn jpeg(side: u32, shade: u8) -> Vec<u8> {
        let mut out = Vec::new();
        JpegEncoder::new_with_quality(&mut out, 80)
            .write_image(
                &vec![shade; (side * side * 3) as usize],
                side,
                side,
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        out
    }

    /// A valid 72×72 key JPEG grown to exactly `size` bytes by a comment
    /// segment after its JFIF header.
    pub(crate) fn padded(shade: u8, size: usize) -> Vec<u8> {
        let base = jpeg(72, shade);
        let header_end = 4 + usize::from(u16::from_be_bytes([base[4], base[5]]));
        let segment = size - base.len();
        let mut out = base[..header_end].to_vec();
        out.extend([0xff, 0xfe]);
        out.extend(u16::try_from(segment - 2).unwrap().to_be_bytes());
        out.resize(header_end + segment, 0);
        out.extend(&base[header_end..]);
        assert_eq!(out.len(), size);
        out
    }

    pub(crate) fn frame(jpeg: &[u8], delay_ms: u32) -> KeyArtworkFrame {
        KeyArtworkFrame {
            jpeg_base64: STANDARD.encode(jpeg),
            delay_ms,
        }
    }

    pub(crate) fn artwork(frames: Vec<KeyArtworkFrame>) -> KeyArtwork {
        KeyArtwork {
            frames,
            loop_count: 0,
            design: None,
        }
    }

    #[test]
    fn frame_delays_are_bounded_inclusively() {
        let key = jpeg(72, 40);
        for delay in [0, 99, 60_001] {
            assert!(
                artwork(vec![frame(&key, delay)])
                    .validate()
                    .unwrap_err()
                    .contains("delay")
            );
        }
        for delay in [100, 60_000] {
            assert_eq!(artwork(vec![frame(&key, delay)]).validate(), Ok(key.len()));
        }
    }

    #[test]
    fn frame_count_is_one_through_one_hundred_twenty() {
        let key = frame(&jpeg(72, 40), 100);
        assert!(
            artwork(Vec::new())
                .validate()
                .unwrap_err()
                .contains("frames")
        );
        assert!(
            artwork(vec![key.clone(); KeyArtwork::MAX_FRAMES + 1])
                .validate()
                .unwrap_err()
                .contains("frames")
        );
        assert!(
            artwork(vec![key; KeyArtwork::MAX_FRAMES])
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn frames_require_canonical_base64_of_a_72px_baseline_jpeg() {
        // 32,768 bytes encode with one padding character.
        let key = padded(40, 32_768);
        let canonical = STANDARD.encode(&key);
        assert!(canonical.ends_with('=') && !canonical.ends_with("=="));
        assert!(frame(&key, 100).jpeg().is_ok());
        let unpadded = canonical.trim_end_matches('=').to_string();
        // Nonzero discarded bits in the last character decode to the same bytes.
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut trailing = canonical.clone().into_bytes();
        let last = trailing.len() - 2;
        let value = ALPHABET.iter().position(|c| *c == trailing[last]).unwrap();
        trailing[last] = ALPHABET[value ^ 1];
        for text in [
            unpadded,
            String::from_utf8(trailing).unwrap(),
            format!(" {canonical}"),
            format!("{canonical}\n"),
        ] {
            let candidate = KeyArtworkFrame {
                jpeg_base64: text,
                delay_ms: 100,
            };
            assert!(candidate.jpeg().is_err());
        }

        let small = frame(&jpeg(64, 40), 100).jpeg().unwrap_err();
        assert!(small.contains("72"), "{small}");
        // The same JPEG marked progressive (SOF2) instead of baseline (SOF0).
        let mut progressive = jpeg(72, 40);
        let sof = progressive
            .windows(2)
            .position(|marker| marker == [0xff, 0xc0])
            .unwrap();
        progressive[sof + 1] = 0xc2;
        assert!(frame(&progressive, 100).jpeg().is_err());
        assert!(frame(b"not a jpeg", 100).jpeg().is_err());
    }

    #[test]
    fn frames_and_artworks_are_byte_bounded() {
        let largest = padded(40, KeyArtwork::MAX_FRAME_BYTES);
        assert_eq!(
            frame(&largest, 100).jpeg().unwrap().len(),
            KeyArtwork::MAX_FRAME_BYTES
        );
        let over = frame(&padded(40, KeyArtwork::MAX_FRAME_BYTES + 1), 100);
        assert!(over.jpeg().unwrap_err().contains("32 KiB"));
        assert!(artwork(vec![over]).validate().is_err());

        let per_artwork = KeyArtwork::MAX_BYTES / KeyArtwork::MAX_FRAME_BYTES;
        let full = artwork(vec![frame(&largest, 100); per_artwork]);
        assert_eq!(full.validate(), Ok(KeyArtwork::MAX_BYTES));
        let mut over = full;
        over.frames.push(frame(&jpeg(72, 40), 100));
        assert!(over.validate().unwrap_err().contains("512 KiB"));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let jpeg_base64 = STANDARD.encode(jpeg(72, 40));
        let frame = json!({"jpeg_base64": jpeg_base64, "delay_ms": 100});
        assert!(
            serde_json::from_value::<KeyArtwork>(json!({"frames": [frame], "loop_count": 0}))
                .is_ok()
        );
        let mut extra = frame.clone();
        extra["fps"] = json!(10);
        assert!(
            serde_json::from_value::<KeyArtwork>(json!({"frames": [extra], "loop_count": 0}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<KeyArtwork>(
                json!({"frames": [frame], "loop_count": 0, "speed": 1})
            )
            .is_err()
        );
    }

    pub(crate) fn design() -> KeyArtworkDesign {
        KeyArtworkDesign {
            icon_id: Some("volume-2".into()),
            text: "Mute".into(),
            text_size: 1,
            icon_size: 1,
            align: KeyArtworkTextAlign::Bottom,
            text_colour: "#ffffff".into(),
            icon_colour: "#fff".into(),
            background: "204060".into(),
        }
    }

    #[test]
    fn design_metadata_is_optional_and_round_trips() {
        let frame_json = json!({"jpeg_base64": STANDARD.encode(jpeg(72, 40)), "delay_ms": 100});
        let old = json!({"frames": [frame_json], "loop_count": 0});
        let parsed: KeyArtwork = serde_json::from_value(old.clone()).unwrap();
        assert!(parsed.design.is_none());
        // Profiles without a design serialize exactly as before, so the field
        // is invisible until a design exists.
        assert_eq!(serde_json::to_value(&parsed).unwrap(), old);

        let designed = KeyArtwork {
            frames: vec![frame(&jpeg(72, 40), 100)],
            loop_count: 0,
            design: Some(design()),
        };
        let value = serde_json::to_value(&designed).unwrap();
        assert_eq!(
            value["design"],
            json!({
                "icon_id": "volume-2", "text": "Mute", "text_size": 1, "icon_size": 1, "align": "bottom",
                "text_colour": "#ffffff", "icon_colour": "#fff", "background": "204060",
            })
        );
        assert_eq!(
            serde_json::from_value::<KeyArtwork>(value).unwrap(),
            designed
        );
        // A text-only design drops the absent icon id instead of writing null.
        let text_only = KeyArtwork {
            frames: vec![frame(&jpeg(72, 40), 100)],
            loop_count: 0,
            design: Some(KeyArtworkDesign {
                icon_id: None,
                ..design()
            }),
        };
        let value = serde_json::to_value(&text_only).unwrap();
        assert!(value["design"].get("icon_id").is_none());
        assert_eq!(
            serde_json::from_value::<KeyArtwork>(value).unwrap(),
            text_only
        );
        assert_eq!(designed.validate(), Ok(jpeg(72, 40).len()));
    }

    #[test]
    fn design_metadata_is_bounded_and_shape_checked() {
        design().validate().unwrap();
        let mut extra = serde_json::to_value(design()).unwrap();
        extra["fps"] = json!(10);
        assert!(serde_json::from_value::<KeyArtworkDesign>(extra).is_err());
        let mut sideways = serde_json::to_value(design()).unwrap();
        sideways["align"] = json!("sideways");
        assert!(serde_json::from_value::<KeyArtworkDesign>(sideways).is_err());

        let mut text = design();
        text.text = "x".repeat(KeyArtworkDesign::MAX_TEXT_BYTES + 1);
        assert!(text.validate().unwrap_err().contains("128"));
        let mut control = design();
        control.text.push('\u{7}');
        assert!(control.validate().unwrap_err().contains("control"));
        let mut sized = design();
        sized.text_size = KeyArtworkDesign::MAX_TEXT_SIZE + 1;
        assert!(sized.validate().unwrap_err().contains("0 through 3"));
        for colour in ["", "#", "#12", "12345", "#12345", "gggggg"] {
            let mut shaded = design();
            shaded.background = colour.into();
            assert!(shaded.validate().unwrap_err().contains("hex colour"));
        }
        let mut icon = design();
        icon.icon_id = Some(String::new());
        assert!(icon.validate().unwrap_err().contains("icon id"));
        let mut long_icon = design();
        long_icon.icon_id = Some(format!(
            "app:{}",
            "x".repeat(KeyArtworkDesign::MAX_ICON_ID_BYTES)
        ));
        assert!(long_icon.validate().unwrap_err().contains("icon id"));
        let mut unresolvable = design();
        unresolvable.icon_id = Some("renamed-icon".into());
        // Resolution is the studio's concern: an unknown but well-formed id
        // validates, and the studio degrades to its defaults instead.
        unresolvable.validate().unwrap();

        // The design is validated wherever the artwork is.
        let mut broken = artwork(vec![frame(&jpeg(72, 40), 100)]);
        broken.design = Some(KeyArtworkDesign {
            text_size: 9,
            icon_size: 1,
            ..design()
        });
        assert!(broken.validate().unwrap_err().contains("0 through 3"));
    }
}
