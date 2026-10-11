mod app_icon;
mod artwork;
mod capabilities;
#[cfg(test)]
mod contract_fixtures;
mod diagnostics;
mod display;
mod export;
mod feed;
mod focus;
mod import_flags;
mod lighting;
mod mhz;
mod migration;
mod obs;
mod power;
#[cfg(target_os = "macos")]
mod quit;
mod replay;
mod service;
mod session;
mod startup;
mod store;
#[cfg(test)]
mod test_profiles;

use cooltek_sdk_service::{
    Gesture, InputValue, Profile, ServiceModule,
    obs::{MAX_LISTED_NAME_BYTES, ObsCatalog, ObsClient},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager, State};

struct AppState {
    store: Mutex<Result<store::ProfileStore, String>>,
    app_data: Result<PathBuf, String>,
    feeds: feed::Feeds,
    inventory: feed::InventoryWatcher,
    services: service::Services,
    sessions: session::Manager,
    focus: focus::FocusState,
    display: display::DisplayState,
    /// The saved profile last used on each keypad and dial serial number.
    last_profiles: startup::LastProfiles,
    /// The OBS endpoint setting and the in-memory OBS password.
    obs: obs::ObsState,
    /// Import records and the dry-run-only flag of imported profiles.
    imports: import_flags::ImportFlags,
    /// Serializes artwork jobs (a single-permit async lock), so concurrent
    /// requests cannot decode several large images at once.
    artwork: tauri::async_runtime::Mutex<()>,
}

/// How long a sleep notification waits for the pre-sleep stop. Every service
/// is stopped in parallel, which takes at most `STOP_GRACE` plus the kill,
/// reap and pipe drain (about 1 s); the rest covers a command the session
/// driver may still be finishing (another bounded stop). macOS delays sleep
/// for an observer only up to about 30 s, so this stays well below that.
const SUSPEND_WAIT: Duration = Duration::from_secs(service::STOP_GRACE.as_secs() * 2 + 4);

/// Bootstrap failures carry the expected storage folder so the frontend can
/// tell the user where to look before retrying.
#[derive(Debug, Serialize)]
struct BootstrapError {
    message: String,
    storage_path: Option<String>,
}

impl AppState {
    fn open(app_data: Result<PathBuf, String>) -> Self {
        let store = app_data
            .clone()
            .and_then(store::ProfileStore::open_for_app_data);
        let focus = focus::FocusState::open(app_data.as_deref().map_err(Clone::clone));
        let display = display::DisplayState::open(app_data.as_deref().map_err(Clone::clone));
        let last_profiles = startup::LastProfiles::open(app_data.as_deref().map_err(Clone::clone));
        let obs = obs::ObsState::open(app_data.as_deref().map_err(Clone::clone));
        let imports = import_flags::ImportFlags::open(app_data.as_deref().map_err(Clone::clone));
        Self {
            store: Mutex::new(store),
            app_data,
            feeds: feed::Feeds::default(),
            inventory: feed::InventoryWatcher::default(),
            services: service::Services::default(),
            sessions: session::Manager::default(),
            focus,
            display,
            last_profiles,
            obs,
            imports,
            artwork: tauri::async_runtime::Mutex::new(()),
        }
    }

    fn with_store<T>(
        &self,
        operation: impl FnOnce(&store::ProfileStore) -> Result<T, String>,
    ) -> Result<T, String> {
        let store = self
            .store
            .lock()
            .map_err(|_| "profile storage lock is unavailable".to_string())?;
        operation(store.as_ref().map_err(Clone::clone)?)
    }

    /// Opens storage again if the previous attempt failed, so a retry after
    /// the user fixes the folder does not require restarting the app.
    fn bootstrap(&self) -> Result<store::AppBootstrap, BootstrapError> {
        if let (Ok(mut store), Ok(app_data)) = (self.store.lock(), &self.app_data)
            && store.is_err()
        {
            *store = store::ProfileStore::open_for_app_data(app_data.clone());
        }
        self.with_store(store::ProfileStore::bootstrap)
            .map(|mut bootstrap| {
                bootstrap.import_flags_error = self.mark_dry_run_only(&mut bootstrap.profiles);
                bootstrap
            })
            .map_err(|message| BootstrapError {
                message,
                storage_path: self
                    .app_data
                    .as_ref()
                    .ok()
                    .map(|path| path.join("profiles").to_string_lossy().into_owned()),
            })
    }

    /// Marks profiles that may run only as a dry run: by id or by content
    /// hash. An unreadable import list marks every profile and is returned,
    /// so the UI shows the recovery action.
    fn mark_dry_run_only(&self, profiles: &mut [store::ProfileSummary]) -> Option<String> {
        match self.imports.load() {
            Ok(records) => {
                let ids = records.dry_run_only();
                for summary in profiles {
                    summary.dry_run_only = ids.contains(&summary.id)
                        || records.flagged_hashes.contains_key(&summary.content_hash);
                }
                None
            }
            Err(error) => {
                for summary in profiles {
                    summary.dry_run_only = true;
                }
                Some(error)
            }
        }
    }

    fn list(&self) -> Result<store::ProfileListing, String> {
        let mut listing = self.with_store(store::ProfileStore::list)?;
        listing.import_flags_error = self.mark_dry_run_only(&mut listing.profiles);
        Ok(listing)
    }

    /// Why `profile` must not run with actions, or `None` (see
    /// `import_flags::ExecutionGuard::blocks`). Reads only the import list,
    /// never the profile store.
    fn execution_block(
        &self,
        profile_id: Option<&str>,
        source_profile_id: Option<&str>,
        profile: &Profile,
    ) -> Option<String> {
        self.execution_guard()
            .blocks(profile_id, source_profile_id, profile)
    }

    /// The flags for one check. A saved profile whose stored content hash
    /// is flagged counts as a flagged id even without a record, so a draft
    /// opened from it (its source) stays blocked after edits. The listing
    /// re-parses only changed files.
    fn execution_guard(&self) -> import_flags::ExecutionGuard {
        let mut guard = self.imports.guard();
        match self.with_store(|store| store.list()) {
            Ok(listing) => guard.flag_ids_by_content(
                listing
                    .profiles
                    .iter()
                    .map(|summary| (summary.id.as_str(), summary.content_hash.as_str())),
            ),
            Err(error) => {
                guard = import_flags::ExecutionGuard::failed(format!(
                    "the saved profiles cannot be listed: {error}"
                ))
            }
        }
        guard
    }

    /// Saves a profile. The saved id becomes dry-run only whenever the
    /// content's hash is flagged, or the target id or the draft's source id
    /// is flagged (the new content's hash is then flagged too). The flag is
    /// recorded before the file is written, under the store lock.
    fn save_profile(&self, request: SaveProfileRequest) -> Result<store::StoredProfile, String> {
        let SaveProfileRequest {
            id,
            profile,
            expected_modified_at_ms,
            source_profile_id,
            keep_version,
        } = request;
        self.with_store(|store| {
            let hash = import_flags::content_hash(&profile);
            // While the list cannot be read, a save that names a saved
            // profile (its target or its source) could carry imported
            // content and is refused. A brand-new profile with no source is
            // still saved. If its content has a flagged hash, it stays
            // flagged by that hash: no hash a saved profile has is ever
            // released or evicted, and "Mark all profiles as needing
            // review" flags every saved profile anyway.
            let records = match self.imports.load() {
                Ok(records) => records,
                Err(error) if id.is_some() || source_profile_id.is_some() => {
                    return Err(format!(
                        "this profile cannot be saved while the list of imported profiles is unreadable: {error}"
                    ));
                }
                Err(_) => return store.save(None, profile, expected_modified_at_ms),
            };
            // A target or source whose stored content is flagged counts as
            // flagged even without a record (a copy saved while the list
            // was unreadable, for example).
            let listing = store.list()?;
            let stored_hash = |id: &str| {
                listing
                    .profiles
                    .iter()
                    .find(|summary| summary.id == id)
                    .map(|summary| summary.content_hash.as_str())
            };
            let ids = records.dry_run_only();
            let flagged = records.flagged_hashes.contains_key(&hash)
                || [id.as_deref(), source_profile_id.as_deref()]
                    .into_iter()
                    .flatten()
                    .any(|id| {
                        ids.contains(id)
                            || stored_hash(id)
                                .is_some_and(|hash| records.flagged_hashes.contains_key(hash))
                    });
            if !flagged {
                return store.save_with(id.as_deref(), profile, expected_modified_at_ms, keep_version);
            }
            let target = match &id {
                Some(id) => {
                    // The conflict check runs before anything is flagged, so
                    // a refused save leaves the target as it was.
                    let current = store.load(id)?;
                    if expected_modified_at_ms
                        .is_some_and(|expected| expected != current.modified_at_ms)
                    {
                        return Err(store::CONFLICT_ERROR.into());
                    }
                    id.clone()
                }
                None => uuid::Uuid::new_v4().to_string(),
            };
            let before = records;
            let mut previous_hash = String::new();
            let tracked = tracked_hash(&profile, &hash);
            self.imports.update(&saved_hashes(store)?, |records| {
                previous_hash = records.flag(&target, &tracked, "derived", now_ms());
                Ok(())
            })?;
            let saved = match id {
                Some(id) => store.save_with(Some(&id), profile, expected_modified_at_ms, keep_version),
                None => store.create(&target, profile),
            };
            // A save can fail after the file was replaced (the folder flush):
            // what is on disk decides, never the error alone.
            let on_disk = store.load(&target).ok().map(|stored| stored.content_hash);
            if saved.is_err() && on_disk.as_deref() != Some(hash.as_str()) {
                // The new content never reached the disk: put back the
                // target's record as it was (its new hash may stay).
                let previous = before.profiles.get(&target).cloned();
                let _ = saved_hashes(store).and_then(|saved| {
                    self.imports.update(&saved, |records| {
                        match previous {
                            Some(record) => records.profiles.insert(target.clone(), record),
                            None => records.profiles.remove(&target),
                        };
                        Ok(())
                    })
                });
            } else if previous_hash != hash {
                // The new version is on disk; the previous version's hash
                // goes unless a saved profile or a record still has it.
                let _ = saved_hashes(store).and_then(|saved| {
                    self.imports.update(&saved, |records| {
                        records.release(&previous_hash, &saved);
                        Ok(())
                    })
                });
            }
            saved
        })
    }

