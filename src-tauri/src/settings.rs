use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::store::{self, Store};

/// Bounds for the poll interval. The floor keeps a mistyped value from
/// hammering GitHub; the ceiling keeps the app from looking dead.
pub const MIN_POLL_SECS: u64 = 30;
pub const MAX_POLL_SECS: u64 = 3600;
pub const DEFAULT_POLL_SECS: u64 = 180;

fn default_poll_secs() -> u64 {
    DEFAULT_POLL_SECS
}

/// App state persisted to `settings.json` in the app data directory. The
/// GitHub token is deliberately absent: it lives only in the system credential store (see `keychain`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub repos: Vec<String>,
    pub github_login: Option<String>,
    /// Seconds between syncs. Persisted values outside the bounds are
    /// clamped on read, so a hand-edited file cannot break the loop.
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            repos: Vec::new(),
            github_login: None,
            poll_interval_secs: DEFAULT_POLL_SECS,
        }
    }
}

pub fn clamp_poll_secs(secs: u64) -> u64 {
    secs.clamp(MIN_POLL_SECS, MAX_POLL_SECS)
}

/// Loads the stored settings. The poll interval is clamped here so that a
/// hand-edited file cannot hand the sync loop an out-of-bounds value.
pub fn load(store: &Store) -> Result<Settings, String> {
    let mut settings = store.load(&store::SETTINGS)?;
    settings.poll_interval_secs = clamp_poll_secs(settings.poll_interval_secs);
    Ok(settings)
}

/// Serializes every load-modify-save on the settings file. Tauri commands run
/// concurrently, and two overlapping updates (e.g. rapid checkbox toggles in
/// the repo browser) would otherwise both read the same base state and the
/// later save would silently drop the earlier change.
static UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Applies `mutate` to the stored settings under [`UPDATE_LOCK`] and returns
/// the saved result. An error from `mutate` aborts without writing. `mutate`
/// must not block: the lock is held across the reload and the write.
pub fn update(
    store: &Store,
    mutate: impl FnOnce(&mut Settings) -> Result<(), String>,
) -> Result<Settings, String> {
    let _guard = UPDATE_LOCK.lock().unwrap();
    let mut settings = load(store)?;
    mutate(&mut settings)?;
    store.save(&store::SETTINGS, &settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Barrier, thread};

    use super::*;
    use crate::store::testing::temp_store;

    fn store_with_settings_file(raw: &str) -> (tempfile::TempDir, Store) {
        let (dir, store) = temp_store();
        fs::write(dir.path().join("settings.json"), raw).unwrap();
        (dir, store)
    }

    /// Settings files written before the poll interval existed have no such
    /// field, and must not load as zero.
    #[test]
    fn a_settings_file_without_a_poll_interval_loads_with_the_default() {
        let (_dir, store) = store_with_settings_file(r#"{"repos": ["acme/widgets"]}"#);
        let settings = load(&store).unwrap();
        assert_eq!(settings.poll_interval_secs, 180);
        assert_eq!(settings.repos, ["acme/widgets"]);
    }

    #[test]
    fn an_out_of_range_poll_interval_is_clamped_on_read() {
        let (_dir, store) = store_with_settings_file(r#"{"poll_interval_secs": 5}"#);
        assert_eq!(load(&store).unwrap().poll_interval_secs, 30);

        let (_dir, store) = store_with_settings_file(r#"{"poll_interval_secs": 86400}"#);
        assert_eq!(load(&store).unwrap().poll_interval_secs, 3600);
    }

    #[test]
    fn an_update_is_saved_and_returned() {
        let (_dir, store) = temp_store();
        let updated = update(&store, |settings| {
            settings.repos.push("acme/widgets".into());
            Ok(())
        })
        .unwrap();
        assert_eq!(updated.repos, ["acme/widgets"]);
        assert_eq!(load(&store).unwrap().repos, ["acme/widgets"]);
    }

    #[test]
    fn a_failed_update_writes_nothing() {
        let (_dir, store) = temp_store();
        let result = update(&store, |settings| {
            settings.repos.push("acme/widgets".into());
            Err("no".into())
        });
        assert_eq!(result.unwrap_err(), "no");
        assert!(load(&store).unwrap().repos.is_empty());
    }

    #[test]
    fn an_update_of_a_corrupt_settings_file_fails_and_leaves_it_alone() {
        let (dir, store) = store_with_settings_file("{not json");
        assert!(update(&store, |_| Ok(())).is_err());
        let raw = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert_eq!(raw, "{not json");
    }

    /// Unserialized, overlapping updates read the same base state and the
    /// later save drops the earlier change.
    #[test]
    fn concurrent_updates_all_take_effect() {
        let (_dir, store) = temp_store();
        let start = Barrier::new(8);
        thread::scope(|scope| {
            for i in 0..8 {
                let (store, start) = (&store, &start);
                scope.spawn(move || {
                    start.wait();
                    update(store, |settings| {
                        settings.repos.push(format!("acme/repo-{i}"));
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });
        let mut repos = load(&store).unwrap().repos;
        repos.sort();
        let expected: Vec<String> = (0..8).map(|i| format!("acme/repo-{i}")).collect();
        assert_eq!(repos, expected);
    }

    #[test]
    fn poll_intervals_clamp_to_the_allowed_range() {
        assert_eq!(clamp_poll_secs(0), MIN_POLL_SECS);
        assert_eq!(clamp_poll_secs(29), MIN_POLL_SECS);
        assert_eq!(clamp_poll_secs(30), 30);
        assert_eq!(clamp_poll_secs(300), 300);
        assert_eq!(clamp_poll_secs(3600), 3600);
        assert_eq!(clamp_poll_secs(1_000_000), MAX_POLL_SECS);
    }
}
