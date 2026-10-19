//! Offline static-image preparation for dial and keypad JPEG packet plans.
//!
//! A JPEG's EXIF orientation is applied to the source before fitting, so a
//! photo shows upright; explicit rotation still turns the fitted square after
//! that. Embedded color profiles are not applied.
//! Alpha is composited in the source's converted eight-bit channel values before
//! filtering. Rotation is explicit and clockwise, applied to the fitted square.
//! No device is opened and no HID transport is initialized by this module.

use std::io::Cursor;

use cooltek_sdk_masterhub::{dial, keypad};
use cooltek_sdk_service::{DialAnimation, KeyArtwork};
use image::{
    AnimationDecoder, DynamicImage, ExtendedColorType, GenericImageView, ImageDecoder, ImageFormat,
    Limits, Rgb, RgbImage,
    codecs::{
        bmp::BmpDecoder,
        gif::GifDecoder,
        jpeg::{JpegDecoder, JpegEncoder},
        png::PngDecoder,
    },
    imageops::{self, FilterType},
    metadata::Orientation,
};

/// Largest encoded source accepted before format inspection or decoding.
pub const MAX_ENCODED_BYTES: usize = 16 * 1024 * 1024;
/// Strict maximum source width and height.
pub const MAX_DIMENSION: u32 = 4096;
/// Maximum decoded pixel buffer, also requested as the decoder allocation limit.
///
/// Decoder allocation limits are best effort in `image`; this is not a total
/// process-memory limit. Dimensions and decoded pixel byte counts are checked
/// independently before pixel decoding. Resampling uses bounded smaller buffers.
pub const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

/// How source artwork fits the square module image.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FitMode {
    /// Preserve the complete image with centered background bars.
    Contain,
    /// Crop a centered source square before scaling, filling the complete output.
    Cover,
    /// Scale both axes independently to fill the output.
    Stretch,
}

/// Explicit image-conversion choices, validated before decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrepareOptions {
    /// JPEG quality, from 1 through 100.
    pub quality: u8,
    /// Aspect-ratio handling.
    pub fit: FitMode,
    /// Clockwise quarter-turns after fitting: 0, 1, 2, or 3.
    pub quarter_turns: u8,
    /// RGB matte and letterbox background, in raw eight-bit channel values.
    pub background: [u8; 3],
}

impl Default for PrepareOptions {
    fn default() -> Self {
        Self {
            quality: 100,
            fit: FitMode::Contain,
            quarter_turns: 0,
            background: [0; 3],
        }
    }
}

/// Converts a single static PNG, JPEG or BMP to an encoded 240×240 RGB baseline JFIF.
///
/// Accepts progressive JPEG input for baseline re-encoding. Rejects APNG and
/// concatenated JPEG streams instead of silently choosing their first frame.
/// EXIF orientation is applied to the source before fitting. ICC profiles and
/// other source metadata are not copied.
///
/// Options, encoded size, source dimensions, and decoded buffer limits are
/// checked before transformation. The output is entropy-encoded by `image` and
/// then checked against the dial's conservative JPEG framing requirements.
pub fn prepare_image(input: &[u8], options: PrepareOptions) -> Result<Vec<u8>, String> {
    prepare_for_target(input, options, ImageTarget::Dial)
}

/// Converts a single static PNG, JPEG or BMP to an encoded 72×72 RGB baseline JFIF.
///
/// Applies the same bounded decoding, matte, fit, explicit clockwise rotation,
/// and metadata handling as [`prepare_image`]. Validates the encoded image for
/// an offline M4315 keypad packet plan without opening a device.
pub fn prepare_keypad_image(input: &[u8], options: PrepareOptions) -> Result<Vec<u8>, String> {
    prepare_for_target(input, options, ImageTarget::Keypad)
}

/// A prepared, bounded keypad animation. Zero loops means infinite playback.
#[derive(Debug)]
pub struct PreparedKeyArtwork {
    /// Complete 72×72 baseline JPEG frames.
    pub frames: Vec<PreparedKeyArtworkFrame>,
    /// Total plays, or zero for infinite playback.
    pub loop_count: u32,
}

/// One composited and fitted animation frame.
#[derive(Debug)]
pub struct PreparedKeyArtworkFrame {
    /// Complete baseline JPEG: 72×72 for a key, 240×240 for the dial.
    pub jpeg: Vec<u8>,
    /// Playback delay in milliseconds: 100 to 60,000 for a key, 150 to 60,000 for the dial.
    pub delay_ms: u32,
}

/// Prepares PNG/JPEG/BMP or GIF artwork offline, enforcing bounds before decoding.
pub fn prepare_keypad_artwork(
    input: &[u8],
    options: PrepareOptions,
) -> Result<PreparedKeyArtwork, String> {
    if input.is_empty() || input.len() > 8 * 1024 * 1024 {
        return Err("Key artwork source must contain 1 byte through 8 MiB".into());
    }
    if !(1..=100).contains(&options.quality) || options.quarter_turns > 3 {
        return Err("Invalid JPEG quality or clockwise rotation".into());
    }
    if image::guess_format(input).ok() != Some(ImageFormat::Gif) {
        return Ok(PreparedKeyArtwork {
            frames: vec![PreparedKeyArtworkFrame {
                jpeg: within_frame_budget(options, |options| prepare_keypad_image(input, options))?,
                delay_ms: 100,
            }],
            loop_count: 1,
        });
    }
    prepare_gif(input, options, KEYPAD_GIF)
}

/// What a GIF's frames are held to for one target.
#[derive(Clone, Copy)]
struct GifTarget {
    target: ImageTarget,
    /// Most frames accepted.
    max_frames: usize,
    /// Shortest frame time; shorter GIF delays are raised to it.
    min_delay_ms: u32,
    /// Most bytes in one prepared frame.
    max_frame_bytes: usize,
    /// Most bytes across the prepared frames.
    max_bytes: usize,
    /// What the target is called in errors.
    name: &'static str,
}

