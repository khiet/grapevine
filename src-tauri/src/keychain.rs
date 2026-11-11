//! The GitHub token's home: the macOS Keychain, or on Linux the Secret Service
//! keyring. Which one backs it is decided by the keyring features in
//! Cargo.toml.

use keyring::Entry;

const SERVICE: &str = "com.khietle.grapevine";
const ACCOUNT: &str = "github-pat";

/// The store's name as the user knows it, so an error points at something they
/// can find on their own system.
#[cfg(target_os = "macos")]
const STORE_NAME: &str = "Keychain";
#[cfg(not(target_os = "macos"))]
const STORE_NAME: &str = "the keyring";

/// `action` reads up to the store's name, e.g. "store token in".
fn failure(action: &str, error: keyring::Error) -> String {
    format!("cannot {action} {STORE_NAME}: {error}")
}

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| failure("access", e))
}

pub fn store(token: &str) -> Result<(), String> {
    entry()?
        .set_password(token)
        .map_err(|e| failure("store token in", e))
}

pub fn load() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(failure("read token from", e)),
    }
}

pub fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(failure("remove token from", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn a_storage_error_on_macos_names_the_keychain() {
        assert_eq!(
            failure("store token in", keyring::Error::NoEntry),
            format!(
                "cannot store token in Keychain: {}",
                keyring::Error::NoEntry
            )
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_storage_error_on_linux_names_the_keyring() {
        assert_eq!(
            failure("store token in", keyring::Error::NoEntry),
            format!(
                "cannot store token in the keyring: {}",
                keyring::Error::NoEntry
            )
        );
    }

    /// Built without a platform store, keyring hands out an in-memory mock
    /// that accepts the token and loses it on quit. Building an entry does not
    /// contact the Secret Service, so this runs without a keyring daemon.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_token_entry_on_linux_is_backed_by_the_secret_service() {
        let entry = entry().expect("building the entry needs no running keyring");
        assert!(entry
            .get_credential()
            .downcast_ref::<keyring::secret_service::SsCredential>()
            .is_some());
    }
}
