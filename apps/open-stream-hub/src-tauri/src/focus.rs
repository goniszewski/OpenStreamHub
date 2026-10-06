//! Focused-app profile rules.
//!
//! Rules map an app bundle id to a saved profile id. They live beside the
//! profiles in `<app data>/focus-rules.json`, never inside a profile, so the
//! profile schema is unchanged. The file uses the same bounded, symlink-safe
//! read and the same atomic 0600 write as the profile store.
//!
//! The feature is opt-in: `enabled` is false until the user turns it on.
//! The watcher reports every frontmost-app change; `match_rule` picks the
//! profile, and the frontend asks the session manager to switch the active
//! sessions to it (`session::Driver::switch_profile`). That switch keeps the
//! session safety rule: an executing session waits for the user's
//! confirmation before actions run with the new profile.
//!
//! macOS observes `NSWorkspaceDidActivateApplicationNotification` (no
//! polling) on the main thread and hands the bundle id to a worker thread.
//! Other platforms have no watcher; the rules can still be edited.
//! See `docs/masterhub-focus-rules.md`.

use crate::store;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread,
};
use uuid::Uuid;

pub const CHANGED_EVENT: &str = "focus://changed";
pub const MAX_RULES: usize = 64;
/// Longest accepted bundle id (`CFBundleIdentifier` has no documented limit;
/// real ids are far shorter).
pub const MAX_BUNDLE_ID: usize = 255;
const MAX_FILE_BYTES: usize = 64 * 1024;
const FILE_NAME: &str = "focus-rules.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusRule {
    pub id: String,
    pub app_bundle_id: String,
    pub profile_id: String,
}

/// The stored rules and the single opt-in switch. Rules are evaluated in
/// list order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FocusRules {
    pub enabled: bool,
    pub rules: Vec<FocusRule>,
}

/// `save_focus_rule` argument: no `id` adds a rule, an existing `id` edits it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleInput {
    pub id: Option<String>,
    pub app_bundle_id: String,
    pub profile_id: String,
}

/// Payload of `focus://changed`. `rule_id` and `profile_id` are set only when
/// the rules are enabled and one matches a profile that still exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FocusChanged {
    pub bundle_id: Option<String>,
    pub rule_id: Option<String>,
    pub profile_id: Option<String>,
}

/// `focus_current_app` result: the last app other than OpenStreamHub that was
/// frontmost, for the "Use current app" button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CurrentApp {
    pub supported: bool,
    pub bundle_id: Option<String>,
}

/// Trims and checks a bundle id: 1–255 ASCII letters, digits, `.`, `-` or
/// `_`, starting with a letter or digit.
pub fn normalize_bundle_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("enter an app bundle id, for example com.apple.Safari".into());
    }
    if value.len() > MAX_BUNDLE_ID {
        return Err(format!(
            "an app bundle id has at most {MAX_BUNDLE_ID} characters"
        ));
    }
    let valid = value.bytes().enumerate().all(|(index, byte)| {
        byte.is_ascii_alphanumeric() || (index > 0 && matches!(byte, b'.' | b'-' | b'_'))
    });
    if !valid {
        return Err("an app bundle id uses only letters, digits, '.', '-' and '_', and starts with a letter or digit".into());
    }
    Ok(value.to_owned())
}

fn validate_rule_id(id: &str) -> Result<(), String> {
    match Uuid::parse_str(id) {
        Ok(parsed) if parsed.to_string() == id => Ok(()),
        _ => Err("rule ID must be a canonical lowercase UUID".into()),
    }
}

/// Checks a whole rule set, as loaded or about to be saved.
pub fn validate(rules: &FocusRules) -> Result<(), String> {
    if rules.rules.len() > MAX_RULES {
        return Err(format!("at most {MAX_RULES} focus rules are allowed"));
    }
    for (index, rule) in rules.rules.iter().enumerate() {
        validate_rule_id(&rule.id)?;
        if rules.rules[..index].iter().any(|other| other.id == rule.id) {
            return Err("focus rule IDs must be unique".into());
        }
        if normalize_bundle_id(&rule.app_bundle_id)? != rule.app_bundle_id {
            return Err("an app bundle id must not have surrounding spaces".into());
        }
        store::validate_id(&rule.profile_id)?;
    }
    Ok(())
}

