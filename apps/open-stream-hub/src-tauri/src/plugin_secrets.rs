//! Plugin secrets: the values of text parameters a manifest marks `"secret": true`. They are kept in the Keychain,
//! one item per plugin id and parameter id ([`crate::keychain::Item::plugin_secret`]), never in a profile, so an
//! exported profile carries none. The plugin host fills them into an invoke just before it is sent.
//!
//! A value is read from the vault the first time a press needs it and then held in memory, so the Keychain is asked
//! once per launch for each. A failed read is held too, for [`RETRY_AFTER`], so a Keychain that is locked or denied
//! does not ask again at every press. Setting or forgetting one replaces what is held. Settings asks only whether an
//! item exists, which reads no value. Errors never contain a value.
//!
//! Every value sent to a plugin is also remembered for that plugin for the rest of the session (at most
//! [`MAX_DISCLOSED`] each), apart from the held values, so text the plugin sends back has it taken out even after the
//! secret was changed or forgotten: the plugin may still hold the old one. That list is used only by [`PluginSecrets::redact`].

use crate::keychain::{self, PasswordVault};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

/// Most values remembered per plugin for redaction; the oldest is dropped first.
pub const MAX_DISCLOSED: usize = 16;

/// How long a failed read is reported again before the vault is asked once more.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Makes the vault for one plugin id, parameter id and the words errors use for it.
pub type VaultFor = Arc<dyn Fn(&str, &str, String) -> Box<dyn PasswordVault> + Send + Sync>;

enum Held {
    /// What the vault holds: `None` when nothing is stored.
    Value(Option<String>),
    /// Why the last read failed, and when.
    Failed(String, Instant),
}

struct Inner {
    vault_for: VaultFor,
    held: Mutex<HashMap<(String, String), Held>>,
    /// The values sent to each plugin this session, oldest first.
    disclosed: Mutex<HashMap<String, std::collections::VecDeque<String>>>,
}

/// The plugin secrets; clones share them.
#[derive(Clone)]
pub struct PluginSecrets(Arc<Inner>);

impl PluginSecrets {
    /// Secrets in the Keychain (memory only in tests and on other platforms).
    pub fn keychain() -> Self {
        Self::with_vaults(Arc::new(|plugin, param, what| {
            keychain::vault(keychain::Item::plugin_secret(plugin, param, what))
        }))
    }

    pub fn with_vaults(vault_for: VaultFor) -> Self {
        Self(Arc::new(Inner {
            vault_for,
            held: Mutex::new(HashMap::new()),
            disclosed: Mutex::new(HashMap::new()),
        }))
    }

    fn vault(&self, plugin: &str, param: &str) -> Box<dyn PasswordVault> {
        (self.0.vault_for)(plugin, param, format!("secret {param} of {plugin}"))
    }

    fn key(plugin: &str, param: &str) -> (String, String) {
        (plugin.to_string(), param.to_string())
    }