    /// Imports an untrusted `.mhz` file as new, dry-run-only profiles. Never
    /// overwrites a profile. A failure removes the profiles created so far;
    /// one that cannot be removed keeps its flag and is named in the error.
    fn import_mhz(&self, bytes: &[u8], now_ms: u64) -> Result<MhzImport, String> {
        if bytes.len() > mhz::archive::MAX_ARCHIVE_BYTES {
            return Err(format!(
                "this file is larger than {} MiB",
                mhz::archive::MAX_ARCHIVE_BYTES / 1024 / 1024
            ));
        }
        let converted = mhz::convert(bytes)?;
        let source_sha256 = mhz::sha256_hex(bytes);
        self.with_store(|store| {
            let listing = store.list()?;
            if listing.profiles.len() + listing.unreadable.len() + converted.len()
                > store::MAX_PROFILES
            {
                return Err(format!(
                    "profile storage permits at most {} profiles; delete a profile before importing",
                    store::MAX_PROFILES
                ));
            }
            let mut names: Vec<String> = listing.profiles.into_iter().map(|p| p.name).collect();
            let mut imported: Vec<ImportedProfile> = Vec::new();
            for item in converted {
                let mut report = item.report;
                let id = uuid::Uuid::new_v4().to_string();
                let created = (|| {
                    let name = mhz::unique_name(&report.source_name, &names)?;
                    let mut profile = item.profile;
                    profile.name = name.clone();
                    replay::validate_profile(&profile)?;
                    let hash = tracked_hash(&profile, &import_flags::content_hash(&profile));
                    self.imports.update(&saved_hashes(store)?, |records| {
                        let _ = records.flag(&id, &hash, "mhz", now_ms);
                        if let Some(record) = records.profiles.get_mut(&id) {
                            record.source_sha256 = source_sha256.clone();
                        }
                        Ok(())
                    })?;
                    store.create(&id, profile)
                })();
                match created {
                    Ok(stored) => {
                        report.profile_name = stored.profile.name.clone();
                        names.push(stored.profile.name.clone());
                        imported.push(ImportedProfile { stored, report });
                    }
                    Err(error) => {
                        // A profile that reached the disk although its write
                        // reported an error (a failing flush) is rolled back
                        // with the others; one that never reached the disk
                        // keeps no record (its hash stays flagged).
                        match store.load(&id) {
                            Ok(stored) => imported.push(ImportedProfile {
                                stored,
                                report: report.clone(),
                            }),
                            Err(_) => {
                                let _ = saved_hashes(store).and_then(|saved| {
                                    self.imports.update(&saved, |records| {
                                        records.profiles.remove(&id);
                                        Ok(())
                                    })
                                });
                            }
                        }
                        let left = self.roll_back(store, &imported);
                        if left.is_empty() {
                            return Err(error);
                        }
                        return Err(format!(
                            "{error}; these imported profiles could not be removed and stay dry run only: {}",
                            left.join(", ")
                        ));
                    }
                }
            }
            Ok(MhzImport {
                profiles: imported,
                source_sha256,
            })
        })
    }

    /// Writes a JSON export. When the draft's source or content is dry-run
    /// only, the exported content's hash is flagged (and pinned) before the
    /// file is written, so importing that file again stays dry run only.
    /// With an unreadable import list nothing is exported.
    fn export_profile(&self, request: ExportRequest) -> Result<export::ExportResult, String> {
        let app_data = self.app_data.as_deref().map_err(Clone::clone)?;
        let hash = import_flags::content_hash(&request.profile);
        self.with_store(|store| {
            let records = self.imports.load()?;
            let flagged = records.flagged_hashes.contains_key(&hash)
                || request
                    .source_profile_id
                    .as_deref()
                    .is_some_and(|id| records.dry_run_only().contains(id));
            if flagged && request.profile.binding_count() > 0 {
                self.imports.update(&saved_hashes(store)?, |records| {
                    records.flag_hash(&hash, true);
                    Ok(())
                })?;
            }
            Ok(())
        })?;
        export::export(&request.profile, app_data, export::now_seconds()?)
    }

    /// Deletes a saved profile. Its import record (if any) becomes a
    /// tombstone with its fingerprint, so its open draft and any copy stay
    /// dry run only; only tombstones are ever evicted from the list. While
    /// the list cannot be read, the id goes to the sentinel instead, and
    /// recovery restores it as a dry-run-only tombstone.
    fn delete_profile(&self, id: &str, expected_modified_at_ms: Option<u64>) -> Result<(), String> {
        self.with_store(|store| {
            match self.imports.load() {
                Ok(records) => {
                    store.delete(id, expected_modified_at_ms)?;
                    if records.profiles.contains_key(id) {
                        let _ = saved_hashes(store).and_then(|saved| {
                            self.imports.update(&saved, |records| {
                                if let Some(record) = records.profiles.get_mut(id) {
                                    record.deleted = true;
                                }
                                Ok(())
                            })
                        });
                    }
                }
                Err(_) => {
                    // Remembered first: a delete that recovery could not
                    // know about is refused.
                    self.imports.remember_deleted(id).map_err(|error| {
                        format!("the profile was not deleted: it could not be remembered for recovering the imported profile list ({error})")
                    })?;
                    store.delete(id, expected_modified_at_ms)?;
                }
            }
            Ok(())
        })
    }

    /// Deletes the profiles of a failed import. A record is dropped only
    /// when its profile was really deleted; the names of the profiles left
    /// behind (still flagged) are returned. Content hashes stay flagged.
    fn roll_back(&self, store: &store::ProfileStore, imported: &[ImportedProfile]) -> Vec<String> {
        let mut left = Vec::new();
        for done in imported {
            if store.delete(&done.stored.id, None).is_ok() {
                let _ = saved_hashes(store).and_then(|saved| {
                    self.imports.update(&saved, |records| {
                        records.profiles.remove(&done.stored.id);
                        Ok(())
                    })
                });
            } else {
                left.push(done.stored.profile.name.clone());
            }
        }
        left
    }

    /// Clears the dry-run-only flag of a saved profile and of its content
    /// hash. The backend recomputes the hash of the saved file (`load`
    /// always hashes what it parsed, never a cached value) and requires
    /// the one the confirmation showed, all under the store lock, so a
    /// version changed after the dialog opened is refused. The backend
    /// cannot prove a person clicked; the confirmation dialog is that step.
    fn allow_execution(&self, request: AllowRequest) -> Result<store::ProfileListing, String> {
        self.with_store(|store| {
            let stored = store.load(&request.id)?;
            if stored.content_hash != request.content_hash {
                return Err(store::CONFLICT_ERROR.into());
            }
            self.imports.update(&saved_hashes(store)?, |records| {
                let by_id = records
                    .profiles
                    .get_mut(&request.id)
                    .filter(|record| record.dry_run_only)
                    .map(|record| record.dry_run_only = false)
                    .is_some();
                let by_hash = records
                    .flagged_hashes
                    .remove(&stored.content_hash)
                    .is_some();
                if by_id || by_hash {
                    Ok(())
                } else {
                    Err("this profile is not an imported dry-run-only profile".into())
                }
            })?;
            Ok(())
        })?;
        self.list()
    }

    /// The recovery action "Mark all profiles as needing review": flags
    /// every saved profile (id and content hash) and rewrites the import
    /// list, keeping the readable records of a list that can still be read
    /// and the hashes and ids the sentinel keeps.
    fn mark_all_for_review(&self) -> Result<store::ProfileListing, String> {
        self.with_store(|store| {
            let mut records = self.imports.load_for_recovery()?;
            let listing = store.list()?;
            if listing.truncated {
                return Err(format!(
                    "more than {} profile files exist, so not every profile can be marked; remove extra files from the profile folder first",
                    store::MAX_PROFILES
                ));
            }
            for summary in &listing.profiles {
                let hash = if summary.binding_count == 0 {
                    ""
                } else {
                    summary.content_hash.as_str()
                };
                let _ = records.flag(&summary.id, hash, "review", now_ms());
            }
            // Profiles that cannot be read are marked by id.
            for unreadable in &listing.unreadable {
                if store::validate_id(&unreadable.id).is_ok() {
                    let _ = records.flag(&unreadable.id, "", "review", now_ms());
                }
            }
            self.imports.replace(&saved_hashes(store)?, &records)
        })?;
        self.list()
    }
}

/// The content hashes of every saved profile, which the import list never
/// releases or evicts while a profile has that content. A listing that
/// fails or cannot show every profile file refuses the write instead of
/// treating the missing profiles as absent.
fn saved_hashes(store: &store::ProfileStore) -> Result<BTreeSet<String>, String> {
    let listing = store.list()?;
    if listing.truncated {
        return Err(format!(
            "more than {} profile files exist, so not every profile can be checked; remove extra files from the profile folder first",
            store::MAX_PROFILES
        ));
    }
    Ok(listing
        .profiles
        .into_iter()
        .map(|summary| summary.content_hash)
        .collect())
}

/// The content hash to flag for `profile`: none for a profile with no
/// binding, which runs nothing and would otherwise match every blank
/// profile. Such a profile stays tracked by its id and by the drafts that
/// name it as their source.
fn tracked_hash(profile: &Profile, hash: &str) -> String {
    if profile.binding_count() == 0 {
        String::new()
    } else {
        hash.to_owned()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// `save_profile` arguments; unknown fields are refused, so a misspelt
/// `sourceProfileId` cannot silently drop the provenance hint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveProfileRequest {
    id: Option<String>,
    profile: Profile,
    #[serde(default)]
    expected_modified_at_ms: Option<u64>,
    /// The saved profile the draft was opened from, also after edits.
    #[serde(default)]
    source_profile_id: Option<String>,
    /// Keep the version this save replaces whatever the history spacing rule
    /// says (restoring an earlier version asks for it).
    #[serde(default)]
    keep_version: bool,
}

/// `export_profile` arguments.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExportRequest {
    profile: Profile,
    /// The saved profile the draft was opened from, also after edits.
    #[serde(default)]
    source_profile_id: Option<String>,
}

/// `allow_imported_profile_execution` arguments.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AllowRequest {
    id: String,
    /// `StoredProfile::content_hash` of the version the confirmation showed.
    content_hash: String,
}

/// One profile created by a `.mhz` import and what happened to each control.
#[derive(Debug, Serialize)]
struct ImportedProfile {
    stored: store::StoredProfile,
    report: mhz::ImportReport,
}

/// `import_mhz_profile` result.
#[derive(Debug, Serialize)]
struct MhzImport {
    profiles: Vec<ImportedProfile>,
    /// SHA-256 of the imported file, the only provenance kept besides time.
    source_sha256: String,
}

