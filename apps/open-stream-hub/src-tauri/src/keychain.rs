//! Where the OBS WebSocket password is kept between launches: the macOS
//! Keychain, as a generic password item for this app. Never a file, a profile,
//! a log or an argument.
//!
//! Errors say what failed, never the password. A Keychain that is locked,
//! denied or missing makes the vault report an error; the app then falls back
//! to holding the password in memory for the session, as 0.2.0 did.
//!
//! The app is not signed with a Developer ID, so macOS ties the item's access
//! list to the exact build that created it. After an update macOS may ask once
//! whether OpenStreamHub may use the item; **Always Allow** keeps it quiet
//! until the next update.

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
    use super::PasswordVault;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    const SERVICE: &str = "dev.cooltek.openstreamhub";
    const ACCOUNT: &str = "obs-websocket-password";
    /// `errSecItemNotFound`.
    const NOT_FOUND: i32 = -25300;

    /// The macOS Keychain item for the OBS password.
    pub struct Keychain;

    fn describe(action: &str, error: &security_framework::base::Error) -> String {
        format!(
            "could not {action} the OBS password in the Keychain (error {})",
            error.code()
        )
    }

    impl PasswordVault for Keychain {
        fn load(&self) -> Result<Option<String>, String> {
            match get_generic_password(SERVICE, ACCOUNT) {
                Ok(bytes) => String::from_utf8(bytes)
                    .map(Some)
                    .map_err(|_| "the OBS password in the Keychain is not valid text".to_string()),
                Err(error) if error.code() == NOT_FOUND => Ok(None),
                Err(error) => Err(describe("read", &error)),
            }
        }

        fn save(&self, password: &str) -> Result<(), String> {
            set_generic_password(SERVICE, ACCOUNT, password.as_bytes())
                .map_err(|error| describe("save", &error))
        }

        fn forget(&self) -> Result<(), String> {
            match delete_generic_password(SERVICE, ACCOUNT) {
                Ok(()) => Ok(()),
                Err(error) if error.code() == NOT_FOUND => Ok(()),
                Err(error) => Err(describe("remove", &error)),
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
    use super::PasswordVault;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };

    const SECURITY: &str = "/usr/bin/security";
    const SERVICE: &str = "dev.cooltek.openstreamhub.dev";
    const ACCOUNT: &str = "obs-websocket-password";
    /// `security` exits with 44 when the item is not found.
    const NOT_FOUND: i32 = 44;

    pub struct DevKeychain;

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
                .args(["find-generic-password", "-s", SERVICE, "-a", ACCOUNT, "-w"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .map_err(|error| format!("could not run the security tool: {}", error.kind()))?;
            match output.status.code() {
                Some(0) => unhex(String::from_utf8_lossy(&output.stdout).trim())
                    .map(Some)
                    .ok_or_else(|| "the stored OBS password is not valid".to_string()),
                Some(NOT_FOUND) => Ok(None),
                other => Err(format!(
                    "could not read the OBS password with the security tool (exit {other:?})"
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
                "add-generic-password -U -s {SERVICE} -a {ACCOUNT} -w {}\n",
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
            if !status.success() {
                return Err("could not save the OBS password with the security tool".into());
            }
            // `security -i` exits 0 even when a command failed: read it back.
            match self.load()? {
                Some(stored) if stored == password => Ok(()),
                _ => Err("could not save the OBS password with the security tool".into()),
            }
        }

        fn forget(&self) -> Result<(), String> {
            let status = Command::new(SECURITY)
                .args(["delete-generic-password", "-s", SERVICE, "-a", ACCOUNT])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("could not run the security tool: {}", error.kind()))?;
            match status.code() {
                Some(0) | Some(NOT_FOUND) => Ok(()),
                other => Err(format!(
                    "could not remove the OBS password with the security tool (exit {other:?})"
                )),
            }
        }
    }
}