const KEYPAD_GIF: GifTarget = GifTarget {
    target: ImageTarget::Keypad,
    max_frames: KeyArtwork::MAX_FRAMES,
    min_delay_ms: KeyArtwork::MIN_DELAY_MS,
    max_frame_bytes: KeyArtwork::MAX_FRAME_BYTES,
    max_bytes: KeyArtwork::MAX_BYTES,
    name: "key",
};

const DIAL_GIF: GifTarget = GifTarget {
    target: ImageTarget::Dial,
    max_frames: DialAnimation::MAX_FRAMES,
    min_delay_ms: DialAnimation::MIN_DELAY_MS,
    max_frame_bytes: DialAnimation::MAX_FRAME_BYTES,
    max_bytes: DialAnimation::MAX_BYTES,
    name: "dial",
};

/// Prepares a GIF as a bounded 240×240 dial animation, offline.
///
/// Fits and rotates every composited frame as [`prepare_image`] does a still,
/// lowers the JPEG quality of a frame that is over the dial's per-frame budget,
/// raises delays under 150 ms to 150 ms, and keeps the loop count (zero is
/// infinite). Only GIF input is accepted: a still goes through
/// [`prepare_image`].
pub fn prepare_dial_animation(
    input: &[u8],
    options: PrepareOptions,
) -> Result<PreparedKeyArtwork, String> {
    if input.is_empty() || input.len() > 8 * 1024 * 1024 {
        return Err("Dial animation source must contain 1 byte through 8 MiB".into());
    }
    if !(1..=100).contains(&options.quality) || options.quarter_turns > 3 {
        return Err("Invalid JPEG quality or clockwise rotation".into());
    }
    if image::guess_format(input).ok() != Some(ImageFormat::Gif) {
        return Err("A dial animation needs a GIF".into());
    }
    prepare_gif(input, options, DIAL_GIF)
}

/// Prepares a GIF's frames, then steps the JPEG quality of the whole GIF down
/// while the frames together exceed the target's total budget, so a long
/// photographic GIF is compressed to fit instead of refused at the first try.
fn prepare_gif(
    input: &[u8],
    mut options: PrepareOptions,
    gif: GifTarget,
) -> Result<PreparedKeyArtwork, String> {
    loop {
        match prepare_gif_pass(input, options, gif)? {
            Ok(prepared) => return Ok(prepared),
            Err(too_big) if options.quality <= 10 => return Err(too_big),
            Err(_) => options.quality = options.quality.saturating_sub(15).max(10),
        }
    }
}

/// One pass at one quality. The inner error is the total-size overflow, which a
/// lower quality can cure; any other failure is the outer error.
fn prepare_gif_pass(
    input: &[u8],
    options: PrepareOptions,
    gif: GifTarget,
) -> Result<Result<PreparedKeyArtwork, String>, String> {
    let (count, loop_count) = gif_preflight(input)?;
    if count > gif.max_frames {
        return Err(format!(
            "A {} animation holds at most {} frames; this GIF has {count}",
            gif.name, gif.max_frames
        ));
    }
    let mut decoder =
        GifDecoder::new(Cursor::new(input)).map_err(|e| format!("Could not read GIF: {e}"))?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(512);
    limits.max_image_height = Some(512);
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder
        .set_limits(limits)
        .map_err(|e| format!("GIF exceeds decoder limits: {e}"))?;
    let mut frames = Vec::with_capacity(count);
    let mut output_bytes = 0usize;
    for frame in decoder.into_frames() {
        if frames.len() >= count {
            return Err("GIF frame count changed during decoding".into());
        }
        let frame = frame.map_err(|e| format!("Could not decode GIF frame: {e}"))?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay_ms = numerator.div_ceil(denominator).max(gif.min_delay_ms);
        if delay_ms > 60_000 {
            return Err("GIF frame delay exceeds 60 seconds".into());
        }
        let source = DynamicImage::ImageRgba8(frame.into_buffer());
        let jpeg = within_budget(options, gif.max_frame_bytes, |options| {
            encode_source(source.clone(), options, gif.target)
        })?;
        output_bytes += jpeg.len();
        if output_bytes > gif.max_bytes {
            return Ok(Err(format!(
                "Prepared {} animation exceeds {} KiB even at JPEG quality {}",
                gif.name,
                gif.max_bytes / 1024,
                options.quality
            )));
        }
        frames.push(PreparedKeyArtworkFrame { jpeg, delay_ms });
    }
    if frames.len() != count {
        return Err("GIF did not decode all declared frames".into());
    }
    Ok(Ok(PreparedKeyArtwork { frames, loop_count }))
}

/// Encodes at the requested quality, then steps the quality down until the
/// frame fits [`KeyArtwork::MAX_FRAME_BYTES`], so a prepared frame always
/// passes profile validation.
pub fn within_frame_budget(
    options: PrepareOptions,
    encode: impl FnMut(PrepareOptions) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    within_budget(options, KeyArtwork::MAX_FRAME_BYTES, encode)
}

fn within_budget(
    mut options: PrepareOptions,
    max_bytes: usize,
    mut encode: impl FnMut(PrepareOptions) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    loop {
        let jpeg = encode(options)?;
        if jpeg.len() <= max_bytes {
            return Ok(jpeg);
        }
        if options.quality <= 10 {
            return Err(format!(
                "Prepared frame exceeds {} KiB even at JPEG quality {}",
                max_bytes / 1024,
                options.quality
            ));
        }
        options.quality = options.quality.saturating_sub(10).max(10);
    }
}

