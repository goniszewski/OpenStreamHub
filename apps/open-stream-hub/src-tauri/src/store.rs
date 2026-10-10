use cooltek_sdk_service::Profile;
use serde::Serialize;
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

pub(crate) const MAX_PROFILES: usize = 128;
/// Versions kept per profile for the editor's "Earlier versions" list.
pub(crate) const HISTORY_LIMIT: usize = 20;
/// Automatic saving writes on every settled edit, so a version is kept only
/// when the previous kept one is at least this much older: the 20 versions
/// then reach back hours, not the last minute of edits. A profile's first
/// overwrite in a run of the app and a restore always keep the version they
/// replace, whatever the gap.
pub(crate) const HISTORY_MIN_GAP_MS: u64 = 5 * 60 * 1000;
/// Total size of one profile's kept versions. The oldest go first; the
/// newest version is never removed, so the last copy of a profile's previous
/// state survives even when it alone is over the cap.
pub(crate) const HISTORY_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Hidden temporary files and staging folders older than this are leftovers
/// of an interrupted write, not work in progress in another app window.
pub(crate) const STALE_TEMPORARY: Duration = Duration::from_secs(60 * 60);
const SAMPLE: &str = include_str!("../../../../examples/profiles/macos.json");
pub(crate) const CONFLICT_ERROR: &str = "this profile changed on disk after it was loaded, possibly in another OpenStreamHub window; reload the saved profile before saving again";

#[derive(Clone, Debug, Serialize)]
pub struct StoredProfile {
    pub id: String,
    pub profile: Profile,
    pub modified_at_ms: u64,
    /// `import_flags::content_hash` of `profile`: the version a confirmation
    /// showed, sent back when the user allows an imported profile.
    pub content_hash: String,
}

/// One kept earlier version of a profile, named by its own modification
/// time (the moment that version was saved). The name and binding count
/// let the list say what changed without loading every version; they are
/// absent for a version file that cannot be read as a profile.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProfileVersion {
    pub modified_at_ms: u64,
    pub name: Option<String>,
    pub binding_count: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileSummary {
    pub id: String,
    pub name: String,
    pub binding_count: usize,
    pub modified_at_ms: u64,
    /// An imported profile that may run only as a dry run until the user
    /// allows it to execute actions (`import_flags`). Filled in by `lib.rs`.
    pub dry_run_only: bool,
    /// Used to mark copies of flagged content; not sent to the frontend.
    #[serde(skip)]
    pub content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnreadableProfile {
    pub id: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProfileListing {
    pub profiles: Vec<ProfileSummary>,
    pub unreadable: Vec<UnreadableProfile>,
    /// Why the imported profile list cannot be read (every profile is then
    /// shown as dry run only); filled in by `lib.rs`.
    pub import_flags_error: Option<String>,
    /// More profile files exist than `MAX_PROFILES`; some are not listed.
    #[serde(skip)]
    pub truncated: bool,
}

#[derive(Debug, Serialize)]
pub struct AppBootstrap {
    /// The most recently modified readable profile, if any. `None` means the
    /// store holds only unreadable data, which is preserved and never reseeded.
    pub profile: Option<StoredProfile>,
    pub profiles: Vec<ProfileSummary>,
    pub unreadable: Vec<UnreadableProfile>,
    /// As in `ProfileListing`.
    pub import_flags_error: Option<String>,
    pub storage_path: String,
    pub desktop: bool,
}

/// Hidden entries (`.DS_Store`, AppleDouble `._*` files, interrupted
/// `.<uuid>.tmp` writes) are never profiles. They are left untouched.
pub(crate) fn is_hidden_entry(name: &OsStr) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

/// True when `metadata` was last modified more than `STALE_TEMPORARY` ago.
pub(crate) fn is_stale(metadata: &fs::Metadata) -> bool {
    metadata.modified().is_ok_and(|modified| {
        SystemTime::now()
            .duration_since(modified)
            .is_ok_and(|age| age > STALE_TEMPORARY)
    })
}

/// True for the exact `.<uuid>.tmp` name `write_private_file` uses: the
/// hyphenated 36-character form only, never the simple, braced or urn forms.
fn is_uuid_temporary(name: &OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.strip_prefix('.'))
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|middle| middle.len() == 36 && Uuid::try_parse(middle).is_ok())
}

/// Deletes stale hidden `.tmp` regular files left by interrupted saves.
/// `uuid_names_only` restricts it to the exact `.<uuid>.tmp` shape, for
/// folders shared with other files (the app data root); the profiles folder
/// is owned by the store, so any hidden `.*.tmp` there goes. Symbolic links,
/// folders and recent files are left alone.
pub(crate) fn remove_stale_temporaries(directory: &Path, uuid_names_only: bool) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let matches = if uuid_names_only {
            is_uuid_temporary(&name)
        } else {
            is_hidden_entry(&name) && name.as_encoded_bytes().ends_with(b".tmp")
        };
        if !matches {
            continue;
        }
        // DirEntry metadata does not follow symbolic links.
        if entry
            .metadata()
            .is_ok_and(|metadata| metadata.is_file() && is_stale(&metadata))
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Test-only faults: a profile write that fails before anything is written,
/// and one that fails after the rename (a failing folder flush).
#[cfg(test)]
pub(crate) mod fault {
    use std::cell::Cell;
    thread_local! {
        pub static WRITE: Cell<bool> = const { Cell::new(false) };
        pub static SYNC: Cell<bool> = const { Cell::new(false) };
        /// Profile files parsed (an operation count for performance tests).
        pub static PARSES: Cell<usize> = const { Cell::new(0) };
    }
}

pub struct ProfileStore {
    directory: PathBuf,
    /// Legacy files that were left behind by migration on this launch; they
    /// are reported with the unreadable profiles.
    migration_skipped: Vec<UnreadableProfile>,
    /// Summaries (with content hashes) of files already parsed, keyed by
    /// id and stamped with the file's modification time (ns) and length. A
    /// listing re-parses only files whose stamp changed, so frequent checks
    /// (flag writes, profile keys, Start) do not re-read every profile.
    summaries: std::sync::Mutex<std::collections::HashMap<String, (Stamp, ProfileSummary)>>,
    /// Profiles whose first overwrite in this run already kept a version.
    kept_this_run: std::sync::Mutex<std::collections::HashSet<String>>,
    /// See `HISTORY_MIN_GAP_MS`; a field so tests can shorten it.
    history_gap_ms: u64,
    /// See `HISTORY_MAX_BYTES`; a field so tests can shrink it.
    history_max_bytes: u64,
}

