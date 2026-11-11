use std::{fs, io, marker::PhantomData, path::PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::{merged::MergedState, settings::Settings, unread::ReadState};

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

/// A corrupt file starts over empty: undismissed merges are lost, which is
/// annoying but never wrong.
pub const MERGED: Document<MergedState> = Document {
    file: "merged.json",
    label: "merged state",
    on_corrupt: OnCorrupt::StartEmpty,
    holds: PhantomData,
};

/// The app data directory, and the one place that reads and writes the JSON
/// documents in it.
pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Store { dir }
    }

    /// Loads `doc`, or its default when the file is missing. Returns `Err`
    /// only for a document whose corrupt-file policy is to fail; the others
    /// load as their default instead.
    pub fn load<T: DeserializeOwned + Default>(&self, doc: &Document<T>) -> Result<T, String> {
        let label = doc.label;
        let parsed = match fs::read_to_string(self.dir.join(doc.file)) {
            Ok(raw) => {
                serde_json::from_str(&raw).map_err(|e| format!("{label} file is corrupt: {e}"))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(T::default()),
            Err(e) => Err(format!("cannot read {label}: {e}")),
        };
        match doc.on_corrupt {
            OnCorrupt::Fail => parsed,
            OnCorrupt::StartEmpty => Ok(parsed.unwrap_or_else(|e| {
                eprintln!("{e}; starting over");
                T::default()
            })),
        }
    }

    /// Writes `value` as `doc`, creating the data directory if needed. The
    /// write is not atomic: a crash mid-write can leave a truncated file.
    pub fn save<T: Serialize>(&self, doc: &Document<T>, value: &T) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("cannot create app data dir: {e}"))?;
        let raw = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
        fs::write(self.dir.join(doc.file), raw)
            .map_err(|e| format!("cannot write {}: {e}", doc.label))
    }
}

#[cfg(test)]
pub mod testing {
    use super::*;

    /// A store over a fresh temporary directory. Keep the guard alive for the
    /// test's duration; dropping it deletes the directory.
    pub fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        (dir, store)
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::temp_store, *};
    use crate::merged::MergedPr;

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
        }
    }

    #[test]
    fn settings_round_trip() {
        let (_dir, store) = temp_store();
        let settings = Settings {
            repos: vec!["acme/widgets".into()],
            github_login: Some("someone".into()),
            poll_interval_secs: 300,
        };
        store.save(&SETTINGS, &settings).unwrap();
        let loaded = store.load(&SETTINGS).unwrap();
        assert_eq!(loaded.repos, ["acme/widgets"]);
        assert_eq!(loaded.github_login.as_deref(), Some("someone"));
        assert_eq!(loaded.poll_interval_secs, 300);
    }

    #[test]
    fn read_state_round_trips() {
        let (_dir, store) = temp_store();
        let state = ReadState::from([("acme/widgets#3".into(), "2026-07-11T10:00:00Z".into())]);
        store.save(&READ_STATE, &state).unwrap();
        assert_eq!(store.load(&READ_STATE).unwrap(), state);
    }

    #[test]
    fn the_merged_list_round_trips() {
        let (_dir, store) = temp_store();
        let state = vec![merged_entry()];
        store.save(&MERGED, &state).unwrap();
        assert_eq!(store.load(&MERGED).unwrap(), state);
    }

    #[test]
    fn saving_creates_a_data_directory_that_does_not_exist_yet() {
        let (dir, _) = temp_store();
        let store = Store::new(dir.path().join("not/yet/created"));
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

    /// Without `#[serde(default)]` on the field the parse fails and the load
    /// silently discards the whole Merged section.
    #[test]
    fn a_merged_file_saved_before_the_owner_avatar_field_still_loads() {
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
    }
}
