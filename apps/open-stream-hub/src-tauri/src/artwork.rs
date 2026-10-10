//! Offline display artwork preparation. `prepare_artwork` turns a PNG or JPEG
//! into the exact JPEG a module display would receive; `save_artwork` writes
//! that JPEG under `<app data>/artwork/`. Nothing is uploaded and nothing is
//! ever sent to a device. Files are never overwritten and only the newest
//! `KEEP_ARTWORK` files of each slug matching the artwork name pattern are kept.

use crate::{
    diagnostics::utc_stamps,
    export::{prune_per_slug, slug, write_new},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use cooltek_sdk_cli::image_prep::{
    FitMode, PrepareOptions, prepare_image, prepare_keypad_artwork, prepare_keypad_image,
    within_frame_budget,
};
use cooltek_sdk_masterhub::{dial, keypad};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DIRECTORY: &str = "artwork";
pub const KEEP_ARTWORK: usize = 50;
/// Decoded source size accepted from the frontend.
pub const MAX_INPUT_BYTES: usize = 8 * 1024 * 1024;
/// Base64 text length of a maximal input, checked before any decoding.
const MAX_BASE64_LEN: usize = MAX_INPUT_BYTES.div_ceil(3) * 4;
const MAX_SLUG: usize = 40;
const STAMP_LEN: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    Dial,
    Keypad,
}