// Inspect GIF block structure before the decoder allocates frame buffers. This
// also distinguishes absent looping metadata (one play) from infinite repeat.
fn gif_preflight(input: &[u8]) -> Result<(usize, u32), String> {
    fn take<'a>(input: &'a [u8], position: &mut usize, length: usize) -> Result<&'a [u8], String> {
        let end = position.checked_add(length).ok_or("Invalid GIF length")?;
        let bytes = input.get(*position..end).ok_or("Truncated GIF")?;
        *position = end;
        Ok(bytes)
    }
    fn blocks(input: &[u8], position: &mut usize) -> Result<(), String> {
        loop {
            let length = take(input, position, 1)?[0] as usize;
            if length == 0 {
                return Ok(());
            }
            take(input, position, length)?;
        }
    }
    let mut position = 0;
    let header = take(input, &mut position, 13)?;
    if &header[..6] != b"GIF87a" && &header[..6] != b"GIF89a" {
        return Err("Invalid GIF signature".into());
    }
    let width = u32::from(u16::from_le_bytes([header[6], header[7]]));
    let height = u32::from(u16::from_le_bytes([header[8], header[9]]));
    if width == 0 || height == 0 || width > 512 || height > 512 {
        return Err("GIF canvas must be 1 through 512 pixels on each axis".into());
    }
    if header[10] & 0x80 != 0 {
        take(input, &mut position, 3usize << ((header[10] & 7) + 1))?;
    }
    let mut count = 0usize;
    let mut loop_count = 1;
    loop {
        match take(input, &mut position, 1)?[0] {
            0x3b => {
                if count == 0 {
                    return Err("GIF contains no frames".into());
                }
                // Bytes after the trailer are common padding; nothing reads them.
                return Ok((count, loop_count));
            }
            0x2c => {
                let frame = take(input, &mut position, 9)?;
                let left = u32::from(u16::from_le_bytes([frame[0], frame[1]]));
                let top = u32::from(u16::from_le_bytes([frame[2], frame[3]]));
                let fw = u32::from(u16::from_le_bytes([frame[4], frame[5]]));
                let fh = u32::from(u16::from_le_bytes([frame[6], frame[7]]));
                if fw == 0 || fh == 0 || left + fw > width || top + fh > height {
                    return Err("GIF frame extends outside its canvas".into());
                }
                count += 1;
                if count > 120 {
                    return Err("GIF exceeds 120 frames".into());
                }
                if u64::from(width) * u64::from(height) * 4 * count as u64 > 32 * 1024 * 1024 {
                    return Err("GIF exceeds 32 MiB cumulative RGBA work".into());
                }
                if frame[8] & 0x80 != 0 {
                    take(input, &mut position, 3usize << ((frame[8] & 7) + 1))?;
                }
                take(input, &mut position, 1)?; // LZW minimum code size
                blocks(input, &mut position)?;
            }
            0x21 => match take(input, &mut position, 1)?[0] {
                0xf9 => {
                    if take(input, &mut position, 1)?[0] != 4 {
                        return Err("Invalid GIF graphic control".into());
                    }
                    let control = take(input, &mut position, 4)?;
                    if u16::from_le_bytes([control[1], control[2]]) > 6000 {
                        return Err("GIF frame delay exceeds 60 seconds".into());
                    }
                    if take(input, &mut position, 1)?[0] != 0 {
                        return Err("Invalid GIF graphic control terminator".into());
                    }
                }
                0xff => {
                    let length = take(input, &mut position, 1)?[0] as usize;
                    let identifier = take(input, &mut position, length)?;
                    let looping = identifier == b"NETSCAPE2.0" || identifier == b"ANIMEXTS1.0";
                    loop {
                        let length = take(input, &mut position, 1)?[0] as usize;
                        if length == 0 {
                            break;
                        }
                        let data = take(input, &mut position, length)?;
                        if looping {
                            match data {
                                [1, low, high] => {
                                    let repeats = u16::from_le_bytes([*low, *high]);
                                    loop_count = if repeats == 0 {
                                        0
                                    } else {
                                        u32::from(repeats) + 1
                                    };
                                }
                                // The buffering sub-block carries a size hint only.
                                [2, _, _, _, _] => {}
                                _ => return Err("Invalid GIF loop metadata".into()),
                            }
                        }
                    }
                }
                0x01 => return Err("GIF plain-text rendering is unsupported".into()),
                _ => blocks(input, &mut position)?,
            },
            _ => return Err("Invalid GIF block".into()),
        }
    }
}

#[derive(Clone, Copy)]
enum ImageTarget {
    Dial,
    Keypad,
}

impl ImageTarget {
    fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Dial => (u32::from(dial::IMAGE_WIDTH), u32::from(dial::IMAGE_HEIGHT)),
            Self::Keypad => (
                u32::from(keypad::IMAGE_WIDTH),
                u32::from(keypad::IMAGE_HEIGHT),
            ),
        }
    }

    fn validate(self, jpeg: &[u8]) -> Result<(), String> {
        match self {
            // This legal maximum container is only an offline framing check.
            // Actual packet plans still require a HID descriptor.
            Self::Dial => dial::JpegPackets::new(jpeg, usize::from(u16::MAX))
                .map(|_| ())
                .map_err(|error| format!("Encoded JPEG does not match the dial format: {error}")),
            Self::Keypad => keypad::validate_jpeg(jpeg)
                .map_err(|error| format!("Encoded JPEG does not match the keypad format: {error}")),
        }
    }
}

fn prepare_for_target(
    input: &[u8],
    options: PrepareOptions,
    target: ImageTarget,
) -> Result<Vec<u8>, String> {
    if !(1..=100).contains(&options.quality) {
        return Err("JPEG quality must be between 1 and 100".into());
    }
    if options.quarter_turns > 3 {
        return Err("Image rotation must contain 0 through 3 clockwise quarter-turns".into());
    }
    if input.is_empty() || input.len() > MAX_ENCODED_BYTES {
        return Err("Encoded source image must contain 1 byte through 16 MiB".into());
    }
    let format = image::guess_format(input)
        .map_err(|error| format!("Could not identify source image: {error}"))?;
    let source = match format {
        ImageFormat::Png => {
            let decoder = PngDecoder::with_limits(Cursor::new(input), decode_limits())
                .map_err(|error| format!("Could not read PNG header within limits: {error}"))?;
            if decoder
                .is_apng()
                .map_err(|error| format!("Could not inspect PNG animation metadata: {error}"))?
            {
                return Err("Animated PNG input is unsupported; supply one static image".into());
            }
            decode_bounded(decoder)?
        }
        ImageFormat::Jpeg => {
            validate_single_jpeg(input)?;
            let decoder = JpegDecoder::new(Cursor::new(input))
                .map_err(|error| format!("Could not read JPEG header: {error}"))?;
            decode_bounded(decoder)?
        }
        ImageFormat::Bmp => {
            let decoder = BmpDecoder::new(Cursor::new(input))
                .map_err(|error| format!("Could not read BMP header: {error}"))?;
            decode_bounded(decoder)?
        }
        _ => return Err("Only static PNG, JPEG and BMP source images are supported".into()),
    };
    encode_source(source, options, target)
}