/// The rule for the focused app: the first rule, in list order, whose bundle
/// id equals `bundle_id` (ignoring ASCII case) and whose profile still exists.
/// Nothing matches while the rules are disabled or without a bundle id.
pub fn match_rule<'a>(
    rules: &'a FocusRules,
    bundle_id: Option<&str>,
    profile_exists: impl Fn(&str) -> bool,
) -> Option<&'a FocusRule> {
    let bundle_id = bundle_id?;
    if !rules.enabled {
        return None;
    }
    rules.rules.iter().find(|rule| {
        rule.app_bundle_id.eq_ignore_ascii_case(bundle_id) && profile_exists(&rule.profile_id)
    })
}

/// Adds a rule (new id) or edits the rule with `input.id` in place.
pub fn upsert(mut rules: FocusRules, input: RuleInput) -> Result<FocusRules, String> {
    let app_bundle_id = normalize_bundle_id(&input.app_bundle_id)?;
    store::validate_id(&input.profile_id)?;
    match input.id {
        Some(id) => {
            let rule = rules
                .rules
                .iter_mut()
                .find(|rule| rule.id == id)
                .ok_or("this focus rule no longer exists; reload the rules")?;
            rule.app_bundle_id = app_bundle_id;
            rule.profile_id = input.profile_id;
        }
        None => {
            if rules.rules.len() >= MAX_RULES {
                return Err(format!(
                    "at most {MAX_RULES} focus rules are allowed; delete one before adding another"
                ));
            }
            rules.rules.push(FocusRule {
                id: Uuid::new_v4().to_string(),
                app_bundle_id,
                profile_id: input.profile_id,
            });
        }
    }
    validate(&rules)?;
    Ok(rules)
}

/// Removes a rule; an unknown id is an error so a stale view is noticed.
pub fn remove(mut rules: FocusRules, id: &str) -> Result<FocusRules, String> {
    let before = rules.rules.len();
    rules.rules.retain(|rule| rule.id != id);
    if rules.rules.len() == before {
        return Err("this focus rule no longer exists; reload the rules".into());
    }
    Ok(rules)
}

/// `focus-rules.json` inside the app data folder.
pub struct RuleFile {
    directory: PathBuf,
}

impl RuleFile {
    pub fn new(app_data: &Path) -> Self {
        Self {
            directory: app_data.to_path_buf(),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join(FILE_NAME)
    }

    /// A missing file means no rules and the feature off. A damaged file is
    /// an error and is never overwritten, like an unreadable profile.
    pub fn load(&self) -> Result<FocusRules, String> {
        let Some((bytes, _)) =
            store::read_private_file(&self.path(), MAX_FILE_BYTES, "focus rules file")?
        else {
            return Ok(FocusRules::default());
        };
        let rules: FocusRules = serde_json::from_slice(&bytes).map_err(|error| {
            format!(
                "focus rules in {} are unreadable ({error}); fix or remove the file",
                self.path().display()
            )
        })?;
        validate(&rules)?;
        Ok(rules)
    }

    pub fn save(&self, rules: &FocusRules) -> Result<(), String> {
        validate(rules)?;
        match std::fs::symlink_metadata(&self.directory) {
            Ok(metadata) if metadata.file_type().is_dir() => {}
            Ok(_) => {
                return Err(
                    "the application data folder must be a directory, not a symbolic link".into(),
                );
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect the application data folder: {error}"
                ));
            }
        }
        let bytes = serde_json::to_vec_pretty(rules)
            .map_err(|error| format!("could not encode focus rules: {error}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("encoded focus rules exceed the size limit".into());
        }
        store::write_private_file(&self.directory, &self.path(), &bytes, "focus rules file")
    }
}

/// Rule storage plus what the watcher needs: a copy of the rules (`None` when
/// the file could not be read) and the last other frontmost app.
pub struct FocusState {
    file: Result<RuleFile, String>,
    /// Serializes read-modify-write and holds the watcher's copy.
    rules: Mutex<Option<FocusRules>>,
    current_app: Mutex<Option<String>>,
}

