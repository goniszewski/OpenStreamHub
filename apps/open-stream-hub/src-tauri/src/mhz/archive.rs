//! A bounded, in-memory ZIP reader for official `.mhz` files.
//!
//! Nothing is extracted to disk and nothing is shelled out. Only the central
//! directory is trusted for the entry list, and every entry is cross-checked
//! against its local header. Every limit is checked before inflating and
//! again on the bytes actually inflated, and any hit aborts the whole import.
//! Image folders (`uploadImages/`, `gifPools/`) are listed and skipped: their
//! data is never read. See `docs/research/masterhub-mhz-format.md`.

use flate2::read::DeflateDecoder;
use std::{collections::HashSet, io::Read};

/// Largest `.mhz` (or outer multi-profile `.zip`) accepted.
pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
/// Most entries in one archive, folders included.
pub const MAX_ENTRIES: usize = 256;
/// Largest inflated profile JSON entry.
pub const MAX_PROFILE_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Largest sum of uncompressed sizes: every declared size, plus every byte
/// actually inflated, across nested archives.
pub const MAX_TOTAL_UNCOMPRESSED: u64 = 64 * 1024 * 1024;
/// Highest uncompressed-to-compressed ratio of one entry.
pub const MAX_RATIO: u64 = 100;
/// Entries at most this large are exempt from the ratio rule: small, highly
/// repetitive JSON can compress better than 100:1 without being a bomb, and
/// the size limits bound them anyway.
pub const RATIO_EXEMPT_BYTES: u64 = 1024 * 1024;
/// Most `.mhz` files inside an outer multi-profile `.zip` (one level only).
pub const MAX_NESTED: usize = 8;
const MAX_NAME_BYTES: usize = 255;

const EOCD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// General-purpose flag bits: 0 encrypted, 3 data descriptor, 6 strong
/// encryption, 11 UTF-8 names, 13 masked central directory.
const FLAG_ENCRYPTED: u16 = 1;
const FLAG_DESCRIPTOR: u16 = 1 << 3;
const FLAG_STRONG: u16 = 1 << 6;
const FLAG_MASKED: u16 = 1 << 13;

/// One profile JSON entry, inflated and CRC-checked.
#[derive(Debug)]
pub struct ProfileData {
    pub json: Vec<u8>,
    /// The `<serial>` part of `profile<id>_<serial>`. Used only to scrub that
    /// serial from imported text; never stored.
    pub entry_serial: String,
}

/// What an archive (and its nested archives) contained.
#[derive(Debug, Default)]
pub struct Contents {
    pub profiles: Vec<ProfileData>,
    /// `uploadImages/` and `gifPools/` files that were skipped unread.
    pub skipped_images: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Profile,
    Image,
    ImageFolder,
    Nested,
}

#[derive(Debug)]
struct Entry {
    name: String,
    kind: Kind,
    method: u16,
    crc: u32,
    compressed: u64,
    uncompressed: u64,
    offset: usize,
    data_start: usize,
    data_end: usize,
}

/// Reads an untrusted archive held in memory.
pub fn read(bytes: &[u8]) -> Result<Contents, String> {
    let mut inflated = 0;
    read_level(bytes, true, &mut inflated)
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, String> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| "the archive is truncated".to_string())
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, String> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| "the archive is truncated".to_string())
}

/// The end-of-central-directory record: the last signature whose comment
/// length ends exactly at the end of the file.
fn find_eocd(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() < 22 {
        return Err("this file is not a ZIP archive".into());
    }
    let lowest = bytes.len().saturating_sub(22 + u16::MAX as usize);
    let mut at = bytes.len() - 22;
    loop {
        if u32_at(bytes, at)? == EOCD && at + 22 + u16_at(bytes, at + 20)? as usize == bytes.len() {
            return Ok(at);
        }
        if at == lowest {
            return Err("this file is not a ZIP archive".into());
        }
        at -= 1;
    }
}

/// Rejects absolute paths, `..`, `.`, empty components, backslashes, drive
/// letters and colons, control and invisible characters, and overlong names.
fn check_name(name: &str) -> Result<(), String> {
    let bad = name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.starts_with('/')
        || name.contains(['\\', ':'])
        || name
            .chars()
            .any(|c| c.is_control() || cooltek_sdk_service::is_invisible_text_character(c));
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if bad
        || trimmed
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("the archive contains an unsafe entry name".into());
    }
    Ok(())
}

