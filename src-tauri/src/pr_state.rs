//! The one owner of what the popover shows: the Snapshot, the read watermarks
//! and the merged list. Everything outside this module changes them only
//! through the transitions on [`PrState`], which hold these invariants:
//!
//! - `snapshot.merged` mirrors the merged list. The one exception is the
//!   unconfigured Snapshot, which is empty so the popover shows setup rather
//!   than a Merged section, while the list itself is kept.
//! - Each `unread_count` and `bot_unread_count` agrees with the read
//!   watermarks.
//! - The tray count is the Snapshot's total unread.
//! - Merge detection compares a fetch against the previous Snapshot's open
//!   PRs.
//!
//! A transition never touches disk, the tray or the webview. It returns
//! [`Effects`], and the caller owes the outside world all of them, in the
//! order the transitions ran: effects delivered out of order would publish or
//! save an older state over a newer one.
//!
//! The only I/O here is [`PrState::detect_merged`], which is not a transition:
//! it asks GitHub about departed PRs on the way to `sync_succeeded`.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use serde::Serialize;

use crate::{
    github,
    github::{GithubError, PullRequest},
    merged, unread,
};

/// Latest sync result. A failed sync leaves the previous snapshot's list in
/// place so the popover keeps showing the last known PRs instead of
/// blanking; only the error field changes.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Snapshot {
    pub prs: Vec<PullRequest>,
    /// Merged-but-not-dismissed PRs, rendered as the Merged section.
    pub merged: Vec<merged::MergedPr>,
    /// False until a sync has completed while configured; the UI uses this
    /// to tell "not set up yet" apart from "no open PRs".
    pub has_synced: bool,
    /// Epoch ms of the last successful sync. Survives failed syncs: it dates
    /// the list the popover is still showing.
    pub last_sync_at: Option<u64>,
    /// User-facing message of the most recent failure; cleared by a success.
    pub sync_error: Option<String>,
}

/// What the caller must do after a transition. `snapshot` is always published
/// as `prs-updated`.
pub struct Effects {
    pub snapshot: Snapshot,
    /// Read watermarks to write to disk; `None` when there is nothing to write.
    pub save_read: Option<unread::ReadState>,
    /// Merged list to write to disk; `None` when there is nothing to write.
    pub save_merged: Option<merged::MergedState>,
    /// The tray count to show; `None` leaves the tray as it is.
    pub tray_count: Option<u64>,
}

#[derive(Default)]
struct Inner {
    snapshot: Snapshot,
    /// Both are authoritative in memory. The copies on disk exist so unread
    /// badges and the Merged section survive restarts.
    read_state: unread::ReadState,
    merged: merged::MergedState,
}

/// One lock over all three pieces, so a transition is atomic: no command can
/// land between a sync computing unread counts and storing them.
#[derive(Default)]
pub struct PrState {
    inner: Mutex<Inner>,
}

impl PrState {
    /// Seeds the state loaded from disk at startup. The Snapshot gets the
    /// merged list too, so the Merged section is visible before the first
    /// sync completes.
    pub fn restore(&self, read_state: unread::ReadState, merged: merged::MergedState) {
        let mut inner = self.inner.lock().unwrap();
        inner.snapshot.merged = merged.clone();
        inner.merged = merged;
        inner.read_state = read_state;
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().unwrap().snapshot.clone()
    }