#[derive(Serialize)]
struct ProfileValidity {
    valid: bool,
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Result<store::AppBootstrap, BootstrapError> {
    state.bootstrap()
}

#[tauri::command]
fn list_profiles(state: State<'_, AppState>) -> Result<store::ProfileListing, String> {
    state.list()
}

#[tauri::command]
fn load_profile(state: State<'_, AppState>, id: String) -> Result<store::StoredProfile, String> {
    state.with_store(|store| store.load(&id))
}

// The editor's "Earlier versions" list: the versions automatic and manual
// saving keep for this profile, newest first. A version is restored by
// loading it and saving it as the profile, so conflict protection and the
// imported-content flags apply unchanged.
#[tauri::command]
fn profile_versions(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<store::ProfileVersion>, String> {
    state.with_store(move |store| store.versions(&id))
}

#[tauri::command]
fn load_profile_version(
    state: State<'_, AppState>,
    id: String,
    modified_at_ms: u64,
) -> Result<store::StoredProfile, String> {
    state.with_store(move |store| store.load_version(&id, modified_at_ms))
}

#[tauri::command]
fn save_profile(
    state: State<'_, AppState>,
    request: SaveProfileRequest,
) -> Result<store::StoredProfile, String> {
    replay::validate_profile(&request.profile)?;
    state.save_profile(request)
}

// Deleting keeps the profile's import record and content hash as tombstones,
// so its draft (still open in the editor) stays dry run only.
#[tauri::command]
fn delete_profile(
    state: State<'_, AppState>,
    id: String,
    expected_modified_at_ms: Option<u64>,
) -> Result<(), String> {
    state.delete_profile(&id, expected_modified_at_ms)
}

// Imports an official MasterHUB `.mhz` file (`mhz`). The webview sends the
// picked or dropped file's bytes as the raw request body; no path is passed.
// Each profile in it becomes a new, dry-run-only saved profile.
#[tauri::command(async)]
fn import_mhz_profile(
    state: State<'_, AppState>,
    request: tauri::ipc::Request<'_>,
) -> Result<MhzImport, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("the .mhz import expects the raw file bytes".into());
    };
    state.import_mhz(bytes, now_ms())
}

// The explicit "Allow this imported profile to execute actions" action. The
// UI calls it only after a confirmation that names the profile and lists its
// capabilities, naming the content hash of the saved version it showed.
#[tauri::command]
fn allow_imported_profile_execution(
    state: State<'_, AppState>,
    request: AllowRequest,
) -> Result<store::ProfileListing, String> {
    state.allow_execution(request)
}

// Recovery when the imported profile list is missing or damaged: every saved
// profile becomes dry run only until allowed again, one by one.
#[tauri::command]
fn mark_all_profiles_for_review(
    state: State<'_, AppState>,
) -> Result<store::ProfileListing, String> {
    state.mark_all_for_review()
}

#[tauri::command]
fn validate_profile(profile: Profile) -> Result<ProfileValidity, String> {
    replay::validate_profile(&profile)?;
    Ok(ProfileValidity { valid: true })
}

// Writes one profile as a local JSON file under `<app data>/exports/`.
#[tauri::command]
fn export_profile(
    state: State<'_, AppState>,
    request: ExportRequest,
) -> Result<export::ExportResult, String> {
    state.export_profile(request)
}

#[tauri::command]
fn replay_demo(profile: Profile, kind: replay::DemoKind) -> Result<replay::ReplayResult, String> {
    replay::replay_demo(profile, kind)
}

#[tauri::command]
fn preview_binding(
    profile: Profile,
    module: ServiceModule,
    control: u8,
    gesture: Gesture,
    serial_number: Option<String>,
    value: Option<InputValue>,
    dial_function: Option<usize>,
) -> Result<replay::ReplayEvent, String> {
    replay::preview_binding(
        profile,
        module,
        control,
        gesture,
        serial_number,
        value,
        dial_function,
    )
}

#[tauri::command]
fn preview_keypad(
    profile: Profile,
    path: Vec<String>,
    control: u8,
    serial_number: Option<String>,
) -> Result<replay::KeypadPreview, String> {
    replay::preview_keypad(profile, path, control, serial_number)
}

// Device feeds are read-only: enumeration and input reads only, never software
// mode, profiles, or actions. Enumeration and joining run off the main thread.
//
// `list_live_devices` enumerates on demand with its own registry rather than
// returning the watcher's latest snapshot, so it is never up to one poll
// interval stale and reports enumeration errors. Both share `live_devices`, so
// the list matches `device://inventory` for the same enumeration.
#[tauri::command(async)]
fn list_live_devices() -> Result<Vec<feed::LiveDevice>, String> {
    let registry = cooltek_sdk_masterhub::DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    Ok(feed::live_devices_with_host_positions(&registry.devices()))
}

#[tauri::command(async)]
fn start_feed(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source: feed::FeedSource,
) -> Result<String, String> {
    let feed_id = feed::feed_id(&source)?;
    // A running service owns its interface; the feed must not open it too.
    // The check and the registration are atomic with service starts, so the
    // feed never opens a path a service is starting on (its feed stop has
    // already run).
    let thread_id = feed_id.clone();
    let checked = source.clone();
    service::start_feed_checked(&state.services, &checked, || {
        state.feeds.start(feed_id, move |stop| {
            feed::run_feed(&thread_id, &source, stop, |message| {
                let emitted = match message {
                    feed::FeedMessage::Input(payload) => app.emit(feed::INPUT_EVENT, payload),
                    feed::FeedMessage::Status(payload) => app.emit(feed::STATUS_EVENT, payload),
                };
                if let Err(error) = emitted {
                    eprintln!("could not emit a device feed event: {error}");
                }
            })
        })
    })
}

#[tauri::command(async)]
fn stop_feed(state: State<'_, AppState>, feed_id: String) -> Result<(), String> {
    state.feeds.stop(&feed_id)
}

// Lighting writes (the Lighting page) go straight to the dials through the
// app's own HID handle, like the CLI's dial commands: a feature-report write
// needs no service session. A run that owns a dial refuses the write, so a
// live display run is never fought over the dial's mode.
#[tauri::command(async)]
fn set_dial_lighting(
    state: State<'_, AppState>,
    targets: Vec<String>,
    backlight: u8,
    colour: [u8; 3],
) -> Result<usize, String> {
    service::check_lighting_allowed(&state.services, &targets)?;
    let registry = cooltek_sdk_masterhub::DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    lighting::apply_dial_lighting(&registry, &targets, backlight, colour)
}

// Knob and fader lighting: one colour, effect and brightness for one LED or
// every LED of each target module, written as output reports.
#[tauri::command(async)]
fn set_led_lighting(
    state: State<'_, AppState>,
    targets: Vec<String>,
    led: Option<u8>,
    colour: [u8; 3],
    mode: u8,
    brightness: u8,
) -> Result<usize, String> {
    service::check_lighting_allowed(&state.services, &targets)?;
    let registry = cooltek_sdk_masterhub::DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    let look = lighting::LedLook {
        colour,
        mode,
        brightness,
    };
    lighting::apply_led_lighting(&registry, &targets, led, look)
}

#[tauri::command(async)]
fn reset_dial_lighting(state: State<'_, AppState>, targets: Vec<String>) -> Result<usize, String> {
    service::check_lighting_allowed(&state.services, &targets)?;
    let registry = cooltek_sdk_masterhub::DeviceRegistry::new()
        .map_err(|error| format!("could not enumerate HID devices: {error}"))?;
    lighting::reset_dial_lighting(&registry, &targets)
}

// Profile actions run only in a supervised `masterhub-service` child process,
// never in the app. A read-only feed on the same interface is stopped first
// so the service owns the device. The run is wrapped in a sleep/wake and
// reconnect session (`session.rs`). The result is the session id (the first
// service id, or `session:<uuid>` when that id still names an older session),
// or the service id of an unmanaged run.
// Once app exit began nothing starts, not even an unmanaged run. Only a
// missing session driver falls back to an unmanaged run; a driver thread that
// died refuses the start (`session::DRIVER_STOPPED`), since an unmanaged run
// would ignore sleep.
#[tauri::command(async)]
fn start_service(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: service::StartRequest,
) -> Result<String, String> {
    if let Some(started) = state.sessions.start(request.clone()) {
        return started;
    }
    if state.sessions.is_closed() || state.services.is_closed() {
        return Err(service::SHUTTING_DOWN.into());
    }
    if request.execute
        && let Some(reason) = state.execution_block(
            request.profile_id.as_deref(),
            request.source_profile_id.as_deref(),
            &request.profile,
        )
    {
        return Err(reason);
    }
    run_service(&app, &request, None)
}

/// Starts one service process; `sessions` also receives its exit.
fn run_service(
    app: &tauri::AppHandle,
    request: &service::StartRequest,
    sessions: Option<mpsc::Sender<session::Input>>,
) -> Result<String, String> {
    let handle = app.clone();
    // Profile key requests are asked for only for processes the session
    // driver starts, and only when the profile binds a `profile` action. A
    // driver start on a device it cannot identify still runs without a
    // session; the driver then ignores that process's requests.
    let profile_switch_requests =
        sessions.is_some() && service::uses_profile_action(&request.profile);
    // Display flags, only for a session-managed process and from
    // the settings its session started with (never the current setting). A
    // flat keypad profile is written as one page so its keys can be drawn;
    // the session keeps its own profile.
    let display = display::spawn_display(sessions.is_some(), request);
    let request = &match display.profile {
        Some(profile) => service::StartRequest {
            profile,
            ..request.clone()
        },
        None => request.clone(),
    };
    // One forwarder per spawn: nothing of this process reaches the driver
    // after its exit (see `SpawnForwarder`).
    let forwarder = sessions.map(session::SpawnForwarder::new);
    let emit: service::Emit = Arc::new(move |message| {
        if let Some(forwarder) = &forwarder {
            forwarder.forward(&message);
        }
        let emitted = match message {
            service::ServiceMessage::Record(payload) => handle.emit(service::RECORD_EVENT, payload),
            service::ServiceMessage::Status(payload) => handle.emit(service::STATUS_EVENT, payload),
        };
        if let Err(error) = emitted {
            eprintln!("could not emit a service event: {error}");
        }
    });
    let state = app.state::<AppState>();
    service::start_service(
        &state.services,
        request.clone(),
        profile_switch_requests,
        display.args,
        state.app_data.as_deref().map_err(Clone::clone),
        service_program(),
        |feed_id| state.feeds.stop(feed_id),
        emit,
    )
}

/// The session driver's side effects against the real services.
struct AppRunner {
    app: tauri::AppHandle,
    sessions: mpsc::Sender<session::Input>,
}

