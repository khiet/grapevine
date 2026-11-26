use std::sync::Mutex;

use serde::{Deserialize, Deserializer, Serialize};

use crate::store::{self, Store};

/// Bounds for the poll interval. The floor keeps a mistyped value from
/// hammering GitHub; the ceiling keeps the app from looking dead.
pub const MIN_POLL_SECS: u64 = 30;
pub const MAX_POLL_SECS: u64 = 3600;
pub const DEFAULT_POLL_SECS: u64 = 180;

fn default_poll_secs() -> u64 {
    DEFAULT_POLL_SECS
}

/// Which palette the popover draws in. `System` follows the desktop's light
/// or dark style, which the frontend resolves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

/// Reads an appearance of any shape, so that a hand-edited value this build
/// does not know falls back to `System` instead of failing the whole file.
fn lenient_appearance<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Appearance, D::Error> {
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw).unwrap_or_default())
}

/// The lengths a snooze limit can be, in days: 1 day, 3 days, 1 week,
/// 2 weeks, 1 month, 3 months. A month is 30 days.
pub const SNOOZE_LIMIT_DAYS: [u32; 6] = [1, 3, 7, 14, 30, 90];

/// How long a snooze may last. Read by the webview when a row is snoozed,
/// so a change applies to snoozes made after it and never to a row already
/// parked.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnoozeLimit {
    /// Off means a snooze has no expiry and only news wakes the row.
    pub enabled: bool,
    /// The first, second and third duration offered when snoozing, each one
    /// of [`SNOOZE_LIMIT_DAYS`]. Kept while `enabled` is off. Two may be
    /// alike.
    pub days: [u32; 3],
}

impl Default for SnoozeLimit {
    fn default() -> Self {
        SnoozeLimit {
            enabled: true,
            days: [1, 7, 30],
        }
    }
}

impl SnoozeLimit {
    pub fn is_valid(&self) -> bool {
        self.days
            .iter()
            .all(|days| SNOOZE_LIMIT_DAYS.contains(days))
    }
}

/// Reads a snooze limit of any shape, so that a hand-edited value this build
/// cannot read falls back to the default instead of failing the whole file.
fn lenient_snooze_limit<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<SnoozeLimit, D::Error> {
    let raw = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(raw)
        .ok()
        .filter(SnoozeLimit::is_valid)
        .unwrap_or_default())
}

/// App state persisted to `settings.json` in the app data directory. The
/// GitHub token is deliberately absent: it lives only in the system credential
/// store (see `keychain`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub repos: Vec<String>,
    pub github_login: Option<String>,
    /// Seconds between syncs. Persisted values outside the bounds are
    /// clamped on read, so a hand-edited file cannot break the loop.
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
    /// A file without this field, or with a value this build does not know,
    /// loads as `System`.
    #[serde(deserialize_with = "lenient_appearance")]
    pub appearance: Appearance,
    /// A file without this field, or with a value this build cannot read,
    /// loads as the default: on, with 1 day, 1 week and 1 month.
    #[serde(deserialize_with = "lenient_snooze_limit")]
    pub snooze_limit: SnoozeLimit,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            repos: Vec::new(),
            github_login: None,
            poll_interval_secs: DEFAULT_POLL_SECS,
            appearance: Appearance::default(),
            snooze_limit: SnoozeLimit::default(),
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

    /// Settings files written before the appearance setting have no such
    /// field.
    #[test]
    fn a_settings_file_without_an_appearance_follows_the_system() {
        let (_dir, store) = store_with_settings_file(r#"{"repos": ["acme/widgets"]}"#);
        assert_eq!(load(&store).unwrap().appearance, Appearance::System);
    }

    #[test]
    fn a_stored_appearance_is_read_back() {
        let (_dir, store) = store_with_settings_file(r#"{"appearance": "dark"}"#);
        assert_eq!(load(&store).unwrap().appearance, Appearance::Dark);
        let (_dir, store) = store_with_settings_file(r#"{"appearance": "light"}"#);
        assert_eq!(load(&store).unwrap().appearance, Appearance::Light);
    }

    /// A hand-edited value must not cost the user the rest of the file.
    #[test]
    fn an_unrecognised_appearance_follows_the_system_and_keeps_the_rest() {
        for raw in [
            r#"{"appearance": "sepia", "repos": ["acme/widgets"]}"#,
            r#"{"appearance": 7, "repos": ["acme/widgets"]}"#,
        ] {
            let (_dir, store) = store_with_settings_file(raw);
            let settings = load(&store).unwrap();
            assert_eq!(settings.appearance, Appearance::System);
            assert_eq!(settings.repos, ["acme/widgets"]);
        }
    }

    #[test]
    fn a_chosen_appearance_is_stored_in_lowercase() {
        let (dir, store) = temp_store();
        update(&store, |settings| {
            settings.appearance = Appearance::Dark;
            Ok(())
        })
        .unwrap();
        let raw = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(stored["appearance"], "dark");
    }

    const DEFAULT_SNOOZE_LIMIT: SnoozeLimit = SnoozeLimit {
        enabled: true,
        days: [1, 7, 30],
    };

    /// Settings files written before the snooze limit have no such field.
    #[test]
    fn a_settings_file_without_a_snooze_limit_loads_with_the_default() {
        let (_dir, store) = store_with_settings_file(
            r#"{"repos": ["acme/widgets"], "poll_interval_secs": 300, "appearance": "dark"}"#,
        );
        let settings = load(&store).unwrap();
        assert_eq!(settings.snooze_limit, DEFAULT_SNOOZE_LIMIT);
        assert_eq!(settings.repos, ["acme/widgets"]);
        assert_eq!(settings.poll_interval_secs, 300);
        assert_eq!(settings.appearance, Appearance::Dark);
    }

    /// A hand-edited value must not cost the user the rest of the file.
    #[test]
    fn an_unreadable_snooze_limit_loads_as_the_default_and_keeps_the_rest() {
        for limit in [
            r#""never""#,
            r#"{"enabled": "yes", "days": [1, 7, 30]}"#,
            r#"{"enabled": true, "days": [1, 7]}"#,
            // Five days is not a length this build offers.
            r#"{"enabled": false, "days": [1, 5, 30]}"#,
        ] {
            let raw = format!(
                r#"{{"snooze_limit": {limit}, "repos": ["acme/widgets"],
                    "poll_interval_secs": 300, "appearance": "dark"}}"#
            );
            let (_dir, store) = store_with_settings_file(&raw);
            let settings = load(&store).unwrap();
            assert_eq!(settings.snooze_limit, DEFAULT_SNOOZE_LIMIT, "{limit}");
            assert_eq!(settings.repos, ["acme/widgets"]);
            assert_eq!(settings.poll_interval_secs, 300);
            assert_eq!(settings.appearance, Appearance::Dark);
        }
    }

    /// The durations outlive the switch: off keeps them for when it is on
    /// again.
    #[test]
    fn a_chosen_snooze_limit_round_trips_with_the_switch_off() {
        let (_dir, store) = temp_store();
        let chosen = SnoozeLimit {
            enabled: false,
            days: [3, 3, 90],
        };
        update(&store, |settings| {
            settings.snooze_limit = chosen;
            Ok(())
        })
        .unwrap();
        assert_eq!(load(&store).unwrap().snooze_limit, chosen);
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
