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

#[cfg(all(target_os = "macos", not(test)))]
pub use macos::Keychain;

#[cfg(all(target_os = "macos", not(test)))]
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