fn encode_source(
    source: DynamicImage,
    options: PrepareOptions,
    target: ImageTarget,
) -> Result<Vec<u8>, String> {
    let (output_width, output_height) = target.dimensions();
    let fitted = fit_and_matte(&source, options, output_width);
    drop(source);
    let rotated = match options.quarter_turns {
        0 => fitted,
        1 => imageops::rotate90(&fitted),
        2 => imageops::rotate180(&fitted),
        3 => imageops::rotate270(&fitted),
        _ => unreachable!("rotation was validated before decoding"),
    };
    let mut encoded = Vec::new();
    let target_name = match target {
        ImageTarget::Dial => "dial",
        ImageTarget::Keypad => "keypad",
    };
    JpegEncoder::new_with_quality(&mut encoded, options.quality)
        .encode(
            rotated.as_raw(),
            output_width,
            output_height,
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Could not encode {target_name} JPEG: {error}"))?;
    target.validate(&encoded)?;
    Ok(encoded)
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

fn decode_bounded<D: ImageDecoder>(mut decoder: D) -> Result<DynamicImage, String> {
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err("Source image dimensions must be between 1 and 4096 on each axis".into());
    }
    if decoder.total_bytes() > MAX_DECODED_BYTES {
        return Err("Decoded source image exceeds the 64 MiB pixel-buffer limit".into());
    }
    decoder
        .set_limits(decode_limits())
        .map_err(|error| format!("Could not apply source decoder limits: {error}"))?;
    // An unreadable orientation tag leaves the picture as stored.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut source = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Could not decode source image within limits: {error}"))?;
    source.apply_orientation(orientation);
    Ok(source)
}

/// An RGB view composites alpha as samples are fetched, avoiding a second full
/// source-sized RGB/RGBA buffer and preventing hidden transparent RGB from
/// leaking into resampling against the chosen matte.
struct MatteView<'a> {
    source: &'a DynamicImage,
    background: [u8; 3],
}

impl GenericImageView for MatteView<'_> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) {
        self.source.dimensions()
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let source = self.source.get_pixel(x, y).0;
        let alpha = u32::from(source[3]);
        Rgb(std::array::from_fn(|channel| {
            let color = u32::from(source[channel]);
            let background = u32::from(self.background[channel]);
            ((color * alpha + background * (255 - alpha) + 127) / 255) as u8
        }))
    }
}

fn fit_and_matte(source: &DynamicImage, options: PrepareOptions, side: u32) -> RgbImage {
    let matte = MatteView {
        source,
        background: options.background,
    };
    let (width, height) = matte.dimensions();
    match options.fit {
        FitMode::Cover => {
            let crop_side = width.min(height);
            let crop = matte.view(
                (width - crop_side) / 2,
                (height - crop_side) / 2,
                crop_side,
                crop_side,
            );
            imageops::resize(&*crop, side, side, FilterType::Lanczos3)
        }
        FitMode::Stretch => imageops::resize(&matte, side, side, FilterType::Lanczos3),
        FitMode::Contain => {
            let (scaled_width, scaled_height) = if width >= height {
                (side, ((height * side + width / 2) / width).max(1))
            } else {
                (((width * side + height / 2) / height).max(1), side)
            };
            let scaled =
                imageops::resize(&matte, scaled_width, scaled_height, FilterType::Lanczos3);
            let mut canvas = RgbImage::from_pixel(side, side, Rgb(options.background));
            imageops::replace(
                &mut canvas,
                &scaled,
                i64::from((side - scaled_width) / 2),
                i64::from((side - scaled_height) / 2),
            );
            canvas
        }
    }
}

/// Walks JPEG marker framing to reject missing EOI and concatenated streams.
/// Actual marker contents and entropy data are validated by the image decoder.
fn validate_single_jpeg(input: &[u8]) -> Result<(), String> {
    if !input.starts_with(&[0xff, 0xd8]) {
        return Err("JPEG source is missing its SOI marker".into());
    }
    let mut position = 2;
    let mut in_scan = false;
    loop {
        if in_scan {
            while input.get(position).is_some_and(|&byte| byte != 0xff) {
                position += 1;
            }
        }
        if input.get(position) != Some(&0xff) {
            return Err("JPEG source is truncated or missing a marker".into());
        }
        while input.get(position) == Some(&0xff) {
            position += 1;
        }
        let marker = *input
            .get(position)
            .ok_or("JPEG source has a truncated marker")?;
        position += 1;
        match marker {
            0x00 if in_scan => continue,
            0xd0..=0xd7 if in_scan => continue,
            0x01 => continue,
            0xd9 if position == input.len() => return Ok(()),
            0xd9 => return Err("JPEG source contains trailing data or multiple images".into()),
            0x00 | 0xd0..=0xd8 => {
                return Err("JPEG source has an unexpected standalone marker".into());
            }
            _ => {}
        }
        let length_bytes = input
            .get(position..position + 2)
            .ok_or("JPEG source has a truncated segment length")?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            return Err("JPEG source has an invalid segment length".into());
        }
        position = position
            .checked_add(length)
            .filter(|&end| end <= input.len())
            .ok_or("JPEG source has a truncated segment")?;
        in_scan = marker == 0xda;
    }
}

#[cfg(test)]
mod tests {
    use image::{ImageEncoder, Rgba, RgbaImage, codecs::png::PngEncoder};

