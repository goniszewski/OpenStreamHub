//! Import of official MasterHUB `.mhz` profile files.
//!
//! A `.mhz` file is untrusted input from a stranger. `convert` reads it in
//! memory (`archive`), parses the one profile JSON entry within bounds
//! (`json`) and maps it onto profile schema 2 (`mapping`). It never runs or
//! writes anything; `lib.rs` saves each result as a new profile that is
//! marked dry-run only (`import_flags`). The webview hands the bytes of the
//! file the user picked or dropped, exactly like the JSON import; no path is
//! ever passed, so the frontend cannot make the backend read another file.
//! See `docs/research/masterhub-mhz-format.md`.

pub mod archive;
pub mod json;
mod mapping;
#[cfg(test)]
pub(crate) mod tests;

use cooltek_sdk_service::Profile;
use sha2::{Digest, Sha256};

pub use mapping::ImportReport;

/// One profile found in the file, before it gets its final name.
#[derive(Debug)]
pub struct Converted {
    pub profile: Profile,
    pub report: ImportReport,
}

/// Reads and maps every profile in an untrusted `.mhz` (or an outer
/// multi-profile `.zip` of up to `archive::MAX_NESTED` `.mhz` files). Any
/// limit hit or malformed part rejects the whole file.
pub fn convert(bytes: &[u8]) -> Result<Vec<Converted>, String> {
    let contents = archive::read(bytes)?;
    let mut converted = Vec::new();
    for data in contents.profiles {
        let root = json::parse(&data.json)?;
        let (profile, mut report) = mapping::map(&root, &data.entry_serial)?;
        if contents.skipped_images > 0 {
            report.warnings.insert(
                0,
                format!(
                    "Image files in the archive skipped unread: {}.",
                    contents.skipped_images
                ),
            );
        }
        converted.push(Converted { profile, report });
    }
    Ok(converted)
}

/// Lowercase hex SHA-256 of the file, kept as import provenance.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `<base> (imported)`, then `<base> (imported 2)`, … : the first name not
/// already used (ignoring case and surrounding spaces), within 128 bytes.
/// The base is shortened, never the suffix.
pub fn unique_name(base: &str, existing: &[String]) -> Result<String, String> {
    let taken = |candidate: &str| {
        existing
            .iter()
            .any(|name| name.trim().to_lowercase() == candidate.trim().to_lowercase())
    };
    for count in 1..10_000 {
        let suffix = if count == 1 {
            " (imported)".to_owned()
        } else {
            format!(" (imported {count})")
        };
        let mut candidate = mapping::clean(base, 128 - suffix.len());
        if candidate.is_empty() {
            candidate = "Imported profile".into();
        }
        candidate.push_str(&suffix);
        if !taken(&candidate) {
            return Ok(candidate);
        }
    }
    Err("could not find an unused profile name".into())
}