impl FocusState {
    /// Also deletes stale `.<uuid>.tmp` files a crashed rules save left in
    /// the app data folder. Only that exact name shape is touched there, since
    /// the folder also holds other app files.
    pub fn open(app_data: Result<&Path, String>) -> Self {
        if let Ok(app_data) = &app_data {
            store::remove_stale_temporaries(app_data, true);
        }
        let file = app_data.map(RuleFile::new);
        let rules = file.as_ref().ok().and_then(|file| file.load().ok());
        Self {
            file,
            rules: Mutex::new(rules),
            current_app: Mutex::new(None),
        }
    }

    pub fn list(&self) -> Result<FocusRules, String> {
        self.update(Ok)
    }

    /// Loads the file, applies `change`, saves a changed result and refreshes
    /// the watcher's copy.
    pub fn update(
        &self,
        change: impl FnOnce(FocusRules) -> Result<FocusRules, String>,
    ) -> Result<FocusRules, String> {
        let file = self.file.as_ref().map_err(Clone::clone)?;
        let mut cached = self
            .rules
            .lock()
            .map_err(|_| "focus rules lock is unavailable".to_string())?;
        let current = match file.load() {
            Ok(current) => current,
            Err(error) => {
                *cached = None;
                return Err(error);
            }
        };
        let next = change(current.clone())?;
        if next != current {
            file.save(&next)?;
        }
        *cached = Some(next.clone());
        Ok(next)
    }

    /// Records a frontmost-app change and matches it against the rules.
    pub fn observe(
        &self,
        bundle_id: Option<String>,
        profile_exists: impl Fn(&str) -> bool,
    ) -> FocusChanged {
        if let (Some(bundle_id), Ok(mut current)) = (&bundle_id, self.current_app.lock()) {
            *current = Some(bundle_id.clone());
        }
        let matched = self.rules.lock().ok().and_then(|rules| {
            rules
                .as_ref()
                .and_then(|rules| match_rule(rules, bundle_id.as_deref(), &profile_exists))
                .map(|rule| (rule.id.clone(), rule.profile_id.clone()))
        });
        let (rule_id, profile_id) = matched.unzip();
        FocusChanged {
            bundle_id,
            rule_id,
            profile_id,
        }
    }

    pub fn current_app(&self) -> CurrentApp {
        CurrentApp {
            supported: cfg!(target_os = "macos"),
            bundle_id: self.current_app.lock().ok().and_then(|app| app.clone()),
        }
    }
}

/// Receives the frontmost app's bundle id (`None` when it has none).
pub type Sink = Arc<dyn Fn(Option<String>) + Send + Sync>;