    fn held(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Held>> {
        self.0
            .held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The stored value, reading the vault the first time (and again [`RETRY_AFTER`] after a failed read). The vault
    /// is read without the lock held, since a Keychain read may wait for the user to allow it.
    pub fn get(&self, plugin: &str, param: &str) -> Result<Option<String>, String> {
        let key = Self::key(plugin, param);
        match self.held().get(&key) {
            Some(Held::Value(value)) => return Ok(value.clone()),
            Some(Held::Failed(error, at)) if at.elapsed() < RETRY_AFTER => {
                return Err(error.clone());
            }
            _ => {}
        }
        let read = self.vault(plugin, param).load();
        let mut held = self.held();
        // A set or forget that finished meanwhile wins over this read.
        if let Some(Held::Value(value)) = held.get(&key) {
            return Ok(value.clone());
        }
        match read {
            Ok(value) => {
                held.insert(key, Held::Value(value.clone()));
                Ok(value)
            }
            Err(error) => {
                held.insert(key, Held::Failed(error.clone(), Instant::now()));
                Err(error)
            }
        }
    }

    /// What is held for the secret, without asking the vault: `None` when a press would have to read it first.
    pub fn cached(&self, plugin: &str, param: &str) -> Option<Result<Option<String>, String>> {
        match self.held().get(&Self::key(plugin, param)) {
            Some(Held::Value(value)) => Some(Ok(value.clone())),
            Some(Held::Failed(error, at)) if at.elapsed() < RETRY_AFTER => Some(Err(error.clone())),
            _ => None,
        }
    }

    /// Whether a value is stored, without reading it: from memory when it was read already, otherwise by asking the
    /// vault whether the item exists. An error means it could not be told.
    pub fn is_set(&self, plugin: &str, param: &str) -> Result<bool, String> {
        if let Some(Held::Value(value)) = self.held().get(&Self::key(plugin, param)) {
            return Ok(value.is_some());
        }
        self.vault(plugin, param).exists()
    }

    /// Remembers that `value` was sent to `plugin`, for [`Self::redact`].
    pub fn disclosed(&self, plugin: &str, value: &str) {
        if value.is_empty() {
            return;
        }
        let mut disclosed = self
            .0
            .disclosed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let values = disclosed.entry(plugin.to_string()).or_default();
        values.retain(|known| known != value);
        if values.len() == MAX_DISCLOSED {
            values.pop_front();
        }
        values.push_back(value.to_string());
    }

    /// `text` from `plugin` with every value it was sent this session, and every value held for it now, replaced by
    /// `[secret]` (see [`redact`]).
    pub fn redact(&self, plugin: &str, text: &str) -> String {
        let mut values: Vec<String> = self
            .held()
            .iter()
            .filter_map(|((owner, _), held)| match held {
                Held::Value(Some(value)) if owner == plugin => Some(value.clone()),
                _ => None,
            })
            .collect();
        if let Some(sent) = self
            .0
            .disclosed
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(plugin)
        {
            values.extend(sent.iter().cloned());
        }
        redact(text, &values)
    }

    /// Stores `value`, replacing any earlier one.
    pub fn set(&self, plugin: &str, param: &str, value: &str) -> Result<(), String> {
        self.vault(plugin, param).save(value)?;
        self.held().insert(
            Self::key(plugin, param),
            Held::Value(Some(value.to_string())),
        );
        Ok(())
    }

    /// Removes the stored value. Nothing stored is not an error.
    pub fn forget(&self, plugin: &str, param: &str) -> Result<(), String> {
        self.vault(plugin, param).forget()?;
        self.held()
            .insert(Self::key(plugin, param), Held::Value(None));
        Ok(())
    }
}

/// Values shorter than this are not taken out of text: they would match too much ordinary text.
pub const MIN_REDACTED_BYTES: usize = 4;

/// Replaces each of `secrets` (of at least [`MIN_REDACTED_BYTES`]) in `text` with `[secret]`, in one pass from left to
/// right that tries the longest value first at each place, so a value inside another is not left behind and a
/// marker already put in is never matched again.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&str> = secrets
        .iter()
        .map(String::as_str)
        .filter(|value| value.len() >= MIN_REDACTED_BYTES)
        .collect();
    if secrets.is_empty() {
        return text.to_string();
    }
    secrets.sort_by_key(|value| std::cmp::Reverse(value.len()));
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(next) = rest.chars().next() {
        match secrets.iter().find(|value| rest.starts_with(**value)) {
            Some(value) => {
                out.push_str("[secret]");
                rest = &rest[value.len()..];
            }
            None => {
                out.push(next);
                rest = &rest[next.len_utf8()..];
            }
        }
    }
    out
}

/// Test secrets: a map shared by every vault, how often values were read and existence asked, and a switch that
/// makes every vault call fail.
#[cfg(test)]
pub mod memory {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Clone, Default)]
    pub struct Store {
        pub values: Arc<Mutex<HashMap<String, String>>>,
        pub reads: Arc<Mutex<usize>>,
        pub checks: Arc<Mutex<usize>>,
        pub failing: Arc<AtomicBool>,
        /// When set, a read waits (at most 5 s) for a message on it, like a Keychain asking the user.
        pub hold: Arc<Mutex<Option<std::sync::mpsc::Receiver<()>>>>,
        /// When set, only reads of this plugin's secrets are held.
        pub held_plugin: Arc<Mutex<Option<String>>>,
    }

    struct Vault {
        account: String,
        store: Store,
    }

    impl Vault {
        fn fail(&self) -> Result<(), String> {
            if self.store.failing.load(Ordering::SeqCst) {
                return Err("the Keychain is locked".into());
            }
            Ok(())
        }
    }

    impl PasswordVault for Vault {
        fn load(&self) -> Result<Option<String>, String> {
            *self.store.reads.lock().unwrap() += 1;
            let held = self
                .store
                .held_plugin
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|plugin| self.account.starts_with(&format!("{plugin}:")));
            if held && let Some(hold) = self.store.hold.lock().unwrap().as_ref() {
                let _ = hold.recv_timeout(std::time::Duration::from_secs(5));
            }
            self.fail()?;
            Ok(self
                .store
                .values
                .lock()
                .unwrap()
                .get(&self.account)
                .cloned())
        }
        fn save(&self, password: &str) -> Result<(), String> {
            self.fail()?;
            self.store
                .values
                .lock()
                .unwrap()
                .insert(self.account.clone(), password.to_string());
            Ok(())
        }
        fn forget(&self) -> Result<(), String> {
            self.fail()?;
            self.store.values.lock().unwrap().remove(&self.account);
            Ok(())
        }
        fn exists(&self) -> Result<bool, String> {
            *self.store.checks.lock().unwrap() += 1;
            self.fail()?;
            Ok(self
                .store
                .values
                .lock()
                .unwrap()
                .contains_key(&self.account))
        }
    }

    /// Secrets kept in `store`, under `<plugin>:<param>`.
    pub fn secrets(store: &Store) -> PluginSecrets {
        let store = store.clone();
        PluginSecrets::with_vaults(Arc::new(move |plugin, param, _| {
            Box::new(Vault {
                account: format!("{plugin}:{param}"),
                store: store.clone(),
            })
        }))
    }

    #[test]
    fn a_secret_is_read_once_and_replaced_when_set_or_forgotten() {
        let store = Store::default();
        store
            .values
            .lock()
            .unwrap()
            .insert("com.example.a:token".into(), "s3cret".into());
        let secrets = secrets(&store);
        // Asking whether it is set reads no value.
        assert_eq!(secrets.is_set("com.example.a", "token"), Ok(true));
        assert_eq!(*store.reads.lock().unwrap(), 0);
        assert_eq!(
            secrets.get("com.example.a", "token"),
            Ok(Some("s3cret".into()))
        );
        assert_eq!(
            secrets.get("com.example.a", "token"),
            Ok(Some("s3cret".into()))
        );
        assert_eq!(*store.reads.lock().unwrap(), 1);
        assert_eq!(secrets.redact("com.example.a", "x s3cret"), "x [secret]");
        assert_eq!(secrets.redact("com.example.b", "x s3cret"), "x s3cret");
        secrets.set("com.example.a", "token", "new").unwrap();
        assert_eq!(
            secrets.get("com.example.a", "token"),
            Ok(Some("new".into()))
        );
        secrets.forget("com.example.a", "token").unwrap();
        assert_eq!(secrets.is_set("com.example.a", "token"), Ok(false));
        assert!(store.values.lock().unwrap().is_empty());
        assert_eq!(*store.reads.lock().unwrap(), 1);
    }

    #[test]
    fn a_failed_read_is_held_and_not_asked_again_at_every_press() {
        let store = Store::default();
        store.failing.store(true, Ordering::SeqCst);
        let secrets = secrets(&store);
        assert_eq!(
            secrets.get("com.example.a", "token"),
            Err("the Keychain is locked".into())
        );
        assert_eq!(
            secrets.get("com.example.a", "token"),
            Err("the Keychain is locked".into())
        );
        assert_eq!(*store.reads.lock().unwrap(), 1);
        assert!(secrets.is_set("com.example.a", "token").is_err());
        // A set replaces the failure.
        store.failing.store(false, Ordering::SeqCst);
        secrets.set("com.example.a", "token", "v").unwrap();
        assert_eq!(secrets.get("com.example.a", "token"), Ok(Some("v".into())));
    }

    #[test]
    fn a_value_sent_to_a_plugin_is_taken_out_after_it_is_changed_or_forgotten() {
        let store = Store::default();
        let secrets = secrets(&store);
        secrets
            .set("com.example.a", "token", "first-token")
            .unwrap();
        secrets.disclosed("com.example.a", "first-token");
        secrets
            .set("com.example.a", "token", "second-token")
            .unwrap();
        assert_eq!(
            secrets.redact("com.example.a", "first-token second-token"),
            "[secret] [secret]"
        );
        secrets.forget("com.example.a", "token").unwrap();
        assert_eq!(
            secrets.redact("com.example.a", "first-token second-token"),
            "[secret] second-token"
        );
        // Bounded: the oldest is dropped first.
        for index in 0..MAX_DISCLOSED {
            secrets.disclosed("com.example.a", &format!("value-{index:02}"));
        }
        assert_eq!(
            secrets.redact("com.example.a", "first-token"),
            "first-token"
        );
        assert_eq!(secrets.redact("com.example.a", "value-00"), "[secret]");
    }

    #[test]
    fn known_values_are_taken_out_of_text() {
        assert_eq!(
            redact(
                "token abcd123 and abcd",
                &["abcd".into(), "abcd123".into(), String::new()]
            ),
            "token [secret] and [secret]"
        );
        // A short value is left alone, and a value that a marker contains never matches the marker.
        assert_eq!(
            redact("ab abcd", &["ab".into(), "abcd".into()]),
            "ab [secret]"
        );
        assert_eq!(
            redact("cret: secret", &["secret".into(), "cret".into()]),
            "[secret]: [secret]"
        );
        assert_eq!(redact("zażółć", &["żółć".into()]), "za[secret]");
    }
}