/// A file's modification time (ns) and length.
type Stamp = (u128, u64);

fn stamp(metadata: &fs::Metadata) -> Option<Stamp> {
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((modified.as_nanos(), metadata.len()))
}

impl ProfileStore {
    pub fn open_for_app_data(app_data: PathBuf) -> Result<Self, String> {
        let (directory, migration_skipped) = crate::migration::prepare_store(&app_data)?;
        let mut store = Self::open(directory)?;
        store.migration_skipped = migration_skipped;
        Ok(store)
    }

    pub fn open(directory: PathBuf) -> Result<Self, String> {
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err("profile storage must be a directory, not a symbolic link".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&directory)
                    .map_err(|error| format!("could not create profile storage: {error}"))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                        .map_err(|error| format!("could not protect profile storage: {error}"))?;
                }
            }
            Err(error) => return Err(format!("could not inspect profile storage: {error}")),
        }
        remove_stale_temporaries(&directory, false);
        Ok(Self {
            directory,
            migration_skipped: Vec::new(),
            summaries: Default::default(),
            kept_this_run: Default::default(),
            history_gap_ms: HISTORY_MIN_GAP_MS,
            history_max_bytes: HISTORY_MAX_BYTES,
        })
    }

    pub fn bootstrap(&self) -> Result<AppBootstrap, String> {
        // Seeding depends only on what the store itself holds.
        let stored = self.list_stored()?;
        if stored.profiles.is_empty() && stored.unreadable.is_empty() {
            // Seed only an empty store, never beside unfamiliar or unreadable
            // data, which is retained for recovery. Hidden entries such as
            // .DS_Store are ignored.
            let mut entries = fs::read_dir(&self.directory)
                .map_err(|error| format!("could not inspect profile storage: {error}"))?;
            let has_data = entries.try_fold(false, |found, entry| {
                entry
                    .map(|entry| found || !is_hidden_entry(&entry.file_name()))
                    .map_err(|error| format!("could not inspect profile storage: {error}"))
            })?;
            if has_data {
                return Err("profile storage contains data but no usable profiles; preserve and inspect the files before recovery".into());
            }
            let mut profile = Profile::from_json(SAMPLE.as_bytes())?;
            profile.name = "Everyday".into();
            self.save(None, profile, None)?;
        }
        let listing = self.list()?;
        let profile = match listing.profiles.first() {
            Some(selected) => Some(self.load(&selected.id)?),
            None => None,
        };
        Ok(AppBootstrap {
            profile,
            profiles: listing.profiles,
            unreadable: listing.unreadable,
            import_flags_error: None,
            storage_path: self.directory.to_string_lossy().into_owned(),
            desktop: true,
        })
    }

    /// Lists readable profiles and reports each unreadable `.json` entry
    /// separately, so one damaged file never hides the others. Legacy files
    /// skipped by migration on this launch are reported as unreadable too.
    pub fn list(&self) -> Result<ProfileListing, String> {
        let mut listing = self.list_stored()?;
        listing
            .unreadable
            .extend(self.migration_skipped.iter().cloned());
        Ok(listing)
    }

    /// The store's own entries. Above `MAX_PROFILES` entries only the first
    /// `MAX_PROFILES` by filename are listed; creating more is refused.
    fn list_stored(&self) -> Result<ProfileListing, String> {
        let entries = fs::read_dir(&self.directory)
            .map_err(|error| format!("could not list profiles: {error}"))?;
        let mut candidates = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("could not read profile entry: {error}"))?;
            let path = entry.path();
            if is_hidden_entry(&entry.file_name())
                || path.extension().and_then(|extension| extension.to_str()) != Some("json")
            {
                continue;
            }
            candidates.push((entry.file_name(), path));
        }
        candidates.sort();
        let truncated = candidates.len() > MAX_PROFILES;
        candidates.truncate(MAX_PROFILES);
        // The cache holds only what this listing can show (at most
        // `MAX_PROFILES` entries).
        if let Ok(mut summaries) = self.summaries.lock() {
            let listed: std::collections::HashSet<String> = candidates
                .iter()
                .filter_map(|(_, path)| path.file_stem()?.to_str().map(str::to_owned))
                .collect();
            summaries.retain(|id, _| listed.contains(id));
        }
        let mut profiles = Vec::new();
        let mut unreadable = Vec::new();
        for (name, path) in candidates {
            let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                unreadable.push(UnreadableProfile {
                    id: name.to_string_lossy().into_owned(),
                    error: "profile filename is not valid UTF-8".into(),
                });
                continue;
            };
            let cached = fs::symlink_metadata(&path)
                .ok()
                .filter(|metadata| metadata.file_type().is_file())
                .and_then(|metadata| stamp(&metadata))
                .and_then(|stamp| self.cached(id, stamp));
            if let Some(summary) = cached {
                profiles.push(summary);
                continue;
            }
            match self.load(id) {
                Ok(stored) => {
                    let binding_count = stored.profile.binding_count();
                    profiles.push(ProfileSummary {
                        id: stored.id,
                        name: stored.profile.name,
                        binding_count,
                        modified_at_ms: stored.modified_at_ms,
                        dry_run_only: false,
                        content_hash: stored.content_hash,
                    });
                }
                Err(error) => unreadable.push(UnreadableProfile {
                    id: id.to_owned(),
                    error,
                }),
            }
        }
        profiles.sort_by(|left, right| {
            right
                .modified_at_ms
                .cmp(&left.modified_at_ms)
                .then_with(|| left.id.cmp(&right.id))
        });
        unreadable.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(ProfileListing {
            profiles,
            unreadable,
            import_flags_error: None,
            truncated,
        })
    }

    /// Ids in the summary cache (tests).
    #[cfg(test)]
    pub fn cached_ids(&self) -> Vec<String> {
        self.summaries
            .lock()
            .map(|summaries| summaries.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn cached(&self, id: &str, stamp: Stamp) -> Option<ProfileSummary> {
        let summaries = self.summaries.lock().ok()?;
        summaries
            .get(id)
            .filter(|(cached, _)| *cached == stamp)
            .map(|(_, summary)| summary.clone())
    }

    pub fn load(&self, id: &str) -> Result<StoredProfile, String> {
        let path = self.path(id)?;
        let (bytes, metadata) = read_private_file(&path, Profile::MAX_JSON_BYTES, "profile")?
            .ok_or("profile no longer exists in storage")?;
        // Legacy profiles are upgraded for the editor only. Reading must not
        // change file bytes, modification time, filename, or opaque identity.
        #[cfg(test)]
        fault::PARSES.with(|count| count.set(count.get() + 1));
        let profile = Profile::from_json(&bytes)?.into_v2()?;
        let modified_at_ms = metadata
            .modified()
            .map_err(|error| format!("could not read profile modification time: {error}"))?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "profile modification time predates the Unix epoch".to_string())?
            .as_millis()
            .try_into()
            .map_err(|_| "profile modification time is out of range".to_string())?;
        // Always computed from what was just parsed: a loaded profile's
        // hash is security relevant (allowing an import compares it), so it
        // never comes from the listing cache, which trusts file stamps.
        let file_stamp = stamp(&metadata);
        let content_hash = crate::import_flags::content_hash(&profile);
        if let (Some(file_stamp), Ok(mut summaries)) = (file_stamp, self.summaries.lock()) {
            summaries.insert(
                id.to_owned(),
                (
                    file_stamp,
                    ProfileSummary {
                        id: id.to_owned(),
                        name: profile.name.clone(),
                        binding_count: profile.binding_count(),
                        modified_at_ms,
                        dry_run_only: false,
                        content_hash: content_hash.clone(),
                    },
                ),
            );
        }
        Ok(StoredProfile {
            id: id.to_owned(),
            content_hash,
            profile,
            modified_at_ms,
        })
    }

    /// Saves a profile. For an existing ID, `expected_modified_at_ms` is the
    /// modification time the editor loaded; a mismatch means another window or
    /// process changed the file, and the save is rejected instead of
    /// silently overwriting it.
    pub fn save(
        &self,
        id: Option<&str>,
        profile: Profile,
        expected_modified_at_ms: Option<u64>,
    ) -> Result<StoredProfile, String> {
        self.save_with(id, profile, expected_modified_at_ms, false)
    }

    /// `save`, where `keep_version` makes the overwrite keep the version it
    /// replaces whatever the spacing rule says (a restore of an earlier
    /// version asks for it, so the restore is always reversible).
    pub fn save_with(
        &self,
        id: Option<&str>,
        profile: Profile,
        expected_modified_at_ms: Option<u64>,
        keep_version: bool,
    ) -> Result<StoredProfile, String> {
        // Explicit Save is the only operation that persists a schema upgrade.
        let profile = profile.into_v2()?;
        let id = match id {
            Some(id) => {
                // Never overwrite an unreadable existing profile during editing.
                let current = self.load(id)?;
                if expected_modified_at_ms
                    .is_some_and(|expected| expected != current.modified_at_ms)
                {
                    return Err(CONFLICT_ERROR.into());
                }
                id.to_owned()
            }
            None => {
                self.check_capacity()?;
                Uuid::new_v4().to_string()
            }
        };
        self.write(&id, profile, keep_version)
    }

    /// Creates a profile under an id chosen by the caller (an import records
    /// its dry-run flag for that id first). Never overwrites: an existing
    /// file or link under that id is refused.
    pub fn create(&self, id: &str, profile: Profile) -> Result<StoredProfile, String> {
        let profile = profile.into_v2()?;
        let path = self.path(id)?;
        if fs::symlink_metadata(&path).is_ok() {
            return Err("a profile with this id already exists".into());
        }
        self.check_capacity()?;
        self.write(id, profile, false)
    }

    fn check_capacity(&self) -> Result<(), String> {
        let listing = self.list_stored()?;
        if listing.profiles.len() + listing.unreadable.len() >= MAX_PROFILES {
            return Err(format!(
                "profile storage permits at most {MAX_PROFILES} profiles; delete a profile before creating another"
            ));
        }
        Ok(())
    }

    fn write(
        &self,
        id: &str,
        profile: Profile,
        keep_version: bool,
    ) -> Result<StoredProfile, String> {
        let path = self.path(id)?;
        let bytes = serde_json::to_vec_pretty(&profile)
            .map_err(|error| format!("could not encode profile: {error}"))?;
        if bytes.len() > Profile::MAX_JSON_BYTES {
            return Err("encoded profile exceeds the size limit".into());
        }
        // An identical rewrite (an automatic save of an unchanged profile)
        // keeps the file, its modification time and the version history as
        // they are.
        if read_private_file(&path, Profile::MAX_JSON_BYTES, "profile")
            .is_ok_and(|current| current.is_some_and(|(current, _)| current == bytes))
        {
            return self.load(id);
        }
        self.snapshot_history(id, keep_version);
        #[cfg(test)]
        if fault::WRITE.with(std::cell::Cell::get) {
            return Err("could not write profile (test fault)".into());
        }
        write_private_file(&self.directory, &path, &bytes, "profile")?;
        // The rename happened; a failing folder flush still reports an error.
        #[cfg(test)]
        if fault::SYNC.with(std::cell::Cell::get) {
            return Err("could not flush profile storage (test fault)".into());
        }
        self.load(id)
    }

    /// The history folder for one profile: `<storage>/.history/<id>`. The
    /// dot keeps the folder invisible to listings and seeding, which ignore
    /// hidden entries.
    fn history_directory(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.directory.join(".history").join(id))
    }

    /// Copies the profile's current file into its history before an
    /// overwrite. Best effort: a failed snapshot never fails the save, and
    /// the version list simply misses that one step. The copy keeps the
    /// original file's modification time, so the version is named by the
    /// moment it was saved.
    fn snapshot_history(&self, id: &str, keep_version: bool) {
        let Ok(path) = self.path(id) else {
            return;
        };
        let Ok(directory) = self.history_directory(id) else {
            return;
        };
        let Ok(Some((bytes, metadata))) =
            read_private_file(&path, Profile::MAX_JSON_BYTES, "profile")
        else {
            return;
        };
        let modified_at_ms = match metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        {
            Some(age) => age.as_millis() as u64,
            None => return,
        };
        // The first overwrite of a profile in this run, and a restore, always
        // keep; any other must be `history_gap_ms` after the newest kept one.
        let first_this_run = self
            .kept_this_run
            .lock()
            .map(|kept| !kept.contains(id))
            .unwrap_or(true);
        let spaced = newest_version(&directory)
            .is_none_or(|newest| modified_at_ms.saturating_sub(newest) >= self.history_gap_ms);
        if !(keep_version || first_this_run || spaced) {
            return;
        }
        if fs::create_dir_all(&directory).is_err() {
            return;
        }
        // Two versions saved within the same millisecond step on each
        // other's name; one bumped millisecond is enough in practice.
        let mut name = modified_at_ms;
        loop {
            let path = directory.join(format!("{name}.json"));
            if !path.exists()
                && write_private_file(&directory, &path, &bytes, "profile version").is_ok()
            {
                let _ = File::options().write(true).open(&path).and_then(|file| {
                    file.set_times(
                        fs::FileTimes::new()
                            .set_modified(UNIX_EPOCH + Duration::from_millis(modified_at_ms)),
                    )
                });
                break;
            }
            name = match name.checked_add(1) {
                Some(next) => next,
                None => break,
            };
        }
        if let Ok(mut kept) = self.kept_this_run.lock() {
            kept.insert(id.to_owned());
        }
        self.prune_history(&directory);
    }

    /// Keeps only the newest `HISTORY_LIMIT` versions, and of those only as
    /// many as fit `HISTORY_MAX_BYTES`, always keeping the newest. Files
    /// whose name is not a millisecond stamp are left alone, never counted
    /// and never removed.
    fn prune_history(&self, directory: &Path) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut versions: Vec<(u64, String, u64)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let size = entry.metadata().map_or(0, |metadata| metadata.len());
                name.strip_suffix(".json")
                    .and_then(|stem| stem.parse::<u64>().ok())
                    .map(|modified_at_ms| (modified_at_ms, name, size))
            })
            .collect();
        versions.sort_by_key(|(modified_at_ms, _, _)| *modified_at_ms);
        let mut total: u64 = versions.iter().map(|(_, _, size)| size).sum();
        while versions.len() > HISTORY_LIMIT
            || (versions.len() > 1 && total > self.history_max_bytes)
        {
            let (_, oldest, size) = versions.remove(0);
            total = total.saturating_sub(size);
            let _ = fs::remove_file(directory.join(oldest));
        }
    }

    /// The kept earlier versions of a profile, newest first. Junk in the
    /// history folder (a name that is not a millisecond stamp) is skipped.
    pub fn versions(&self, id: &str) -> Result<Vec<ProfileVersion>, String> {
        let directory = self.history_directory(id)?;
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("could not list profile versions: {error}")),
        };
        let mut versions = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            if let Ok(modified_at_ms) = stem.parse::<u64>() {
                let parsed =
                    read_private_file(&entry.path(), Profile::MAX_JSON_BYTES, "profile version")
                        .ok()
                        .flatten()
                        .and_then(|(bytes, _)| Profile::from_json(&bytes).ok());
                versions.push(ProfileVersion {
                    modified_at_ms,
                    name: parsed.as_ref().map(|profile| profile.name.clone()),
                    binding_count: parsed.as_ref().map(|profile| profile.binding_count()),
                });
            }
        }
        versions.sort_by_key(|version| std::cmp::Reverse(version.modified_at_ms));
        Ok(versions)
    }

    /// One earlier version, as a full stored profile (upgraded like a load,
    /// never touching the version file).
    pub fn load_version(&self, id: &str, modified_at_ms: u64) -> Result<StoredProfile, String> {
        if !self
            .versions(id)?
            .iter()
            .any(|version| version.modified_at_ms == modified_at_ms)
        {
            return Err("this profile version no longer exists in storage".into());
        }
        let path = self
            .history_directory(id)?
            .join(format!("{modified_at_ms}.json"));
        let (bytes, _) = read_private_file(&path, Profile::MAX_JSON_BYTES, "profile version")?
            .ok_or("this profile version no longer exists in storage")?;
        let profile = Profile::from_json(&bytes)?.into_v2()?;
        Ok(StoredProfile {
            id: id.to_owned(),
            content_hash: crate::import_flags::content_hash(&profile),
            profile,
            modified_at_ms,
        })
    }

    /// Deletes a readable profile. `expected_modified_at_ms` protects against
    /// removing a version changed elsewhere since it was listed or loaded.
    pub fn delete(&self, id: &str, expected_modified_at_ms: Option<u64>) -> Result<(), String> {
        let current = self.load(id)?;
        if expected_modified_at_ms.is_some_and(|expected| expected != current.modified_at_ms) {
            return Err(CONFLICT_ERROR.into());
        }
        fs::remove_file(self.path(id)?)
            .map_err(|error| format!("could not delete profile: {error}"))?;
        if let Ok(mut summaries) = self.summaries.lock() {
            summaries.remove(id);
        }
        // The versions are unreachable without the profile; the folder is
        // removed best effort (a failure leaves hidden data, never an error).
        if let Ok(directory) = self.history_directory(id) {
            let _ = fs::remove_dir_all(directory);
        }
        self.sync_directory()
    }

    fn sync_directory(&self) -> Result<(), String> {
        sync_directory(&self.directory, "profile")
    }

    fn path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.directory.join(format!("{id}.json")))
    }
}