    /// A sync fetched `prs`, and `newly_merged` are the departed PRs GitHub
    /// confirmed as merged. Unread counts are computed against the watermarks
    /// as they are now, not as they were when the fetch started.
    pub fn sync_succeeded(
        &self,
        mut prs: Vec<PullRequest>,
        newly_merged: Vec<merged::MergedPr>,
        now_epoch_ms: u64,
    ) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let inner = &mut *inner;
        let merged_changed = merged::apply(&prs, newly_merged, &mut inner.merged);
        let read_changed = unread::apply(&mut prs, &mut inner.read_state);
        inner.snapshot = Snapshot {
            prs,
            merged: inner.merged.clone(),
            has_synced: true,
            last_sync_at: Some(now_epoch_ms),
            sync_error: None,
        };
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: read_changed.then(|| inner.read_state.clone()),
            save_merged: merged_changed.then(|| inner.merged.clone()),
            tray_count: Some(total_unread(&inner.snapshot.prs)),
        }
    }

    /// A sync failed: keep the last known list and its sync time and surface
    /// only the error. The tray is left alone, since a stale count beats a
    /// vanishing one.
    pub fn sync_failed(&self, message: String) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        inner.snapshot.sync_error = Some(message);
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: None,
            save_merged: None,
            tray_count: None,
        }
    }

    /// No token or no repos: clear any list left over from previous settings,
    /// but keep the read watermarks and merged list for when a token comes
    /// back.
    pub fn unconfigured(&self) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        inner.snapshot = Snapshot::default();
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: None,
            save_merged: None,
            tray_count: Some(0),
        }
    }

    pub fn mark_read(&self, key: &str) -> Effects {
        self.mark(Some(key))
    }

    pub fn mark_all_read(&self) -> Effects {
        self.mark(None)
    }

    /// Moves the read watermark of the matching PR (or all PRs, for `None`)
    /// to its newest activity in the Snapshot. Anchoring to activity the app
    /// has actually seen means anything that lands on GitHub afterwards stays
    /// unread.
    fn mark(&self, only: Option<&str>) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let inner = &mut *inner;
        for pr in &mut inner.snapshot.prs {
            let key = unread::key(pr);
            if only.is_none_or(|k| k == key) {
                pr.unread_count = 0;
                pr.bot_unread_count = 0;
                inner.read_state.insert(key, unread::read_watermark(pr));
            }
        }
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: Some(inner.read_state.clone()),
            save_merged: None,
            tray_count: Some(total_unread(&inner.snapshot.prs)),
        }
    }

    pub fn dismiss_merged(&self, key: &str) -> Effects {
        self.edit_merged(|merged| merged.retain(|e| merged::key(e) != key))
    }

    pub fn clear_merged(&self) -> Effects {
        self.edit_merged(|merged| merged.clear())
    }

    /// The tray is left alone: merged PRs never carry unread activity.
    fn edit_merged(&self, edit: impl FnOnce(&mut merged::MergedState)) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        edit(&mut inner.merged);
        inner.snapshot.merged = inner.merged.clone();
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: None,
            save_merged: Some(inner.merged.clone()),
            tray_count: None,
        }
    }

    /// The tracked PRs that left the open set since the previous Snapshot:
    /// the candidates for having merged.
    fn departed(&self, live_prs: &[PullRequest], repos: &[String]) -> Vec<PullRequest> {
        departed_prs(&self.inner.lock().unwrap().snapshot.prs, live_prs, repos)
    }

    /// Finds tracked PRs that left the open set since the previous sync and
    /// asks GitHub which of them merged. A failed status query must fail the
    /// whole sync: the caller then reports `sync_failed`, which keeps the old
    /// Snapshot so the departure is re-detected next round and a network blip
    /// never silently drops a merge. Merges that happen while the app is quit
    /// are missed entirely: the previous open set lives only in memory.
    pub async fn detect_merged(
        &self,
        token: &str,
        repos: &[String],
        live_prs: &[PullRequest],
    ) -> Result<Vec<merged::MergedPr>, GithubError> {
        let departed = self.departed(live_prs, repos);
        if departed.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = departed.iter().map(unread::key).collect();
        let merged_at = github::fetch_merged_status(token, &keys).await?;
        Ok(merged_entries(&departed, &merged_at))
    }
}

fn total_unread(prs: &[PullRequest]) -> u64 {
    prs.iter().map(|pr| pr.unread_count).sum()
}

/// The tracked PRs in `previous` that are gone from the newly fetched
/// `live_prs`.
fn departed_prs(
    previous: &[PullRequest],
    live_prs: &[PullRequest],
    repos: &[String],
) -> Vec<PullRequest> {
    let live: HashSet<String> = live_prs.iter().map(unread::key).collect();
    previous
        .iter()
        .filter(|pr| !live.contains(&unread::key(pr)))
        // A repo removed from the watchlist takes its PRs with it; those
        // departed because of settings, not because they merged.
        .filter(|pr| repos.iter().any(|r| r.eq_ignore_ascii_case(&pr.repo)))
        .cloned()
        .collect()
}

