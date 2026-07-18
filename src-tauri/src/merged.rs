use std::{collections::HashSet, fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

use crate::{github::PullRequest, unread};

/// A PR that merged while tracked. Shown in the popover's Merged section
/// until the user dismisses it; persisted so the section survives restarts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MergedPr {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub author: String,
    pub avatar_url: String,
    /// Repo owner's avatar URL, snapshotted for the org badge. Empty for
    /// entries saved before this field existed; the UI shows no badge then.
    #[serde(default)]
    pub owner_avatar_url: String,
    /// GitHub's mergedAt (ISO-8601 UTC); orders the section newest first.
    pub merged_at: String,
}

/// Merged-but-not-dismissed PRs, newest merge first.
pub type MergedState = Vec<MergedPr>;

pub fn key(entry: &MergedPr) -> String {
    format!("{}#{}", entry.repo, entry.number)
}

/// Captures the display data of a departed PR at the moment the merge is
/// detected — merged PRs are gone from later fetches, so this snapshot is
/// all the section will ever have.
pub fn from_pr(pr: &PullRequest, merged_at: String) -> MergedPr {
    MergedPr {
        number: pr.number,
        title: pr.title.clone(),
        url: pr.url.clone(),
        repo: pr.repo.clone(),
        author: pr.author.clone(),
        avatar_url: pr.avatar_url.clone(),
        owner_avatar_url: pr.owner_avatar_url.clone(),
        merged_at,
    }
}

/// Folds one sync's merge detections into the section: a reopened PR leaves
/// (it belongs to the open list, and re-enters here if it merges again), and
/// each newly merged PR replaces any older entry sharing its key. Returns true
/// when the state changed and needs persisting.
pub fn apply(live: &[PullRequest], newly_merged: Vec<MergedPr>, state: &mut MergedState) -> bool {
    let live: HashSet<String> = live.iter().map(unread::key).collect();
    let before = state.len();
    state.retain(|entry| !live.contains(&key(entry)));
    let mut changed = state.len() != before;

    for entry in newly_merged {
        state.retain(|e| key(e) != key(&entry));
        state.push(entry);
        changed = true;
    }
    // ISO-8601 UTC timestamps sort lexically; newest merge first.
    state.sort_by(|a, b| b.merged_at.cmp(&a.merged_at));
    changed
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {e}"))?;
    Ok(dir.join("merged.json"))
}

/// A missing or unreadable file degrades to an empty section instead of
/// failing startup: undismissed merges are lost, which is annoying but never
/// wrong.
pub fn load(app: &tauri::AppHandle) -> MergedState {
    let Ok(path) = path(app) else {
        return MergedState::default();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return MergedState::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("merged state file is corrupt, starting over: {e}");
        MergedState::default()
    })
}

pub fn save(app: &tauri::AppHandle, state: &MergedState) -> Result<(), String> {
    let path = path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create app data dir: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("cannot write merged state: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Section;

    fn pr(repo: &str, number: u64) -> PullRequest {
        PullRequest {
            number,
            title: "Fix the thing".into(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            repo: repo.into(),
            author: "someone".into(),
            avatar_url: "https://avatars.example/someone".into(),
            owner_avatar_url: "https://avatars.example/acme".into(),
            created_at: "2026-07-01T00:00:00Z".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
            section: Section::All,
            unread_count: 0,
            activity: vec![],
        }
    }

    fn entry(repo: &str, number: u64, merged_at: &str) -> MergedPr {
        from_pr(&pr(repo, number), merged_at.into())
    }

    #[test]
    fn newly_merged_prs_enter_the_section() {
        let mut state = MergedState::new();
        let changed = apply(
            &[pr("acme/widgets", 1)],
            vec![entry("acme/widgets", 7, "2026-07-12T10:00:00Z")],
            &mut state,
        );
        assert!(changed);
        assert_eq!(state.len(), 1);
        assert_eq!(key(&state[0]), "acme/widgets#7");
    }

    #[test]
    fn a_sync_without_merges_leaves_the_section_alone() {
        let mut state = vec![entry("acme/widgets", 7, "2026-07-12T10:00:00Z")];
        let changed = apply(&[pr("acme/widgets", 1)], vec![], &mut state);
        assert!(!changed);
        assert_eq!(state.len(), 1);
    }

    #[test]
    fn a_reopened_pr_leaves_the_section() {
        let mut state = vec![entry("acme/widgets", 7, "2026-07-12T10:00:00Z")];
        let changed = apply(&[pr("acme/widgets", 7)], vec![], &mut state);
        assert!(changed);
        assert!(state.is_empty());
    }

    #[test]
    fn a_re_merge_replaces_the_older_entry_instead_of_duplicating() {
        let mut state = vec![entry("acme/widgets", 7, "2026-07-12T10:00:00Z")];
        apply(
            &[],
            vec![entry("acme/widgets", 7, "2026-07-14T09:00:00Z")],
            &mut state,
        );
        assert_eq!(state.len(), 1);
        assert_eq!(state[0].merged_at, "2026-07-14T09:00:00Z");
    }

    #[test]
    fn entries_sort_newest_merge_first() {
        let mut state = vec![entry("acme/widgets", 1, "2026-07-10T10:00:00Z")];
        apply(
            &[],
            vec![
                entry("acme/widgets", 2, "2026-07-14T09:00:00Z"),
                entry("acme/widgets", 3, "2026-07-12T09:00:00Z"),
            ],
            &mut state,
        );
        let numbers: Vec<u64> = state.iter().map(|e| e.number).collect();
        assert_eq!(numbers, vec![2, 3, 1]);
    }
}
