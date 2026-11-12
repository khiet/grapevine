use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{github, github::PullRequest, unread};

/// A PR that merged while tracked. Shown in the popover's Merged section
/// until the user dismisses it; persisted so the section survives restarts.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
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
    /// The production deploy of the merge commit. `None` until a deploy run
    /// is found, which is forever for a repo that has none, and for entries
    /// saved before this field existed.
    #[serde(default)]
    pub deploy: Option<Deploy>,
}

/// Where the deploy of a merge commit stands, folded from every deploy run on
/// that commit.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deploy {
    pub state: DeployState,
    /// When the run behind `state` finished (ISO-8601 UTC); `None` while
    /// deploying.
    pub completed_at: Option<String>,
    /// The workflow run behind `state`.
    pub url: String,
}

/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployState {
    Deploying,
    Deployed,
    Failed,
    Cancelled,
}

impl DeployState {
    /// A final state is never asked about again, so a run re-run after it
    /// was recorded goes unseen.
    pub fn is_final(self) -> bool {
        self != DeployState::Deploying
    }
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
        deploy: None,
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

/// How long after its merge an entry is asked about. Past it, a repo with no
/// deploy workflow stops costing a query, and a deploy that never finished
/// keeps the last state it reported.
const DEPLOY_WATCH_MS: u64 = 24 * 60 * 60 * 1000;

/// Keys of the entries whose deploy is worth asking GitHub about: no final
/// status yet, and merged within the last day.
pub fn awaiting_deploy(state: &MergedState, now_epoch_ms: u64) -> Vec<String> {
    state
        .iter()
        .filter(|entry| {
            entry
                .deploy
                .as_ref()
                .is_none_or(|deploy| !deploy.state.is_final())
        })
        .filter(|entry| {
            github::rfc3339_utc_to_epoch_secs(&entry.merged_at)
                .is_some_and(|merged| now_epoch_ms < merged * 1000 + DEPLOY_WATCH_MS)
        })
        .map(key)
        .collect()
}

/// Records what GitHub answered, by entry key. An entry missing from `found`
/// keeps what it had, and an answer for an entry that has left the section
/// is dropped. Returns true when the state changed and needs persisting.
pub fn apply_deploys(found: &HashMap<String, Deploy>, state: &mut MergedState) -> bool {
    let mut changed = false;
    for entry in state.iter_mut() {
        if let Some(deploy) = found.get(&key(entry)) {
            if entry.deploy.as_ref() != Some(deploy) {
                entry.deploy = Some(deploy.clone());
                changed = true;
            }
        }
    }
    changed
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
            author_is_bot: false,
            avatar_url: "https://avatars.example/someone".into(),
            owner_avatar_url: "https://avatars.example/acme".into(),
            created_at: "2026-07-01T00:00:00Z".into(),
            updated_at: "2026-07-01T00:00:00Z".into(),
            section: Section::All,
            blocked_reasons: vec![],
            is_draft: false,
            review_requested: false,
            team_review_requested: false,
            awaiting_review: false,
            approved: false,
            merge_queue_position: None,
            failing_checks: Vec::new(),
            failing_checks_truncated: false,
            checks_running: false,
            auto_merge: false,
            ready_to_merge: false,
            turn: None,
            turn_since: None,
            can_fix_conflict: false,
            fix_requested: false,
            mergeability_unknown: false,
            node_id: String::new(),
            unread_count: 0,
            bot_unread_count: 0,
            activity: vec![],
            bot_activity: vec![],
        }
    }

    fn entry(repo: &str, number: u64, merged_at: &str) -> MergedPr {
        from_pr(&pr(repo, number), merged_at.into())
    }

    fn deploy(state: DeployState) -> Deploy {
        Deploy {
            state,
            completed_at: state.is_final().then(|| "2026-07-12T10:09:00Z".into()),
            url: "https://github.com/acme/widgets/actions/runs/2".into(),
        }
    }

    /// 2026-07-12T10:00:00Z, the merge time the deploy tests use.
    const MERGED_AT_MS: u64 = 1_783_850_400_000;
    const HOUR_MS: u64 = 3_600_000;

    #[test]
    fn an_entry_is_asked_about_until_its_deploy_is_final() {
        let with = |state: Option<DeployState>| MergedPr {
            deploy: state.map(deploy),
            ..entry("acme/widgets", 7, "2026-07-12T10:00:00Z")
        };
        let asked = |state| !awaiting_deploy(&vec![with(state)], MERGED_AT_MS + HOUR_MS).is_empty();

        assert!(asked(None));
        assert!(asked(Some(DeployState::Deploying)));
        assert!(!asked(Some(DeployState::Deployed)));
        assert!(!asked(Some(DeployState::Failed)));
        assert!(!asked(Some(DeployState::Cancelled)));
    }

    #[test]
    fn an_entry_stops_being_asked_about_a_day_after_it_merged() {
        let state = vec![
            entry("acme/widgets", 7, "2026-07-12T10:00:00Z"),
            entry("acme/widgets", 8, "2026-07-12T11:00:00Z"),
        ];
        assert_eq!(
            awaiting_deploy(&state, MERGED_AT_MS + 24 * HOUR_MS - 1),
            ["acme/widgets#7", "acme/widgets#8"]
        );
        assert_eq!(
            awaiting_deploy(&state, MERGED_AT_MS + 24 * HOUR_MS),
            ["acme/widgets#8"]
        );
        assert!(awaiting_deploy(&state, MERGED_AT_MS + 25 * HOUR_MS).is_empty());
    }

    #[test]
    fn a_found_deploy_lands_on_its_entry_only() {
        let mut state = vec![
            entry("acme/widgets", 8, "2026-07-12T11:00:00Z"),
            entry("acme/widgets", 7, "2026-07-12T10:00:00Z"),
        ];
        let found = HashMap::from([
            ("acme/widgets#7".to_string(), deploy(DeployState::Deploying)),
            // Dismissed while the answer was on its way.
            ("acme/widgets#9".to_string(), deploy(DeployState::Failed)),
        ]);

        assert!(apply_deploys(&found, &mut state));
        assert_eq!(state[0].deploy, None);
        assert_eq!(state[1].deploy, Some(deploy(DeployState::Deploying)));
        assert_eq!(state.len(), 2);
        // The same answer again changes nothing, so nothing needs saving.
        assert!(!apply_deploys(&found, &mut state));
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