/// Starts the frontmost-app watcher. `on_change` runs on a worker thread for
/// every activation of another app; OpenStreamHub's own activations
/// (`own_bundle_id`) are skipped, so focusing the app never switches profiles.
/// Must be called on the main thread.
pub fn watch(
    own_bundle_id: String,
    on_change: impl Fn(Option<String>) + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::channel::<Option<String>>();
    thread::Builder::new()
        .name("focus rules".into())
        .spawn(move || {
            for bundle_id in receiver {
                if bundle_id
                    .as_deref()
                    .is_some_and(|id| id.eq_ignore_ascii_case(&own_bundle_id))
                {
                    continue;
                }
                on_change(bundle_id);
            }
        })
        .map_err(|error| format!("could not start the focus watcher: {error}"))?;
    let sender = Mutex::new(sender);
    let sink: Sink = Arc::new(move |bundle_id| {
        if let Ok(sender) = sender.lock() {
            let _ = sender.send(bundle_id);
        }
    });
    platform::start(sink)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::Sink;
    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceDidActivateApplicationNotification};
    use objc2_foundation::NSNotification;
    use std::ptr::NonNull;

    fn frontmost() -> Option<String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let application = workspace.frontmostApplication()?;
        application.bundleIdentifier().map(|id| id.to_string())
    }

    /// Reports the current frontmost app once, then every activation. The
    /// notification is posted on the main thread, which runs the block; the
    /// block only reads two properties and sends them to the worker.
    pub fn start(sink: Sink) -> Result<(), String> {
        sink(frontmost());
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| sink(frontmost()));
        // SAFETY: AppKit's immutable notification-name constant; no object
        // filter and no queue (the block runs on the posting thread); the
        // block only calls a Send + Sync sink.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(NSWorkspaceDidActivateApplicationNotification),
                None,
                None,
                &block,
            )
        };
        // Observed for the whole process lifetime.
        std::mem::forget(observer);
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::Sink;

    pub fn start(_sink: Sink) -> Result<(), String> {
        Err("focused-app rules need macOS; the rules can be edited but never match".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAFARI: &str = "com.apple.Safari";
    const PROFILE_A: &str = "00000000-0000-4000-8000-00000000000a";
    const PROFILE_B: &str = "00000000-0000-4000-8000-00000000000b";

    fn rule(id: u8, bundle: &str, profile: &str) -> FocusRule {
        FocusRule {
            id: format!("00000000-0000-4000-8000-0000000001{id:02x}"),
            app_bundle_id: bundle.into(),
            profile_id: profile.into(),
        }
    }

    fn enabled(rules: Vec<FocusRule>) -> FocusRules {
        FocusRules {
            enabled: true,
            rules,
        }
    }

    fn all(_: &str) -> bool {
        true
    }

    fn input(id: Option<&str>, bundle: &str, profile: &str) -> RuleInput {
        RuleInput {
            id: id.map(str::to_owned),
            app_bundle_id: bundle.into(),
            profile_id: profile.into(),
        }
    }

    #[test]
    fn the_first_matching_rule_wins_ignoring_bundle_id_case() {
        let rules = enabled(vec![
            rule(1, "com.apple.Terminal", PROFILE_B),
            rule(2, SAFARI, PROFILE_A),
            rule(3, "COM.APPLE.SAFARI", PROFILE_B),
        ]);
        let matched = match_rule(&rules, Some("com.apple.safari"), all).unwrap();
        assert_eq!(matched.id, rules.rules[1].id);
        assert_eq!(matched.profile_id, PROFILE_A);
    }

    #[test]
    fn unknown_apps_and_apps_without_a_bundle_id_match_nothing() {
        let rules = enabled(vec![rule(1, SAFARI, PROFILE_A)]);
        assert!(match_rule(&rules, Some("com.apple.Safari.helper"), all).is_none());
        assert!(match_rule(&rules, Some("com.apple"), all).is_none());
        assert!(match_rule(&rules, None, all).is_none());
        assert!(match_rule(&enabled(vec![]), Some(SAFARI), all).is_none());
    }

    #[test]
    fn disabled_rules_never_match() {
        let mut rules = enabled(vec![rule(1, SAFARI, PROFILE_A)]);
        rules.enabled = false;
        assert!(match_rule(&rules, Some(SAFARI), all).is_none());
    }

    #[test]
    fn a_rule_for_a_deleted_profile_is_skipped_for_the_next_match() {
        let rules = enabled(vec![rule(1, SAFARI, PROFILE_A), rule(2, SAFARI, PROFILE_B)]);
        let matched = match_rule(&rules, Some(SAFARI), |id| id != PROFILE_A).unwrap();
        assert_eq!(matched.profile_id, PROFILE_B);
        assert!(match_rule(&rules, Some(SAFARI), |_| false).is_none());
    }

    #[test]
    fn bundle_ids_are_trimmed_and_restricted() {
        assert_eq!(normalize_bundle_id("  com.apple.Safari ").unwrap(), SAFARI);
        assert_eq!(
            normalize_bundle_id("org.mozilla.firefox_beta-1").unwrap(),
            "org.mozilla.firefox_beta-1"
        );
        for bad in [
            "",
            "   ",
            ".com.apple",
            "-x",
            "com apple",
            "com/apple",
            "ü.app",
            "a\nb",
        ] {
            assert!(normalize_bundle_id(bad).is_err(), "{bad:?} was accepted");
        }
        assert!(normalize_bundle_id(&"a".repeat(MAX_BUNDLE_ID)).is_ok());
        assert!(normalize_bundle_id(&"a".repeat(MAX_BUNDLE_ID + 1)).is_err());
    }

    #[test]
    fn upsert_adds_edits_and_enforces_limits() {
        let rules = upsert(
            FocusRules::default(),
            input(None, " com.apple.Safari ", PROFILE_A),
        )
        .unwrap();
        assert!(!rules.enabled, "adding a rule never enables the feature");
        assert_eq!(rules.rules.len(), 1);
        let id = rules.rules[0].id.clone();
        assert!(validate_rule_id(&id).is_ok());
        assert_eq!(rules.rules[0].app_bundle_id, SAFARI);

        let edited = upsert(
            rules.clone(),
            input(Some(&id), "com.apple.Terminal", PROFILE_B),
        )
        .unwrap();
        assert_eq!(
            edited.rules,
            [rule_with(&id, "com.apple.Terminal", PROFILE_B)]
        );

        let missing = "00000000-0000-4000-8000-0000000001ff";
        assert!(upsert(rules.clone(), input(Some(missing), SAFARI, PROFILE_A)).is_err());
        assert!(upsert(rules.clone(), input(None, SAFARI, "not-a-uuid")).is_err());
        assert!(
            upsert(
                rules.clone(),
                input(None, SAFARI, &PROFILE_A.to_uppercase())
            )
            .is_err()
        );
        assert!(upsert(rules.clone(), input(None, "bad id", PROFILE_A)).is_err());

        let mut full = FocusRules::default();
        for _ in 0..MAX_RULES {
            full = upsert(full, input(None, SAFARI, PROFILE_A)).unwrap();
        }
        let error = upsert(full.clone(), input(None, SAFARI, PROFILE_A)).unwrap_err();
        assert!(error.contains("at most 64"), "{error}");
        // Editing still works at the limit.
        let id = full.rules[0].id.clone();
        assert!(upsert(full, input(Some(&id), SAFARI, PROFILE_B)).is_ok());
    }

    fn rule_with(id: &str, bundle: &str, profile: &str) -> FocusRule {
        FocusRule {
            id: id.into(),
            app_bundle_id: bundle.into(),
            profile_id: profile.into(),
        }
    }

    #[test]
    fn remove_deletes_only_a_known_rule() {
        let rules = enabled(vec![rule(1, SAFARI, PROFILE_A), rule(2, SAFARI, PROFILE_B)]);
        let removed = remove(rules.clone(), &rules.rules[0].id).unwrap();
        assert_eq!(removed.rules, [rules.rules[1].clone()]);
        assert!(remove(removed, &rules.rules[0].id).is_err());
    }

    #[test]
    fn validation_rejects_bad_ids_duplicates_and_oversized_sets() {
        assert!(validate(&enabled(vec![rule(1, SAFARI, PROFILE_A)])).is_ok());
        for id in ["not-a-uuid", "00000000-0000-4000-8000-0000000001AB"] {
            let mut bad = rule(1, SAFARI, PROFILE_A);
            bad.id = id.into();
            assert!(validate(&enabled(vec![bad])).is_err(), "{id} was accepted");
        }
        assert!(
            validate(&enabled(vec![
                rule(1, SAFARI, PROFILE_A),
                rule(1, SAFARI, PROFILE_B)
            ]))
            .is_err()
        );
        assert!(validate(&enabled(vec![rule(1, " com.apple.Safari", PROFILE_A)])).is_err());
        assert!(validate(&enabled(vec![rule(1, SAFARI, "x")])).is_err());
        let many = (0..=MAX_RULES)
            .map(|index| {
                rule_with(
                    &Uuid::new_v4().to_string(),
                    &format!("app.n{index}"),
                    PROFILE_A,
                )
            })
            .collect();
        assert!(validate(&enabled(many)).is_err());
    }

    #[test]
    fn the_rule_file_round_trips_atomically_and_privately() {
        let directory = tempfile::tempdir().unwrap();
        let file = RuleFile::new(directory.path());
        assert_eq!(file.load().unwrap(), FocusRules::default());
        let rules = enabled(vec![rule(1, SAFARI, PROFILE_A)]);
        file.save(&rules).unwrap();
        assert_eq!(file.load().unwrap(), rules);
        let path = directory.path().join(FILE_NAME);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // No temporary file is left behind.
        let names: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, [std::ffi::OsString::from(FILE_NAME)]);
    }

    #[test]
    fn opening_removes_only_stale_temporary_rule_files() {
        let directory = tempfile::tempdir().unwrap();
        let old = std::time::SystemTime::now()
            - store::STALE_TEMPORARY
            - std::time::Duration::from_secs(60);
        let stale = directory.path().join(format!(".{}.tmp", Uuid::new_v4()));
        let recent = directory.path().join(format!(".{}.tmp", Uuid::new_v4()));
        let visible = directory.path().join("notes.tmp");
        let folder = directory.path().join(".profiles-migration-x.tmp");
        // Hidden `.tmp` files of other shapes in the app data root are not ours.
        let other = directory.path().join(".other-app.tmp");
        let not_uuid = directory.path().join(".0000-not-a-uuid.tmp");
        for path in [&stale, &recent, &visible, &other, &not_uuid] {
            std::fs::write(path, b"{}").unwrap();
        }
        std::fs::create_dir(&folder).unwrap();
        for path in [&stale, &visible, &other, &not_uuid] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(old))
                .unwrap();
        }
        FocusState::open(Ok(directory.path()));
        assert!(!stale.exists(), "the stale temporary file was kept");
        assert!(recent.exists(), "a recent save may still be in progress");
        assert!(visible.exists() && folder.exists());
        assert!(
            other.exists() && not_uuid.exists(),
            "only `.<uuid>.tmp` is removed"
        );
    }

    #[test]
    fn damaged_oversized_or_unknown_rule_files_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        let state = FocusState::open(Ok(directory.path()));
        for bytes in [
            b"not json".to_vec(),
            br#"{"enabled":true,"rules":[],"extra":1}"#.to_vec(),
            vec![b' '; MAX_FILE_BYTES + 1],
        ] {
            std::fs::write(&path, &bytes).unwrap();
            assert!(state.list().is_err());
            assert!(
                state
                    .update(|rules| Ok(FocusRules {
                        enabled: true,
                        ..rules
                    }))
                    .is_err()
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes, "the file was changed");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_rule_file_is_never_read_or_written_through() {
        let directory = tempfile::tempdir().unwrap();
        let outside = directory.path().join("outside.json");
        std::fs::write(&outside, br#"{"enabled":true,"rules":[]}"#).unwrap();
        let app = directory.path().join("app");
        std::fs::create_dir(&app).unwrap();
        std::os::unix::fs::symlink(&outside, app.join(FILE_NAME)).unwrap();
        let state = FocusState::open(Ok(&app));
        let error = state.list().unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");
        assert!(
            state
                .update(|rules| Ok(FocusRules {
                    enabled: false,
                    ..rules
                }))
                .is_err()
        );
        assert_eq!(
            std::fs::read(&outside).unwrap(),
            br#"{"enabled":true,"rules":[]}"#
        );
        // A symbolic app data folder is refused for writing as well.
        let linked = directory.path().join("linked");
        std::os::unix::fs::symlink(directory.path().join("missing"), &linked).unwrap();
        let file = RuleFile::new(&linked);
        assert!(file.save(&FocusRules::default()).is_err());
    }

    #[test]
    fn state_updates_feed_the_watcher_copy() {
        let directory = tempfile::tempdir().unwrap();
        let state = FocusState::open(Ok(directory.path()));
        // A plain listing of a missing file writes nothing.
        assert_eq!(state.list().unwrap(), FocusRules::default());
        assert!(!directory.path().join(FILE_NAME).exists());

        state
            .update(|rules| upsert(rules, input(None, SAFARI, PROFILE_A)))
            .unwrap();
        let disabled = state.observe(Some(SAFARI.into()), all);
        assert_eq!(disabled.rule_id, None);
        assert_eq!(disabled.profile_id, None);

        let rules = state
            .update(|rules| {
                Ok(FocusRules {
                    enabled: true,
                    ..rules
                })
            })
            .unwrap();
        let matched = state.observe(Some(SAFARI.into()), all);
        assert_eq!(
            matched,
            FocusChanged {
                bundle_id: Some(SAFARI.into()),
                rule_id: Some(rules.rules[0].id.clone()),
                profile_id: Some(PROFILE_A.into()),
            }
        );
        assert_eq!(state.current_app().bundle_id.as_deref(), Some(SAFARI));
        // An app without a bundle id keeps the last known app for the button.
        assert_eq!(state.observe(None, all).profile_id, None);
        assert_eq!(state.current_app().bundle_id.as_deref(), Some(SAFARI));

        // Reopening reads the saved file.
        let reopened = FocusState::open(Ok(directory.path()));
        assert_eq!(
            reopened
                .observe(Some(SAFARI.into()), all)
                .profile_id
                .as_deref(),
            Some(PROFILE_A)
        );
        assert!(FocusState::open(Err("no app data".into())).list().is_err());
    }
}