impl Target {
    fn label(self) -> &'static str {
        match self {
            Self::Dial => "dial",
            Self::Keypad => "keypad",
        }
    }

    fn size(self) -> (u32, u32) {
        match self {
            Self::Dial => (dial::IMAGE_WIDTH.into(), dial::IMAGE_HEIGHT.into()),
            Self::Keypad => (keypad::IMAGE_WIDTH.into(), keypad::IMAGE_HEIGHT.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fit {
    Contain,
    Cover,
    Stretch,
}

/// The caller sends every field; there are no defaults here.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRequest {
    pub image_base64: String,
    pub target: Target,
    pub quality: u32,
    pub fit: Fit,
    pub quarter_turns: u32,
    pub background: [u8; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub image_base64: String,
    pub target: Target,
    pub quality: u32,
    pub fit: Fit,
    pub quarter_turns: u32,
    pub background: [u8; 3],
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrepareResult {
    pub jpeg_base64: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SaveResult {
    pub path: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

struct Prepared {
    jpeg: Vec<u8>,
    width: u32,
    height: u32,
}

impl SaveRequest {
    fn prepare_request(&self) -> PrepareRequest {
        PrepareRequest {
            image_base64: self.image_base64.clone(),
            target: self.target,
            quality: self.quality,
            fit: self.fit,
            quarter_turns: self.quarter_turns,
            background: self.background,
        }
    }
}

fn decode_input(text: &str) -> Result<Vec<u8>, String> {
    if text.is_empty() {
        return Err("Choose an image first".into());
    }
    if text.len() > MAX_BASE64_LEN {
        return Err("The image is larger than 8 MiB".into());
    }
    let bytes = STANDARD
        .decode(text)
        .map_err(|_| "The image data is not valid base64".to_string())?;
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("The image is larger than 8 MiB".into());
    }
    Ok(bytes)
}

fn options(request: &PrepareRequest) -> Result<PrepareOptions, String> {
    let quality = u8::try_from(request.quality)
        .ok()
        .filter(|quality| (1..=100).contains(quality))
        .ok_or("JPEG quality must be between 1 and 100")?;
    let quarter_turns = u8::try_from(request.quarter_turns)
        .ok()
        .filter(|turns| *turns <= 3)
        .ok_or("Rotation must be 0, 90, 180 or 270 degrees")?;
    Ok(PrepareOptions {
        quality,
        fit: match request.fit {
            Fit::Contain => FitMode::Contain,
            Fit::Cover => FitMode::Cover,
            Fit::Stretch => FitMode::Stretch,
        },
        quarter_turns,
        background: request.background,
    })
}

fn prepare(request: &PrepareRequest) -> Result<Prepared, String> {
    let options = options(request)?;
    let input = decode_input(&request.image_base64)?;
    let jpeg = match request.target {
        Target::Dial => prepare_image(&input, options),
        Target::Keypad => {
            within_frame_budget(options, |options| prepare_keypad_image(&input, options))
        }
    }?;
    let (width, height) = request.target.size();
    Ok(Prepared {
        jpeg,
        width,
        height,
    })
}

/// Prepares portable static/GIF key frames without opening hardware.
pub fn prepare_key_artwork(
    request: &PrepareRequest,
) -> Result<cooltek_sdk_service::KeyArtwork, String> {
    if request.target != Target::Keypad {
        return Err("Per-key artwork requires the keypad target".into());
    }
    let options = options(request)?;
    let input = decode_input(&request.image_base64)?;
    let prepared = prepare_keypad_artwork(&input, options)?;
    let artwork = cooltek_sdk_service::KeyArtwork {
        frames: prepared
            .frames
            .into_iter()
            .map(|frame| cooltek_sdk_service::KeyArtworkFrame {
                jpeg_base64: STANDARD.encode(frame.jpeg),
                delay_ms: frame.delay_ms,
            })
            .collect(),
        loop_count: prepared.loop_count,
        // Prepared images carry no studio design; the editor attaches one for
        // designs so reopening restores them (an upload clears it).
        design: None,
    };
    // The same check a saved profile gets, so the editor never holds artwork
    // that the profile would then refuse.
    artwork.validate()?;
    Ok(artwork)
}

pub fn prepare_artwork(request: &PrepareRequest) -> Result<PrepareResult, String> {
    let prepared = prepare(request)?;
    Ok(PrepareResult {
        jpeg_base64: STANDARD.encode(&prepared.jpeg),
        width: prepared.width,
        height: prepared.height,
        bytes: prepared.jpeg.len() as u64,
    })
}

/// `<slug>-<target>-yyyymmdd-hhmmss.jpg`; the slug falls back to `artwork`.
pub fn file_name(name: &str, target: Target, stamp: &str) -> String {
    let slug = if name
        .chars()
        .any(|character| character.is_ascii_alphanumeric())
    {
        slug(name)
    } else {
        "artwork".into()
    };
    format!("{slug}-{}-{stamp}.jpg", target.label())
}

/// `(slug, sort stamp)` of `<slug>-<dial|keypad>-yyyymmdd-hhmmss.jpg`, or None
/// for any other name.
fn artwork_stamp(name: &str) -> Option<(&str, &str)> {
    let body = name.strip_suffix(".jpg")?;
    let stamp = body.get(body.len().checked_sub(STAMP_LEN)?..)?;
    let rest = body[..body.len() - STAMP_LEN].strip_suffix('-')?;
    let slug = rest
        .strip_suffix("-dial")
        .or_else(|| rest.strip_suffix("-keypad"))?;
    let stamp_ok = stamp.bytes().enumerate().all(|(index, byte)| {
        if index == 8 {
            byte == b'-'
        } else {
            byte.is_ascii_digit()
        }
    });
    let slug_ok = !slug.is_empty()
        && slug.len() <= MAX_SLUG
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    (stamp_ok && slug_ok).then_some((slug, stamp))
}

/// Deletes all but the newest `keep` artwork files of each slug (both
/// targets together); other files are ignored.
pub fn prune(directory: &Path, keep: usize) {
    prune_per_slug(directory, keep, artwork_stamp);
}

/// The save command without Tauri: prepares, writes, prunes.
pub fn save_artwork(
    request: &SaveRequest,
    app_data: &Path,
    generated_at: u64,
) -> Result<SaveResult, String> {
    let prepared = prepare(&request.prepare_request())?;
    let directory = app_data.join(DIRECTORY);
    let (_, stamp) = utc_stamps(generated_at);
    let path = write_new(
        &directory,
        &file_name(&request.name, request.target, &stamp),
        &prepared.jpeg,
        "artwork",
    )?;
    prune(&directory, KEEP_ARTWORK);
    Ok(SaveResult {
        path: path.to_string_lossy().into_owned(),
        bytes: prepared.jpeg.len() as u64,
        width: prepared.width,
        height: prepared.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        ExtendedColorType, ImageEncoder, ImageFormat, Rgb, RgbImage, RgbaImage,
        codecs::{jpeg::JpegEncoder, png::PngEncoder},
    };
    use std::{fs, path::PathBuf};

    fn png(image: &RgbImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                image,
                image.width(),
                image.height(),
                ExtendedColorType::Rgb8,
            )
            .unwrap();
        bytes
    }

    fn jpeg(image: &RgbImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        JpegEncoder::new_with_quality(&mut bytes, 95)
            .encode_image(image)
            .unwrap();
        bytes
    }

    /// Left half red, right half blue, 40x20.
    fn wide() -> RgbImage {
        RgbImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                Rgb([255, 0, 0])
            } else {
                Rgb([0, 0, 255])
            }
        })
    }

    fn request(input: &[u8], target: Target, fit: Fit, quarter_turns: u32) -> PrepareRequest {
        PrepareRequest {
            image_base64: STANDARD.encode(input),
            target,
            quality: 100,
            fit,
            quarter_turns,
            background: [0, 0, 0],
        }
    }

    fn decode(result: &PrepareResult) -> RgbImage {
        let bytes = STANDARD.decode(&result.jpeg_base64).unwrap();
        assert_eq!(bytes.len() as u64, result.bytes);
        image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
            .unwrap()
            .to_rgb8()
    }

    fn near(pixel: &Rgb<u8>, expected: [u8; 3]) -> bool {
        pixel
            .0
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) < 40)
    }

    #[test]
    fn key_artwork_static_output_is_portable_and_rejects_dial_target() {
        let input = png(&wide());
        let request = request(&input, Target::Keypad, Fit::Contain, 0);
        let result = prepare_key_artwork(&request).unwrap();
        assert!(result.validate().is_ok());
        assert_eq!(result.loop_count, 1);
        assert_eq!(result.frames.len(), 1);
        assert_eq!(result.frames[0].delay_ms, 100);
        keypad::validate_jpeg(&STANDARD.decode(&result.frames[0].jpeg_base64).unwrap()).unwrap();
        let mut wrong_target = request;
        wrong_target.target = Target::Dial;
        assert!(
            prepare_key_artwork(&wrong_target)
                .unwrap_err()
                .contains("keypad target")
        );
    }

    #[test]
    fn a_full_size_key_design_survives_the_key_pipeline_pixel_identically() {
        // The icon-and-label studio renders exactly 72×72 PNGs; the contain
        // fit at the key's own size must pass through without resampling, so
        // the editor preview is what the key receives (a pixel test, not just
        // a dimension check).
        let design = RgbImage::from_fn(72, 72, |x, y| {
            if (14..58).contains(&x) && (14..58).contains(&y) {
                Rgb([235, 235, 240])
            } else {
                Rgb([8, 8, 12])
            }
        });
        let mut asked = request(&png(&design), Target::Keypad, Fit::Contain, 0);
        asked.quality = 85;
        let artwork = prepare_key_artwork(&asked).unwrap();
        assert_eq!(artwork.frames.len(), 1);
        let decoded = image::load_from_memory_with_format(
            &STANDARD.decode(&artwork.frames[0].jpeg_base64).unwrap(),
            ImageFormat::Jpeg,
        )
        .unwrap()
        .to_rgb8();
        assert_eq!(decoded.dimensions(), (72, 72));
        for (x, y, pixel) in decoded.enumerate_pixels() {
            let original = design.get_pixel(x, y);
            assert!(
                near(pixel, original.0),
                "pixel {x},{y}: {pixel:?} vs {original:?}"
            );
        }
    }

    #[test]
    fn png_and_jpeg_inputs_produce_target_sized_jpegs() {
        for input in [png(&wide()), jpeg(&wide())] {
            for (target, size) in [(Target::Dial, 240), (Target::Keypad, 72)] {
                let result = prepare_artwork(&request(&input, target, Fit::Contain, 0)).unwrap();
                assert_eq!((result.width, result.height), (size, size));
                let decoded = decode(&result);
                assert_eq!(decoded.dimensions(), (size, size));
            }
        }
    }

    #[test]
    fn fit_modes_differ_as_documented() {
        let input = png(&wide());
        let run = |fit| decode(&prepare_artwork(&request(&input, Target::Dial, fit, 0)).unwrap());
        // Contain: 40x20 becomes 240x120 centred, black bars above and below.
        let contain = run(Fit::Contain);
        assert!(near(contain.get_pixel(120, 5), [0, 0, 0]));
        assert!(near(contain.get_pixel(40, 120), [255, 0, 0]));
        assert!(near(contain.get_pixel(200, 120), [0, 0, 255]));
        // Cover: centred 20x20 crop spans the red/blue seam, no bars.
        let cover = run(Fit::Cover);
        assert!(!near(cover.get_pixel(120, 5), [0, 0, 0]));
        assert!(near(cover.get_pixel(20, 120), [255, 0, 0]));
        assert!(near(cover.get_pixel(220, 120), [0, 0, 255]));
        // Stretch: whole image fills the square, no bars.
        let stretch = run(Fit::Stretch);
        assert!(!near(stretch.get_pixel(120, 5), [0, 0, 0]));
        assert!(near(stretch.get_pixel(30, 5), [255, 0, 0]));
        assert!(near(stretch.get_pixel(210, 235), [0, 0, 255]));
    }

    #[test]
    fn rotation_is_clockwise_quarter_turns_and_background_is_used() {
        let input = png(&wide());
        let mut base = request(&input, Target::Dial, Fit::Contain, 0);
        base.background = [0, 255, 0];
        let upright = decode(&prepare_artwork(&base).unwrap());
        assert!(near(upright.get_pixel(120, 5), [0, 255, 0]));
        assert!(near(upright.get_pixel(40, 120), [255, 0, 0]));
        // 90 degrees clockwise: left (red) goes to the top, bars to the sides.
        base.quarter_turns = 1;
        let turned = decode(&prepare_artwork(&base).unwrap());
        assert!(near(turned.get_pixel(120, 40), [255, 0, 0]));
        assert!(near(turned.get_pixel(120, 200), [0, 0, 255]));
        assert!(near(turned.get_pixel(5, 120), [0, 255, 0]));
        base.quarter_turns = 2;
        let flipped = decode(&prepare_artwork(&base).unwrap());
        assert!(near(flipped.get_pixel(40, 120), [0, 0, 255]));
        base.quarter_turns = 3;
        let back = decode(&prepare_artwork(&base).unwrap());
        assert!(near(back.get_pixel(120, 40), [0, 0, 255]));
    }

    #[test]
    fn transparency_is_composited_on_the_background() {
        let transparent = RgbaImage::from_pixel(10, 10, image::Rgba([255, 0, 0, 0]));
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&transparent, 10, 10, ExtendedColorType::Rgba8)
            .unwrap();
        let mut asked = request(&bytes, Target::Keypad, Fit::Contain, 0);
        asked.background = [10, 200, 10];
        let decoded = decode(&prepare_artwork(&asked).unwrap());
        assert!(near(decoded.get_pixel(36, 36), [10, 200, 10]));
    }

    #[test]
    fn quality_changes_the_size() {
        let noisy = RgbImage::from_fn(64, 64, |x, y| {
            Rgb([
                (x * 37 % 256) as u8,
                (y * 91 % 256) as u8,
                (x * y % 256) as u8,
            ])
        });
        let input = png(&noisy);
        let mut low = request(&input, Target::Dial, Fit::Stretch, 0);
        low.quality = 5;
        let high = request(&input, Target::Dial, Fit::Stretch, 0);
        assert!(prepare_artwork(&low).unwrap().bytes < prepare_artwork(&high).unwrap().bytes);
    }

    #[test]
    fn keypad_artwork_is_lowered_to_fit_the_frame_budget() {
        let noisy = RgbImage::from_fn(256, 256, |x, y| {
            Rgb([
                ((x * 37) ^ (y * 11)) as u8,
                (y * 91 + x * 7) as u8,
                (x * y % 251) as u8,
            ])
        });
        let asked = request(&png(&noisy), Target::Keypad, Fit::Stretch, 0);
        let prepared = prepare_artwork(&asked).unwrap();
        assert!(prepared.bytes <= cooltek_sdk_service::KeyArtwork::MAX_FRAME_BYTES as u64);
    }

    #[test]
    fn invalid_options_are_reported_readably() {
        let input = png(&wide());
        let mut asked = request(&input, Target::Dial, Fit::Contain, 0);
        for quality in [0, 101, 100_000] {
            asked.quality = quality;
            assert!(
                prepare_artwork(&asked)
                    .unwrap_err()
                    .contains("between 1 and 100")
            );
        }
        asked.quality = 100;
        asked.quarter_turns = 4;
        assert!(prepare_artwork(&asked).unwrap_err().contains("Rotation"));
    }

    #[test]
    fn oversized_input_is_rejected_before_decoding() {
        // Over the base64 cap: rejected without trying to decode (the text is
        // not even valid base64).
        let mut asked = request(b"x", Target::Dial, Fit::Contain, 0);
        asked.image_base64 = "!".repeat(MAX_BASE64_LEN + 1);
        assert!(prepare_artwork(&asked).unwrap_err().contains("8 MiB"));
        // Within the base64 cap but over the decoded cap.
        asked.image_base64 = STANDARD.encode(vec![0u8; MAX_INPUT_BYTES + 1]);
        assert!(prepare_artwork(&asked).unwrap_err().contains("8 MiB"));
    }

    #[test]
    fn bad_base64_empty_and_garbage_are_rejected_with_messages() {
        let mut asked = request(b"", Target::Dial, Fit::Contain, 0);
        assert!(prepare_artwork(&asked).unwrap_err().contains("Choose"));
        for text in [
            "not base64!",
            "aGVsbG8",
            "aGVs bG8=",
            "data:image/png;base64,AAAA",
        ] {
            asked.image_base64 = text.into();
            assert!(
                prepare_artwork(&asked).unwrap_err().contains("base64"),
                "{text}"
            );
        }
        let garbage = request(b"definitely not an image", Target::Dial, Fit::Contain, 0);
        assert!(!prepare_artwork(&garbage).unwrap_err().is_empty());
        let gif = request(
            b"GIF89a\x01\0\x01\0\0\0\0;",
            Target::Keypad,
            Fit::Contain,
            0,
        );
        assert!(!prepare_artwork(&gif).unwrap_err().is_empty());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = u32::MAX;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        !crc
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        let crc = crc32(&chunk[4..]);
        chunk.extend_from_slice(&crc.to_be_bytes());
        chunk
    }

    #[test]
    fn animated_png_is_rejected_with_the_cli_message() {
        let source = png(&RgbImage::from_pixel(1, 1, Rgb([128; 3])));
        let mut animated = source[..33].to_vec();
        animated.extend_from_slice(&png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        let mut control = Vec::new();
        for value in [0u32, 1, 1, 0, 0] {
            control.extend_from_slice(&value.to_be_bytes());
        }
        control.extend_from_slice(&[0, 1, 0, 60, 0, 0]);
        animated.extend_from_slice(&png_chunk(b"fcTL", &control));
        animated.extend_from_slice(&source[33..]);
        let error =
            prepare_artwork(&request(&animated, Target::Dial, Fit::Contain, 0)).unwrap_err();
        assert!(error.contains("Animated PNG"), "{error}");
    }

    #[test]
    fn requests_deserialize_strictly() {
        let good = serde_json::json!({
            "image_base64": "AAAA", "target": "dial", "quality": 90, "fit": "cover",
            "quarter_turns": 1, "background": [1, 2, 3]
        });
        let parsed: PrepareRequest = serde_json::from_value(good.clone()).unwrap();
        assert_eq!((parsed.target, parsed.fit), (Target::Dial, Fit::Cover));
        let mut extra = good.clone();
        extra["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<PrepareRequest>(extra).is_err());
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("quality");
        assert!(serde_json::from_value::<PrepareRequest>(missing).is_err());
        let mut wrong = good;
        wrong["target"] = serde_json::json!("watch");
        assert!(serde_json::from_value::<PrepareRequest>(wrong).is_err());
    }

    fn save_request(name: &str, target: Target) -> SaveRequest {
        SaveRequest {
            image_base64: STANDARD.encode(png(&wide())),
            target,
            quality: 100,
            fit: Fit::Contain,
            quarter_turns: 0,
            background: [0, 0, 0],
            name: name.into(),
        }
    }

    #[test]
    fn file_names_use_the_slug_target_and_stamp() {
        assert_eq!(
            file_name("My Cover Art!", Target::Dial, "20261005-120000"),
            "my-cover-art-dial-20261005-120000.jpg"
        );
        assert_eq!(
            file_name("  ", Target::Keypad, "20261005-120000"),
            "artwork-keypad-20261005-120000.jpg"
        );
        assert_eq!(
            file_name("Zażółć", Target::Keypad, "20261005-120000"),
            "za-keypad-20261005-120000.jpg"
        );
        for name in [
            "my-art-dial-20261005-120000.jpg",
            "a-keypad-20261005-120000.jpg",
        ] {
            assert!(artwork_stamp(name).is_some(), "{name}");
        }
        for name in [
            "my-art-20261005-120000.jpg",
            "my-art-dial-20261005-120000.png",
            "-dial-20261005-120000.jpg",
            "My-dial-20261005-120000.jpg",
            "my-art-dial-20261005_120000.jpg",
            "notes.jpg",
        ] {
            assert!(artwork_stamp(name).is_none(), "{name}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn saved_artwork_is_private_valid_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let request = save_request("Cover", Target::Keypad);
        let result = save_artwork(&request, root.path(), 1_791_201_600).unwrap();
        let directory = root.path().join("artwork");
        let path = directory.join("cover-keypad-20261005-120000.jpg");
        assert_eq!(PathBuf::from(&result.path), path);
        assert_eq!((result.width, result.height), (72, 72));
        let written = fs::read(&path).unwrap();
        assert_eq!(result.bytes, written.len() as u64);
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&directory), 0o700);
        // Identical to the preview bytes.
        let preview = prepare_artwork(&request_to_prepare(&request)).unwrap();
        assert_eq!(STANDARD.decode(preview.jpeg_base64).unwrap(), written);
        let error = save_artwork(&request, root.path(), 1_791_201_600).unwrap_err();
        assert!(error.contains("already exists"));
        assert_eq!(fs::read(&path).unwrap(), written);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    }

    fn request_to_prepare(request: &SaveRequest) -> PrepareRequest {
        request.prepare_request()
    }

    #[test]
    fn invalid_images_write_nothing() {
        let root = tempfile::tempdir().unwrap();
        let mut request = save_request("Cover", Target::Dial);
        request.image_base64 = STANDARD.encode(b"nope");
        assert!(save_artwork(&request, root.path(), 1_791_201_600).is_err());
        assert!(!root.path().join("artwork").exists());
    }

    #[test]
    fn retention_keeps_the_newest_artwork_per_slug_and_ignores_other_files() {
        let directory = tempfile::tempdir().unwrap();
        let directory = directory.path();
        // Older artwork under another name survives many newer saves of "art".
        let logo = [
            file_name("Logo", Target::Dial, "20000101-000000"),
            file_name("Logo", Target::Keypad, "20000101-000001"),
        ];
        for name in &logo {
            fs::write(directory.join(name), "x").unwrap();
        }
        for index in 0..53 {
            let target = if index % 2 == 0 {
                Target::Dial
            } else {
                Target::Keypad
            };
            let name = file_name("art", target, &format!("20261005-12{index:04}"));
            fs::write(directory.join(name), "x").unwrap();
        }
        let unrelated = [
            "notes.jpg",
            "Other-dial-20261005-120000.jpg",
            "art-dial-20261005-120000.png",
            "art-20261005-120000.jpg",
            ".export-x.tmp",
        ];
        for name in unrelated {
            fs::write(directory.join(name), "").unwrap();
        }
        fs::create_dir(directory.join("art-dial-20000101-000000.jpg")).unwrap();
        prune(directory, KEEP_ARTWORK);
        let names: Vec<String> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        let kept = names
            .iter()
            .filter(|name| {
                *name != "art-dial-20000101-000000.jpg"
                    && artwork_stamp(name).is_some_and(|(slug, _)| slug == "art")
            })
            .count();
        assert_eq!(kept, KEEP_ARTWORK);
        for name in &logo {
            assert!(names.contains(name), "{name}");
        }
        assert!(!names.contains(&file_name("art", Target::Keypad, "20261005-120001")));
        assert!(!names.contains(&file_name("art", Target::Dial, "20261005-120002")));
        assert!(names.contains(&file_name("art", Target::Keypad, "20261005-120003")));
        assert!(!names.contains(&file_name("art", Target::Dial, "20261005-120000")));
        assert!(names.contains(&file_name("art", Target::Keypad, "20261005-120051")));
        assert!(names.contains(&file_name("art", Target::Dial, "20261005-120052")));
        for name in unrelated {
            assert!(names.iter().any(|item| item == name), "{name}");
        }
        assert!(directory.join("art-dial-20000101-000000.jpg").is_dir());
    }
}