    use super::*;

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut encoded = Vec::new();
        PngEncoder::new(&mut encoded)
            .write_image(
                image.as_raw(),
                image.width(),
                image.height(),
                ExtendedColorType::Rgba8,
            )
            .unwrap();
        encoded
    }

    // This decodes the entropy-coded output, independently of the SDK's framing
    // validator. Test images are original synthetic artwork, not device captures.
    fn decoded(input: &[u8], options: PrepareOptions) -> RgbImage {
        let output = prepare_image(input, options).unwrap();
        assert!(output.starts_with(&[0xff, 0xd8, 0xff, 0xe0]));
        let image = image::load_from_memory_with_format(&output, ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8();
        assert_eq!(image.dimensions(), (240, 240));
        image
    }

    fn color_near(image: &RgbImage, x: u32, y: u32, expected: [u8; 3]) {
        let actual = image.get_pixel(x, y).0;
        for channel in 0..3 {
            assert!(
                actual[channel].abs_diff(expected[channel]) <= 5,
                "pixel ({x},{y}) {actual:?} differs from {expected:?}"
            );
        }
    }

    #[test]
    fn default_contain_preserves_full_source_with_centered_background() {
        let source = png(&RgbaImage::from_pixel(80, 40, Rgba([240, 30, 20, 255])));
        let output = decoded(
            &source,
            PrepareOptions {
                background: [5, 10, 50],
                ..PrepareOptions::default()
            },
        );
        color_near(&output, 120, 20, [5, 10, 50]);
        color_near(&output, 120, 120, [240, 30, 20]);
        color_near(&output, 120, 220, [5, 10, 50]);
        color_near(&output, 20, 100, [240, 30, 20]);
        assert_eq!(PrepareOptions::default().fit, FitMode::Contain);
    }

    #[test]
    fn cover_center_crop_and_stretch_produce_distinct_expected_colors() {
        let source = png(&RgbaImage::from_fn(300, 100, |x, _| {
            Rgba(match x {
                0..100 => [240, 20, 20, 255],
                100..200 => [20, 20, 240, 255],
                _ => [20, 240, 20, 255],
            })
        }));
        let cover = decoded(
            &source,
            PrepareOptions {
                fit: FitMode::Cover,
                ..PrepareOptions::default()
            },
        );
        for x in [20, 120, 220] {
            color_near(&cover, x, 120, [20, 20, 240]);
        }
        let stretch = decoded(
            &source,
            PrepareOptions {
                fit: FitMode::Stretch,
                ..PrepareOptions::default()
            },
        );
        color_near(&stretch, 20, 20, [240, 20, 20]);
        color_near(&stretch, 120, 20, [20, 20, 240]);
        color_near(&stretch, 220, 220, [20, 240, 20]);
    }

    #[test]
    fn every_explicit_quarter_turn_rotates_clockwise() {
        let colors = [[240, 20, 20], [20, 240, 20], [20, 20, 240], [240, 240, 20]];
        let source = png(&RgbaImage::from_fn(80, 80, |x, y| {
            let index = match (x >= 40, y >= 40) {
                (false, false) => 0,
                (true, false) => 1,
                (true, true) => 2,
                (false, true) => 3,
            };
            let color = colors[index];
            Rgba([color[0], color[1], color[2], 255])
        }));
        for quarter_turns in 0..=3 {
            let output = decoded(
                &source,
                PrepareOptions {
                    quarter_turns,
                    ..PrepareOptions::default()
                },
            );
            for (corner, (x, y)) in [(60, 60), (180, 60), (180, 180), (60, 180)]
                .into_iter()
                .enumerate()
            {
                color_near(
                    &output,
                    x,
                    y,
                    colors[(corner + 4 - usize::from(quarter_turns)) % 4],
                );
            }
        }
    }

    #[test]
    fn alpha_is_matted_before_filtering_and_transparent_rgb_is_ignored() {
        let source = png(&RgbaImage::from_fn(40, 40, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 128])
            } else {
                Rgba([0, 255, 0, 0])
            }
        }));
        let output = decoded(
            &source,
            PrepareOptions {
                background: [0, 0, 200],
                ..PrepareOptions::default()
            },
        );
        color_near(&output, 40, 120, [128, 0, 100]);
        color_near(&output, 200, 120, [0, 0, 200]);
        let black = decoded(&source, PrepareOptions::default());
        color_near(&black, 40, 120, [128, 0, 0]);
        color_near(&black, 200, 120, [0, 0, 0]);
    }

    #[test]
    fn keypad_preparation_preserves_matte_contain_and_clockwise_rotation_at_72_pixels() {
        let source = png(&RgbaImage::from_fn(80, 40, |x, _| {
            if x < 40 {
                Rgba([255, 0, 0, 128])
            } else {
                Rgba([0, 255, 0, 0])
            }
        }));
        let jpeg = prepare_keypad_image(
            &source,
            PrepareOptions {
                quarter_turns: 1,
                background: [0, 0, 200],
                ..PrepareOptions::default()
            },
        )
        .unwrap();
        keypad::validate_jpeg(&jpeg).unwrap();
        assert!(dial::JpegPackets::new(&jpeg, 65).is_err());
        let output = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8();
        assert_eq!(output.dimensions(), (72, 72));
        color_near(&output, 36, 10, [128, 0, 100]);
        color_near(&output, 36, 62, [0, 0, 200]);
        color_near(&output, 10, 36, [0, 0, 200]);
        color_near(&output, 62, 36, [0, 0, 200]);
    }

    #[test]
    fn keypad_cover_and_stretch_keep_their_expected_aspect_ratio_behavior() {
        let source = png(&RgbaImage::from_fn(300, 100, |x, _| {
            Rgba(match x {
                0..100 => [240, 20, 20, 255],
                100..200 => [20, 20, 240, 255],
                _ => [20, 240, 20, 255],
            })
        }));
        for fit in [FitMode::Cover, FitMode::Stretch] {
            let jpeg = prepare_keypad_image(
                &source,
                PrepareOptions {
                    fit,
                    ..PrepareOptions::default()
                },
            )
            .unwrap();
            let output = image::load_from_memory_with_format(&jpeg, ImageFormat::Jpeg)
                .unwrap()
                .into_rgb8();
            assert_eq!(output.dimensions(), (72, 72));
            let expected = match fit {
                FitMode::Cover => [[20, 20, 240]; 3],
                FitMode::Stretch => [[240, 20, 20], [20, 20, 240], [20, 240, 20]],
                FitMode::Contain => unreachable!(),
            };
            for (x, color) in [8, 36, 64].into_iter().zip(expected) {
                color_near(&output, x, 36, color);
            }
        }
    }

    #[test]
    fn jpeg_input_is_reencoded_as_rgb_and_packets_reassemble_exactly() {
        let mut source = Vec::new();
        JpegEncoder::new_with_quality(&mut source, 95)
            .encode(&vec![100; 60 * 40], 60, 40, ExtendedColorType::L8)
            .unwrap();
        let options = PrepareOptions {
            fit: FitMode::Stretch,
            ..PrepareOptions::default()
        };
        let output = prepare_image(&source, options).unwrap();
        let image = decoded(&source, options);
        color_near(&image, 120, 120, [100; 3]);
        let decoder = JpegDecoder::new(Cursor::new(&output)).unwrap();
        assert_eq!(decoder.color_type(), image::ColorType::Rgb8);
        let mut reconstructed = Vec::new();
        let packets: Vec<_> = dial::JpegPackets::new(&output, 64).unwrap().collect();
        for (index, packet) in packets.iter().enumerate() {
            assert_eq!(packet.len(), 64);
            assert_eq!(packet[7], u8::from(index + 1 == packets.len()));
            let length = usize::from(u16::from_le_bytes([packet[4], packet[5]]));
            reconstructed.extend_from_slice(&packet[8..8 + length]);
        }
        assert_eq!(reconstructed, output);
    }

    #[test]
    fn options_are_validated_before_format_inspection_and_decoding() {
        for quality in [0, 101, 255] {
            let error = prepare_image(
                b"not an image",
                PrepareOptions {
                    quality,
                    ..PrepareOptions::default()
                },
            )
            .unwrap_err();
            assert!(error.contains("quality"));
        }
        let error = prepare_image(
            b"not an image",
            PrepareOptions {
                quarter_turns: 4,
                ..PrepareOptions::default()
            },
        )
        .unwrap_err();
        assert!(error.contains("rotation"));
        let source = png(&RgbaImage::from_pixel(1, 1, Rgba([128; 4])));
        for quality in [1, 100] {
            assert!(
                prepare_image(
                    &source,
                    PrepareOptions {
                        quality,
                        ..PrepareOptions::default()
                    },
                )
                .is_ok()
            );
        }
    }

    #[test]
    fn a_bmp_is_decoded_like_a_png_and_a_truncated_one_is_rejected() {
        let options = PrepareOptions::default();
        let rgb = RgbImage::from_fn(8, 4, |x, y| Rgb([(x * 30) as u8, (y * 60) as u8, 200]));
        let mut bmp = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut bmp)
            .write_image(rgb.as_raw(), 8, 4, ExtendedColorType::Rgb8)
            .unwrap();
        let from_bmp = prepare_image(&bmp, options).unwrap();
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(rgb.as_raw(), 8, 4, ExtendedColorType::Rgb8)
            .unwrap();
        assert_eq!(from_bmp, prepare_image(&png, options).unwrap());
        assert!(prepare_keypad_image(&bmp, options).is_ok());
        assert!(prepare_image(&bmp[..bmp.len() / 2], options).is_err());
    }

    #[test]
    fn corrupt_unsupported_and_oversized_encoded_sources_are_rejected() {
        let options = PrepareOptions::default();
        for source in [&[][..], b"not an image", b"GIF89a\x01\0\x01\0\0\0\0"] {
            assert!(prepare_image(source, options).is_err());
        }
        assert!(prepare_image(&vec![0; MAX_ENCODED_BYTES + 1], options).is_err());
        let source = png(&RgbaImage::from_pixel(2, 2, Rgba([128; 4])));
        assert!(prepare_image(&source[..source.len() / 2], options).is_err());
        let jpeg = prepare_image(&source, options).unwrap();
        assert!(prepare_image(&jpeg[..jpeg.len() - 2], options).is_err());
        let mut concatenated = jpeg.clone();
        concatenated.extend_from_slice(&jpeg);
        assert!(
            prepare_image(&concatenated, options)
                .unwrap_err()
                .contains("multiple images")
        );
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

    fn change_png_dimensions(png: &mut [u8], width: u32, height: u32) {
        assert_eq!(&png[12..16], b"IHDR");
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
    }

    #[test]
    fn source_dimension_and_16bit_decoded_allocation_limits_precede_pixel_decoding() {
        let mut source = png(&RgbaImage::from_pixel(1, 1, Rgba([128; 4])));
        change_png_dimensions(&mut source, 4097, 1);
        assert!(prepare_image(&source, PrepareOptions::default()).is_err());
        let mut source = Vec::new();
        PngEncoder::new(&mut source)
            .write_image(&[0; 8], 1, 1, ExtendedColorType::Rgba16)
            .unwrap();
        change_png_dimensions(&mut source, 3072, 3072);
        let error = prepare_image(&source, PrepareOptions::default()).unwrap_err();
        assert!(error.contains("64 MiB"), "unexpected error: {error}");
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
    fn apng_is_rejected_instead_of_silently_choosing_a_frame() {
        let source = png(&RgbaImage::from_pixel(1, 1, Rgba([128; 4])));
        let mut animated = source[..33].to_vec();
        animated.extend_from_slice(&png_chunk(b"acTL", &[0, 0, 0, 1, 0, 0, 0, 0]));
        let mut control = Vec::new();
        for value in [0u32, 1, 1, 0, 0] {
            control.extend_from_slice(&value.to_be_bytes());
        }
        control.extend_from_slice(&[0, 1, 0, 60, 0, 0]);
        animated.extend_from_slice(&png_chunk(b"fcTL", &control));
        animated.extend_from_slice(&source[33..]);
        let error = prepare_image(&animated, PrepareOptions::default()).unwrap_err();
        assert!(error.contains("Animated PNG"), "unexpected error: {error}");
    }

    #[test]
    fn exif_orientation_is_applied_and_explicit_rotation_still_follows() {
        let source = RgbImage::from_pixel(80, 40, Rgb([240, 20, 20]));
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode_image(&source)
            .unwrap();
        // An original little-endian EXIF IFD with orientation 6 (90° clockwise).
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        let mut metadata = vec![0xff, 0xe1];
        metadata.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        metadata.extend_from_slice(exif);
        jpeg.splice(2..2, metadata);
        // Orientation 6 turns the 80×40 picture upright into 40×80, which
        // fills the middle of the square with bars left and right.
        let output = decoded(&jpeg, PrepareOptions::default());
        color_near(&output, 20, 120, [0; 3]);
        color_near(&output, 120, 20, [240, 20, 20]);
        // The explicit turn then applies to the fitted square.
        let output = decoded(
            &jpeg,
            PrepareOptions {
                quarter_turns: 1,
                ..PrepareOptions::default()
            },
        );
        color_near(&output, 120, 20, [0; 3]);
        color_near(&output, 20, 120, [240, 20, 20]);
    }
}

#[cfg(test)]
mod keypad_animation_tests {
    use super::*;
    use std::borrow::Cow;

    fn gif_source(
        width: u16,
        height: u16,
        count: usize,
        delay: u16,
        repeat: Option<gif::Repeat>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder =
                gif::Encoder::new(&mut bytes, width, height, &[0, 0, 0, 255, 0, 0, 0, 0, 255])
                    .unwrap();
            if let Some(repeat) = repeat {
                encoder.set_repeat(repeat).unwrap();
            }
            for _ in 0..count {
                let frame = gif::Frame {
                    width,
                    height,
                    delay,
                    buffer: Cow::Owned(vec![1; width as usize * height as usize]),
                    ..Default::default()
                };
                encoder.write_frame(&frame).unwrap();
            }
        }
        bytes
    }

    #[test]
    fn a_dial_animation_has_240_pixel_frames_slower_delays_and_its_own_bounds() {
        let source = gif_source(300, 200, 3, 5, Some(gif::Repeat::Finite(1)));
        let result = prepare_dial_animation(&source, PrepareOptions::default()).unwrap();
        assert_eq!(result.frames.len(), 3);
        assert_eq!(result.loop_count, 2);
        for frame in &result.frames {
            // 5 cs is 50 ms, raised to the dial's 150 ms floor.
            assert_eq!(frame.delay_ms, 150);
            dial::validate_jpeg(&frame.jpeg).unwrap();
            assert!(frame.jpeg.len() <= DialAnimation::MAX_FRAME_BYTES);
        }
        // Long delays are kept, and an infinite loop stays zero.
        let slow = gif_source(2, 2, 2, 150, Some(gif::Repeat::Infinite));
        let result = prepare_dial_animation(&slow, PrepareOptions::default()).unwrap();
        assert_eq!((result.frames[0].delay_ms, result.loop_count), (1500, 0));
        // More frames than a dial holds, and anything that is not a GIF.
        let long = gif_source(2, 2, DialAnimation::MAX_FRAMES + 1, 20, None);
        let error = prepare_dial_animation(&long, PrepareOptions::default()).unwrap_err();
        assert!(error.contains("at most 30 frames"), "{error}");
        let still = RgbImage::from_pixel(8, 8, Rgb([1, 2, 3]));
        let mut png = Vec::new();
        still
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .unwrap();
        assert!(
            prepare_dial_animation(&png, PrepareOptions::default())
                .unwrap_err()
                .contains("needs a GIF")
        );
        assert!(prepare_dial_animation(&[], PrepareOptions::default()).is_err());
    }

    #[test]
    fn gif_timing_loop_metadata_and_source_bounds() {
        let source = gif_source(2, 2, 2, 1, Some(gif::Repeat::Finite(2)));
        let result = prepare_keypad_artwork(&source, PrepareOptions::default()).unwrap();
        assert_eq!(result.loop_count, 3);
        assert_eq!(result.frames.len(), 2);
        assert!(
            result
                .frames
                .iter()
                .all(|frame| frame.delay_ms == 100 && keypad::validate_jpeg(&frame.jpeg).is_ok())
        );
        let infinite = gif_source(1, 1, 1, 15, Some(gif::Repeat::Infinite));
        let result = prepare_keypad_artwork(&infinite, PrepareOptions::default()).unwrap();
        assert_eq!(result.loop_count, 0);
        assert_eq!(result.frames[0].delay_ms, 150);
        assert_eq!(
            prepare_keypad_artwork(&gif_source(1, 1, 1, 10, None), PrepareOptions::default())
                .unwrap()
                .loop_count,
            1
        );
        assert!(
            prepare_keypad_artwork(&vec![0; 8 * 1024 * 1024 + 1], PrepareOptions::default())
                .is_err()
        );
        assert!(
            prepare_keypad_artwork(&source[..source.len() - 1], PrepareOptions::default()).is_err()
        );
    }

    #[test]
    fn gif_buffering_metadata_and_trailing_bytes_are_ignored_but_bad_loops_rejected() {
        let source = gif_source(2, 2, 2, 10, Some(gif::Repeat::Finite(2)));
        let at = source
            .windows(11)
            .position(|window| window == b"NETSCAPE2.0")
            .unwrap()
            + 11;
        assert_eq!(source[at..at + 5], [3, 1, 2, 0, 0]);
        // A NETSCAPE buffering sub-block after the loop sub-block, then padding.
        let mut buffered = source.clone();
        buffered.splice(at + 4..at + 4, [5, 2, 0, 16, 0, 0]);
        buffered.extend([0; 7]);
        let result = prepare_keypad_artwork(&buffered, PrepareOptions::default()).unwrap();
        assert_eq!((result.frames.len(), result.loop_count), (2, 3));
        for (offset, byte) in [(1, 3), (0, 2)] {
            let mut malformed = source.clone();
            malformed[at + offset] = byte;
            assert!(
                prepare_keypad_artwork(&malformed, PrepareOptions::default())
                    .unwrap_err()
                    .contains("loop metadata")
            );
        }
    }

    #[test]
    fn oversized_frames_step_jpeg_quality_down_or_fail_clearly() {
        let mut tried = Vec::new();
        let jpeg = within_frame_budget(PrepareOptions::default(), |options| {
            tried.push(options.quality);
            Ok(vec![
                0;
                if options.quality > 70 {
                    KeyArtwork::MAX_FRAME_BYTES + 1
                } else {
                    10
                }
            ])
        })
        .unwrap();
        assert_eq!(jpeg.len(), 10);
        assert_eq!(tried.first(), Some(&PrepareOptions::default().quality));
        assert!(tried.windows(2).all(|pair| pair[1] < pair[0]));
        assert!(*tried.last().unwrap() <= 70);

        let mut tried = Vec::new();
        let error = within_frame_budget(PrepareOptions::default(), |options| {
            tried.push(options.quality);
            Ok(vec![0; KeyArtwork::MAX_FRAME_BYTES + 1])
        })
        .unwrap_err();
        assert!(error.contains("32 KiB"), "{error}");
        assert_eq!(tried.last(), Some(&10));
        // Seeded 72×72 RGB noise at quality 100 is close to the worst case this
        // encoder produces for a key; it fits the budget unchanged, so the
        // step-down above is a guard rather than a routine path.
        let mut state = 0x9e37_79b9u32;
        let noise = RgbImage::from_fn(72, 72, |_, _| {
            let mut channel = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            };
            Rgb([channel(), channel(), channel()])
        });
        let mut source = Vec::new();
        noise
            .write_to(&mut Cursor::new(&mut source), ImageFormat::Png)
            .unwrap();
        let options = PrepareOptions {
            quality: 100,
            ..PrepareOptions::default()
        };
        let result = prepare_keypad_artwork(&source, options).unwrap();
        let jpeg = &result.frames[0].jpeg;
        assert!(jpeg.len() <= KeyArtwork::MAX_FRAME_BYTES, "{}", jpeg.len());
        assert!(jpeg.len() > 16 * 1024, "noise is not near the worst case");
        assert_eq!(*jpeg, prepare_keypad_image(&source, options).unwrap());
        keypad::validate_jpeg(jpeg).unwrap();
    }

    #[test]
    fn gif_limits_are_checked_before_pixel_decoding() {
        for (source, expected) in [
            (gif_source(513, 1, 1, 10, None), "canvas"),
            (gif_source(1, 1, 121, 10, None), "120 frames"),
            (gif_source(512, 512, 33, 10, None), "cumulative"),
            (gif_source(1, 1, 1, 6001, None), "60 seconds"),
        ] {
            assert!(
                prepare_keypad_artwork(&source, PrepareOptions::default())
                    .unwrap_err()
                    .contains(expected)
            );
        }
        let source = gif_source(1, 1, 1, 6000, None);
        assert_eq!(
            prepare_keypad_artwork(&source, PrepareOptions::default())
                .unwrap()
                .frames[0]
                .delay_ms,
            60_000
        );
    }

    #[test]
    fn gif_composites_transparency_and_restores_previous_and_background() {
        let mut bytes = Vec::new();
        {
            let mut encoder =
                gif::Encoder::new(&mut bytes, 2, 1, &[0, 0, 0, 255, 0, 0, 0, 0, 255]).unwrap();
            for (pixels, disposal) in [
                (vec![1, 1], gif::DisposalMethod::Keep),
                (vec![2, 0], gif::DisposalMethod::Previous),
                (vec![0, 2], gif::DisposalMethod::Background),
                (vec![0, 0], gif::DisposalMethod::Keep),
            ] {
                encoder
                    .write_frame(&gif::Frame {
                        width: 2,
                        height: 1,
                        delay: 10,
                        transparent: Some(0),
                        dispose: disposal,
                        buffer: Cow::Owned(pixels),
                        ..Default::default()
                    })
                    .unwrap();
            }
        }
        let options = PrepareOptions {
            fit: FitMode::Stretch,
            background: [0, 255, 0],
            ..Default::default()
        };
        let result = prepare_keypad_artwork(&bytes, options).unwrap();
        let rgb = |index: usize| {
            image::load_from_memory(&result.frames[index].jpeg)
                .unwrap()
                .to_rgb8()
        };
        let near = |pixel: Rgb<u8>, expected: [u8; 3]| {
            pixel
                .0
                .iter()
                .zip(expected)
                .all(|(actual, expected)| actual.abs_diff(expected) < 30)
        };
        assert!(near(*rgb(1).get_pixel(5, 36), [0, 0, 255]));
        assert!(near(*rgb(1).get_pixel(66, 36), [255, 0, 0]));
        assert!(near(*rgb(2).get_pixel(5, 36), [255, 0, 0]));
        assert!(near(*rgb(2).get_pixel(66, 36), [0, 0, 255]));
        assert!(near(*rgb(3).get_pixel(5, 36), [0, 255, 0]));
        assert!(near(*rgb(3).get_pixel(66, 36), [0, 255, 0]));
    }

    #[test]
    fn output_budget_rejects_large_prepared_animation() {
        let mut bytes = Vec::new();
        let palette: Vec<u8> = (0..=255u8)
            .flat_map(|value| [value, value.wrapping_mul(73), value.wrapping_mul(137)])
            .collect();
        {
            let mut encoder = gif::Encoder::new(&mut bytes, 72, 72, &palette).unwrap();
            let mut state = 12345u32;
            for _ in 0..120 {
                let pixels = (0..72 * 72)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 17;
                        state ^= state << 5;
                        state as u8
                    })
                    .collect();
                encoder
                    .write_frame(&gif::Frame {
                        width: 72,
                        height: 72,
                        delay: 10,
                        buffer: Cow::Owned(pixels),
                        ..Default::default()
                    })
                    .unwrap();
            }
        }
        // At quality 100 the frames together are over the 512 KiB budget; the
        // whole GIF is re-encoded at lower quality until it fits.
        let prepared = prepare_keypad_artwork(&bytes, PrepareOptions::default()).unwrap();
        assert_eq!(prepared.frames.len(), 120);
        let total: usize = prepared.frames.iter().map(|frame| frame.jpeg.len()).sum();
        assert!(total <= KEYPAD_GIF.max_bytes, "{total} bytes");
        let at_full_quality = match prepare_gif_pass(&bytes, PrepareOptions::default(), KEYPAD_GIF)
        {
            Ok(result) => result,
            Err(error) => panic!("{error}"),
        };
        assert!(at_full_quality.unwrap_err().contains("512 KiB"));
    }
}
