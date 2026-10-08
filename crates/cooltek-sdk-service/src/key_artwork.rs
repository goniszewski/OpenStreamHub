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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Independently timed artwork for one key on one page.
pub struct KeyArtwork {
    /// One static frame, or an ordered bounded animation sequence.
    pub frames: Vec<KeyArtworkFrame>,
    /// Zero loops forever; positive values count total plays, then hold the last frame.
    pub loop_count: u32,
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
}