impl AppRunner {
    /// True when `service_id` is still running after a bounded wait. A
    /// crashed process emits `exited` just before its handle is marked done;
    /// letting it settle keeps a restart (or a user start right after the
    /// exit) from being refused as a duplicate.
    fn still_running(&self, service_id: &str) -> bool {
        let services = &self.app.state::<AppState>().services;
        let deadline = Instant::now() + Duration::from_millis(500);
        while services.is_running(service_id) {
            if Instant::now() >= deadline {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }
}

impl session::Runner for AppRunner {
    fn check(&mut self, request: &service::StartRequest) -> Result<(), String> {
        let id = service::service_id(&request.path_hex)?;
        if self.still_running(&id) {
            return Err(format!("service {id} is already running"));
        }
        service::check_request(request)
    }

    fn start(&mut self, request: &service::StartRequest) -> Result<String, String> {
        if let Ok(id) = service::service_id(&request.path_hex) {
            self.still_running(&id);
        }
        run_service(&self.app, request, Some(self.sessions.clone()))
    }

    fn stop(&mut self, service_id: &str) {
        let state = self.app.state::<AppState>();
        if let Err(error) = state.services.stop(service_id, service::STOP_GRACE) {
            eprintln!("could not stop {service_id}: {error}");
        }
    }

    fn stop_many(&mut self, service_ids: &[String]) {
        let state = self.app.state::<AppState>();
        state.services.stop_many(service_ids, service::STOP_GRACE);
    }

    fn closed(&self) -> bool {
        let state = self.app.state::<AppState>();
        state.sessions.is_closed() || state.services.is_closed()
    }

    fn emit(&mut self, state: &session::SessionState) {
        if let Err(error) = self.app.emit(session::STATE_EVENT, state) {
            eprintln!("could not emit a session event: {error}");
        }
    }

    fn enumerate(&mut self) -> Option<Vec<feed::LiveDevice>> {
        let registry = cooltek_sdk_masterhub::DeviceRegistry::new().ok()?;
        Some(feed::live_devices_with_host_positions(&registry.devices()))
    }

    // The listing comes from the store's summary cache (only changed files
    // are parsed), so a profile key reads and parses just the profile it
    // switches to (`load_saved`). Unreadable profiles are not listed, as
    // `switch_session_profile` would refuse them; a listed profile is valid
    // (it parsed and fits the store limit).
    fn saved_profiles(&mut self) -> Result<Vec<session::SavedEntry>, String> {
        let state = self.app.state::<AppState>();
        Ok(state
            .with_store(store::ProfileStore::list)?
            .profiles
            .into_iter()
            .map(|summary| session::SavedEntry {
                id: summary.id,
                name: summary.name,
                content_hash: summary.content_hash,
            })
            .collect())
    }

    fn load_saved(&mut self, id: &str) -> Result<Profile, String> {
        let stored = self
            .app
            .state::<AppState>()
            .with_store(|store| store.load(id))?;
        replay::validate_profile(&stored.profile)?;
        Ok(stored.profile)
    }

    fn display_settings(&mut self) -> display::DisplaySettings {
        self.app.state::<AppState>().display.for_new_session()
    }

    fn obs_launch(&mut self) -> Option<obs::ObsLaunch> {
        self.app.state::<AppState>().obs.for_new_session()
    }

    fn execution_guard(&mut self) -> import_flags::ExecutionGuard {
        self.app.state::<AppState>().execution_guard()
    }

    // The global default profile, else the last saved profile used on this
    // serial, each only while it still loads and validates, otherwise the
    // blank built-in profile (also when the record file is unreadable).
    // Whether it may execute is decided by the session driver.
    fn startup_profile(
        &mut self,
        module: ServiceModule,
        serial: Option<&str>,
    ) -> Option<(Option<String>, Profile)> {
        let state = self.app.state::<AppState>();
        Some(startup::choose(
            state.last_profiles.default_profile(),
            state.last_profiles.get(module, serial),
            |id| {
                let stored = state.with_store(|store| store.load(id))?;
                replay::validate_profile(&stored.profile)?;
                Ok(stored.profile)
            },
        ))
    }

    fn remember_profile(&mut self, module: ServiceModule, serial: Option<&str>, profile_id: &str) {
        let state = self.app.state::<AppState>();
        if let Err(error) = state.last_profiles.remember(module, serial, profile_id) {
            eprintln!("could not remember the module's last profile: {error}");
        }
    }
}

fn service_program() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().ok();
    // The override is a development aid; release builds never read it.
    let env_override = cfg!(debug_assertions)
        .then(|| std::env::var_os(service::BINARY_ENV))
        .flatten();
    service::resolve_service_binary(
        env_override.as_deref(),
        exe.as_deref().and_then(Path::parent),
        cfg!(debug_assertions),
    )
}

// Writes a local diagnostic bundle; nothing is uploaded. Probing is read-only
// and runs on a blocking worker; interfaces held by a running feed or service
// are checked right before each open and skipped.
#[tauri::command]
async fn create_diagnostics(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: diagnostics::DiagnosticsRequest,
) -> Result<diagnostics::DiagnosticsResult, String> {
    let app_data = state.app_data.clone()?;
    let running = diagnostics::Running {
        feeds: state.feeds.running_ids(),
        services: state.services.running_ids(),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let in_use = |path: &[u8]| {
            let hex = cooltek_sdk_cli::capture::encode_hex(path);
            state.feeds.is_running(&format!("live:{hex}"))
                || state.services.is_running(&format!("service:{hex}"))
        };
        diagnostics::create(
            request,
            &app_data,
            &running,
            &in_use,
            service_program(),
            &mut diagnostics::HidProbe::default(),
        )
    })
    .await
    .map_err(|error| format!("the diagnostics worker failed: {error}"))?
}

// Offline artwork preparation: decode, fit and encode on a blocking worker.
// Nothing is sent to a device or over the network. One job runs at a time.
#[tauri::command]
async fn prepare_artwork(
    state: State<'_, AppState>,
    request: artwork::PrepareRequest,
) -> Result<artwork::PrepareResult, String> {
    let _permit = state.artwork.lock().await;
    tauri::async_runtime::spawn_blocking(move || artwork::prepare_artwork(&request))
        .await
        .map_err(|error| format!("the artwork worker failed: {error}"))?
}

#[tauri::command]
async fn prepare_key_artwork(
    state: State<'_, AppState>,
    request: artwork::PrepareRequest,
) -> Result<cooltek_sdk_service::KeyArtwork, String> {
    let _permit = state.artwork.lock().await;
    tauri::async_runtime::spawn_blocking(move || artwork::prepare_key_artwork(&request))
        .await
        .map_err(|error| format!("the artwork worker failed: {error}"))?
}

// Writes the prepared JPEG under `<app data>/artwork/`; nothing is uploaded.
#[tauri::command]
async fn save_artwork(
    state: State<'_, AppState>,
    request: artwork::SaveRequest,
) -> Result<artwork::SaveResult, String> {
    let app_data = state.app_data.clone()?;
    let generated_at = export::now_seconds()?;
    tauri::async_runtime::spawn_blocking(move || {
        artwork::save_artwork(&request, &app_data, generated_at)
    })
    .await
    .map_err(|error| format!("the artwork worker failed: {error}"))?
}

// The installed app's icon for the key artwork suggestion of an Open
// application action; one job at a time, like the other artwork work.
#[tauri::command]
async fn app_icon(
    state: State<'_, AppState>,
    request: app_icon::AppIconRequest,
) -> Result<app_icon::AppIconResult, String> {
    let _permit = state.artwork.lock().await;
    tauri::async_runtime::spawn_blocking(move || app_icon::app_icon(&request))
        .await
        .map_err(|error| format!("the artwork worker failed: {error}"))?
}

// Stopping a managed service's current process (or its session id) ends the
// session, so it is not restarted.
#[tauri::command(async)]
fn stop_service(state: State<'_, AppState>, service_id: String) -> Result<(), String> {
    if state.sessions.stop(&service_id, false)? {
        return Ok(());
    }
    state.services.stop(&service_id, service::STOP_GRACE)
}

// Restarts a session waiting in `needs_confirmation` with its stored
// request. The UI calls it only after the user confirmed resuming actions,
// naming the `profile_generation` of the profile the dialog showed; a later
// profile switch refuses it with `session::PROFILE_CHANGED`.
#[tauri::command(async)]
fn resume_session(
    state: State<'_, AppState>,
    session_id: String,
    profile_generation: u64,
) -> Result<(), String> {
    state.sessions.resume(&session_id, profile_generation)
}

// Ends a session from any phase; unknown ids are ignored. A dead session
// driver is reported (`session::DRIVER_STOPPED`) instead of doing nothing.
#[tauri::command(async)]
fn stop_session(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    state.sessions.stop(&session_id, true).map(drop)
}

// The current state of every session, as its last `session://state` event.
// A window asks once it listens: a session started automatically when the
// app opened was announced before the webview existed, and a reload forgets.
#[tauri::command(async)]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<session::SessionState>, String> {
    state.sessions.sessions()
}

// Focused-app rules (`focus.rs`). Opt-in: nothing matches until enabled.
#[tauri::command]
fn list_focus_rules(state: State<'_, AppState>) -> Result<focus::FocusRules, String> {
    state.focus.list()
}

// Adds or edits one rule; its profile must be a readable saved profile.
#[tauri::command]
fn save_focus_rule(
    state: State<'_, AppState>,
    rule: focus::RuleInput,
) -> Result<focus::FocusRules, String> {
    state.with_store(|store| store.load(&rule.profile_id).map(drop))?;
    state.focus.update(|rules| focus::upsert(rules, rule))
}

#[tauri::command]
fn delete_focus_rule(state: State<'_, AppState>, id: String) -> Result<focus::FocusRules, String> {
    state.focus.update(|rules| focus::remove(rules, &id))
}

#[tauri::command]
fn set_focus_rules_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<focus::FocusRules, String> {
    state
        .focus
        .update(|rules| Ok(focus::FocusRules { enabled, ..rules }))
}

// Module control and the dial content (`display.rs`). App by default. A
// change applies at the next Start; sessions the user started keep the value
// they started with. Automatic sessions run only under App: a save, or a
// read that finds the file unreadable, lets the session driver end the ones
// the setting no longer allows.
#[tauri::command]
fn display_settings(state: State<'_, AppState>) -> Result<display::DisplaySettings, String> {
    let settings = state.display.get();
    if settings.is_err() {
        state.sessions.display_settings_changed();
    }
    settings
}

#[tauri::command]
fn set_display_settings(
    state: State<'_, AppState>,
    settings: display::DisplaySettings,
) -> Result<display::DisplaySettings, String> {
    let saved = state.display.set(settings);
    state.sessions.display_settings_changed();
    saved
}

/// `default_profile` result: the saved profile every automatic start uses
/// first, or null for none.
#[derive(Debug, PartialEq, Serialize)]
struct DefaultProfile {
    profile_id: Option<String>,
}

// The global default profile for automatic starts (`startup.rs`), stored as
// an id beside the last profile of each module. An unreadable file is an
// error here and means "no default" for automatic starts.
#[tauri::command]
fn default_profile(state: State<'_, AppState>) -> Result<DefaultProfile, String> {
    state
        .last_profiles
        .default_profile()
        .map(|profile_id| DefaultProfile { profile_id })
}

// Sets or (with null) clears it; the profile must be a readable saved
// profile. It applies at the next automatic start.
#[tauri::command]
fn set_default_profile(
    state: State<'_, AppState>,
    profile_id: Option<String>,
) -> Result<DefaultProfile, String> {
    if let Some(id) = &profile_id {
        let stored = state.with_store(|store| store.load(id))?;
        replay::validate_profile(&stored.profile)?;
    }
    state
        .last_profiles
        .set_default_profile(profile_id.as_deref())?;
    Ok(DefaultProfile { profile_id })
}

// The OBS WebSocket setting (`obs.rs`). Off by default; loopback endpoints
// only. A change applies at the next Start. The password is held in memory
// only and is never returned: the reply says only whether one is set.
#[tauri::command]
fn obs_settings(state: State<'_, AppState>) -> Result<obs::ObsSettingsView, String> {
    state.obs.get()
}