fn is_profile_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("profile") else {
        return false;
    };
    let Some((id, serial)) = rest.split_once('_') else {
        return false;
    };
    (1..=9).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_digit())
        && (1..=64).contains(&serial.len())
        && serial.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn classify(name: &str, nested_allowed: bool) -> Result<Kind, String> {
    if is_profile_name(name) {
        return Ok(Kind::Profile);
    }
    if name == "uploadImages/" || name == "gifPools/" {
        return Ok(Kind::ImageFolder);
    }
    for folder in ["uploadImages/", "gifPools/"] {
        if let Some(file) = name.strip_prefix(folder)
            && !file.is_empty()
            && !file.contains('/')
        {
            return Ok(Kind::Image);
        }
    }
    if nested_allowed
        && !name.contains('/')
        && name.len() > 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".mhz")
    {
        return Ok(Kind::Nested);
    }
    Err("the archive contains an unexpected entry; only one profile JSON entry and the optional uploadImages/ and gifPools/ folders are accepted".into())
}

/// Rejects AES (`0x9901`), ZIP64 (`0x0001`) and strong-encryption
/// (`0x0017`) extra fields, and malformed extra blocks.
fn check_extra(mut extra: &[u8]) -> Result<(), String> {
    while !extra.is_empty() {
        if extra.len() < 4 {
            return Err("the archive has a malformed extra field".into());
        }
        let id = u16::from_le_bytes([extra[0], extra[1]]);
        let size = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        match id {
            0x9901 | 0x0017 => return Err("encrypted archive entries are not supported".into()),
            0x0001 => return Err("ZIP64 archives are not supported".into()),
            _ => {}
        }
        extra = extra
            .get(4 + size..)
            .ok_or("the archive has a malformed extra field")?;
    }
    Ok(())
}

