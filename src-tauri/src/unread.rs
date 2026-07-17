use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

use tauri::Manager;

use crate::github::PullRequest;

/// Per-PR last-read watermarks, keyed by `owner/repo#number`. A PR's unread
/// count is its activity strictly newer than the watermark; ISO-8601 UTC
/// timestamps make plain string comparison correct.
pub type ReadState = HashMap<String, String>;

pub fn key(pr: &PullRequest) -> String {
    format!("{}#{}", pr.repo, pr.number)
}

/// The watermark that makes `pr` fully read: its newest known activity, or its
/// creation time when there is no discussion yet. Anchoring to GitHub's own
/// timestamps rather than the local clock keeps this immune to clock skew.
pub fn read_watermark(pr: &PullRequest) -> String {
    pr.activity
        .last()
        .cloned()
        .unwrap_or_else(|| pr.created_at.clone())
}

/// Computes each PR's unread count from the watermarks, mutating both sides.
/// PRs seen for the first time are baselined as read at their newest activity,
/// so a fresh install (or a lost state file) never flags historical discussion;
/// watermarks for PRs that left the list are dropped. Returns true when the
/// state changed and needs persisting.
pub fn apply(prs: &mut [PullRequest], state: &mut ReadState) -> bool {
    let live: HashSet<String> = prs.iter().map(key).collect();
    let before = state.len();
    state.retain(|k, _| live.contains(k));
    let mut changed = state.len() != before;

    for pr in prs.iter_mut() {
        match state.get(&key(pr)) {
            Some(watermark) => {
                pr.unread_count = pr
                    .activity
                    .iter()
                    .filter(|t| t.as_str() > watermark.as_str())
                    .count() as u64;
            }
            None => {
                state.insert(key(pr), read_watermark(pr));
                pr.unread_count = 0;
                changed = true;
            }
        }
    }
    changed
}

fn path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {e}"))?;
    Ok(dir.join("unread.json"))
}

/// A missing or unreadable file degrades to an empty state instead of failing
/// sync: baselining then re-marks everything read, which loses pending unread
/// counts but never mis-flags old activity.
pub fn load(app: &tauri::AppHandle) -> ReadState {
    let Ok(path) = path(app) else {
        return ReadState::default();
    };
    let Ok(raw) = fs::read_to_string(&path) else {
        return ReadState::default();
    };
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        eprintln!("unread state file is corrupt, starting over: {e}");
        ReadState::default()
    })
}

pub fn save(app: &tauri::AppHandle, state: &ReadState) -> Result<(), String> {
    let path = path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create app data dir: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("cannot write unread state: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Section;

    fn pr(repo: &str, number: u64, activity: &[&str]) -> PullRequest {
        PullRequest {
            number,
            title: "Fix the thing".into(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            repo: repo.into(),
            author: "someone".into(),
            avatar_url: "https://avatars.example/someone".into(),
            created_at: "2026-07-01T00:00:00Z".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
            section: Section::All,
            unread_count: 0,
            activity: activity.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn first_sight_baselines_as_read_instead_of_flagging_history() {
        let mut prs = vec![pr("acme/widgets", 7, &["2026-07-10T12:00:00Z"])];
        let mut state = ReadState::new();
        let changed = apply(&mut prs, &mut state);
        assert!(changed);
        assert_eq!(prs[0].unread_count, 0);
        assert_eq!(
            state.get("acme/widgets#7").map(String::as_str),
            Some("2026-07-10T12:00:00Z")
        );
    }

    #[test]
    fn first_sight_of_a_pr_without_discussion_baselines_at_creation() {
        let mut prs = vec![pr("acme/widgets", 7, &[])];
        let mut state = ReadState::new();
        apply(&mut prs, &mut state);
        assert_eq!(
            state.get("acme/widgets#7").map(String::as_str),
            Some("2026-07-01T00:00:00Z")
        );
    }

    #[test]
    fn only_activity_after_the_watermark_counts_as_unread() {
        let mut prs = vec![pr(
            "acme/widgets",
            7,
            &[
                "2026-07-10T12:00:00Z", // read (at the watermark, not after)
                "2026-07-11T08:00:00Z",
                "2026-07-12T09:30:00Z",
            ],
        )];
        let mut state = ReadState::from([(
            "acme/widgets#7".to_string(),
            "2026-07-10T12:00:00Z".to_string(),
        )]);
        let changed = apply(&mut prs, &mut state);
        assert!(!changed);
        assert_eq!(prs[0].unread_count, 2);
    }

    #[test]
    fn watermarks_for_departed_prs_are_pruned() {
        let mut prs = vec![pr("acme/widgets", 7, &[])];
        let mut state = ReadState::from([
            (
                "acme/widgets#7".to_string(),
                "2026-07-01T00:00:00Z".to_string(),
            ),
            (
                "acme/widgets#3".to_string(),
                "2026-06-01T00:00:00Z".to_string(),
            ),
        ]);
        let changed = apply(&mut prs, &mut state);
        assert!(changed);
        assert_eq!(state.len(), 1);
        assert!(state.contains_key("acme/widgets#7"));
    }

    #[test]
    fn unchanged_state_reports_nothing_to_persist() {
        let mut prs = vec![pr("acme/widgets", 7, &["2026-07-10T12:00:00Z"])];
        let mut state = ReadState::from([(
            "acme/widgets#7".to_string(),
            "2026-07-01T00:00:00Z".to_string(),
        )]);
        assert!(!apply(&mut prs, &mut state));
        assert_eq!(prs[0].unread_count, 1);
    }

    #[test]
    fn read_watermark_prefers_the_newest_activity() {
        let with = pr("a/b", 1, &["2026-07-10T12:00:00Z", "2026-07-12T09:30:00Z"]);
        assert_eq!(read_watermark(&with), "2026-07-12T09:30:00Z");
        let without = pr("a/b", 1, &[]);
        assert_eq!(read_watermark(&without), "2026-07-01T00:00:00Z");
    }
}
