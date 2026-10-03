//! Offline static-image preparation for the dial's first JPEG implementation.
//!
//! EXIF orientation and embedded color profiles are not automatically applied.
//! Alpha is composited in the source's converted eight-bit channel values before
//! filtering. Rotation is explicit and clockwise, applied to the fitted square.
//! No device is opened and no HID transport is initialized by this module.

use std::io::Cursor;

use cooltek_sdk_masterhub::dial;
use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageDecoder, ImageFormat, Limits, Rgb,
    RgbImage,
    codecs::{
        jpeg::{JpegDecoder, JpegEncoder},
        png::PngDecoder,
    },
    imageops::{self, FilterType},
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

/// How source artwork fits the square dial image.
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

/// Converts a single static PNG or JPEG to an encoded 240×240 RGB baseline JFIF.
///
/// Accepts progressive JPEG input for baseline re-encoding. Rejects APNG and
/// concatenated JPEG streams instead of silently choosing their first frame.
/// EXIF orientation, ICC profiles, and other source metadata are not copied.
///
/// Options, encoded size, source dimensions, and decoded buffer limits are
/// checked before transformation. The output is entropy-encoded by `image` and
/// then checked against the dial's conservative JPEG framing requirements.
pub fn prepare_image(input: &[u8], options: PrepareOptions) -> Result<Vec<u8>, String> {
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
        _ => return Err("Only static PNG and JPEG source images are supported".into()),
    };
    let fitted = fit_and_matte(&source, options);
    drop(source);
    let rotated = match options.quarter_turns {
        0 => fitted,
        1 => imageops::rotate90(&fitted),
        2 => imageops::rotate180(&fitted),
        3 => imageops::rotate270(&fitted),
        _ => unreachable!("rotation was validated before decoding"),
    };
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, options.quality)
        .encode(
            rotated.as_raw(),
            u32::from(dial::IMAGE_WIDTH),
            u32::from(dial::IMAGE_HEIGHT),
            ExtendedColorType::Rgb8,
        )
        .map_err(|error| format!("Could not encode dial JPEG: {error}"))?;
    // This legal maximum container is only an offline framing check. It is not
    // a device report length: actual packet plans still require a HID descriptor.
    dial::JpegPackets::new(&encoded, usize::from(u16::MAX))
        .map_err(|error| format!("Encoded JPEG does not match the dial format: {error}"))?;
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
    DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Could not decode source image within limits: {error}"))
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

fn fit_and_matte(source: &DynamicImage, options: PrepareOptions) -> RgbImage {
    let matte = MatteView {
        source,
        background: options.background,
    };
    let (width, height) = matte.dimensions();
    let side = u32::from(dial::IMAGE_WIDTH);
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
    fn exif_orientation_is_ignored_and_explicit_rotation_still_applies() {
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
        let output = decoded(&jpeg, PrepareOptions::default());
        color_near(&output, 120, 20, [0; 3]);
        color_near(&output, 20, 120, [240, 20, 20]);
        let output = decoded(
            &jpeg,
            PrepareOptions {
                quarter_turns: 1,
                ..PrepareOptions::default()
            },
        );
        color_near(&output, 20, 120, [0; 3]);
        color_near(&output, 120, 20, [240, 20, 20]);
    }
}