/// Turns GitHub's answer into section entries. A departed PR missing from
/// `merged_at` was closed without merging, so it yields nothing and
/// disappears from the popover silently.
fn merged_entries(
    departed: &[PullRequest],
    merged_at: &HashMap<String, String>,
) -> Vec<merged::MergedPr> {
    departed
        .iter()
        .filter_map(|pr| {
            merged_at
                .get(&unread::key(pr))
                .map(|t| merged::from_pr(pr, t.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{BlockedReason, Section};
    use serde_json::json;

    const T1: &str = "2026-07-02T00:00:00Z";
    const T2: &str = "2026-07-03T00:00:00Z";
    const NOW_MS: u64 = 1_784_205_296_000;

    /// A PR you take part in, so one that can carry unread activity.
    fn pr(repo: &str, number: u64, activity: &[&str]) -> PullRequest {
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
            section: Section::Participated,
            blocked_reasons: vec![],
            is_draft: false,
            review_requested: false,
            team_review_requested: false,
            awaiting_review: false,
            approved: false,
            merge_queue_position: None,
            unread_count: 0,
            bot_unread_count: 0,
            activity: activity.iter().map(|t| t.to_string()).collect(),
            bot_activity: vec![],
        }
    }

    fn repos() -> Vec<String> {
        vec!["acme/widgets".to_string()]
    }

    /// State after two syncs: #7 was first seen with one comment and has
    /// gained a second, so it carries one unread; #8 is quiet.
    fn state_with_one_unread() -> PrState {
        let state = PrState::default();
        state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[T1]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );
        state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[T1, T2]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );
        state
    }

    fn unread_counts(snapshot: &Snapshot) -> Vec<u64> {
        snapshot.prs.iter().map(|pr| pr.unread_count).collect()
    }

    #[test]
    fn a_failed_sync_keeps_the_pr_list_and_sets_only_the_error() {
        let state = PrState::default();
        let before = state
            .sync_succeeded(vec![pr("acme/widgets", 7, &[T1])], vec![], NOW_MS)
            .snapshot;

        let effects = state.sync_failed("Could not reach GitHub.".into());

        let after = effects.snapshot;
        assert_eq!(after.sync_error.as_deref(), Some("Could not reach GitHub."));
        assert_eq!(after.prs.len(), 1);
        assert_eq!(after.prs[0].number, 7);
        assert_eq!(after.merged, before.merged);
        assert_eq!(after.has_synced, before.has_synced);
        assert_eq!(after.last_sync_at, Some(NOW_MS));
        assert_eq!(effects.save_read, None);
        assert_eq!(effects.save_merged, None);
    }

    #[test]
    fn a_failed_sync_does_not_change_the_tray_count() {
        let state = state_with_one_unread();
        let effects = state.sync_failed("Could not reach GitHub.".into());
        assert_eq!(effects.tray_count, None);
        assert_eq!(unread_counts(&effects.snapshot), [1, 0]);
    }

    #[test]
    fn a_successful_sync_clears_the_error_and_counts_unread_into_the_tray() {
        let state = state_with_one_unread();
        state.sync_failed("Could not reach GitHub.".into());

        let effects = state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[T1, T2]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS + 1,
        );

        assert_eq!(effects.snapshot.sync_error, None);
        assert_eq!(effects.snapshot.last_sync_at, Some(NOW_MS + 1));
        assert_eq!(unread_counts(&effects.snapshot), [1, 0]);
        assert_eq!(effects.tray_count, Some(1));
    }

    #[test]
    fn a_sync_reports_the_watermarks_for_saving_only_when_they_changed() {
        let state = PrState::default();
        let first = state.sync_succeeded(vec![pr("acme/widgets", 7, &[T1])], vec![], NOW_MS);
        assert_eq!(
            first.save_read,
            Some(HashMap::from([(
                "acme/widgets#7".to_string(),
                T1.to_string()
            )]))
        );

        let second = state.sync_succeeded(vec![pr("acme/widgets", 7, &[T1])], vec![], NOW_MS);
        assert_eq!(second.save_read, None);
        assert_eq!(second.save_merged, None);
    }

    #[test]
    fn an_unconfigured_sync_clears_the_pr_list_and_keeps_the_read_watermarks() {
        let state = state_with_one_unread();

        let effects = state.unconfigured();

        assert!(effects.snapshot.prs.is_empty());
        assert!(!effects.snapshot.has_synced);
        assert_eq!(effects.save_read, None);
        assert_eq!(effects.tray_count, Some(0));
        // Had the watermark been dropped, #7 would be baselined as read here.
        let back = state.sync_succeeded(vec![pr("acme/widgets", 7, &[T1, T2])], vec![], NOW_MS);
        assert_eq!(unread_counts(&back.snapshot), [1]);
    }

    /// The previous open set is gone, so reconfiguring cannot report the PRs
    /// of the old settings as departed.
    #[test]
    fn nothing_has_departed_after_an_unconfigured_sync() {
        let state = state_with_one_unread();
        state.unconfigured();
        assert!(state.departed(&[], &repos()).is_empty());
    }

    #[test]
    fn mark_read_moves_the_watermark_to_the_seen_activity_and_clears_that_pr() {
        let state = PrState::default();
        let both = || vec![pr("acme/widgets", 7, &[T1]), pr("acme/widgets", 8, &[T1])];
        state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );
        state.sync_succeeded(both(), vec![], NOW_MS);

        let effects = state.mark_read("acme/widgets#7");

        assert_eq!(unread_counts(&effects.snapshot), [0, 1]);
        assert_eq!(effects.tray_count, Some(1));
        let saved = effects.save_read.expect("watermarks need saving");
        assert_eq!(saved["acme/widgets#7"], T1);
        // Activity that lands after the mark is unread again.
        let mut later = both();
        later[0].activity.push(T2.to_string());
        let next = state.sync_succeeded(later, vec![], NOW_MS);
        assert_eq!(unread_counts(&next.snapshot), [1, 1]);
    }

    #[test]
    fn mark_all_read_clears_every_unread_count_and_the_tray() {
        let state = PrState::default();
        state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );
        let unread = state.sync_succeeded(
            vec![
                pr("acme/widgets", 7, &[T1]),
                pr("acme/widgets", 8, &[T1, T2]),
            ],
            vec![],
            NOW_MS,
        );
        assert_eq!(unread.tray_count, Some(3));

        let effects = state.mark_all_read();

        assert_eq!(unread_counts(&effects.snapshot), [0, 0]);
        assert_eq!(effects.tray_count, Some(0));
        assert_eq!(unread_counts(&state.snapshot()), [0, 0]);
    }

    /// Unread counts come from the watermarks at the moment the result is
    /// applied, so a fetch that started earlier cannot bring back a badge
    /// cleared meanwhile.
    #[test]
    fn a_mark_read_between_a_fetch_starting_and_its_result_arriving_survives() {
        let state = state_with_one_unread();
        // The fetch starts and returns what GitHub has now.
        let fetched = vec![pr("acme/widgets", 7, &[T1, T2]), pr("acme/widgets", 8, &[])];
        let newly_merged = merged_entries(&state.departed(&fetched, &repos()), &HashMap::new());

        state.mark_read("acme/widgets#7");
        let effects = state.sync_succeeded(fetched, newly_merged, NOW_MS);

        assert_eq!(unread_counts(&effects.snapshot), [0, 0]);
        assert_eq!(effects.tray_count, Some(0));
    }

    #[test]
    fn a_pr_is_open_then_departs_is_reported_merged_and_is_dismissed() {
        let state = PrState::default();
        let open = state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[T1]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );
        assert_eq!(open.snapshot.prs.len(), 2);
        assert!(open.snapshot.merged.is_empty());

        // Next fetch: #7 is gone from the open list and GitHub says it merged.
        let live = vec![pr("acme/widgets", 8, &[])];
        let departed = state.departed(&live, &repos());
        assert_eq!(departed.len(), 1);
        assert_eq!(departed[0].number, 7);
        let merged_at = HashMap::from([("acme/widgets#7".to_string(), T2.to_string())]);
        let synced =
            state.sync_succeeded(live.clone(), merged_entries(&departed, &merged_at), NOW_MS);

        assert_eq!(synced.snapshot.prs.len(), 1);
        assert_eq!(synced.snapshot.merged.len(), 1);
        assert_eq!(synced.snapshot.merged[0].number, 7);
        assert_eq!(synced.save_merged, Some(synced.snapshot.merged.clone()));

        // A later sync neither re-detects the merge nor drops the entry.
        assert!(state.departed(&live, &repos()).is_empty());
        let quiet = state.sync_succeeded(live.clone(), vec![], NOW_MS);
        assert_eq!(quiet.snapshot.merged.len(), 1);
        assert_eq!(quiet.save_merged, None);

        let dismissed = state.dismiss_merged("acme/widgets#7");
        assert!(dismissed.snapshot.merged.is_empty());
        assert_eq!(dismissed.save_merged, Some(vec![]));
        let after = state.sync_succeeded(live, vec![], NOW_MS);
        assert!(after.snapshot.merged.is_empty());
    }

    #[test]
    fn dismissing_a_merged_pr_leaves_the_tray_count_untouched() {
        let state = PrState::default();
        state.restore(
            HashMap::from([("acme/widgets#7".to_string(), T1.to_string())]),
            vec![
                merged::from_pr(&pr("acme/widgets", 4, &[]), T2.to_string()),
                merged::from_pr(&pr("acme/widgets", 3, &[]), T1.to_string()),
            ],
        );
        state.sync_succeeded(
            vec![pr("acme/widgets", 7, &[T1, T2]), pr("acme/widgets", 8, &[])],
            vec![],
            NOW_MS,
        );

        let effects = state.dismiss_merged("acme/widgets#3");

        assert_eq!(effects.tray_count, None);
        assert_eq!(effects.save_read, None);
        assert_eq!(effects.snapshot.merged.len(), 1);
        assert_eq!(effects.snapshot.merged[0].number, 4);
        assert_eq!(unread_counts(&effects.snapshot), [1, 0]);
    }

    #[test]
    fn clearing_merged_empties_the_section_and_leaves_the_tray_count_untouched() {
        let state = PrState::default();
        state.restore(
            HashMap::new(),
            vec![merged::from_pr(&pr("acme/widgets", 3, &[]), T1.to_string())],
        );

        let effects = state.clear_merged();

        assert!(effects.snapshot.merged.is_empty());
        assert_eq!(effects.save_merged, Some(vec![]));
        assert_eq!(effects.tray_count, None);
    }

    #[test]
    fn restored_merged_prs_show_before_the_first_sync() {
        let state = PrState::default();
        state.restore(
            HashMap::new(),
            vec![merged::from_pr(&pr("acme/widgets", 3, &[]), T1.to_string())],
        );
        let snapshot = state.snapshot();
        assert_eq!(snapshot.merged.len(), 1);
        assert!(!snapshot.has_synced);
    }

    /// Unconfigured hides the Merged section so the popover shows setup, but
    /// the list comes back with the token.
    #[test]
    fn an_unconfigured_sync_hides_the_merged_list_without_losing_it() {
        let state = PrState::default();
        state.restore(
            HashMap::new(),
            vec![merged::from_pr(&pr("acme/widgets", 3, &[]), T1.to_string())],
        );

        let hidden = state.unconfigured();
        assert!(hidden.snapshot.merged.is_empty());
        assert_eq!(hidden.save_merged, None);

        let back = state.sync_succeeded(vec![], vec![], NOW_MS);
        assert_eq!(back.snapshot.merged.len(), 1);
    }

    #[test]
    fn a_pr_gone_from_the_open_list_has_departed() {
        let previous = vec![pr("acme/widgets", 7, &[]), pr("acme/widgets", 8, &[])];
        let live = vec![pr("acme/widgets", 8, &[])];
        let departed = departed_prs(&previous, &live, &repos());
        assert_eq!(departed.len(), 1);
        assert_eq!(departed[0].number, 7);
    }

    #[test]
    fn prs_still_open_have_not_departed() {
        let prs = vec![pr("acme/widgets", 7, &[])];
        assert!(departed_prs(&prs, &prs, &repos()).is_empty());
    }

    /// Un-watching a repo drops its PRs from the fetch, so without this guard
    /// removing a repo would spray its open PRs into the Merged section.
    #[test]
    fn prs_of_an_unwatched_repo_never_count_as_departed() {
        let previous = vec![pr("acme/widgets", 7, &[]), pr("acme/gadgets", 1, &[])];
        let departed = departed_prs(&previous, &[], &repos());
        assert_eq!(departed.len(), 1);
        assert_eq!(departed[0].repo, "acme/widgets");
    }

    /// The watchlist stores GitHub's canonical casing while a PR's repo name
    /// comes back from the API; a case mismatch must not read as un-watched.
    #[test]
    fn the_watchlist_check_ignores_repo_name_casing() {
        let previous = vec![pr("acme/widgets", 7, &[])];
        let repos = vec!["Acme/Widgets".to_string()];
        assert_eq!(departed_prs(&previous, &[], &repos).len(), 1);
    }

    #[test]
    fn departed_prs_that_merged_become_section_entries() {
        let departed = vec![pr("acme/widgets", 7, &[])];
        let merged_at = HashMap::from([(
            "acme/widgets#7".to_string(),
            "2026-07-12T10:00:00Z".to_string(),
        )]);
        let entries = merged_entries(&departed, &merged_at);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].number, 7);
        assert_eq!(entries[0].title, "Fix the thing");
        assert_eq!(entries[0].merged_at, "2026-07-12T10:00:00Z");
    }

    /// A PR closed without merging is absent from GitHub's answer.
    #[test]
    fn departed_prs_that_did_not_merge_yield_no_entry() {
        let departed = vec![pr("acme/widgets", 7, &[])];
        assert!(merged_entries(&departed, &HashMap::new()).is_empty());
    }

    /// The `Snapshot` interface in `src/PrList.tsx` consumes this JSON
    /// verbatim: field names and section values are the wire contract, so a
    /// rename or a `rename_all` attribute here silently breaks the popover.
    #[test]
    fn snapshot_serializes_to_the_shape_the_frontend_expects() {
        let snapshot = Snapshot {
            prs: vec![PullRequest {
                number: 7,
                title: "Fix the thing".into(),
                url: "https://github.com/acme/widgets/pull/7".into(),
                repo: "acme/widgets".into(),
                author: "copilot".into(),
                author_is_bot: true,
                avatar_url: "https://avatars.example/copilot".into(),
                owner_avatar_url: "https://avatars.example/acme".into(),
                created_at: "2026-07-10T12:00:00Z".into(),
                updated_at: "2026-07-11T09:30:00Z".into(),
                section: Section::Mine,
                blocked_reasons: vec![BlockedReason::Ci],
                is_draft: true,
                review_requested: false,
                team_review_requested: false,
                awaiting_review: false,
                approved: true,
                merge_queue_position: Some(2),
                unread_count: 3,
                bot_unread_count: 8,
                // Internal working data; must not leak into the payload.
                activity: vec!["2026-07-10T12:00:00Z".into()],
                bot_activity: vec!["2026-07-10T12:00:00Z".into()],
            }],
            merged: vec![merged::MergedPr {
                number: 3,
                title: "Ship the other thing".into(),
                url: "https://github.com/acme/widgets/pull/3".into(),
                repo: "acme/widgets".into(),
                author: "someone".into(),
                avatar_url: "https://avatars.example/someone".into(),
                owner_avatar_url: "https://avatars.example/acme".into(),
                merged_at: "2026-07-11T10:00:00Z".into(),
            }],
            has_synced: true,
            last_sync_at: Some(1_784_205_296_000),
            sync_error: Some("Could not reach GitHub.".into()),
        };
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            json!({
                "prs": [{
                    "number": 7,
                    "title": "Fix the thing",
                    "url": "https://github.com/acme/widgets/pull/7",
                    "repo": "acme/widgets",
                    "author": "copilot",
                    "author_is_bot": true,
                    "avatar_url": "https://avatars.example/copilot",
                    "owner_avatar_url": "https://avatars.example/acme",
                    "created_at": "2026-07-10T12:00:00Z",
                    "updated_at": "2026-07-11T09:30:00Z",
                    "section": "mine",
                    "blocked_reasons": ["ci"],
                    "is_draft": true,
                    "review_requested": false,
                    "team_review_requested": false,
                    "awaiting_review": false,
                    "approved": true,
                    "merge_queue_position": 2,
                    "unread_count": 3,
                    "bot_unread_count": 8
                }],
                "merged": [{
                    "number": 3,
                    "title": "Ship the other thing",
                    "url": "https://github.com/acme/widgets/pull/3",
                    "repo": "acme/widgets",
                    "author": "someone",
                    "avatar_url": "https://avatars.example/someone",
                    "owner_avatar_url": "https://avatars.example/acme",
                    "merged_at": "2026-07-11T10:00:00Z"
                }],
                "has_synced": true,
                "last_sync_at": 1_784_205_296_000u64,
                "sync_error": "Could not reach GitHub."
            })
        );
    }

    /// The footer reads null as "never synced" / "no error", so the fields
    /// must serialize as null rather than being omitted.
    #[test]
    fn absent_sync_status_serializes_as_nulls() {
        let value = serde_json::to_value(Snapshot::default()).unwrap();
        assert_eq!(value["last_sync_at"], json!(null));
        assert_eq!(value["sync_error"], json!(null));
    }
}
