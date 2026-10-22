//! Dry-run provenance of imported profiles.
//!
//! Profiles use `deny_unknown_fields`, so the dry-run-only state cannot live
//! in the profile JSON. It lives in `<app data>/imported-profiles.json`, with
//! the same bounded, symlink-safe read and atomic 0600 write as the profile
//! store and `display-settings.json`. The backend is the only authority; the
//! frontend's badge and disabled Execute button are convenience only.
//!
//! Provenance is tracked two ways, so neither a new id nor a missing hint
//! loses it:
//!
//! - **Content hashes** (`content_hash`: SHA-256 of the canonical profile JSON
//!   with its name emptied). A copy, a re-imported JSON export, a save over
//!   another id, or a draft of a deleted profile has the same hash and stays
//!   dry-run only. Hashes are never dropped when a profile is deleted.
//! - **Saved ids and their lineage.** Every save whose source (the saved
//!   profile a draft came from) or target id is flagged flags the saved id
//!   and adds the new content's hash, so an edited derivative stays flagged.
//!   Records of deleted profiles are kept as tombstones.
//!
//! The first import also writes a sentinel (`profiles/.imported-profiles`),
//! so "imports existed but the list is gone or damaged" is an error that
//! refuses every Execute run, never an empty list. The in-app recovery is
//! "Mark all profiles as needing review", which flags every saved profile and
//! rewrites the list. Allowing a profile (`allow_imported_profile_execution`)
//! clears its id and its content hash after the backend recomputed the hash
//! (from the parsed file, never from the store's listing cache)
//! of the saved file and matched it to the one the confirmation showed. The
//! backend cannot prove that a person clicked: that is what the confirmation
//! dialog is for.

use crate::store;
use cooltek_sdk_service::Profile;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE_NAME: &str = "imported-profiles.json";
/// Written inside the profile store on the first import.
pub const SENTINEL: &str = ".imported-profiles";
const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Records kept at most (tombstones included); far above the 128 profiles
/// the store holds.
pub const MAX_RECORDS: usize = 1024;
/// Flagged content hashes kept at most (tombstones included).
pub const MAX_HASHES: usize = 4096;

/// Refusal of an Execute run of a flagged profile.
pub const DRY_RUN_ONLY: &str = "this profile was imported (from a MasterHUB .mhz file or an app preset pack) or derived from an import, and can run only as a dry run until you allow it to execute actions in the Profiles page";
const RECOVERY: &str = "actions stay off until you choose \"Mark all profiles as needing review\" in the Profiles page";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRecord {
    /// `mhz` (imported), `pack` (created from an app preset pack), `derived` (saved from a flagged profile) or
    /// `review` (marked by the recovery action).
    pub source: String,
    pub dry_run_only: bool,
    pub imported_at_ms: u64,
    /// SHA-256 of the imported file, empty unless `source` is `mhz`. No file
    /// name or path is kept.
    pub source_sha256: String,
    /// Content hash of the version last saved under this id, so saving a
    /// new version replaces it instead of piling up hashes.
    #[serde(default)]
    pub content_hash: String,
    /// The profile was deleted; the record is kept as a tombstone.
    #[serde(default)]
    pub deleted: bool,
}

/// One flagged content hash.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HashEntry {
    /// When it was last flagged (a counter); within a group, the oldest go
    /// first when the list is compacted.
    pub seq: u64,
    /// Exported to a file: never released when the profile is saved again,
    /// and evicted last by compaction (only when the caps leave no other
    /// choice, and never while a live record names it).
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedProfiles {
    pub schema_version: u32,
    /// Saved profile id to its record; records of deleted profiles stay.
    pub profiles: BTreeMap<String, ImportRecord>,
    /// Content hashes that may run only as a dry run.
    pub flagged_hashes: BTreeMap<String, HashEntry>,
    /// The next `HashEntry::seq`.
    #[serde(default)]
    pub next_seq: u64,
}

impl Default for ImportedProfiles {
    fn default() -> Self {
        Self {
            schema_version: 2,
            profiles: BTreeMap::new(),
            flagged_hashes: BTreeMap::new(),
            next_seq: 0,
        }
    }
}