fn read_level(bytes: &[u8], nested_allowed: bool, inflated: &mut u64) -> Result<Contents, String> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(format!(
            "the archive is larger than {} MiB",
            MAX_ARCHIVE_BYTES / 1024 / 1024
        ));
    }
    let eocd = find_eocd(bytes)?;
    if eocd >= 20 && u32_at(bytes, eocd - 20)? == ZIP64_LOCATOR {
        return Err("ZIP64 archives are not supported".into());
    }
    let disk = u16_at(bytes, eocd + 4)?;
    let directory_disk = u16_at(bytes, eocd + 6)?;
    let disk_entries = u16_at(bytes, eocd + 8)?;
    let total_entries = u16_at(bytes, eocd + 10)?;
    let directory_size = u32_at(bytes, eocd + 12)?;
    let directory_offset = u32_at(bytes, eocd + 16)?;
    if disk != 0 || directory_disk != 0 || disk_entries != total_entries {
        return Err("multi-disk archives are not supported".into());
    }
    if total_entries == u16::MAX || directory_size == u32::MAX || directory_offset == u32::MAX {
        return Err("ZIP64 archives are not supported".into());
    }
    let count = total_entries as usize;
    if count == 0 {
        return Err("the archive is empty".into());
    }
    if count > MAX_ENTRIES {
        return Err(format!("the archive has more than {MAX_ENTRIES} entries"));
    }
    let directory_offset = directory_offset as usize;
    let directory_end = directory_offset
        .checked_add(directory_size as usize)
        .filter(|end| *end == eocd)
        .ok_or("the archive's central directory is malformed")?;

    let mut entries = Vec::with_capacity(count);
    let mut names = HashSet::new();
    let mut declared_total: u64 = 0;
    let mut at = directory_offset;
    for _ in 0..count {
        if at + 46 > directory_end || u32_at(bytes, at)? != CENTRAL {
            return Err("the archive's central directory is malformed".into());
        }
        let made_by = u16_at(bytes, at + 4)?;
        let flags = u16_at(bytes, at + 8)?;
        let method = u16_at(bytes, at + 10)?;
        let crc = u32_at(bytes, at + 16)?;
        let compressed = u32_at(bytes, at + 20)? as u64;
        let uncompressed = u32_at(bytes, at + 24)? as u64;
        let name_len = u16_at(bytes, at + 28)? as usize;
        let extra_len = u16_at(bytes, at + 30)? as usize;
        let comment_len = u16_at(bytes, at + 32)? as usize;
        let start_disk = u16_at(bytes, at + 34)?;
        let external = u32_at(bytes, at + 38)?;
        let offset = u32_at(bytes, at + 42)? as usize;
        let name_start = at + 46;
        let extra_start = name_start + name_len;
        let next = extra_start + extra_len + comment_len;
        if next > directory_end {
            return Err("the archive's central directory is malformed".into());
        }
        if flags & (FLAG_ENCRYPTED | FLAG_STRONG | FLAG_MASKED) != 0 || method == 99 {
            return Err("encrypted archive entries are not supported".into());
        }
        if start_disk != 0 {
            return Err("multi-disk archives are not supported".into());
        }
        if compressed == u32::MAX as u64 || uncompressed == u32::MAX as u64 {
            return Err("ZIP64 archives are not supported".into());
        }
        check_extra(&bytes[extra_start..extra_start + extra_len])?;
        let name = std::str::from_utf8(&bytes[name_start..extra_start])
            .map_err(|_| "the archive contains an unsafe entry name".to_string())?
            .to_owned();
        check_name(&name)?;
        let kind = classify(&name, nested_allowed)?;
        // Duplicates are compared case-insensitively, with any folder slash
        // removed, so `A` and `a/` cannot both appear.
        if !names.insert(name.trim_end_matches('/').to_lowercase()) {
            return Err("the archive contains duplicate entry names".into());
        }
        // Unix symbolic links, devices and FIFOs are rejected by mode; a DOS
        // folder attribute must match a folder name.
        if made_by >> 8 == 3 {
            let file_type = (external >> 16) & 0o170_000;
            let allowed = match kind {
                Kind::ImageFolder => matches!(file_type, 0 | 0o040_000),
                _ => matches!(file_type, 0 | 0o100_000),
            };
            if !allowed {
                return Err("the archive contains a link or special file".into());
            }
        }
        if external & 0x10 != 0 && kind != Kind::ImageFolder {
            return Err("the archive contains an unexpected folder".into());
        }
        if !matches!(method, STORED | DEFLATED) {
            return Err("the archive uses an unsupported compression method".into());
        }
        if method == STORED && compressed != uncompressed {
            return Err("the archive has inconsistent entry sizes".into());
        }
        if kind == Kind::ImageFolder && uncompressed != 0 {
            return Err("the archive has inconsistent entry sizes".into());
        }
        if uncompressed > RATIO_EXEMPT_BYTES
            && (compressed == 0 || uncompressed > compressed.saturating_mul(MAX_RATIO))
        {
            return Err(format!(
                "an archive entry is compressed more than {MAX_RATIO}:1 (possible decompression bomb)"
            ));
        }
        declared_total = declared_total.saturating_add(uncompressed);
        if declared_total > MAX_TOTAL_UNCOMPRESSED {
            return Err(format!(
                "the archive would expand to more than {} MiB",
                MAX_TOTAL_UNCOMPRESSED / 1024 / 1024
            ));
        }

        // The local header must agree with the central directory.
        if offset + 30 > directory_offset || u32_at(bytes, offset)? != LOCAL {
            return Err("an archive entry's local header is malformed".into());
        }
        let local_flags = u16_at(bytes, offset + 6)?;
        let local_method = u16_at(bytes, offset + 8)?;
        let local_name_len = u16_at(bytes, offset + 26)? as usize;
        let local_extra_len = u16_at(bytes, offset + 28)? as usize;
        let local_name_start = offset + 30;
        let data_start = local_name_start + local_name_len + local_extra_len;
        if local_flags & (FLAG_ENCRYPTED | FLAG_STRONG) != 0 {
            return Err("encrypted archive entries are not supported".into());
        }
        if local_method != method
            || bytes.get(local_name_start..local_name_start + local_name_len)
                != Some(name.as_bytes())
        {
            return Err("an archive entry's local header does not match the directory".into());
        }
        check_extra(
            bytes
                .get(local_name_start + local_name_len..data_start)
                .ok_or("the archive is truncated")?,
        )?;
        if local_flags & FLAG_DESCRIPTOR == 0
            && (u32_at(bytes, offset + 14)? != crc
                || u32_at(bytes, offset + 18)? as u64 != compressed
                || u32_at(bytes, offset + 22)? as u64 != uncompressed)
        {
            return Err("an archive entry's local header does not match the directory".into());
        }
        let data_end = data_start
            .checked_add(compressed as usize)
            .filter(|end| *end <= directory_offset)
            .ok_or("an archive entry extends past its data area")?;
        entries.push(Entry {
            name,
            kind,
            method,
            crc,
            compressed,
            uncompressed,
            offset,
            data_start,
            data_end,
        });
        at = next;
    }
    if at != directory_end {
        return Err("the archive's central directory is malformed".into());
    }
    // Overlapping entries (one compressed stream shared by many names) are a
    // classic bomb; every entry must own its byte range.
    let mut ranges: Vec<(usize, usize)> = entries.iter().map(|e| (e.offset, e.data_end)).collect();
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[1].0 < pair[0].1) {
        return Err("archive entries overlap".into());
    }

    let profiles: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Profile).collect();
    let nested: Vec<&Entry> = entries.iter().filter(|e| e.kind == Kind::Nested).collect();
    let skipped_images = entries.iter().filter(|e| e.kind == Kind::Image).count();
    let mut contents = Contents {
        profiles: Vec::new(),
        skipped_images,
    };
    match (profiles.len(), nested.len()) {
        (1, 0) => {
            let entry = profiles[0];
            let json = inflate(bytes, entry, MAX_PROFILE_JSON_BYTES, inflated)?;
            let entry_serial = entry.name.split_once('_').map(|(_, s)| s.to_owned());
            contents.profiles.push(ProfileData {
                json,
                entry_serial: entry_serial.unwrap_or_default(),
            });
        }
        (0, count) if count > 0 => {
            if count > MAX_NESTED {
                return Err(format!(
                    "the archive holds more than {MAX_NESTED} profile files"
                ));
            }
            if skipped_images > 0 || entries.iter().any(|e| e.kind == Kind::ImageFolder) {
                return Err("a multi-profile archive may contain only .mhz files".into());
            }
            for entry in nested {
                let inner = inflate(bytes, entry, MAX_ARCHIVE_BYTES, inflated)?;
                let found = read_level(&inner, false, inflated)?;
                contents.profiles.extend(found.profiles);
                contents.skipped_images += found.skipped_images;
            }
        }
        (0, 0) => return Err("the archive does not contain a MasterHUB profile".into()),
        (_, 0) => return Err("the archive contains more than one profile entry".into()),
        _ => return Err("the archive mixes a profile entry with nested profile files".into()),
    }
    Ok(contents)
}

