use std::{fs, marker::PhantomData, path::PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    fix_requests::FixRequests, merged::MergedState, settings::Settings, snooze::Snoozes,
    turns::TurnStarts, unread::ReadState,
};

/// What a load does when the file exists but cannot be read or parsed. A
/// missing file is never an error: every document loads as its default then.
#[derive(Debug, Clone, Copy, PartialEq)]
enum OnCorrupt {
    /// Surface the error. For data the user typed in, where silently starting
    /// over would discard their configuration on the next save.
    Fail,
    /// Log and load the default. For data the app can rebuild or afford to
    /// lose, where failing would block sync or startup.
    StartEmpty,
}

/// A named JSON document in the app data directory, tied to the type it holds.
pub struct Document<T> {
    file: &'static str,
    /// Names the document in error messages.
    label: &'static str,
    on_corrupt: OnCorrupt,
    holds: PhantomData<fn() -> T>,
}

/// A corrupt settings file is an error: falling back to the default would let
/// the next update overwrite the user's repo list with an empty one.
pub const SETTINGS: Document<Settings> = Document {
    file: "settings.json",
    label: "settings",
    on_corrupt: OnCorrupt::Fail,
    holds: PhantomData,
};

/// A corrupt file starts over empty: baselining then re-marks everything
/// read, which loses pending unread counts but never mis-flags old activity.
pub const READ_STATE: Document<ReadState> = Document {
    file: "unread.json",
    label: "unread state",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// `None` is the document never written: turns have not been tracked yet. A
/// corrupt file starts over the same way, which shows turns under way with an
/// unknown start rather than a wrong one.
pub const TURN_STARTS: Document<Option<TurnStarts>> = Document {
    file: "turns.json",
    label: "turn starts",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// A corrupt file starts over empty: undismissed merges are lost, which is
/// annoying but never wrong.
pub const MERGED: Document<MergedState> = Document {
    file: "merged.json",
    label: "merged state",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// A corrupt file starts over empty: every snoozed row wakes, which is noisy
/// once but never hides a row that has news.
pub const SNOOZES: Document<Snoozes> = Document {
    file: "snoozed.json",
    label: "snoozed state",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// A corrupt file starts over empty: every PR handed to Copilot reads as the
/// viewer's again, which is noisy once but never hides a blocked PR.
pub const FIX_REQUESTS: Document<FixRequests> = Document {
    file: "fix_requests.json",
    label: "fix requests",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// The app data directory, and the one place that reads and writes the JSON
/// documents in it.
pub struct Store {
    /// Resolved once at startup. A failure is kept instead of aborting the
    /// launch, and surfaces from each load or save as that document's policy
    /// dictates.
    dir: Result<PathBuf, String>,
}

impl Store {
    pub fn new(dir: Result<PathBuf, String>) -> Self {
        Store { dir }
    }

    /// Loads `doc`, or its default when the file is missing. Returns `Err`
    /// only for a document whose corrupt-file policy is to fail; the others
    /// load as their default instead.
    pub fn load<T: DeserializeOwned + Default>(&self, doc: &Document<T>) -> Result<T, String> {
        let label = doc.label;
        match doc.on_corrupt {
            OnCorrupt::Fail => {
                let path = self.path(doc)?;
                if !path.exists() {
                    return Ok(T::default());
                }
                let raw =
                    fs::read_to_string(&path).map_err(|e| format!("cannot read {label}: {e}"))?;
                serde_json::from_str(&raw).map_err(|e| format!("{label} file is corrupt: {e}"))
            }
            OnCorrupt::StartEmpty => {
                let Ok(raw) = self.path(doc).and_then(|path| {
                    fs::read_to_string(path).map_err(|e| format!("cannot read {label}: {e}"))
                }) else {
                    return Ok(T::default());
                };
                Ok(serde_json::from_str(&raw).unwrap_or_else(|e| {
                    eprintln!("{label} file is corrupt, starting over: {e}");
                    T::default()
                }))
            }
        }
    }

    /// Writes `value` as `doc`, creating the data directory if needed. The
    /// write is not atomic: a crash mid-write can leave a truncated file.
    pub fn save<T: Serialize>(&self, doc: &Document<T>, value: &T) -> Result<(), String> {
        let path = self.path(doc)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create app data dir: {e}"))?;
        }
        let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        fs::write(&path, raw).map_err(|e| format!("cannot write {}: {e}", doc.label))
    }

    fn path<T>(&self, doc: &Document<T>) -> Result<PathBuf, String> {
        let dir = self.dir.as_ref().map_err(Clone::clone)?;
        Ok(dir.join(doc.file))
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// A store over a fresh temporary directory. Keep the guard alive for the
    /// test's duration; dropping it deletes the directory.
    pub fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(Ok(dir.path().to_path_buf()));
        (dir, store)
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::temp_store, *};
    use crate::{
        fix_requests::FixRequest,
        merged::{Deploy, DeployState, MergedPr},
        settings::Appearance,
        snooze::Snooze,
    };

    fn merged_entry() -> MergedPr {
        MergedPr {
            number: 3,
            title: "Ship the other thing".into(),
            url: "https://github.com/acme/widgets/pull/3".into(),
            repo: "acme/widgets".into(),
            author: "someone".into(),
            avatar_url: "https://avatars.example/someone".into(),
            owner_avatar_url: "https://avatars.example/acme".into(),
            merged_at: "2026-07-11T10:00:00Z".into(),
            deploy: Some(Deploy {
                state: DeployState::Failed,
                completed_at: Some("2026-07-11T10:09:00Z".into()),
                url: "https://github.com/acme/widgets/actions/runs/2".into(),
            }),
        }
    }

    #[test]
    fn settings_round_trips() {
        let (_dir, store) = temp_store();
        let settings = Settings {
            repos: vec!["acme/widgets".into()],
            github_login: Some("someone".into()),
            poll_interval_secs: 300,
            appearance: Appearance::Dark,
        };
        store.save(&SETTINGS, &settings).unwrap();
        let loaded = store.load(&SETTINGS).unwrap();
        assert_eq!(loaded.repos, ["acme/widgets"]);
        assert_eq!(loaded.github_login.as_deref(), Some("someone"));
        assert_eq!(loaded.poll_interval_secs, 300);
        assert_eq!(loaded.appearance, Appearance::Dark);
    }

    #[test]
    fn read_state_round_trips() {
        let (_dir, store) = temp_store();
        let state = ReadState::from([("acme/widgets#3".into(), "2026-07-11T10:00:00Z".into())]);
        store.save(&READ_STATE, &state).unwrap();
        assert_eq!(store.load(&READ_STATE).unwrap(), state);
    }

    #[test]
    fn turn_starts_round_trip_known_and_unknown() {
        let (_dir, store) = temp_store();
        let starts = Some(TurnStarts::from([
            ("acme/widgets#3".into(), Some(1_784_205_296_000)),
            ("acme/widgets#4".into(), None),
        ]));
        store.save(&TURN_STARTS, &starts).unwrap();
        assert_eq!(store.load(&TURN_STARTS).unwrap(), starts);
    }

    /// Empty and never-written are different states: only the second makes
    /// the next sync treat every turn under way as of unknown start.
    #[test]
    fn an_empty_turn_starts_file_is_not_a_missing_one() {
        let (dir, store) = temp_store();
        assert_eq!(store.load(&TURN_STARTS).unwrap(), None);
        store.save(&TURN_STARTS, &Some(TurnStarts::new())).unwrap();
        assert_eq!(store.load(&TURN_STARTS).unwrap(), Some(TurnStarts::new()));
        fs::write(dir.path().join("turns.json"), "{not json").unwrap();
        assert_eq!(store.load(&TURN_STARTS).unwrap(), None);
    }

    #[test]
    fn the_merged_list_round_trips() {
        let (_dir, store) = temp_store();
        let state = vec![merged_entry()];
        store.save(&MERGED, &state).unwrap();
        assert_eq!(store.load(&MERGED).unwrap(), state);
    }

    #[test]
    fn the_snoozed_set_round_trips() {
        let (_dir, store) = temp_store();
        let state = Snoozes::from([(
            "acme/widgets#3".into(),
            Snooze {
                baseline_at: 1_784_205_296_000,
                request_outstanding: true,
                expires_at: 1_784_250_000_000,
            },
        )]);
        store.save(&SNOOZES, &state).unwrap();
        assert_eq!(store.load(&SNOOZES).unwrap(), state);
    }

    #[test]
    fn the_fix_requests_round_trip() {
        let (_dir, store) = temp_store();
        let state = FixRequests::from([(
            "acme/widgets#3".into(),
            FixRequest {
                requested_at: 1_784_205_296_000,
            },
        )]);
        store.save(&FIX_REQUESTS, &state).unwrap();
        assert_eq!(store.load(&FIX_REQUESTS).unwrap(), state);
    }

    #[test]
    fn a_corrupt_fix_requests_file_loads_as_empty() {
        let (dir, store) = temp_store();
        fs::write(dir.path().join("fix_requests.json"), "{not json").unwrap();
        assert!(store.load(&FIX_REQUESTS).unwrap().is_empty());
    }

    #[test]
    fn saving_creates_a_data_directory_that_does_not_exist_yet() {
        let (dir, _) = temp_store();
        let store = Store::new(Ok(dir.path().join("not/yet/created")));
        store.save(&MERGED, &vec![merged_entry()]).unwrap();
        assert_eq!(store.load(&MERGED).unwrap(), [merged_entry()]);
    }

    #[test]
    fn a_missing_file_loads_as_the_default_for_every_document() {
        let (_dir, store) = temp_store();
        let settings = store.load(&SETTINGS).unwrap();
        assert!(settings.repos.is_empty());
        assert_eq!(settings.github_login, None);
        assert_eq!(settings.poll_interval_secs, 180);
        assert!(store.load(&READ_STATE).unwrap().is_empty());
        assert!(store.load(&MERGED).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_read_state_file_loads_as_empty() {
        let (dir, store) = temp_store();
        fs::write(dir.path().join("unread.json"), "{not json").unwrap();
        assert!(store.load(&READ_STATE).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_merged_file_loads_as_empty() {
        let (dir, store) = temp_store();
        fs::write(dir.path().join("merged.json"), "{not json").unwrap();
        assert!(store.load(&MERGED).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_settings_file_is_an_error() {
        let (dir, store) = temp_store();
        fs::write(dir.path().join("settings.json"), "{not json").unwrap();
        let err = store.load(&SETTINGS).unwrap_err();
        assert!(err.starts_with("settings file is corrupt"), "{err}");
    }

    #[test]
    fn an_unresolved_data_directory_fails_settings_and_empties_the_rest() {
        let store = Store::new(Err("cannot resolve app data dir: nope".into()));
        assert_eq!(
            store.load(&SETTINGS).unwrap_err(),
            "cannot resolve app data dir: nope"
        );
        assert!(store.load(&READ_STATE).unwrap().is_empty());
        assert!(store.load(&MERGED).unwrap().is_empty());
        assert_eq!(
            store.save(&MERGED, &vec![merged_entry()]).unwrap_err(),
            "cannot resolve app data dir: nope"
        );
    }

    /// Without `#[serde(default)]` on the field the parse fails and the load
    /// silently discards the whole Merged section.
    #[test]
    fn a_merged_file_saved_before_the_newer_fields_still_loads() {
        let (dir, store) = temp_store();
        let legacy = r#"[{
            "number": 3,
            "title": "Ship the other thing",
            "url": "https://github.com/acme/widgets/pull/3",
            "repo": "acme/widgets",
            "author": "someone",
            "avatar_url": "https://avatars.example/someone",
            "merged_at": "2026-07-11T10:00:00Z"
        }]"#;
        fs::write(dir.path().join("merged.json"), legacy).unwrap();
        let state = store.load(&MERGED).unwrap();
        assert_eq!(state.len(), 1);
        assert_eq!(state[0].number, 3);
        assert_eq!(state[0].owner_avatar_url, "");
        assert_eq!(state[0].deploy, None);
    }
}
