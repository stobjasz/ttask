//! Optional password caching in the OS keychain (Windows Credential Manager,
//! macOS Keychain, Secret Service on Linux). One entry per database path.

use std::path::Path;

use secrecy::{ExposeSecret, SecretString};

const SERVICE: &str = "ttask";

fn account(db: &Path) -> String {
    std::fs::canonicalize(db)
        .unwrap_or_else(|_| db.to_owned())
        .to_string_lossy()
        .into_owned()
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
mod imp {
    use super::*;

    fn entry(db: &Path) -> Option<keyring::Entry> {
        keyring::Entry::new(SERVICE, &account(db)).ok()
    }

    pub fn get(db: &Path) -> Option<SecretString> {
        entry(db)?.get_password().ok().map(SecretString::from)
    }

    pub fn set(db: &Path, password: &SecretString) -> Result<(), String> {
        entry(db)
            .ok_or_else(|| "the OS keychain is not available".to_owned())?
            .set_password(password.expose_secret())
            .map_err(|e| e.to_string())
    }

    /// Returns whether an entry existed.
    pub fn delete(db: &Path) -> Result<bool, String> {
        let Some(entry) = entry(db) else {
            return Ok(false);
        };
        match entry.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod imp {
    use super::*;

    pub fn get(_db: &Path) -> Option<SecretString> {
        None
    }

    pub fn set(_db: &Path, _password: &SecretString) -> Result<(), String> {
        Err("no OS keychain support on this platform".into())
    }

    pub fn delete(_db: &Path) -> Result<bool, String> {
        Ok(false)
    }
}

pub use imp::{delete, get, set};