#[tauri::command]
fn set_obs_settings(
    state: State<'_, AppState>,
    settings: obs::ObsSettings,
) -> Result<obs::ObsSettingsView, String> {
    state.obs.set(settings)
}

#[tauri::command]
fn set_obs_password(
    state: State<'_, AppState>,
    password: Option<String>,
) -> Result<obs::ObsSettingsView, String> {
    state.obs.set_password(password)
}

// The OBS listings for the editor pickers: one short loopback connection per
// reply, using the saved endpoint and the in-memory password. Refused while
// OBS is off in Settings, with the reason the editor shows in its place. The
// name arguments are data: they travel only as JSON strings to OBS. The
// name bound is the profile's own limit, like the listings.

// "Test connection": probes the saved endpoint with the in-memory password and
// names the result. Sends no OBS request.
#[tauri::command(async)]
fn obs_status(state: State<'_, AppState>) -> Result<obs::ObsConnection, String> {
    let settings = state.obs.get()?;
    if !settings.settings.enabled {
        return Ok(obs::ObsConnection::describe(
            None,
            false,
            settings.settings.port,
        ));
    }
    let launch = state.obs.for_listing()?;
    let status = ObsClient::new(launch.endpoint, launch.password).probe();
    Ok(obs::ObsConnection::describe(
        Some(status),
        obs::obs_process_running(),
        settings.settings.port,
    ))
}

#[tauri::command(async)]
fn obs_catalog(state: State<'_, AppState>) -> Result<ObsCatalog, String> {
    let launch = state.obs.for_listing()?;
    ObsClient::new(launch.endpoint, launch.password).catalog()
}

#[tauri::command(async)]
fn obs_source_filters(state: State<'_, AppState>, source: String) -> Result<Vec<String>, String> {
    if source.len() > MAX_LISTED_NAME_BYTES {
        return Err(format!(
            "the source name is longer than {MAX_LISTED_NAME_BYTES} bytes"
        ));
    }
    let launch = state.obs.for_listing()?;
    ObsClient::new(launch.endpoint, launch.password).source_filters(&source)
}

#[tauri::command(async)]
fn obs_scene_sources(state: State<'_, AppState>, scene: String) -> Result<Vec<String>, String> {
    if scene.len() > MAX_LISTED_NAME_BYTES {
        return Err(format!(
            "the scene name is longer than {MAX_LISTED_NAME_BYTES} bytes"
        ));
    }
    let launch = state.obs.for_listing()?;
    ObsClient::new(launch.endpoint, launch.password).scene_sources(&scene)
}

// The last app other than OpenStreamHub that was frontmost.
#[tauri::command]
fn focus_current_app(state: State<'_, AppState>) -> focus::CurrentApp {
    state.focus.current_app()
}

/// `switch_session_profile` result: the saved profile the session now uses
/// and the session's new `profile_generation`, which a later
/// `resume_session` must name.
#[derive(Serialize)]
struct SwitchedProfile {
    #[serde(flatten)]
    stored: store::StoredProfile,
    profile_generation: u64,
}

// Restarts a managed session with a saved profile (a focused-app rule
// matched). The session safety rule applies: a dry run restarts at once, an
// executing session waits in `needs_confirmation` for `resume_session`.
#[tauri::command(async)]
fn switch_session_profile(
    state: State<'_, AppState>,
    session_id: String,
    profile_id: String,
) -> Result<SwitchedProfile, String> {
    let stored = state.with_store(|store| store.load(&profile_id))?;
    replay::validate_profile(&stored.profile)?;
    let profile_generation =
        state
            .sessions
            .switch_profile(&session_id, &stored.id, stored.profile.clone())?;
    Ok(SwitchedProfile {
        stored,
        profile_generation,
    })
}

// The profile a paused session holds for `profile_generation`, exactly as
// `resume_session` restarts it. The Resume dialog lists its capabilities
// after a switch whose reply carried no profile body (a profile key). A stale
// generation is refused with `session::PROFILE_CHANGED`.
#[tauri::command(async)]
fn session_profile(
    state: State<'_, AppState>,
    session_id: String,
    profile_generation: u64,
) -> Result<session::HeldProfile, String> {
    state
        .sessions
        .session_profile(&session_id, profile_generation)
}

/// `apply_saved_profile` result: the saved profile the sessions now use and,
/// per affected session, its new `profile_generation` and whether an
/// executing session paused for confirmation. A session with an `error`
/// keeps running its previous profile.
#[derive(Serialize)]
struct SavedProfileApplied {
    #[serde(flatten)]
    stored: store::StoredProfile,
    sessions: Vec<session::AppliedSession>,
}