impl ImportedProfiles {
    /// Ids that may run only as a dry run.
    pub fn dry_run_only(&self) -> BTreeSet<String> {
        self.profiles
            .iter()
            .filter(|(_, record)| record.dry_run_only)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Flags (or refreshes) one content hash.
    pub fn flag_hash(&mut self, hash: &str, pinned: bool) {
        let seq = self.next_seq;
        self.next_seq += 1;
        let entry = self.flagged_hashes.entry(hash.to_owned()).or_default();
        entry.seq = seq;
        entry.pinned |= pinned;
    }

    /// Flags `id` (keeping an import record's provenance) and the hash of
    /// its saved content, and returns the id's previous hash. That previous
    /// hash stays flagged: the caller releases it (`release`) only after the
    /// new version was really saved, so a failed save never unflags the
    /// version still on disk.
    pub fn flag(&mut self, id: &str, hash: &str, source: &str, at_ms: u64) -> String {
        let record = self
            .profiles
            .entry(id.to_owned())
            .or_insert_with(|| ImportRecord {
                source: source.into(),
                dry_run_only: true,
                imported_at_ms: at_ms,
                source_sha256: String::new(),
                content_hash: String::new(),
                deleted: false,
            });
        record.dry_run_only = true;
        record.deleted = false;
        let previous = std::mem::replace(&mut record.content_hash, hash.to_owned());
        if !hash.is_empty() {
            self.flag_hash(hash, false);
        }
        previous
    }

    /// Drops a superseded hash unless an export pinned it, any record (a
    /// twin, or a deleted profile's tombstone) still names it, or any saved
    /// profile (`saved`: the content hashes of every saved profile) still
    /// has that content, with or without a record.
    pub fn release(&mut self, hash: &str, saved: &BTreeSet<String>) {
        let pinned = self
            .flagged_hashes
            .get(hash)
            .is_some_and(|entry| entry.pinned);
        if !hash.is_empty()
            && !pinned
            && !saved.contains(hash)
            && !self
                .profiles
                .values()
                .any(|record| record.content_hash == hash)
        {
            self.flagged_hashes.remove(hash);
        }
    }

    /// Keeps the list within `MAX_RECORDS` and `MAX_HASHES` without ever
    /// unflagging a saved profile. Only tombstones (records of deleted
    /// profiles) are evicted, oldest first. A hash is never evicted while a
    /// live record names it or a saved profile (`saved`) has that content;
    /// of the others, unpinned ones no record names go first, then hashes
    /// named only by tombstones, and pinned export hashes last, oldest
    /// first within each group. When the caps still cannot be met, the
    /// write is refused.
    pub fn compact(&mut self, saved: &BTreeSet<String>) -> Result<(), String> {
        if self.profiles.len() > MAX_RECORDS {
            let mut tombstones: Vec<(u64, String)> = self
                .profiles
                .iter()
                .filter(|(_, record)| record.deleted)
                .map(|(id, record)| (record.imported_at_ms, id.clone()))
                .collect();
            tombstones.sort_unstable();
            let excess = self.profiles.len() - MAX_RECORDS;
            for (_, id) in tombstones.into_iter().take(excess) {
                self.profiles.remove(&id);
            }
        }
        if self.flagged_hashes.len() > MAX_HASHES {
            let live: BTreeSet<&str> = self
                .profiles
                .values()
                .filter(|record| !record.deleted)
                .map(|record| record.content_hash.as_str())
                .collect();
            let tombstoned: BTreeSet<&str> = self
                .profiles
                .values()
                .filter(|record| record.deleted)
                .map(|record| record.content_hash.as_str())
                .collect();
            let mut candidates: Vec<((bool, bool, u64), String)> = self
                .flagged_hashes
                .iter()
                .filter(|(hash, _)| !live.contains(hash.as_str()) && !saved.contains(hash.as_str()))
                .map(|(hash, entry)| {
                    (
                        (entry.pinned, tombstoned.contains(hash.as_str()), entry.seq),
                        hash.clone(),
                    )
                })
                .collect();
            candidates.sort_unstable();
            let excess = self.flagged_hashes.len() - MAX_HASHES;
            for (_, hash) in candidates.into_iter().take(excess) {
                self.flagged_hashes.remove(&hash);
            }
        }
        if self.profiles.len() > MAX_RECORDS || self.flagged_hashes.len() > MAX_HASHES {
            return Err(format!(
                "the imported profile list is full ({} saved profiles come from imports); delete imported profiles you no longer need and try again",
                self.profiles
                    .values()
                    .filter(|record| !record.deleted)
                    .count()
            ));
        }
        Ok(())
    }

    pub fn guard(&self) -> ExecutionGuard {
        ExecutionGuard {
            ids: self.dry_run_only(),
            hashes: self.flagged_hashes.keys().cloned().collect(),
            error: None,
        }
    }
}

/// SHA-256 (hex) of the canonical schema-2 profile JSON with an empty name.
pub fn content_hash(profile: &Profile) -> String {
    let mut canonical = profile
        .clone()
        .into_current()
        .unwrap_or_else(|_| profile.clone());
    canonical.name = String::new();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The flags, loaded once and checked in memory (a Start, a resume, a
/// switch or a whole profile-key step).
#[derive(Clone, Debug, Default)]
pub struct ExecutionGuard {
    ids: BTreeSet<String>,
    hashes: BTreeSet<String>,
    error: Option<String>,
}

impl ExecutionGuard {
    #[cfg(test)]
    pub fn with_ids(ids: BTreeSet<String>, hashes: BTreeSet<String>) -> Self {
        Self {
            ids,
            hashes,
            ..Self::default()
        }
    }

    /// Counts every saved profile whose stored content hash is flagged as a
    /// flagged id, record or not.
    pub fn flag_ids_by_content<'a>(&mut self, saved: impl IntoIterator<Item = (&'a str, &'a str)>) {
        for (id, hash) in saved {
            if self.hashes.contains(hash) {
                self.ids.insert(id.to_owned());
            }
        }
    }

    /// A guard that refuses every Execute run.
    pub fn failed(error: String) -> Self {
        Self {
            error: Some(error),
            ..Self::default()
        }
    }

    /// Why `profile` must not run with actions, or `None`. `profile_id` is
    /// the saved profile it is identical to, `source` the saved profile it
    /// was opened from (possibly edited since).
    pub fn blocks(
        &self,
        profile_id: Option<&str>,
        source: Option<&str>,
        profile: &Profile,
    ) -> Option<String> {
        let hash = if self.hashes.is_empty() {
            String::new()
        } else {
            content_hash(profile)
        };
        self.blocks_hash(profile_id, source, &hash)
    }

    /// `blocks` with the content hash already known (a stored profile's
    /// `content_hash`), so a profile-key step hashes nothing again.
    pub fn blocks_hash(
        &self,
        profile_id: Option<&str>,
        source: Option<&str>,
        hash: &str,
    ) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("actions cannot run: {error}"));
        }
        let flagged = [profile_id, source]
            .into_iter()
            .flatten()
            .any(|id| self.ids.contains(id))
            || self.hashes.contains(hash);
        flagged.then(|| DRY_RUN_ONLY.to_owned())
    }
}