/// Reads a regular file of at most `max_bytes`, never following a symbolic
/// link and never blocking on a FIFO or device node. `Ok(None)` means the file
/// does not exist. Also returns the opened file's metadata.
/// The newest kept version's stamp in a history folder, from the file names
/// alone (no parsing), or none when there is no folder or no version.
fn newest_version(directory: &Path) -> Option<u64> {
    fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json")?.parse::<u64>().ok()
        })
        .max()
}

pub(crate) fn read_private_file(
    path: &Path,
    max_bytes: usize,
    what: &str,
) -> Result<Option<(Vec<u8>, fs::Metadata)>, String> {
    // Checked before opening so a FIFO or device node never blocks open().
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not inspect {what}: {error}")),
    };
    if !metadata.file_type().is_file() {
        return Err(format!(
            "{what} must be a regular file, not a symbolic link"
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .map_err(|error| format!("could not open {what}: {error}"))?;
    // Re-check the opened file, not the path, in case it was swapped.
    let metadata = file
        .metadata()
        .map_err(|error| format!("could not inspect opened {what}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!(
            "{what} must be a regular file, not a symbolic link"
        ));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(format!("stored {what} exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read {what}: {error}"))?;
    Ok(Some((bytes, metadata)))
}

/// Atomically replaces `path` (inside `directory`) with `bytes`: a private
/// 0600 `.<uuid>.tmp` file is written, flushed and renamed over the target,
/// then the folder is flushed so the rename survives a crash. `rename`
/// replaces a symbolic link itself and never writes through it.
pub(crate) fn write_private_file(
    directory: &Path,
    path: &Path,
    bytes: &[u8],
    what: &str,
) -> Result<(), String> {
    let temporary = directory.join(format!(".{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<(), String> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| format!("could not create temporary {what}: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("could not write {what}: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("could not flush {what}: {error}"))?;
        fs::rename(&temporary, path)
            .map_err(|error| format!("could not atomically save {what}: {error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    // Persist the rename itself, not only the file contents.
    sync_directory(directory, what)
}

fn sync_directory(directory: &Path, what: &str) -> Result<(), String> {
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("could not flush {what} storage: {error}"))
}

pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    let parsed =
        Uuid::parse_str(id).map_err(|_| "profile ID must be an opaque UUID".to_string())?;
    if id != parsed.to_string() {
        return Err("profile ID must be a canonical lowercase UUID".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_profiles;

    /// A store that keeps every overwritten version (no spacing), so tests
    /// that count versions are not about the spacing rule.
    fn store() -> (tempfile::TempDir, ProfileStore) {
        let (directory, mut store) = spaced_store();
        store.history_gap_ms = 0;
        (directory, store)
    }

    /// A store with the real spacing rule.
    fn spaced_store() -> (tempfile::TempDir, ProfileStore) {
        let directory = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(directory.path().join("profiles")).unwrap();
        (directory, store)
    }

    /// Visible entries of a directory: what the store's own rules count as
    /// data (hidden entries such as the version-history folder are never
    /// profiles).
    fn visible_entries(path: &std::path::Path) -> usize {
        fs::read_dir(path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| !is_hidden_entry(&entry.file_name()))
            .count()
    }

    #[test]
    fn seed_save_restart_roundtrip_keeps_bindings_and_identity() {
        let (directory, store) = store();
        let initial = store.bootstrap().unwrap();
        assert!(initial.desktop);
        assert_eq!(initial.profiles.len(), 1);
        let initial = initial.profile.unwrap();
        assert_eq!(initial.profile.name, "Everyday");
        let id = initial.id;
        let mut profile = initial.profile;
        profile.name = "Work".into();
        profile.bindings[0].action = cooltek_sdk_service::Action::Volume { delta: 8 };
        let saved = store.save(Some(&id), profile.clone(), None).unwrap();
        assert_eq!(saved.profile, profile);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.load(&id).unwrap().profile, profile);
        assert_eq!(restarted.bootstrap().unwrap().profile.unwrap().id, id);
        assert_eq!(restarted.list().unwrap().profiles.len(), 1);
        assert_eq!(visible_entries(&directory.path().join("profiles")), 1);
    }

    #[test]
    fn legacy_load_upgrades_only_in_memory_until_explicit_save() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        let bytes = include_bytes!("../../../../examples/profiles/all-modules.json");
        fs::write(&path, bytes).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let expected = Profile::from_json(bytes).unwrap().into_v2().unwrap();

        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.id, id);
        assert_eq!(loaded.profile, expected);
        assert_eq!(loaded.profile.schema_version, 2);
        assert!(loaded.profile.keypad.is_some());
        assert_eq!(
            store.bootstrap().unwrap().profile.unwrap().profile,
            expected
        );
        assert_eq!(
            store.list().unwrap().profiles[0].binding_count,
            expected.binding_count()
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

        let saved = store.save(Some(&id), loaded.profile, None).unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.profile, expected);
        let saved_bytes = fs::read(&path).unwrap();
        assert_ne!(saved_bytes, bytes);
        assert_eq!(Profile::from_json(&saved_bytes).unwrap().schema_version, 2);
        assert_eq!(visible_entries(&directory.path().join("profiles")), 1);
    }

    #[test]
    fn schema_two_roundtrip_and_summary_include_all_nested_tile_bindings() {
        let (directory, store) = store();
        let profile = test_profiles::folders();
        let saved = store.save(None, profile.clone(), None).unwrap();
        assert_eq!(saved.profile, profile);
        assert_eq!(saved.profile.binding_count(), 7);
        assert_eq!(store.list().unwrap().profiles[0].binding_count, 7);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.load(&saved.id).unwrap().profile, profile);
        assert_eq!(restarted.bootstrap().unwrap().profile.unwrap().id, saved.id);
    }

    #[test]
    fn invalid_folder_profiles_cannot_overwrite_or_reseed_storage() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        let original = fs::read(&path).unwrap();
        let mut invalid = test_profiles::folders_json();
        invalid["keypad"]["pages"][0]["slots"][0]["page_id"] = "missing".into();
        let invalid_profile: Profile = serde_json::from_value(invalid.clone()).unwrap();
        assert!(store.save(Some(&saved.id), invalid_profile, None).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
        fs::write(&path, &invalid_bytes).unwrap();
        assert!(store.load(&saved.id).is_err());
        let recovered = store.bootstrap().unwrap();
        assert!(recovered.profile.is_none());
        assert!(recovered.profiles.is_empty());
        assert_eq!(recovered.unreadable.len(), 1);
        assert_eq!(recovered.unreadable[0].id, saved.id);
        assert_eq!(fs::read(&path).unwrap(), invalid_bytes);
        assert_eq!(visible_entries(&directory.path().join("profiles")), 1);
    }

    #[test]
    fn invalid_ids_and_invalid_profiles_cannot_write() {
        let (_directory, store) = store();
        let initial = store.bootstrap().unwrap();
        for id in [
            "../escape",
            "",
            "/tmp/outside",
            "a.json",
            "00000000-0000-0000-0000-00000000000A",
        ] {
            assert!(store.load(id).is_err());
            assert!(
                store
                    .save(
                        Some(id),
                        initial.profile.as_ref().unwrap().profile.clone(),
                        None
                    )
                    .is_err()
            );
        }
        let mut invalid = initial.profile.unwrap().profile;
        invalid.bindings[0].control = 3;
        assert!(store.save(None, invalid, None).is_err());
        assert_eq!(store.list().unwrap().profiles.len(), 1);
    }

    #[test]
    fn corrupt_or_oversized_data_is_preserved_and_never_reseeded() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        fs::write(&path, b"corrupt").unwrap();
        let recovered = store.bootstrap().unwrap();
        assert!(recovered.profile.is_none());
        assert_eq!(recovered.unreadable[0].id, id);
        assert_eq!(fs::read(&path).unwrap(), b"corrupt");
        assert!(
            store
                .save(
                    Some(&id),
                    Profile::from_json(SAMPLE.as_bytes()).unwrap(),
                    None
                )
                .is_err()
        );
        fs::write(&path, vec![b' '; Profile::MAX_JSON_BYTES + 1]).unwrap();
        assert!(store.load(&id).unwrap_err().contains("size limit"));
        assert_eq!(visible_entries(&directory.path().join("profiles")), 1);
    }

    #[test]
    fn strict_unknown_fields_and_partial_writes_are_rejected() {
        let (directory, store) = store();
        let id = Uuid::new_v4().to_string();
        let path = directory.path().join("profiles").join(format!("{id}.json"));
        fs::write(
            &path,
            br#"{"schema_version":1,"name":"Desk","bindings":[],"extra":true}"#,
        )
        .unwrap();
        assert!(store.load(&id).is_err());
        fs::remove_file(path).unwrap();
        // Unfamiliar visible data still blocks seeding.
        let unfamiliar = directory.path().join("profiles/notes.txt");
        fs::write(&unfamiliar, b"keep me").unwrap();
        assert!(store.bootstrap().is_err());
        assert_eq!(fs::read(&unfamiliar).unwrap(), b"keep me");
    }

    #[test]
    fn hidden_entries_never_block_seeding_and_are_left_untouched() {
        let (directory, store) = store();
        let profiles = directory.path().join("profiles");
        for (name, bytes) in [
            (".DS_Store", &b"finder"[..]),
            ("._sidecar.json", &b"appledouble"[..]),
            (".interrupted.tmp", &b"partial"[..]),
        ] {
            fs::write(profiles.join(name), bytes).unwrap();
        }
        let initial = store.bootstrap().unwrap();
        assert_eq!(initial.profiles.len(), 1);
        assert!(initial.unreadable.is_empty());
        assert_eq!(fs::read(profiles.join(".DS_Store")).unwrap(), b"finder");
        assert_eq!(
            fs::read(profiles.join("._sidecar.json")).unwrap(),
            b"appledouble"
        );
        assert_eq!(
            fs::read(profiles.join(".interrupted.tmp")).unwrap(),
            b"partial"
        );
    }

    #[test]
    fn one_unreadable_profile_does_not_hide_others_or_block_saving() {
        let (directory, store) = store();
        let good = store.save(None, test_profiles::folders(), None).unwrap();
        let bad_id = Uuid::new_v4().to_string();
        let bad_path = directory
            .path()
            .join("profiles")
            .join(format!("{bad_id}.json"));
        fs::write(&bad_path, b"corrupt").unwrap();
        fs::write(directory.path().join("profiles/not-a-uuid.json"), b"{}").unwrap();

        let listing = store.list().unwrap();
        assert_eq!(listing.profiles.len(), 1);
        assert_eq!(listing.profiles[0].id, good.id);
        let ids: Vec<_> = listing
            .unreadable
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&bad_id.as_str()) && ids.contains(&"not-a-uuid"));
        assert!(listing.unreadable.iter().all(|item| !item.error.is_empty()));

        let bootstrap = store.bootstrap().unwrap();
        assert_eq!(bootstrap.profile.unwrap().id, good.id);
        assert_eq!(bootstrap.unreadable, listing.unreadable);

        let created = store.save(None, test_profiles::folders(), None).unwrap();
        let updated = store
            .save(
                Some(&good.id),
                test_profiles::folders(),
                Some(good.modified_at_ms),
            )
            .unwrap();
        assert_eq!(updated.id, good.id);
        assert_eq!(store.list().unwrap().profiles.len(), 2);
        assert!(store.load(&created.id).is_ok());
        assert_eq!(fs::read(&bad_path).unwrap(), b"corrupt");
    }

    #[test]
    fn stale_saves_and_deletes_are_rejected_as_conflicts() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        // Simulate another instance writing the file later.
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                UNIX_EPOCH + std::time::Duration::from_millis(saved.modified_at_ms + 5_000),
            ))
            .unwrap();
        let before = fs::read(&path).unwrap();
        let mut edited = test_profiles::folders();
        edited.name = "Mine".into();
        let error = store
            .save(Some(&saved.id), edited.clone(), Some(saved.modified_at_ms))
            .unwrap_err();
        assert_eq!(error, CONFLICT_ERROR);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            store
                .delete(&saved.id, Some(saved.modified_at_ms))
                .unwrap_err(),
            CONFLICT_ERROR
        );
        assert!(path.exists());

        let current = store.load(&saved.id).unwrap();
        let resaved = store
            .save(Some(&saved.id), edited, Some(current.modified_at_ms))
            .unwrap();
        assert_eq!(resaved.profile.name, "Mine");
        store
            .delete(&saved.id, Some(resaved.modified_at_ms))
            .unwrap();
        assert!(!path.exists());
        assert!(
            store
                .load(&saved.id)
                .unwrap_err()
                .contains("no longer exists")
        );
    }

    #[test]
    fn deleting_frees_capacity_and_never_touches_invalid_ids_or_unreadable_files() {
        let (directory, store) = store();
        let mut last = None;
        for _ in 0..MAX_PROFILES {
            last = Some(store.save(None, test_profiles::folders(), None).unwrap());
        }
        let error = store
            .save(None, test_profiles::folders(), None)
            .unwrap_err();
        assert!(error.contains("delete a profile"));
        let last = last.unwrap();
        store.delete(&last.id, None).unwrap();
        assert!(store.save(None, test_profiles::folders(), None).is_ok());
        for id in ["../escape", "", "a.json"] {
            assert!(store.delete(id, None).is_err());
        }
        let bad_id = Uuid::new_v4().to_string();
        let bad_path = directory
            .path()
            .join("profiles")
            .join(format!("{bad_id}.json"));
        fs::write(&bad_path, b"corrupt").unwrap();
        assert!(store.delete(&bad_id, None).is_err());
        assert_eq!(fs::read(&bad_path).unwrap(), b"corrupt");
    }

    #[test]
    fn overfull_storage_lists_a_partial_set_and_refuses_only_new_profiles() {
        let (directory, store) = store();
        let profiles = directory.path().join("profiles");
        let bytes = serde_json::to_vec(&test_profiles::folders()).unwrap();
        for _ in 0..MAX_PROFILES + 2 {
            fs::write(profiles.join(format!("{}.json", Uuid::new_v4())), &bytes).unwrap();
        }
        let listing = store.list().unwrap();
        assert_eq!(listing.profiles.len(), MAX_PROFILES);
        assert!(listing.unreadable.is_empty());
        let bootstrap = store.bootstrap().unwrap();
        assert_eq!(bootstrap.profiles.len(), MAX_PROFILES);
        let first = bootstrap.profile.unwrap();
        let error = store
            .save(None, test_profiles::folders(), None)
            .unwrap_err();
        assert!(error.contains("delete a profile"), "{error}");
        // Existing profiles can still be edited and deleted.
        let saved = store
            .save(Some(&first.id), first.profile, Some(first.modified_at_ms))
            .unwrap();
        store.delete(&saved.id, Some(saved.modified_at_ms)).unwrap();
        assert_eq!(visible_entries(&profiles), MAX_PROFILES + 1);
    }

    #[test]
    fn opening_removes_only_stale_hidden_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let profiles = directory.path().join("profiles");
        fs::create_dir(&profiles).unwrap();
        let old =
            std::time::SystemTime::now() - STALE_TEMPORARY - std::time::Duration::from_secs(60);
        let age = |path: &Path| {
            File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(old))
                .unwrap();
        };
        let stale = profiles.join(format!(".{}.tmp", Uuid::new_v4()));
        let fresh = profiles.join(format!(".{}.tmp", Uuid::new_v4()));
        let visible = profiles.join("visible.tmp");
        let finder = profiles.join(".DS_Store");
        for path in [&stale, &fresh, &visible, &finder] {
            fs::write(path, b"partial").unwrap();
        }
        for path in [&stale, &visible, &finder] {
            age(path);
        }
        fs::create_dir(profiles.join(".folder.tmp")).unwrap();
        let store = ProfileStore::open(profiles.clone()).unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists() && visible.exists() && finder.exists());
        assert!(profiles.join(".folder.tmp").is_dir());
        // Hidden leftovers never count as data: once the visible file is gone
        // the store is seeded.
        fs::remove_file(&visible).unwrap();
        assert_eq!(store.bootstrap().unwrap().profiles.len(), 1);
    }

    #[test]
    fn only_hyphenated_uuid_temporaries_match_in_shared_folders() {
        let uuid = Uuid::new_v4();
        let name = |middle: String| std::ffi::OsString::from(format!(".{middle}.tmp"));
        assert!(is_uuid_temporary(&name(uuid.hyphenated().to_string())));
        for other in [
            uuid.simple().to_string(),
            uuid.braced().to_string(),
            uuid.urn().to_string(),
            "not-a-uuid".into(),
        ] {
            assert!(!is_uuid_temporary(&name(other.clone())), "{other}");
        }
        assert!(!is_uuid_temporary(OsStr::new(&format!("{uuid}.tmp"))));
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_profile_paths_do_not_read_or_overwrite_external_data() {
        let (directory, store) = store();
        let outside = directory.path().join("outside.json");
        let profile = Profile::from_json(SAMPLE.as_bytes()).unwrap();
        fs::write(&outside, serde_json::to_vec(&profile).unwrap()).unwrap();
        let before = fs::read(&outside).unwrap();
        let id = Uuid::new_v4().to_string();
        std::os::unix::fs::symlink(&outside, store.path(&id).unwrap()).unwrap();
        assert!(store.load(&id).is_err());
        assert!(store.save(Some(&id), profile, None).is_err());
        assert!(store.delete(&id, None).is_err());
        assert!(store.path(&id).unwrap().symlink_metadata().is_ok());
        assert_eq!(fs::read(outside).unwrap(), before);
    }

    /// Sets the profile file's modification time, so each test version has a
    /// distinct, controlled millisecond stamp.
    fn age_to(path: &Path, modified_at_ms: u64) {
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(UNIX_EPOCH + Duration::from_millis(modified_at_ms)),
            )
            .unwrap();
    }

    #[test]
    fn overwrites_keep_a_rolling_history_that_restores_reversibly() {
        let (directory, store) = store();
        let first = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", first.id));
        age_to(&path, 1_000);
        let mut second = test_profiles::folders();
        second.name = "Second".into();
        store.save(Some(&first.id), second, Some(1_000)).unwrap();

        let versions = store.versions(&first.id).unwrap();
        assert_eq!(
            versions,
            vec![ProfileVersion {
                modified_at_ms: 1_000,
                name: Some(first.profile.name.clone()),
                binding_count: Some(first.profile.binding_count()),
            }]
        );

        // The version content is the first save, never touched by the read.
        let restored_version = store.load_version(&first.id, 1_000).unwrap();
        assert_eq!(restored_version.profile, first.profile);

        // Restoring goes through save(): the second version is snapshotted
        // first, so the restore itself is reversible.
        age_to(&path, 2_000);
        store
            .save(Some(&first.id), restored_version.profile, Some(2_000))
            .unwrap();
        let versions = store.versions(&first.id).unwrap();
        assert_eq!(
            versions
                .iter()
                .map(|version| (version.modified_at_ms, version.name.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (2_000, Some("Second")),
                (1_000, Some(first.profile.name.as_str()))
            ]
        );
        assert_eq!(store.load(&first.id).unwrap().profile, first.profile);

        // Unknown versions and ids are refused, never invented.
        assert!(store.load_version(&first.id, 3_000).is_err());
        assert!(store.load_version("../escape", 1_000).is_err());
        assert!(store.versions("../escape").is_err());
    }

    #[test]
    fn history_is_capped_hidden_from_listings_and_removed_with_the_profile() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        for stamp in 1..=HISTORY_LIMIT as u64 + 5 {
            age_to(&path, stamp * 1_000);
            let mut version = test_profiles::folders();
            version.name = format!("Version {stamp}");
            store
                .save(Some(&saved.id), version, Some(stamp * 1_000))
                .unwrap();
        }
        let versions = store.versions(&saved.id).unwrap();
        assert_eq!(versions.len(), HISTORY_LIMIT);
        assert_eq!(
            versions[0].modified_at_ms,
            (HISTORY_LIMIT as u64 + 5) * 1_000
        );
        assert_eq!(
            versions.last().unwrap().modified_at_ms,
            6 * 1_000,
            "the oldest kept version is the one after the first evicted"
        );

        // The history folder is hidden: never a profile, never a blocker.
        let listing = store.list().unwrap();
        assert_eq!(listing.profiles.len(), 1);
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        assert_eq!(restarted.list().unwrap().profiles.len(), 1);
        assert_eq!(restarted.versions(&saved.id).unwrap(), versions);

        // Deleting the profile removes its history with it.
        restarted
            .delete(
                &saved.id,
                Some(restarted.load(&saved.id).unwrap().modified_at_ms),
            )
            .unwrap();
        assert!(
            !directory
                .path()
                .join("profiles/.history")
                .join(&saved.id)
                .exists()
        );
        assert_eq!(restarted.versions(&saved.id).unwrap(), Vec::new());
    }

    #[test]
    fn identical_rewrites_and_failed_saves_change_no_history() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        age_to(&path, 1_000);
        let same = store.load(&saved.id).unwrap();
        store
            .save(Some(&saved.id), same.profile, Some(1_000))
            .unwrap();
        assert_eq!(store.versions(&saved.id).unwrap(), Vec::new());
        assert_eq!(
            fs::metadata(&path).unwrap().modified().unwrap(),
            UNIX_EPOCH + Duration::from_millis(1_000),
            "an identical automatic save keeps the file as it is"
        );

        // A refused overwrite (bad content) snapshots nothing.
        let mut invalid = test_profiles::folders_json();
        invalid["keypad"]["pages"][0]["slots"][0]["page_id"] = "missing".into();
        let invalid_profile: Profile = serde_json::from_value(invalid).unwrap();
        assert!(
            store
                .save(Some(&saved.id), invalid_profile, Some(1_000))
                .is_err()
        );
        assert_eq!(store.versions(&saved.id).unwrap(), Vec::new());

        // A brand-new profile has no previous version to keep.
        let created = store.save(None, test_profiles::folders(), None).unwrap();
        assert_eq!(store.versions(&created.id).unwrap(), Vec::new());
    }

    /// The stamps of a profile's kept versions, newest first.
    fn stamps(store: &ProfileStore, id: &str) -> Vec<u64> {
        store
            .versions(id)
            .unwrap()
            .iter()
            .map(|version| version.modified_at_ms)
            .collect()
    }

    fn overwrite(store: &ProfileStore, directory: &Path, id: &str, at: u64, name: &str) {
        let path = directory.join("profiles").join(format!("{id}.json"));
        age_to(&path, at);
        let mut next = test_profiles::folders();
        next.name = name.into();
        store.save(Some(id), next, Some(at)).unwrap();
    }

    #[test]
    fn versions_are_spaced_so_a_burst_of_edits_cannot_push_out_the_history() {
        let (directory, store) = spaced_store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        let minute = 60_000;
        // The first overwrite in this run keeps the good original.
        overwrite(&store, directory.path(), &saved.id, minute, "Edit 1");
        assert_eq!(stamps(&store, &saved.id), vec![minute]);
        // A burst of settled edits inside the gap keeps nothing more.
        for step in 2..=30 {
            overwrite(
                &store,
                directory.path(),
                &saved.id,
                minute + step * 1_000,
                &format!("Edit {step}"),
            );
        }
        assert_eq!(stamps(&store, &saved.id), vec![minute]);
        // Once the newest kept version is old enough, the next overwrite
        // keeps what it replaces.
        let later = minute + HISTORY_MIN_GAP_MS + 1;
        overwrite(&store, directory.path(), &saved.id, later, "Edit 31");
        overwrite(
            &store,
            directory.path(),
            &saved.id,
            later + 1_000,
            "Edit 32",
        );
        assert_eq!(stamps(&store, &saved.id), vec![later, minute]);
        // The original is still the oldest copy and reads back intact.
        assert_eq!(
            store.load_version(&saved.id, minute).unwrap().profile,
            saved.profile
        );
    }

    #[test]
    fn a_restore_and_the_first_overwrite_of_a_run_always_keep_a_version() {
        let (directory, store) = spaced_store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        overwrite(&store, directory.path(), &saved.id, 1_000, "Edit 1");
        overwrite(&store, directory.path(), &saved.id, 2_000, "Edit 2");
        assert_eq!(
            stamps(&store, &saved.id),
            vec![1_000],
            "inside the gap, nothing new"
        );

        // A restore asks for the replaced version to be kept.
        let path = directory
            .path()
            .join("profiles")
            .join(format!("{}.json", saved.id));
        age_to(&path, 3_000);
        let original = store.load_version(&saved.id, 1_000).unwrap().profile;
        store
            .save_with(Some(&saved.id), original, Some(3_000), true)
            .unwrap();
        assert_eq!(stamps(&store, &saved.id), vec![3_000, 1_000]);

        // After a restart the first overwrite of the profile keeps again.
        let restarted = ProfileStore::open(directory.path().join("profiles")).unwrap();
        overwrite(&restarted, directory.path(), &saved.id, 4_000, "Edit 3");
        assert_eq!(stamps(&restarted, &saved.id), vec![4_000, 3_000, 1_000]);
        overwrite(&restarted, directory.path(), &saved.id, 5_000, "Edit 4");
        assert_eq!(
            stamps(&restarted, &saved.id).len(),
            3,
            "later ones are spaced again"
        );
    }

    #[test]
    fn history_is_capped_by_size_and_never_drops_the_newest_version() {
        let (directory, mut store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        overwrite(&store, directory.path(), &saved.id, 1_000, "Edit 1");
        let one = fs::metadata(
            directory
                .path()
                .join("profiles/.history")
                .join(&saved.id)
                .join("1000.json"),
        )
        .unwrap()
        .len();
        // Room for two versions' worth of bytes.
        store.history_max_bytes = one * 2 + one / 2;
        for step in 2..=6u64 {
            overwrite(
                &store,
                directory.path(),
                &saved.id,
                step * 1_000,
                &format!("Edit {step}"),
            );
        }
        let kept = stamps(&store, &saved.id);
        assert_eq!(kept.len(), 2, "the oldest versions went first");
        assert_eq!(kept[0], 6_000, "the newest version is kept");

        // A cap below one version still keeps that newest one.
        store.history_max_bytes = 1;
        overwrite(&store, directory.path(), &saved.id, 7_000, "Edit 7");
        assert_eq!(stamps(&store, &saved.id), vec![7_000]);
    }

    #[test]
    fn versions_say_what_they_hold_without_loading_them() {
        let (directory, store) = store();
        let saved = store.save(None, test_profiles::folders(), None).unwrap();
        overwrite(&store, directory.path(), &saved.id, 1_000, "Renamed");
        // A version file that is not a profile is listed with no summary
        // and can still be told apart from the readable ones.
        let history = directory.path().join("profiles/.history").join(&saved.id);
        fs::write(history.join("500.json"), b"not json").unwrap();
        let versions = store.versions(&saved.id).unwrap();
        assert_eq!(
            versions[0].name.as_deref(),
            Some(saved.profile.name.as_str())
        );
        assert_eq!(
            versions[0].binding_count,
            Some(saved.profile.binding_count())
        );
        assert_eq!(
            (
                versions[1].modified_at_ms,
                versions[1].name.clone(),
                versions[1].binding_count
            ),
            (500, None, None)
        );
    }
}
