//! Where the OBS WebSocket password and the Streamlabs API token are kept
//! between launches: the macOS Keychain, as one generic password item each for
//! this app ([`OBS_PASSWORD`], [`STREAMLABS_TOKEN`]: the same service name,
//! separate accounts). Never a file, a profile, a log or an argument.
//!
//! Errors say what failed, never the secret. A Keychain that is locked,
//! denied or missing makes the vault report an error; the app then falls back
//! to holding the password in memory for the session, as 0.2.0 did.
//!
//! The app is not signed with a Developer ID, so macOS ties the item's access
//! list to the exact build that created it. After an update macOS may ask once
//! whether OpenStreamHub may use the item; **Always Allow** keeps it quiet
//! until the next update.

/// One Keychain item: its account under this app's service name, and what
/// errors call it.
#[cfg(all(target_os = "macos", not(test)))]
#[derive(Clone, Copy)]
pub struct Item {
    account: &'static str,
    what: &'static str,
}

/// The OBS WebSocket password (the account 0.2.0 already used).
#[cfg(all(target_os = "macos", not(test)))]
pub const OBS_PASSWORD: Item = Item {
    account: "obs-websocket-password",
    what: "OBS password",
};

/// The Streamlabs Remote Control API token.
#[cfg(all(target_os = "macos", not(test)))]
pub const STREAMLABS_TOKEN: Item = Item {
    account: "streamlabs-api-token",
    what: "Streamlabs token",
};

/// A place to keep one password between launches.
pub trait PasswordVault: Send + Sync {
    /// The stored password, or `None` when nothing is stored.
    fn load(&self) -> Result<Option<String>, String>;
    /// Stores `password`, replacing any earlier one.
    fn save(&self, password: &str) -> Result<(), String>;
    /// Removes the stored password. Nothing stored is not an error.
    fn forget(&self) -> Result<(), String>;
}

/// Keeps nothing: the password lives in memory only (other platforms, tests).
#[cfg(any(test, not(target_os = "macos")))]
pub struct MemoryOnly;

#[cfg(any(test, not(target_os = "macos")))]
impl PasswordVault for MemoryOnly {
    fn load(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn save(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn forget(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(all(target_os = "macos", not(test), debug_assertions))]
pub use dev::DevKeychain;
#[cfg(all(target_os = "macos", not(test), not(debug_assertions)))]
pub use macos::Keychain;

#[cfg(all(target_os = "macos", not(test), not(debug_assertions)))]
mod macos {
    use super::{Item, PasswordVault};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "dev.cooltek.openstreamhub";
    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    /// One macOS Keychain item of this app.
    pub struct Keychain(pub Item);

    impl Keychain {
        fn describe(&self, action: &str, error: &security_framework::base::Error) -> String {
            format!(
                "could not {action} the {} in the Keychain (error {})",
                self.0.what,
                error.code()
            )
        }
    }

    impl PasswordVault for Keychain {
        fn load(&self) -> Result<Option<String>, String> {
            match get_generic_password(SERVICE, self.0.account) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| format!("the {} in the Keychain is not valid text", self.0.what)),
                Err(error) if error.code() == NOT_FOUND => Ok(None),
                Err(error) => Err(self.describe("read", &error)),
            }
        }

        fn save(&self, password: &str) -> Result<(), String> {
            set_generic_password(SERVICE, self.0.account, password.as_bytes())
                .map_err(|error| self.describe("save", &error))
        }

        fn forget(&self) -> Result<(), String> {
            match delete_generic_password(SERVICE, self.0.account) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == NOT_FOUND => Ok(()),
                Err(error) => Err(self.describe("remove", &error)),
            }
        }
    }
}

/// Development builds only. Every rebuild is a new binary, which macOS does
/// not recognise as the app that created the Keychain item, so it asks for the
/// login password again each time. This vault keeps the item through Apple's
/// own `/usr/bin/security` tool instead: the item then trusts that tool, whose
/// identity never changes, so nothing is asked. The password is stored as hex
/// (so `security` never has to print or parse arbitrary text) under a
/// separate service name, and travels to the tool on its stdin, never in an
/// argument. Release builds never use this: they talk to the Keychain
/// directly and the item trusts the signed app.
#[cfg(all(target_os = "macos", not(test), debug_assertions))]
mod dev {
    use super::{Item, PasswordVault};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    const SECURITY: &str = "/usr/bin/security";
    const SERVICE: &str = "dev.cooltek.openstreamhub.dev";
    /// `security` exits with 44 when the item is not found.
    const NOT_FOUND: i32 = 44;

    pub struct DevKeychain(pub Item);

    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Option<String> {
        let digits = text.as_bytes();
        if !digits.len().is_multiple_of(2) {
            return None;
        }
        let bytes = digits
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        String::from_utf8(bytes).ok()
    }

    impl PasswordVault for DevKeychain {
        fn load(&self) -> Result<Option<String>, String> {
            let output = Command::new(SECURITY)
                .args([
                    "find-generic-password",
                    "-s",
                    SERVICE,
                    "-a",
                    self.0.account,
                    "-w",
                ])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map_err(|error| format!("could not run the security tool: {}", error.kind()))?;
            match output.status.code() {
                Some(0) => unhex(String::from_utf8_lossy(&output.stdout).trim())
                    .map(Some)
                    .ok_or_else(|| format!("the stored {} is not valid", self.0.what)),
                Some(NOT_FOUND) => Ok(None),
                other => Err(format!(
                    "could not read the {} with the security tool (exit {other:?})",
                    self.0.what
                )),
            }
        }

        fn save(&self, password: &str) -> Result<(), String> {
            // Interactive mode reads the command, password included, from
            // stdin, so it never appears in a process listing.
            let mut child = Command::new(SECURITY)
                .arg("-i")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("could not run the security tool: {}", error.kind()))?;
            let command = format!(
                "add-generic-password -U -s {SERVICE} -a {} -w {}\n",
                self.0.account,
                hex(password)
            );
            child
                .stdin
                .take()
                .ok_or("could not talk to the security tool")?
                .write_all(command.as_bytes())
                .map_err(|error| {
                    format!("could not talk to the security tool: {}", error.kind())
                })?;
            let status = child
                .wait()
                .map_err(|error| format!("the security tool did not finish: {}", error.kind()))?;
            let failed = || format!("could not save the {} with the security tool", self.0.what);
            if !status.success() {
                return Err(failed());
            }
            // `security -i` exits 0 even when a command failed: read it back.
            match self.load()? {
                Some(stored) if stored == password => Ok(()),
                _ => Err(failed()),
            }
        }

        fn forget(&self) -> Result<(), String> {
            let status = Command::new(SECURITY)
                .args([
                    "delete-generic-password",
                    "-s",
                    SERVICE,
                    "-a",
                    self.0.account,
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("could not run the security tool: {}", error.kind()))?;
            match status.code() {
                Some(0) | Some(NOT_FOUND) => Ok(()),
                other => Err(format!(
                    "could not remove the {} with the security tool (exit {other:?})",
                    self.0.what
                )),
            }
        }
    }
}