/// `imported-profiles.json`; the lock serializes read-modify-write.
pub struct ImportFlags {
    directory: Result<PathBuf, String>,
    lock: Mutex<()>,
}

impl ImportFlags {
    pub fn open(app_data: Result<&Path, String>) -> Self {
        Self {
            directory: app_data.map(Path::to_path_buf),
            lock: Mutex::new(()),
        }
    }

    fn sentinel(directory: &Path) -> PathBuf {
        directory.join("profiles").join(SENTINEL)
    }

    /// The lines of the sentinel: `hash <hex>` and `id <uuid>` (the
    /// redundant copy recovery reads). Malformed lines are ignored; a
    /// sentinel that exists but cannot be read (a link, too large) is an
    /// error, never an empty copy.
    fn sentinel_lines(directory: &Path) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
        let text = store::read_private_file(
            &Self::sentinel(directory),
            MAX_FILE_BYTES,
            "imported profile marker",
        )
        .map_err(|error| {
            format!(
                "the imported profile marker (profiles/{SENTINEL}) cannot be read: {error}. To recover by hand: quit OpenStreamHub, replace profiles/{SENTINEL} in the app data folder with an empty file, start the app again and choose \"Mark all profiles as needing review\" (profiles deleted earlier are then no longer tracked)"
            )
        })?
        .map(|(bytes, _)| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
        let mut hashes = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for line in text.lines() {
            if let Some(hash) = line.strip_prefix("hash ")
                && hash.len() == 64
                && hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                hashes.insert(hash.to_owned());
            } else if let Some(id) = line.strip_prefix("id ")
                && store::validate_id(id).is_ok()
                && ids.len() < MAX_RECORDS
            {
                ids.insert(id.to_owned());
            }
        }
        Ok((hashes, ids))
    }

    /// For the recovery action: the list as far as it can be read (an empty
    /// one when it cannot), plus every hash and dry-run-only id the sentinel
    /// still holds. Ids it adds become dry-run-only tombstones, so drafts
    /// whose source was a deleted imported profile stay blocked.
    pub fn load_for_recovery(&self) -> Result<ImportedProfiles, String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "imported profile list lock is unavailable".to_string())?;
        let mut records = Self::read(directory).unwrap_or_default();
        let (hashes, ids) = Self::sentinel_lines(directory)?;
        for hash in hashes {
            if !records.flagged_hashes.contains_key(&hash) {
                records.flag_hash(&hash, true);
            }
        }
        for id in ids {
            records.profiles.entry(id).or_insert_with(|| ImportRecord {
                source: "review".into(),
                dry_run_only: true,
                imported_at_ms: 0,
                source_sha256: String::new(),
                content_hash: String::new(),
                deleted: true,
            });
        }
        Ok(records)
    }

    /// A profile was deleted while the list could not be read: its id is
    /// added to the sentinel, which recovery turns into a dry-run-only
    /// tombstone (every profile is reviewed after recovery anyway, so this
    /// errs on the safe side for ids that were never imported).
    pub fn remember_deleted(&self, id: &str) -> Result<(), String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "imported profile list lock is unavailable".to_string())?;
        let path = Self::sentinel(directory);
        let Some((bytes, _)) =
            store::read_private_file(&path, MAX_FILE_BYTES, "imported profile marker")?
        else {
            return Err(format!(
                "the imported profile marker (profiles/{SENTINEL}) is missing"
            ));
        };
        if Self::sentinel_lines(directory)?.1.len() >= MAX_RECORDS {
            return Err(format!(
                "the imported profile marker already remembers {MAX_RECORDS} deleted profiles"
            ));
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        text.push_str("id ");
        text.push_str(id);
        text.push('\n');
        store::write_private_file(
            &directory.join("profiles"),
            &path,
            text.as_bytes(),
            "imported profile marker",
        )
    }

    fn read(directory: &Path) -> Result<ImportedProfiles, String> {
        let path = directory.join(FILE_NAME);
        let Some((bytes, _)) =
            store::read_private_file(&path, MAX_FILE_BYTES, "imported profile list")?
        else {
            return match std::fs::symlink_metadata(Self::sentinel(directory)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Ok(ImportedProfiles::default())
                }
                _ => Err(format!(
                    "the imported profile list is missing although profiles were imported; {RECOVERY}"
                )),
            };
        };
        let records: ImportedProfiles = serde_json::from_slice(&bytes).map_err(|error| {
            format!("the imported profile list is unreadable ({error}); {RECOVERY}")
        })?;
        if records.schema_version != 2
            || records.profiles.len() > MAX_RECORDS
            || records.flagged_hashes.len() > MAX_HASHES
        {
            return Err(format!(
                "the imported profile list is unsupported; {RECOVERY}"
            ));
        }
        Ok(records)
    }

    pub fn load(&self) -> Result<ImportedProfiles, String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "imported profile list lock is unavailable".to_string())?;
        Self::read(directory)
    }

    /// The guard for one check; an unreadable list refuses every Execute run.
    pub fn guard(&self) -> ExecutionGuard {
        self.load()
            .map(|records| records.guard())
            .unwrap_or_else(ExecutionGuard::failed)
    }

    /// Applies `change` and writes the result. A damaged list is reported
    /// and never overwritten here (see `replace`).
    /// `saved` is the set of content hashes of every saved profile, which
    /// compaction never evicts (see `ImportedProfiles::compact`).
    pub fn update(
        &self,
        saved: &BTreeSet<String>,
        change: impl FnOnce(&mut ImportedProfiles) -> Result<(), String>,
    ) -> Result<ImportedProfiles, String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "imported profile list lock is unavailable".to_string())?;
        let mut records = Self::read(directory)?;
        change(&mut records)?;
        records.compact(saved)?;
        Self::write(directory, &records)?;
        Ok(records)
    }

    /// The recovery path: writes `records` whatever the stored list holds.
    pub fn replace(
        &self,
        saved: &BTreeSet<String>,
        records: &ImportedProfiles,
    ) -> Result<(), String> {
        let directory = self.directory.as_ref().map_err(Clone::clone)?;
        let _guard = self
            .lock
            .lock()
            .map_err(|_| "imported profile list lock is unavailable".to_string())?;
        let mut records = records.clone();
        records.compact(saved)?;
        Self::write(directory, &records)
    }

    fn write(directory: &Path, records: &ImportedProfiles) -> Result<(), String> {
        let profiles = directory.join("profiles");
        for (folder, what) in [
            (directory, "application data folder"),
            (profiles.as_path(), "profile storage"),
        ] {
            match std::fs::symlink_metadata(folder) {
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(format!(
                        "the {what} must be a directory, not a symbolic link"
                    ));
                }
                Err(error) => return Err(format!("could not inspect the {what}: {error}")),
            }
        }
        // The sentinel first: from now on a missing list is an error. It
        // also keeps a redundant copy of the flagged hashes (tombstones
        // included), which recovery reads when the list is damaged.
        let mut sentinel = String::from("imported profiles exist; see imported-profiles.json\n");
        for hash in records.flagged_hashes.keys() {
            sentinel.push_str("hash ");
            sentinel.push_str(hash);
            sentinel.push('\n');
        }
        for (id, _) in records.profiles.iter().filter(|(_, r)| r.dry_run_only) {
            sentinel.push_str("id ");
            sentinel.push_str(id);
            sentinel.push('\n');
        }
        store::write_private_file(
            &profiles,
            &Self::sentinel(directory),
            sentinel.as_bytes(),
            "imported profile marker",
        )?;
        let bytes = serde_json::to_vec_pretty(records)
            .map_err(|error| format!("could not encode the imported profile list: {error}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("the imported profile list exceeds its size limit".into());
        }
        store::write_private_file(
            directory,
            &directory.join(FILE_NAME),
            &bytes,
            "imported profile list",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags() -> (tempfile::TempDir, ImportFlags) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("profiles")).unwrap();
        let flags = ImportFlags::open(Ok(directory.path()));
        (directory, flags)
    }

    fn profile(name: &str) -> Profile {
        let mut profile = crate::test_profiles::folders();
        profile.name = name.into();
        profile
    }

    #[test]
    fn records_round_trip_and_a_damaged_file_is_never_overwritten() {
        let (directory, flags) = flags();
        assert_eq!(flags.load().unwrap(), ImportedProfiles::default());
        flags
            .update(&BTreeSet::new(), |records| {
                records.flag("a", &"a".repeat(64), "mhz", 1);
                Ok(())
            })
            .unwrap();
        assert_eq!(
            flags.load().unwrap().dry_run_only(),
            BTreeSet::from(["a".to_string()])
        );
        let path = directory.path().join(FILE_NAME);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, b"{\"schema_version\":2,\"x\":1}").unwrap();
        let error = flags.load().unwrap_err();
        assert!(
            error.contains("Mark all profiles as needing review"),
            "{error}"
        );
        assert!(!error.contains("remove"), "{error}");
        assert!(flags.update(&BTreeSet::new(), |_| Ok(())).is_err());
        assert!(flags.guard().blocks(None, None, &profile("x")).is_some());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"schema_version\":2,\"x\":1}"
        );
        // The recovery path rewrites it.
        flags
            .replace(&BTreeSet::new(), &ImportedProfiles::default())
            .unwrap();
        assert!(flags.load().is_ok());
    }

    #[test]
    fn a_missing_list_after_an_import_fails_closed() {
        let (directory, flags) = flags();
        assert!(flags.guard().blocks(None, None, &profile("x")).is_none());
        flags.update(&BTreeSet::new(), |_| Ok(())).unwrap();
        assert!(directory.path().join("profiles").join(SENTINEL).is_file());
        std::fs::remove_file(directory.path().join(FILE_NAME)).unwrap();
        let error = flags.load().unwrap_err();
        assert!(error.contains("missing"), "{error}");
        let blocked = flags.guard().blocks(None, None, &profile("x")).unwrap();
        assert!(blocked.starts_with("actions cannot run"), "{blocked}");
    }

    #[test]
    fn content_hashes_ignore_the_name_and_match_ids_and_sources() {
        let a = profile("A");
        let renamed = profile("Renamed");
        assert_eq!(content_hash(&a), content_hash(&renamed));
        let mut edited = a.clone();
        edited.bindings.clear();
        assert_ne!(content_hash(&a), content_hash(&edited));
        let mut records = ImportedProfiles::default();
        records.flag("id", &content_hash(&a), "mhz", 0);
        let guard = records.guard();
        assert!(guard.blocks(None, None, &renamed).is_some());
        assert!(guard.blocks(None, None, &edited).is_none());
        assert!(guard.blocks(None, Some("id"), &edited).is_some());
        assert!(guard.blocks(Some("id"), None, &edited).is_some());
        assert!(
            guard
                .blocks(Some("other"), Some("other"), &edited)
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_is_not_followed() {
        let (directory, _) = flags();
        let outside = directory.path().join("outside.json");
        std::fs::write(&outside, b"{}").unwrap();
        std::os::unix::fs::symlink(&outside, directory.path().join(FILE_NAME)).unwrap();
        let flags = ImportFlags::open(Ok(directory.path()));
        assert!(flags.load().is_err());
        assert!(flags.update(&BTreeSet::new(), |_| Ok(())).is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"{}");
    }
}