/// Inflates one entry, counting actual bytes against `limit` and the shared
/// total, then checks the declared size and CRC-32.
fn inflate(
    bytes: &[u8],
    entry: &Entry,
    limit: usize,
    inflated: &mut u64,
) -> Result<Vec<u8>, String> {
    if entry.uncompressed > limit as u64 {
        return Err(format!(
            "an archive entry is larger than {} MiB",
            limit / 1024 / 1024
        ));
    }
    let data = &bytes[entry.data_start..entry.data_end];
    if entry.uncompressed > MAX_TOTAL_UNCOMPRESSED.saturating_sub(*inflated) {
        return Err(format!(
            "the archive would expand to more than {} MiB",
            MAX_TOTAL_UNCOMPRESSED / 1024 / 1024
        ));
    }
    // Inflating stops one byte past the declared size, so a header that
    // understates a bomb is caught after at most that many bytes.
    let budget = entry.uncompressed;
    let mut out = Vec::new();
    match entry.method {
        STORED => out.extend_from_slice(data),
        _ => {
            DeflateDecoder::new(data)
                .take(budget + 1)
                .read_to_end(&mut out)
                .map_err(|_| "an archive entry is corrupt".to_string())?;
        }
    }
    if out.len() as u64 > budget {
        return Err("an archive entry expands past its declared size".into());
    }
    if out.len() as u64 != entry.uncompressed {
        return Err("an archive entry's size does not match the directory".into());
    }
    if entry.uncompressed > RATIO_EXEMPT_BYTES
        && out.len() as u64 > entry.compressed.saturating_mul(MAX_RATIO)
    {
        return Err(format!(
            "an archive entry is compressed more than {MAX_RATIO}:1 (possible decompression bomb)"
        ));
    }
    if crc32fast::hash(&out) != entry.crc {
        return Err("an archive entry failed its CRC check".into());
    }
    *inflated += out.len() as u64;
    Ok(out)
}
