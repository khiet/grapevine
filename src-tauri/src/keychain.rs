use keyring::Entry;

const SERVICE: &str = "com.khietle.grapevine";
const ACCOUNT: &str = "github-pat";

fn entry() -> Result<Entry, String> {
    Entry::new(SERVICE, ACCOUNT).map_err(|e| format!("cannot access Keychain: {e}"))
}

pub fn store(token: &str) -> Result<(), String> {
    entry()?
        .set_password(token)
        .map_err(|e| format!("cannot store token in Keychain: {e}"))
}

pub fn load() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(token) => Ok(Some(token)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("cannot read token from Keychain: {e}")),
    }
}

pub fn clear() -> Result<(), String> {
    match entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("cannot remove token from Keychain: {e}")),
    }
}
