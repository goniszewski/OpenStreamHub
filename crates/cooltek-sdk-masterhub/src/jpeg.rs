//! Shared conservative baseline-JFIF framing validation for IPS image plans.
//!
//! This checks container structure, component parameters, and expected dimensions.
//! It does not entropy-decode JPEG data or establish physical display acceptance.

use hidapi::{HidError, HidResult};

pub(crate) const MAX_JPEG_LENGTH: usize = 16 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> HidError {
    HidError::HidApiError {
        message: message.into(),
    }
}

pub(crate) fn validate(
    jpeg: &[u8],
    module_name: &str,
    expected_width: u16,
    expected_height: u16,
) -> HidResult<()> {
    if jpeg.len() > MAX_JPEG_LENGTH {
        return Err(invalid(format!(
            "{module_name} JPEG exceeds the 16 MiB implementation limit"
        )));
    }
    if !jpeg.starts_with(&[0xff, 0xd8, 0xff, 0xe0]) {
        return Err(invalid(format!(
            "{module_name} image must begin with a JPEG SOI and JFIF APP0 marker"
        )));
    }
    let mut position = 2;
    let mut components = None;
    let mut has_quantization_table = false;
    let mut has_huffman_table = false;
    loop {
        if jpeg.get(position) != Some(&0xff) {
            return Err(invalid("JPEG is missing a marker prefix"));
        }
        while jpeg.get(position) == Some(&0xff) {
            position += 1;
        }
        let marker = *jpeg
            .get(position)
            .ok_or_else(|| invalid("JPEG marker is truncated"))?;
        position += 1;
        if matches!(marker, 0x00 | 0x01 | 0xd0..=0xd9) {
            return Err(invalid(
                "Unexpected standalone JPEG marker before image scan",
            ));
        }
        let length_bytes = jpeg
            .get(position..position + 2)
            .ok_or_else(|| invalid("JPEG segment length is truncated"))?;
        let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
        if length < 2 {
            return Err(invalid("JPEG segment length is smaller than its header"));
        }
        let body = jpeg
            .get(position + 2..position + length)
            .ok_or_else(|| invalid("JPEG segment is truncated"))?;
        let is_first_segment = position == 4;
        position += length;
        if is_first_segment && (length < 16 || !body.starts_with(b"JFIF\0")) {
            return Err(invalid("JPEG APP0 does not contain a complete JFIF header"));
        }
        match marker {
            0xc0 => {
                if components.is_some() || body.len() != 15 || body[0] != 8 || body[5] != 3 {
                    return Err(invalid(format!(
                        "{module_name} JPEG requires one 8-bit three-component baseline frame"
                    )));
                }
                let height = u16::from_be_bytes([body[1], body[2]]);
                let width = u16::from_be_bytes([body[3], body[4]]);
                if width != expected_width || height != expected_height {
                    return Err(invalid(format!(
                        "{module_name} JPEG dimensions must be {expected_width}×{expected_height}"
                    )));
                }
                let ids = [body[6], body[9], body[12]];
                if ids[0] == ids[1] || ids[0] == ids[2] || ids[1] == ids[2] {
                    return Err(invalid("JPEG frame has duplicate component identifiers"));
                }
                for component in body[6..].as_chunks::<3>().0 {
                    let sampling = component[1];
                    if !(1..=4).contains(&(sampling >> 4))
                        || !(1..=4).contains(&(sampling & 0x0f))
                        || component[2] > 3
                    {
                        return Err(invalid("JPEG frame has invalid component parameters"));
                    }
                }
                components = Some(ids);
            }
            0xc1..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return Err(invalid("Only baseline JPEG frames are currently supported"));
            }
            0xdb => has_quantization_table = !body.is_empty(),
            0xc4 => has_huffman_table = !body.is_empty(),
            0xda => {
                let ids =
                    components.ok_or_else(|| invalid("JPEG scan precedes its baseline frame"))?;
                if !has_quantization_table || !has_huffman_table {
                    return Err(invalid(
                        "JPEG scan is missing quantization or Huffman tables",
                    ));
                }
                if body.len() != 10 || body[0] != 3 || body[7..] != [0, 63, 0] {
                    return Err(invalid(format!(
                        "{module_name} JPEG requires one interleaved baseline scan"
                    )));
                }
                let scan_ids = [body[1], body[3], body[5]];
                if scan_ids[0] == scan_ids[1]
                    || scan_ids[0] == scan_ids[2]
                    || scan_ids[1] == scan_ids[2]
                    || !scan_ids.iter().all(|id| ids.contains(id))
                    || [body[2], body[4], body[6]]
                        .iter()
                        .any(|table| table >> 4 > 3 || table & 0x0f > 3)
                {
                    return Err(invalid("JPEG scan has invalid component parameters"));
                }
                return validate_scan_tail(&jpeg[position..]);
            }
            _ => {}
        }
    }
}

fn validate_scan_tail(tail: &[u8]) -> HidResult<()> {
    let mut position = 0;
    let mut has_entropy_bytes = false;
    while position < tail.len() {
        if tail[position] != 0xff {
            has_entropy_bytes = true;
            position += 1;
            continue;
        }
        while tail.get(position) == Some(&0xff) {
            position += 1;
        }
        let marker = *tail
            .get(position)
            .ok_or_else(|| invalid("JPEG scan marker is truncated"))?;
        position += 1;
        match marker {
            0x00 => has_entropy_bytes = true,
            0xd0..=0xd7 => {}
            0xd9 if has_entropy_bytes && position == tail.len() => return Ok(()),
            0xd9 => {
                return Err(invalid(
                    "JPEG scan is empty or contains trailing data after EOI",
                ));
            }
            _ => {
                return Err(invalid(
                    "JPEG scan contains an unsupported marker or additional scan",
                ));
            }
        }
    }
    Err(invalid("JPEG is missing its EOI marker"))
}