// Applies a saved profile to every running session that holds it (the user
// saved the profile a run uses; the save path calls this). The swap keeps
// the session, its mode and its device: the process restarts with the saved
// content, a dry run at once, an executing session without a new
// confirmation only when the saved profile adds nothing the confirmed
// profile could not do. A refused session keeps its old profile, with the
// reason in its result.
#[tauri::command(async)]
fn apply_saved_profile(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<SavedProfileApplied, String> {
    let stored = state.with_store(|store| store.load(&profile_id))?;
    replay::validate_profile(&stored.profile)?;
    let sessions = state
        .sessions
        .apply_saved(&stored.id, stored.profile.clone())?;
    Ok(SavedProfileApplied { stored, sessions })
}

/// Matches a frontmost-app change and tells the frontend.
fn focus_changed(app: &tauri::AppHandle, bundle_id: Option<String>) {
    let state = app.state::<AppState>();
    let payload = state.focus.observe(bundle_id, |profile_id| {
        state
            .with_store(|store| store.load(profile_id).map(drop))
            .is_ok()
    });
    if let Err(error) = app.emit(focus::CHANGED_EVENT, payload) {
        eprintln!("could not emit a focus event: {error}");
    }
}

pub fn run() {
    // A second launch focuses the existing window instead of opening another
    // editor over the same profile files.
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(
        |app, _arguments, _working_directory| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        },
    ));
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::menu).on_menu_event(quit::on_menu_event);

    let app = builder
        .setup(|app| {
            let app_data = app
                .path()
                .app_data_dir()
                .map_err(|error| format!("could not resolve profile storage: {error}"));
            if let Ok(app_data) = &app_data {
                service::clean_stale_profiles(&service::profile_dir(app_data));
            }
            app.manage(AppState::open(app_data));
            let state = app.state::<AppState>();
            let sessions = state.sessions.sender();
            let runner = AppRunner {
                app: app.handle().clone(),
                sessions: sessions.clone(),
            };
            if let Err(error) = state.sessions.spawn(runner) {
                eprintln!("{error}");
            }
            // Sleep/wake events. A suspend waits (bounded by SUSPEND_WAIT)
            // for the driver to stop every service in parallel, so each can
            // restore hardware mode first.
            let power_events = sessions.clone();
            let sink: power::Sink = Arc::new(move |event| {
                let (done, finished) = mpsc::channel();
                let wait = event == power::PowerEvent::Suspend;
                let done = wait.then_some(done);
                if power_events
                    .send(session::Input::Power(event, done))
                    .is_ok()
                    && wait
                {
                    let _ = finished.recv_timeout(SUSPEND_WAIT);
                }
            });
            if let Err(error) = power::system_source().start(sink) {
                eprintln!("{error}");
            }
            // Hot-plug detection: the watcher emits `device://inventory` once
            // at start and whenever the input-capable device list changes.
            // Without it the app still works; devices can be listed on demand.
            let handle = app.handle().clone();
            let started = state.inventory.start(move |stop| {
                feed::watch_inventory(stop, feed::hid_inventory(), feed::wait, |payload| {
                    let _ = sessions.send(session::Input::Inventory(payload.devices.clone()));
                    if let Err(error) = handle.emit(feed::INVENTORY_EVENT, payload) {
                        eprintln!("could not emit the device inventory: {error}");
                    }
                })
            });
            if let Err(error) = started {
                eprintln!("{error}");
            }
            // Frontmost-app changes for the opt-in focused-app rules.
            let handle = app.handle().clone();
            let own_bundle_id = app.config().identifier.clone();
            if let Err(error) = focus::watch(own_bundle_id, move |bundle_id| {
                focus_changed(&handle, bundle_id);
            }) {
                eprintln!("{error}");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            list_profiles,
            load_profile,
            profile_versions,
            load_profile_version,
            save_profile,
            delete_profile,
            import_mhz_profile,
            allow_imported_profile_execution,
            mark_all_profiles_for_review,
            validate_profile,
            export_profile,
            replay_demo,
            preview_binding,
            preview_keypad,
            list_live_devices,
            start_feed,
            stop_feed,
            set_dial_lighting,
            set_led_lighting,
            reset_dial_lighting,
            start_service,
            stop_service,
            resume_session,
            stop_session,
            list_sessions,
            list_focus_rules,
            save_focus_rule,
            delete_focus_rule,
            set_focus_rules_enabled,
            focus_current_app,
            display_settings,
            set_display_settings,
            default_profile,
            set_default_profile,
            obs_settings,
            set_obs_settings,
            set_obs_password,
            obs_status,
            obs_catalog,
            obs_source_filters,
            obs_scene_sources,
            switch_session_profile,
            session_profile,
            apply_saved_profile,
            create_diagnostics,
            prepare_artwork,
            prepare_key_artwork,
            save_artwork,
            app_icon
        ])
        .build(tauri::generate_context!())
        .expect("could not start the OpenStreamHub desktop app");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            let state = app.state::<AppState>();
            // Sessions end first so nothing restarts while services stop.
            // Both refuse later starts, so a session still busy past the
            // wait cannot spawn an orphan after stop_all.
            state.sessions.shutdown(Duration::from_millis(500));
            // Bounded: SIGINT, up to STOP_GRACE, then SIGKILL; never joins.
            state.services.stop_all(service::STOP_GRACE);
            state.feeds.stop_all();
            state.inventory.stop();
            return;
        }
        // Only user/system exit requests (code None) are routed through the
        // draft guard. An explicit app.exit(code) proceeds as requested.
        if let tauri::RunEvent::ExitRequested {
            api, code: None, ..
        } = event
            && let Some(window) = app.get_webview_window("main")
        {
            // Tauri exit requests pass through the same draft guard as the
            // close button. Closing queues CloseRequested; the frontend can cancel
            // it or destroy the window after the user's decision. Once the
            // last window is destroyed, Tauri removes it before requesting
            // exit again, so that final request proceeds without a loop.
            api.prevent_exit();
            if let Err(error) = window.close() {
                eprintln!("could not request closing the OpenStreamHub window: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(app_data: Result<PathBuf, String>) -> AppState {
        AppState {
            store: Mutex::new(Err("could not open storage".into())),
            app_data,
            feeds: feed::Feeds::default(),
            inventory: feed::InventoryWatcher::default(),
            services: service::Services::default(),
            sessions: session::Manager::default(),
            focus: focus::FocusState::open(Err("no app data".into())),
            display: display::DisplayState::open(Err("no app data".into())),
            last_profiles: startup::LastProfiles::open(Err("no app data".into())),
            obs: obs::ObsState::open(Err("no app data".into())),
            imports: import_flags::ImportFlags::open(Err("no app data".into())),
            artwork: tauri::async_runtime::Mutex::new(()),
        }
    }

    #[test]
    fn app_state_reports_the_store_open_error_for_every_storage_operation() {
        let state = failed(Err("no app data".into()));
        let mut called = false;
        let result = state.with_store(|_| {
            called = true;
            Ok(())
        });
        assert_eq!(result.unwrap_err(), "could not open storage");
        assert!(!called);
        assert_eq!(
            state.with_store(|store| store.list()).unwrap_err(),
            "could not open storage"
        );
        let error = state.bootstrap().unwrap_err();
        assert_eq!(error.message, "could not open storage");
        assert_eq!(error.storage_path, None);
    }

    #[test]
    fn bootstrap_retry_reopens_storage_after_the_folder_is_fixed() {
        let directory = tempfile::tempdir().unwrap();
        let app_data = directory.path().join("dev.cooltek.openstreamhub");
        // A file where the app data folder belongs blocks opening storage.
        std::fs::write(&app_data, b"not a folder").unwrap();
        let state = AppState::open(Ok(app_data.clone()));
        let error = state.bootstrap().unwrap_err();
        assert!(error.message.contains("must be a directory"));
        assert_eq!(
            error.storage_path.as_deref(),
            Some(app_data.join("profiles").to_str().unwrap())
        );
        assert!(state.with_store(|store| store.list()).is_err());

        std::fs::remove_file(&app_data).unwrap();
        std::fs::create_dir(&app_data).unwrap();
        let bootstrap = state.bootstrap().unwrap();
        assert_eq!(bootstrap.profiles.len(), 1);
        assert!(state.with_store(|store| store.list()).is_ok());
    }

    /// Every command in `generate_handler!` must also be in the build.rs app
    /// manifest (which generates its permission file) and allowed by the main
    /// capability, or Tauri refuses it at runtime.
    #[test]
    fn invoke_handlers_build_manifest_and_capability_list_the_same_commands() {
        use std::collections::BTreeSet;
        fn quoted_or_bare(block: &str) -> BTreeSet<String> {
            block
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_owned())
                .filter(|item| !item.is_empty())
                .collect()
        }
        let source = include_str!("lib.rs");
        let needle = concat!("generate_", "handler![");
        let start = source.find(needle).unwrap() + needle.len();
        let handlers = quoted_or_bare(&source[start..start + source[start..].find(']').unwrap()]);
        let build = include_str!("../build.rs");
        let start = build.find(".commands(&[").unwrap() + ".commands(&[".len();
        let manifest = quoted_or_bare(&build[start..start + build[start..].find(']').unwrap()]);
        assert_eq!(handlers, manifest, "generate_handler! vs build.rs");
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/main.json")).unwrap();
        let allowed: BTreeSet<String> = capability["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|permission| permission.as_str())
            .filter_map(|permission| permission.strip_prefix("allow-"))
            .map(|permission| permission.replace('-', "_"))
            .collect();
        assert_eq!(
            handlers, allowed,
            "generate_handler! vs capabilities/main.json"
        );
        let generated = Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions/autogenerated");
        for command in &handlers {
            assert!(
                generated.join(format!("{command}.toml")).is_file(),
                "missing permissions/autogenerated/{command}.toml"
            );
        }
    }

    #[test]
    fn app_state_runs_operations_against_an_open_store() {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::open(Ok(directory.path().join("app")));
        let bootstrap = state.bootstrap().unwrap();
        assert_eq!(bootstrap.profiles.len(), 1);
    }

    fn open_state() -> (tempfile::TempDir, AppState) {
        let directory = tempfile::tempdir().unwrap();
        let state = AppState::open(Ok(directory.path().join("app")));
        state.bootstrap().unwrap();
        (directory, state)
    }

    fn imported_state() -> (tempfile::TempDir, AppState, store::StoredProfile) {
        let (directory, state) = open_state();
        let file = mhz::tests::mhz(&mhz::tests::full_profile());
        let stored = state
            .import_mhz(&file, 1)
            .unwrap()
            .profiles
            .remove(0)
            .stored;
        (directory, state, stored)
    }

    fn save(
        state: &AppState,
        id: Option<&str>,
        profile: &Profile,
        source: Option<&str>,
    ) -> Result<store::StoredProfile, String> {
        state.save_profile(SaveProfileRequest {
            id: id.map(str::to_owned),
            profile: profile.clone(),
            expected_modified_at_ms: None,
            source_profile_id: source.map(str::to_owned),
            keep_version: false,
        })
    }

    fn flagged(state: &AppState, id: &str) -> bool {
        state
            .list()
            .unwrap()
            .profiles
            .iter()
            .find(|summary| summary.id == id)
            .unwrap()
            .dry_run_only
    }

    fn refused() -> Option<String> {
        Some(import_flags::DRY_RUN_ONLY.to_string())
    }

    #[test]
    fn an_mhz_import_creates_new_dry_run_only_profiles_and_never_overwrites() {
        let (directory, state) = open_state();
        let existing = state.list().unwrap().profiles[0].clone();
        let existing_path = directory
            .path()
            .join(format!("app/profiles/{}.json", existing.id));
        let existing_bytes = std::fs::read(&existing_path).unwrap();
        let file = mhz::tests::mhz(&mhz::tests::full_profile());

        let first = state.import_mhz(&file, 42).unwrap();
        assert_eq!(first.profiles.len(), 1);
        assert_eq!(first.source_sha256, mhz::sha256_hex(&file));
        let imported = &first.profiles[0];
        assert_eq!(imported.stored.profile.name, "Desk setup (imported)");
        assert_eq!(imported.report.profile_name, "Desk setup (imported)");
        let second = state.import_mhz(&file, 43).unwrap();
        assert_eq!(
            second.profiles[0].stored.profile.name,
            "Desk setup (imported 2)"
        );
        let listing = state.list().unwrap();
        assert_eq!(listing.profiles.len(), 3);
        assert_eq!(listing.import_flags_error, None);
        for summary in &listing.profiles {
            assert_eq!(
                summary.dry_run_only,
                summary.id != existing.id,
                "{summary:?}"
            );
        }
        assert!(
            state
                .bootstrap()
                .unwrap()
                .profiles
                .iter()
                .any(|s| s.dry_run_only)
        );
        assert_eq!(std::fs::read(&existing_path).unwrap(), existing_bytes);
        let records = state.imports.load().unwrap();
        let record = &records.profiles[&imported.stored.id];
        assert_eq!(
            (
                record.source.as_str(),
                record.dry_run_only,
                record.imported_at_ms
            ),
            ("mhz", true, 42)
        );
        assert!(
            records
                .flagged_hashes
                .contains_key(&imported.stored.content_hash)
        );
        assert!(
            directory
                .path()
                .join("app/profiles")
                .join(import_flags::SENTINEL)
                .is_file()
        );
        let stored_text = std::fs::read_to_string(
            directory
                .path()
                .join(format!("app/profiles/{}.json", imported.stored.id)),
        )
        .unwrap()
            + &std::fs::read_to_string(directory.path().join("app/imported-profiles.json"))
                .unwrap();
        for secret in [
            mhz::tests::BASE_SERIAL,
            mhz::tests::KEYPAD_SERIAL,
            "someone",
        ] {
            assert!(!stored_text.contains(secret), "{secret} stored");
        }
    }

    #[test]
    fn a_rejected_mhz_file_leaves_no_profile_and_no_record() {
        let (directory, state) = open_state();
        for bad in [
            b"garbage".to_vec(),
            vec![0; mhz::archive::MAX_ARCHIVE_BYTES + 1],
            mhz::tests::zip(&[mhz::tests::item("profile1_A", b"{")]),
        ] {
            assert!(state.import_mhz(&bad, 1).is_err());
        }
        assert_eq!(state.list().unwrap().profiles.len(), 1);
        assert!(!directory.path().join("app/imported-profiles.json").exists());
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        let file = mhz::tests::mhz(&mhz::tests::full_profile());
        assert!(state.import_mhz(&file, 1).is_err());
        assert_eq!(state.list().unwrap().profiles.len(), 1);
    }

    #[test]
    fn deleting_the_open_profile_keeps_its_draft_dry_run_only_for_start_and_save() {
        let (_directory, state, imported) = imported_state();
        state
            .with_store(|store| store.delete(&imported.id, None))
            .unwrap();
        // The draft stays open with no saved id; its source id is gone.
        assert_eq!(
            state.execution_block(None, None, &imported.profile),
            refused()
        );
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &imported.profile),
            refused()
        );
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &edited),
            refused()
        );
        // Saving the draft again, with or without its source, stays flagged.
        let resaved = save(&state, None, &imported.profile, None).unwrap();
        assert!(flagged(&state, &resaved.id));
        let edited_saved = save(&state, None, &edited, Some(&imported.id)).unwrap();
        assert!(flagged(&state, &edited_saved.id));
        assert_eq!(
            state.execution_block(Some(&edited_saved.id), None, &edited),
            refused()
        );
    }

    #[test]
    fn copies_and_re_imported_exports_stay_dry_run_only_without_any_hint() {
        let (_directory, state, imported) = imported_state();
        // A JSON export re-imported: a new profile with a new name, no source.
        let mut exported = imported.profile.clone();
        exported.name = "Desk (imported 3)".into();
        let reimported = save(&state, None, &exported, None).unwrap();
        assert!(flagged(&state, &reimported.id));
        assert_eq!(
            state.execution_block(Some(&reimported.id), None, &reimported.profile),
            refused()
        );
        // A bogus source hint does not help either.
        let bogus = save(
            &state,
            None,
            &exported,
            Some(&uuid::Uuid::new_v4().to_string()),
        )
        .unwrap();
        assert!(flagged(&state, &bogus.id));
        // An edited draft is refused through its source and, once saved with
        // that source, its new content hash is flagged for later copies.
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        edited.name = "Edited".into();
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &edited),
            refused()
        );
        save(&state, None, &edited, Some(&imported.id)).unwrap();
        let copy_of_edit = save(&state, None, &edited, None).unwrap();
        assert!(flagged(&state, &copy_of_edit.id));
    }

    #[test]
    fn saving_flagged_content_over_an_unflagged_profile_flags_that_profile() {
        let (_directory, state, imported) = imported_state();
        let other = state
            .list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|summary| !summary.dry_run_only)
            .unwrap();
        let mut over = imported.profile.clone();
        over.name = "Everyday".into();
        let saved = save(&state, Some(&other.id), &over, Some(&other.id)).unwrap();
        assert_eq!(saved.id, other.id);
        assert!(flagged(&state, &other.id));
        assert_eq!(
            state.execution_block(Some(&other.id), Some(&other.id), &over),
            refused()
        );
        // An unrelated profile still runs.
        let plain = crate::test_profiles::folders();
        let fresh = save(&state, None, &plain, None).unwrap();
        assert!(!flagged(&state, &fresh.id));
        assert_eq!(
            state.execution_block(Some(&fresh.id), Some(&fresh.id), &plain),
            None
        );
    }

    #[test]
    fn allowing_needs_the_reviewed_content_hash_and_clears_id_and_hash() {
        let (_directory, state, imported) = imported_state();
        let copy = save(&state, None, &imported.profile, Some(&imported.id)).unwrap();
        let wrong = AllowRequest {
            id: imported.id.clone(),
            content_hash: "0".repeat(64),
        };
        assert_eq!(
            state.allow_execution(wrong).unwrap_err(),
            store::CONFLICT_ERROR
        );
        assert!(flagged(&state, &imported.id));
        let allow = || AllowRequest {
            id: imported.id.clone(),
            content_hash: imported.content_hash.clone(),
        };
        let listing = state.allow_execution(allow()).unwrap();
        assert!(
            !listing
                .profiles
                .iter()
                .find(|p| p.id == imported.id)
                .unwrap()
                .dry_run_only
        );
        assert_eq!(
            state.execution_block(Some(&imported.id), Some(&imported.id), &imported.profile),
            None
        );
        assert!(state.allow_execution(allow()).is_err());
        // The copy keeps its own id flag.
        assert!(flagged(&state, &copy.id));
        assert_eq!(
            state.execution_block(Some(&copy.id), None, &copy.profile),
            refused()
        );
    }

    #[test]
    fn a_missing_or_damaged_list_fails_closed_until_everything_is_marked_for_review() {
        let (directory, state, imported) = imported_state();
        let plain = crate::test_profiles::folders();
        let list = directory.path().join("app/imported-profiles.json");
        for damage in [None, Some(&b"{"[..])] {
            match damage {
                None => std::fs::remove_file(&list).unwrap(),
                Some(bytes) => std::fs::write(&list, bytes).unwrap(),
            }
            let blocked = state.execution_block(None, None, &plain).unwrap();
            assert!(blocked.starts_with("actions cannot run"), "{blocked}");
            let listing = state.list().unwrap();
            assert!(listing.import_flags_error.is_some());
            assert!(listing.profiles.iter().all(|summary| summary.dry_run_only));
            assert!(state.bootstrap().unwrap().import_flags_error.is_some());
            let listing = state.mark_all_for_review().unwrap();
            assert_eq!(listing.import_flags_error, None);
            assert!(listing.profiles.iter().all(|summary| summary.dry_run_only));
            assert_eq!(
                state.execution_block(Some(&imported.id), None, &imported.profile),
                refused()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_rollback_that_cannot_delete_keeps_the_record_and_reports_it() {
        use std::os::unix::fs::PermissionsExt;
        let (directory, state, imported) = imported_state();
        let profiles = directory.path().join("app/profiles");
        let done = vec![ImportedProfile {
            stored: imported.clone(),
            report: mhz::convert(&mhz::tests::mhz(&mhz::tests::full_profile()))
                .unwrap()
                .remove(0)
                .report,
        }];
        std::fs::set_permissions(&profiles, std::fs::Permissions::from_mode(0o500)).unwrap();
        let left = state
            .with_store(|store| Ok(state.roll_back(store, &done)))
            .unwrap();
        std::fs::set_permissions(&profiles, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(left, std::slice::from_ref(&imported.profile.name));
        assert!(state.imports.load().unwrap().profiles[&imported.id].dry_run_only);
        assert!(flagged(&state, &imported.id));
        // When the delete works, the record goes; the content hash stays.
        let left = state
            .with_store(|store| Ok(state.roll_back(store, &done)))
            .unwrap();
        assert!(left.is_empty());
        let records = state.imports.load().unwrap();
        assert!(!records.profiles.contains_key(&imported.id));
        assert!(records.flagged_hashes.contains_key(&imported.content_hash));
    }

    #[test]
    fn new_command_arguments_refuse_unknown_fields() {
        let profile = serde_json::to_value(crate::test_profiles::folders()).unwrap();
        let ok: SaveProfileRequest = serde_json::from_value(serde_json::json!({
            "id": null, "profile": profile, "expectedModifiedAtMs": null, "sourceProfileId": "x"
        }))
        .unwrap();
        assert_eq!(ok.source_profile_id.as_deref(), Some("x"));
        for typo in [
            "copyOf",
            "sourceProfileID",
            "source_profile_id",
            "expected_modified_at_ms",
        ] {
            let mut value = serde_json::json!({ "id": null, "profile": profile });
            value[typo] = serde_json::json!("x");
            assert!(
                serde_json::from_value::<SaveProfileRequest>(value).is_err(),
                "{typo}"
            );
        }
        assert!(
            serde_json::from_value::<AllowRequest>(
                serde_json::json!({"id": "a", "contentHash": "h"})
            )
            .is_ok()
        );
        assert!(
            serde_json::from_value::<AllowRequest>(
                serde_json::json!({"id": "a", "expectedModifiedAtMs": 1})
            )
            .is_err()
        );
    }

    #[test]
    fn an_exported_edit_of_a_flagged_draft_stays_flagged_when_imported_again() {
        let (_directory, state, imported) = imported_state();
        // Edited, unsaved, then exported: the file holds content nobody saved.
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        edited.name = "Edited".into();
        state
            .export_profile(ExportRequest {
                profile: edited.clone(),
                source_profile_id: Some(imported.id.clone()),
            })
            .unwrap();
        // Imported again as JSON: a new profile with no source.
        edited.name = "Edited (imported)".into();
        let reimported = save(&state, None, &edited, None).unwrap();
        assert!(flagged(&state, &reimported.id));
        assert_eq!(state.execution_block(None, None, &edited), refused());
    }

    #[test]
    fn a_failed_save_over_an_unflagged_profile_does_not_flag_it() {
        let (_directory, state, imported) = imported_state();
        let other = state
            .list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|summary| !summary.dry_run_only)
            .unwrap();
        let error = state
            .save_profile(SaveProfileRequest {
                id: Some(other.id.clone()),
                profile: imported.profile.clone(),
                expected_modified_at_ms: Some(other.modified_at_ms + 1),
                source_profile_id: Some(other.id.clone()),
                keep_version: false,
            })
            .unwrap_err();
        assert_eq!(error, store::CONFLICT_ERROR);
        assert!(!flagged(&state, &other.id));
        assert!(
            !state
                .imports
                .load()
                .unwrap()
                .profiles
                .contains_key(&other.id)
        );
    }

    #[test]
    fn marking_everything_for_review_keeps_hashes_of_deleted_imports() {
        let (directory, state, imported) = imported_state();
        state
            .with_store(|store| store.delete(&imported.id, None))
            .unwrap();
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        state.mark_all_for_review().unwrap();
        // The deleted profile's open draft, saved again, is still dry run only.
        assert_eq!(
            state.execution_block(None, None, &imported.profile),
            refused()
        );
        let resaved = save(&state, None, &imported.profile, None).unwrap();
        assert!(flagged(&state, &resaved.id));
    }

    #[test]
    fn r4_compaction_never_unflags_a_live_profile_and_refuses_an_over_cap_write() {
        let (_directory, state, imported) = imported_state();
        // More than 1,024 tombstoned flagged rounds and more than 4,096 pinned
        // export fingerprints, all newer than the import.
        let mut records = state.imports.load().unwrap();
        for index in 0..1100u64 {
            let id = uuid::Uuid::new_v4().to_string();
            let _ = records.flag(
                &id,
                &format!("{:064x}", index + 1),
                "derived",
                1_000 + index,
            );
            records.profiles.get_mut(&id).unwrap().deleted = true;
        }
        for index in 0..4200u64 {
            records.flag_hash(&format!("{:064x}", 100_000 + index), true);
        }
        state
            .imports
            .replace(&state.with_store(saved_hashes).unwrap(), &records)
            .unwrap();
        let records = state.imports.load().unwrap();
        assert!(records.profiles.len() <= import_flags::MAX_RECORDS);
        assert!(records.flagged_hashes.len() <= import_flags::MAX_HASHES);
        // The live import stays blocked: listed, Start, a hint-less copy, an
        // edited draft with its source.
        assert!(flagged(&state, &imported.id));
        assert_eq!(
            state.execution_block(Some(&imported.id), None, &imported.profile),
            refused()
        );
        assert_eq!(
            state.execution_block(None, None, &imported.profile),
            refused()
        );
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &edited),
            refused()
        );
        let copy = save(&state, None, &imported.profile, None).unwrap();
        assert!(flagged(&state, &copy.id));
        // More live records than the cap: the write is refused, nothing cleared.
        let mut over = state.imports.load().unwrap();
        for index in 0..=import_flags::MAX_RECORDS as u64 {
            over.flag(
                &uuid::Uuid::new_v4().to_string(),
                &format!("{:064x}", 900_000 + index),
                "derived",
                5_000 + index,
            );
        }
        assert!(
            state
                .imports
                .replace(&state.with_store(saved_hashes).unwrap(), &over)
                .is_err()
        );
        assert!(flagged(&state, &imported.id));
        assert_eq!(
            state.execution_block(None, None, &imported.profile),
            refused()
        );
        // Deleting a saved profile turns its record into a tombstone.
        state.delete_profile(&copy.id, None).unwrap();
        assert!(state.imports.load().unwrap().profiles[&copy.id].deleted);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn r4_a_failed_save_keeps_the_fingerprint_of_the_version_on_disk() {
        let (directory, state, imported) = imported_state();
        let path = directory
            .path()
            .join(format!("app/profiles/{}.json", imported.id));
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        let lock = |flag: &str| {
            assert!(
                std::process::Command::new("chflags")
                    .arg(flag)
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        };
        lock("uchg");
        let result = save(&state, Some(&imported.id), &edited, Some(&imported.id));
        lock("nouchg");
        assert!(result.is_err());
        // A hint-less copy of the version still on disk stays dry run only.
        assert_eq!(
            state.execution_block(None, None, &imported.profile),
            refused()
        );
        let copy = save(&state, None, &imported.profile, None).unwrap();
        assert!(flagged(&state, &copy.id));
    }

    #[test]
    fn r4_saves_that_could_derive_from_flagged_content_are_refused_while_the_list_is_unreadable() {
        let (directory, state, imported) = imported_state();
        let other = state
            .list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|summary| !summary.dry_run_only)
            .unwrap();
        let list = directory.path().join("app/imported-profiles.json");
        let good = std::fs::read(&list).unwrap();
        std::fs::write(&list, b"{").unwrap();
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        assert!(save(&state, Some(&other.id), &edited, Some(&other.id)).is_err());
        assert!(save(&state, None, &edited, Some(&imported.id)).is_err());
        assert!(save(&state, Some(&imported.id), &edited, Some(&imported.id)).is_err());
        // A brand-new profile without a source can still be saved; content
        // matching a flagged fingerprint is caught by content once the list
        // is readable again.
        let fresh = save(&state, None, &imported.profile, None).unwrap();
        std::fs::write(&list, good).unwrap();
        assert!(flagged(&state, &fresh.id));
        assert_eq!(
            state.execution_block(Some(&fresh.id), None, &fresh.profile),
            refused()
        );
    }

    fn unflagged(state: &AppState) -> store::ProfileSummary {
        state
            .list()
            .unwrap()
            .profiles
            .into_iter()
            .find(|summary| !summary.dry_run_only)
            .unwrap()
    }

    #[test]
    fn r5_a_copy_saved_without_a_record_stays_flagged_when_the_original_is_edited() {
        let (directory, state, imported) = imported_state();
        let list = directory.path().join("app/imported-profiles.json");
        let good = std::fs::read(&list).unwrap();
        std::fs::write(&list, b"{").unwrap();
        let copy = save(&state, None, &imported.profile, None).unwrap();
        std::fs::write(&list, good).unwrap();
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        save(&state, Some(&imported.id), &edited, Some(&imported.id)).unwrap();
        assert!(flagged(&state, &copy.id));
        assert_eq!(
            state.execution_block(Some(&copy.id), Some(&copy.id), &copy.profile),
            refused()
        );
    }

    #[test]
    fn r5_a_save_that_reached_the_disk_keeps_its_flag_when_the_flush_fails() {
        let (_directory, state, imported) = imported_state();
        let other = unflagged(&state);
        let mut over = imported.profile.clone();
        over.name = "Everyday".into();
        store::fault::SYNC.with(|fault| fault.set(true));
        let result = save(&state, Some(&other.id), &over, Some(&other.id));
        store::fault::SYNC.with(|fault| fault.set(false));
        assert!(result.is_err());
        // The new content is on disk, so the target stays flagged by id: an
        // edit of it (no other hint) is still dry run only.
        let mut edited = over.clone();
        edited.bindings.clear();
        let saved = save(&state, Some(&other.id), &edited, Some(&other.id)).unwrap();
        assert!(flagged(&state, &saved.id));
    }

    #[test]
    fn r5_recovery_restores_the_ids_of_deleted_imports() {
        let (directory, state, imported) = imported_state();
        state.delete_profile(&imported.id, None).unwrap();
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        state.mark_all_for_review().unwrap();
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &edited),
            refused()
        );
    }

    #[test]
    fn r5_a_delete_while_the_list_is_unreadable_is_remembered_for_recovery() {
        let (directory, state, imported) = imported_state();
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        state.delete_profile(&imported.id, None).unwrap();
        state.mark_all_for_review().unwrap();
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &edited),
            refused()
        );
    }

    #[test]
    fn r5_a_failed_import_leaves_no_live_record_for_a_missing_profile() {
        let (directory, state) = open_state();
        let file = mhz::tests::mhz(&mhz::tests::full_profile());
        store::fault::WRITE.with(|fault| fault.set(true));
        let result = state.import_mhz(&file, 1);
        store::fault::WRITE.with(|fault| fault.set(false));
        assert!(result.is_err());
        let profiles = directory.path().join("app/profiles");
        for (id, record) in state.imports.load().unwrap().profiles {
            assert!(
                record.deleted || profiles.join(format!("{id}.json")).is_file(),
                "live record for missing {id}"
            );
        }
    }

    #[test]
    fn r6_a_recordless_flagged_copy_stays_flagged_when_edited() {
        let (directory, state, imported) = imported_state();
        let list = directory.path().join("app/imported-profiles.json");
        let good = std::fs::read(&list).unwrap();
        std::fs::write(&list, b"{").unwrap();
        let copy = save(&state, None, &imported.profile, None).unwrap();
        std::fs::write(&list, good).unwrap();
        let mut edited = copy.profile.clone();
        edited.bindings.clear();
        // An unsaved edit of the copy (the copy is its source) is blocked.
        assert_eq!(
            state.execution_block(None, Some(&copy.id), &edited),
            refused()
        );
        // Saving the edit keeps the copy dry run only.
        let saved = save(&state, Some(&copy.id), &edited, Some(&copy.id)).unwrap();
        assert!(flagged(&state, &saved.id));
        assert_eq!(
            state.execution_block(Some(&saved.id), Some(&saved.id), &saved.profile),
            refused()
        );
    }

    fn parses() -> usize {
        store::fault::PARSES.with(std::cell::Cell::get)
    }

    #[test]
    fn r6_listing_and_flagged_saves_do_not_reparse_every_profile() {
        let (_directory, state, imported) = imported_state();
        for index in 0..20 {
            let mut profile = crate::test_profiles::folders();
            profile.name = format!("P{index}");
            profile.bindings[0].action = cooltek_sdk_service::Action::Volume {
                delta: index as i16 + 1,
            };
            save(&state, None, &profile, None).unwrap();
        }
        state.list().unwrap();
        let before = parses();
        state.list().unwrap();
        assert_eq!(parses() - before, 0, "an unchanged listing parses nothing");
        let mut edited = imported.profile.clone();
        edited.bindings.clear();
        let before = parses();
        save(&state, Some(&imported.id), &edited, Some(&imported.id)).unwrap();
        assert!(parses() - before <= 5, "{}", parses() - before);
        let before = parses();
        state.mark_all_for_review().unwrap();
        assert!(parses() - before <= 2, "{}", parses() - before);
    }

    #[test]
    fn r6_mark_all_refuses_when_not_every_profile_can_be_listed() {
        let (directory, state, _imported) = imported_state();
        let profiles = directory.path().join("app/profiles");
        let bytes = serde_json::to_vec(&crate::test_profiles::folders()).unwrap();
        for _ in 0..store::MAX_PROFILES {
            std::fs::write(
                profiles.join(format!("{}.json", uuid::Uuid::new_v4())),
                &bytes,
            )
            .unwrap();
        }
        let error = state.mark_all_for_review().unwrap_err();
        assert!(error.contains("profile files"), "{error}");
    }

    #[test]
    fn r6_an_import_whose_flush_fails_leaves_no_unreported_profile() {
        let (_directory, state) = open_state();
        let before = state.list().unwrap().profiles.len();
        let file = mhz::tests::mhz(&mhz::tests::full_profile());
        store::fault::SYNC.with(|fault| fault.set(true));
        let result = state.import_mhz(&file, 1);
        store::fault::SYNC.with(|fault| fault.set(false));
        let error = result.unwrap_err();
        let after = state.list().unwrap().profiles;
        assert!(
            after.len() == before || after.iter().any(|p| error.contains(&p.name)),
            "{error}: {after:?}"
        );
    }

    #[test]
    fn r6_recovery_reports_an_unreadable_marker_and_deletes_refuse_without_one() {
        let (directory, state, imported) = imported_state();
        let marker = directory
            .path()
            .join("app/profiles")
            .join(import_flags::SENTINEL);
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        // An oversized marker cannot be read: recovery says so.
        std::fs::write(&marker, vec![b'x'; 2 * 1024 * 1024]).unwrap();
        let error = state.mark_all_for_review().unwrap_err();
        assert!(error.contains("marker"), "{error}");
        // A delete that cannot be remembered for recovery is refused.
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(state.delete_profile(&imported.id, None).is_err());
        assert!(state.with_store(|store| store.load(&imported.id)).is_ok());
    }

    #[test]
    fn r7_allow_recomputes_the_hash_of_a_same_stamp_rewrite() {
        let (directory, state, imported) = imported_state();
        let path = directory
            .path()
            .join(format!("app/profiles/{}.json", imported.id));
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"delta\": -5"));
        // Same length, same modification time, different content.
        std::fs::write(&path, text.replacen("\"delta\": -5", "\"delta\": -6", 1)).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(before))
            .unwrap();
        state.list().unwrap();
        let error = state
            .allow_execution(AllowRequest {
                id: imported.id.clone(),
                content_hash: imported.content_hash.clone(),
            })
            .unwrap_err();
        assert_eq!(error, store::CONFLICT_ERROR);
        let rewritten = state.with_store(|store| store.load(&imported.id)).unwrap();
        assert_ne!(rewritten.content_hash, imported.content_hash);
        assert_eq!(
            state.execution_block(Some(&imported.id), Some(&imported.id), &rewritten.profile),
            refused()
        );
    }

    #[test]
    fn r7_an_import_that_binds_nothing_never_flags_blank_profiles() {
        let (_directory, state) = open_state();
        let source = serde_json::json!({
            "profileName": "Nothing",
            "knobsList": {"MHKBFAKE0003": {"rotateR": {"btnFunc": "timer", "left": {}, "right": {}}}}
        });
        let imported = state
            .import_mhz(&mhz::tests::mhz(&source), 1)
            .unwrap()
            .profiles
            .remove(0)
            .stored;
        assert_eq!(imported.profile.binding_count(), 0);
        assert!(flagged(&state, &imported.id));
        let blank = Profile {
            schema_version: 1,
            name: "New profile".into(),
            bindings: Vec::new(),
            keypad: None,
            dial_functions: None,
            dial_display: None,
        };
        let saved = save(&state, None, &blank, None).unwrap();
        assert!(!flagged(&state, &saved.id));
        assert_eq!(state.execution_block(None, None, &saved.profile), None);
        // Edits of the import itself stay tracked by id and source.
        assert_eq!(
            state.execution_block(None, Some(&imported.id), &saved.profile),
            refused()
        );
    }

    #[test]
    fn r7_deleting_drops_the_cached_summary() {
        let (_directory, state, imported) = imported_state();
        state.list().unwrap();
        assert!(
            state
                .with_store(|store| Ok(store.cached_ids()))
                .unwrap()
                .contains(&imported.id)
        );
        state.delete_profile(&imported.id, None).unwrap();
        assert!(
            !state
                .with_store(|store| Ok(store.cached_ids()))
                .unwrap()
                .contains(&imported.id)
        );
    }

    #[test]
    fn r7_a_delete_that_cannot_be_remembered_is_refused() {
        let (directory, state, imported) = imported_state();
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        std::fs::remove_file(
            directory
                .path()
                .join("app/profiles")
                .join(import_flags::SENTINEL),
        )
        .unwrap();
        let error = state.delete_profile(&imported.id, None).unwrap_err();
        assert!(error.contains("not deleted"), "{error}");
        assert!(state.with_store(|store| store.load(&imported.id)).is_ok());
    }

    #[test]
    fn r7_an_unreadable_marker_explains_how_to_recover_by_hand() {
        let (directory, state, _imported) = imported_state();
        std::fs::write(directory.path().join("app/imported-profiles.json"), b"{").unwrap();
        std::fs::write(
            directory
                .path()
                .join("app/profiles")
                .join(import_flags::SENTINEL),
            vec![b'x'; 2 * 1024 * 1024],
        )
        .unwrap();
        let error = state.mark_all_for_review().unwrap_err();
        assert!(error.contains("empty file"), "{error}");
    }
}
