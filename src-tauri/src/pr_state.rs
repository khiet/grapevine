//! The one owner of what the popover shows: the Snapshot, the read
//! watermarks, the turn starts, the snoozes and the merged list. Everything outside this
//! module changes them only through the transitions on [`PrState`], which
//! hold these invariants:
//!
//! - `snapshot.merged` mirrors the merged list. The one exception is the
//!   unconfigured Snapshot, which is empty so the popover shows setup rather
//!   than a Merged section, while the list itself is kept.
//! - Each `unread_count` and `bot_unread_count` agrees with the read
//!   watermarks.
//! - The tray count is the number of PRs in the Snapshot whose `turn` is
//!   yours and that are not snoozed. Unread never feeds it.
//! - `snapshot.snoozed` mirrors the snoozes, which hold entries only for PRs
//!   in the Snapshot.
//! - Each `turn_since` agrees with the turn starts, which hold an entry for
//!   exactly the PRs whose turn was yours at the last successful sync.
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
    merged, snooze, turns, unread,
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
    /// Keys of the snoozed PRs, sorted. Each names a PR in `prs`, which the
    /// popover folds away at the foot of its section.
    pub snoozed: Vec<String>,
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
    /// Turn starts to write to disk; `None` when there is nothing to write.
    pub save_turns: Option<turns::TurnStarts>,
    /// Merged list to write to disk; `None` when there is nothing to write.
    pub save_merged: Option<merged::MergedState>,
    /// Snoozes to write to disk; `None` when there is nothing to write.
    pub save_snoozes: Option<snooze::Snoozes>,
    /// The tray count to show; `None` leaves the tray as it is.
    pub tray_count: Option<u64>,
}

#[derive(Default)]
struct Inner {
    snapshot: Snapshot,
    /// This and the three fields below are authoritative in memory. The
    /// copies on disk exist so unread badges, waiting times, snoozes and the
    /// Merged section survive restarts.
    read_state: unread::ReadState,
    /// `None` until turns have been tracked at all; see [`turns::apply`].
    turn_starts: Option<turns::TurnStarts>,
    merged: merged::MergedState,
    snoozes: snooze::Snoozes,
}

impl Inner {
    fn tray_count(&self) -> u64 {
        turns::yours_count(&self.snapshot.prs, &self.snoozes)
    }

    /// Restores the mirror invariant after the snoozes changed.
    fn mirror_snoozes(&mut self) {
        let mut keys: Vec<String> = self.snoozes.keys().cloned().collect();
        keys.sort();
        self.snapshot.snoozed = keys;
    }

    /// The effects of a transition that changed nothing but the snoozes.
    fn snooze_effects(&mut self, changed: bool) -> Effects {
        self.mirror_snoozes();
        Effects {
            snapshot: self.snapshot.clone(),
            save_read: None,
            save_turns: None,
            save_merged: None,
            save_snoozes: changed.then(|| self.snoozes.clone()),
            tray_count: changed.then(|| self.tray_count()),
        }
    }
}

/// One lock over every piece, so a transition is atomic: no command can
/// land between a sync computing unread counts and storing them.
#[derive(Default)]
pub struct PrState {
    inner: Mutex<Inner>,
}

impl PrState {
    /// Seeds the state loaded from disk at startup. The Snapshot gets the
    /// merged list too, so the Merged section is visible before the first
    /// sync completes.
    pub fn restore(
        &self,
        read_state: unread::ReadState,
        merged: merged::MergedState,
        turn_starts: Option<turns::TurnStarts>,
        snoozes: snooze::Snoozes,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.snoozes = snoozes;
        inner.snapshot.merged = merged.clone();
        inner.merged = merged;
        inner.read_state = read_state;
        inner.turn_starts = turn_starts;
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
        // Before the unread pass: that pass baselines first-seen PRs into the
        // watermarks, and the turn pass tells first-seen by their absence.
        let turns_changed = turns::apply(
            &mut prs,
            &mut inner.turn_starts,
            &inner.read_state,
            now_epoch_ms,
        );
        let read_changed = unread::apply(&mut prs, &mut inner.read_state);
        let snoozes_changed = snooze::apply(&prs, &mut inner.snoozes, now_epoch_ms);
        inner.snapshot = Snapshot {
            prs,
            merged: inner.merged.clone(),
            snoozed: Vec::new(),
            has_synced: true,
            last_sync_at: Some(now_epoch_ms),
            sync_error: None,
        };
        inner.mirror_snoozes();
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: read_changed.then(|| inner.read_state.clone()),
            save_turns: turns_changed.then(|| inner.turn_starts.clone()).flatten(),
            save_merged: merged_changed.then(|| inner.merged.clone()),
            save_snoozes: snoozes_changed.then(|| inner.snoozes.clone()),
            tray_count: Some(inner.tray_count()),
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
            save_turns: None,
            save_merged: None,
            save_snoozes: None,
            tray_count: None,
        }
    }

    /// No token or no repos: clear any list left over from previous settings,
    /// but keep the read watermarks and merged list for when a token comes
    /// back. The turn starts are forgotten: every PR has left the Snapshot,
    /// and nothing watches the turns until syncing resumes. The snoozes go
    /// with the PRs they parked.
    pub fn unconfigured(&self) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let inner = &mut *inner;
        inner.snapshot = Snapshot::default();
        let turns_changed = turns::forget_starts(&mut inner.turn_starts);
        let snoozes_changed = !inner.snoozes.is_empty();
        inner.snoozes.clear();
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: None,
            save_turns: turns_changed.then(|| inner.turn_starts.clone()).flatten(),
            save_merged: None,
            save_snoozes: snoozes_changed.then(|| inner.snoozes.clone()),
            tray_count: Some(inner.tray_count()),
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
    /// unread. Reading is not acting, so the turn starts and the tray count
    /// come out as they went in.
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
            save_turns: None,
            save_merged: None,
            save_snoozes: None,
            tray_count: Some(inner.tray_count()),
        }
    }

    /// Parks the matching PR until it has news or `expires_at` (epoch ms)
    /// passes. Nothing happens for a key that is not a Mine or Participated
    /// PR in the Snapshot. The read watermark stays where it is: snoozing is
    /// not reading.
    pub fn snooze(&self, key: &str, now_epoch_ms: u64, expires_at: u64) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let inner = &mut *inner;
        let parked = inner
            .snapshot
            .prs
            .iter()
            .find(|pr| unread::key(pr) == key && snooze::can_snooze(pr))
            .map(|pr| snooze::entry(pr, now_epoch_ms, expires_at));
        let changed = parked.is_some();
        if let Some(entry) = parked {
            inner.snoozes.insert(key.to_string(), entry);
        }
        inner.snooze_effects(changed)
    }

    /// Wakes the matching PR by hand.
    pub fn wake(&self, key: &str) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let changed = inner.snoozes.remove(key).is_some();
        inner.snooze_effects(changed)
    }

    /// Wakes every PR whose snooze has run out by `now_epoch_ms`. Needs no
    /// sync, so a failing or rate-limited GitHub cannot keep a row parked.
    pub fn wake_expired(&self, now_epoch_ms: u64) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        let changed = snooze::wake_expired(&mut inner.snoozes, now_epoch_ms);
        inner.snooze_effects(changed)
    }

    /// When the clock next has a snoozed row to wake, in epoch ms.
    pub fn next_snooze_expiry(&self) -> Option<u64> {
        snooze::next_expiry(&self.inner.lock().unwrap().snoozes)
    }

    pub fn dismiss_merged(&self, key: &str) -> Effects {
        self.edit_merged(|merged| merged.retain(|e| merged::key(e) != key))
    }

    pub fn clear_merged(&self) -> Effects {
        self.edit_merged(|merged| merged.clear())
    }

    /// The tray is left alone: a merged PR waits on nobody.
    fn edit_merged(&self, edit: impl FnOnce(&mut merged::MergedState)) -> Effects {
        let mut inner = self.inner.lock().unwrap();
        edit(&mut inner.merged);
        inner.snapshot.merged = inner.merged.clone();
        Effects {
            snapshot: inner.snapshot.clone(),
            save_read: None,
            save_turns: None,
            save_merged: Some(inner.merged.clone()),
            save_snoozes: None,
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
    use crate::github::{BlockedReason, Section, Turn};
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
            failing_checks: Vec::new(),
            failing_checks_truncated: false,
            checks_running: false,
            auto_merge: false,
            ready_to_merge: false,
            turn: None,
            turn_since: None,
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

    /// The same PR, waiting on you.
    fn yours(pr: PullRequest) -> PullRequest {
        PullRequest {
            turn: Some(Turn::Yours),
            ..pr
        }
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
    fn a_successful_sync_clears_the_error_and_keeps_unread_out_of_the_tray() {
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
        assert_eq!(effects.tray_count, Some(0));
    }

    #[test]
    fn the_tray_counts_the_prs_waiting_on_you_not_the_unread() {
        let state = PrState::default();
        let prs = |quiet: &[&str], busy: &[&str]| {
            vec![
                yours(pr("acme/widgets", 1, &[])),
                yours(pr("acme/widgets", 2, &[])),
                pr("acme/widgets", 3, quiet),
                PullRequest {
                    turn: Some(Turn::Theirs),
                    ..pr("acme/widgets", 4, busy)
                },
            ]
        };
        state.sync_succeeded(prs(&[], &[]), vec![], NOW_MS);

        let effects = state.sync_succeeded(
            prs(&[T1, T2], &[T1, T2, "2026-07-04T00:00:00Z"]),
            vec![],
            NOW_MS,
        );

        assert_eq!(unread_counts(&effects.snapshot), [0, 0, 2, 3]);
        assert_eq!(effects.tray_count, Some(2));
    }

    /// Reading is not acting: the PR still waits on you until a sync says
    /// otherwise.
    #[test]
    fn marking_read_leaves_the_tray_count_unchanged() {
        let state = PrState::default();
        let prs = |activity: &[&str]| {
            vec![
                yours(pr("acme/widgets", 7, activity)),
                pr("acme/widgets", 8, activity),
            ]
        };
        state.sync_succeeded(prs(&[]), vec![], NOW_MS);
        let synced = state.sync_succeeded(prs(&[T1]), vec![], NOW_MS);
        assert_eq!(synced.tray_count, Some(1));

        assert_eq!(state.mark_read("acme/widgets#7").tray_count, Some(1));
        let all = state.mark_all_read();
        assert_eq!(unread_counts(&all.snapshot), [0, 0]);
        assert_eq!(all.tray_count, Some(1));
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
        let saved = effects.save_read.expect("watermarks need saving");
        assert_eq!(saved["acme/widgets#7"], T1);
        // Activity that lands after the mark is unread again.
        let mut later = both();
        later[0].activity.push(T2.to_string());
        let next = state.sync_succeeded(later, vec![], NOW_MS);
        assert_eq!(unread_counts(&next.snapshot), [1, 1]);
    }

    #[test]
    fn mark_all_read_clears_every_unread_count() {
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
        assert_eq!(unread_counts(&unread.snapshot), [1, 2]);

        let effects = state.mark_all_read();

        assert_eq!(unread_counts(&effects.snapshot), [0, 0]);
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
    }

    const SYNC_1: u64 = NOW_MS;
    const SYNC_2: u64 = NOW_MS + 180_000;
    const SYNC_3: u64 = NOW_MS + 360_000;
    const SYNC_4: u64 = NOW_MS + 540_000;

    fn sync_one(state: &PrState, pr: PullRequest, at: u64) -> Effects {
        state.sync_succeeded(vec![pr], vec![], at)
    }

    fn turn_since(effects: &Effects) -> Option<u64> {
        effects.snapshot.prs[0].turn_since
    }

    #[test]
    fn a_turn_starts_at_the_sync_that_first_saw_it_as_yours_and_stays_there() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);

        let became = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);
        assert_eq!(turn_since(&became), Some(SYNC_2));

        let still = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_3);
        assert_eq!(turn_since(&still), Some(SYNC_2));
    }

    /// The start is not derived from `updated_at`, which any unrelated
    /// comment moves.
    #[test]
    fn a_comment_that_moves_updated_at_does_not_move_the_turn_start() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);
        sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);

        let commented = PullRequest {
            updated_at: T2.into(),
            ..yours(pr("acme/widgets", 7, &[T2]))
        };
        let effects = sync_one(&state, commented, SYNC_3);

        assert_eq!(effects.snapshot.prs[0].updated_at, T2);
        assert_eq!(turn_since(&effects), Some(SYNC_2));
    }

    #[test]
    fn a_turn_that_ends_and_comes_back_starts_at_the_later_sync() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);
        sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);

        let ended = sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_3);
        assert_eq!(turn_since(&ended), None);
        assert_eq!(ended.save_turns, Some(HashMap::new()));

        let again = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_4);
        assert_eq!(turn_since(&again), Some(SYNC_4));
    }

    /// How long it had been waiting before Grapevine saw it is unknowable, so
    /// no start is claimed, the same way a first-seen PR is baselined as
    /// read. It stays unknown for as long as that turn lasts.
    #[test]
    fn a_pr_first_seen_already_yours_has_no_turn_start() {
        let state = PrState::default();

        let first = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_1);
        assert_eq!(turn_since(&first), None);
        assert_eq!(first.tray_count, Some(1));

        let second = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);
        assert_eq!(turn_since(&second), None);
    }

    #[test]
    fn a_pr_that_leaves_the_snapshot_loses_its_turn_start() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);
        let waiting = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);
        assert_eq!(
            waiting.save_turns,
            Some(HashMap::from([(
                "acme/widgets#7".to_string(),
                Some(SYNC_2)
            )]))
        );

        // It merged, or its repo left the watchlist.
        let gone = state.sync_succeeded(vec![], vec![], SYNC_3);

        assert_eq!(gone.save_turns, Some(HashMap::new()));
    }

    #[test]
    fn turn_starts_survive_a_restart() {
        let before = PrState::default();
        sync_one(&before, pr("acme/widgets", 7, &[]), SYNC_1);
        let saved_read = before.mark_all_read().save_read.unwrap();
        let saved_turns = sync_one(&before, yours(pr("acme/widgets", 7, &[])), SYNC_2)
            .save_turns
            .expect("turn starts need saving");

        let after = PrState::default();
        after.restore(saved_read, vec![], Some(saved_turns), HashMap::new());
        let effects = sync_one(&after, yours(pr("acme/widgets", 7, &[])), SYNC_3);

        assert_eq!(turn_since(&effects), Some(SYNC_2));
    }

    /// A turn that began while the app was quit is first observed at the
    /// next sync, so that is its start.
    #[test]
    fn a_known_pr_that_became_yours_while_the_app_was_quit_starts_at_the_next_sync() {
        let state = PrState::default();
        state.restore(
            HashMap::from([("acme/widgets#7".to_string(), T1.to_string())]),
            vec![],
            Some(HashMap::new()),
            HashMap::new(),
        );

        let effects = sync_one(&state, yours(pr("acme/widgets", 7, &[T1])), SYNC_1);

        assert_eq!(turn_since(&effects), Some(SYNC_1));
    }

    /// An install that has read watermarks but has never tracked turns (an
    /// upgrade, or a lost file) knows its PRs yet has never watched a turn
    /// begin. Claiming "now" would show every waiting row as just started.
    #[test]
    fn an_install_that_never_tracked_turns_claims_no_start_for_turns_under_way() {
        let state = PrState::default();
        state.restore(
            HashMap::from([("acme/widgets#7".to_string(), T1.to_string())]),
            vec![],
            None,
            HashMap::new(),
        );

        let first = sync_one(&state, yours(pr("acme/widgets", 7, &[T1])), SYNC_1);
        assert_eq!(turn_since(&first), None);
        assert_eq!(
            first.save_turns,
            Some(HashMap::from([("acme/widgets#7".to_string(), None)]))
        );
    }

    #[test]
    fn a_sync_reports_the_turn_starts_for_saving_only_when_they_changed() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);
        sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);

        let quiet = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_3);

        assert_eq!(quiet.save_turns, None);
    }

    /// Nothing watches the turns during a spell without a token or repos, so
    /// a turn found still yours afterwards may not be the one that was
    /// timed. It claims neither its old start nor the sync that came back.
    #[test]
    fn an_unconfigured_spell_forgets_when_the_turns_started() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SYNC_1);
        sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_2);

        let cleared = state.unconfigured();
        assert_eq!(
            cleared.save_turns,
            Some(HashMap::from([("acme/widgets#7".to_string(), None)]))
        );
        assert_eq!(cleared.tray_count, Some(0));

        let back = sync_one(&state, yours(pr("acme/widgets", 7, &[])), SYNC_3);
        assert_eq!(turn_since(&back), None);
        assert_eq!(back.tray_count, Some(1));
    }

    const SNOOZED_AT: u64 = 1_784_205_296_000; // 2026-07-16T12:34:56Z
    const BEFORE_SNOOZE: &str = "2026-07-16T12:00:00Z";
    const AFTER_SNOOZE: &str = "2026-07-16T13:00:00Z";
    const EXPIRES_AT: u64 = SNOOZED_AT + 20 * 3_600_000;
    const KEY: &str = "acme/widgets#7";

    fn requested(pr: PullRequest) -> PullRequest {
        PullRequest {
            review_requested: true,
            ..yours(pr)
        }
    }

    /// One PR synced, then snoozed at `SNOOZED_AT`.
    fn snoozed_state(pr: PullRequest) -> PrState {
        let state = PrState::default();
        sync_one(&state, pr, SNOOZED_AT - 1);
        state.snooze(KEY, SNOOZED_AT, EXPIRES_AT);
        state
    }

    fn is_snoozed(effects: &Effects) -> bool {
        effects.snapshot.snoozed == [KEY]
    }

    #[test]
    fn snoozing_a_your_turn_pr_lowers_the_tray_count_by_one() {
        let state = PrState::default();
        let synced = state.sync_succeeded(
            vec![
                yours(pr("acme/widgets", 7, &[])),
                yours(pr("acme/widgets", 8, &[])),
            ],
            vec![],
            SNOOZED_AT - 1,
        );
        assert_eq!(synced.tray_count, Some(2));

        let effects = state.snooze(KEY, SNOOZED_AT, EXPIRES_AT);

        assert_eq!(effects.tray_count, Some(1));
        assert!(is_snoozed(&effects));
        // The row stays in the Snapshot: the popover folds it, not drops it.
        assert_eq!(effects.snapshot.prs.len(), 2);
        // A later sync with nothing new keeps it out of the count.
        let next = state.sync_succeeded(
            vec![
                yours(pr("acme/widgets", 7, &[])),
                yours(pr("acme/widgets", 8, &[])),
            ],
            vec![],
            SNOOZED_AT + 1,
        );
        assert_eq!(next.tray_count, Some(1));
    }

    #[test]
    fn a_human_comment_after_the_baseline_wakes_the_row_at_the_next_sync() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));

        let effects = sync_one(
            &state,
            yours(pr("acme/widgets", 7, &[AFTER_SNOOZE])),
            SNOOZED_AT + 1,
        );

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.tray_count, Some(1));
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
    }

    #[test]
    fn a_bot_comment_after_the_baseline_does_not_wake_the_row() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));

        let commented = PullRequest {
            bot_activity: vec![AFTER_SNOOZE.to_string()],
            ..yours(pr("acme/widgets", 7, &[]))
        };
        let effects = sync_one(&state, commented, SNOOZED_AT + 1);

        assert!(is_snoozed(&effects));
        assert_eq!(effects.tray_count, Some(0));
        assert_eq!(effects.save_snoozes, None);
    }

    /// The comment landed before the snooze and is still unread: the row was
    /// parked with it in view, so it is not news.
    #[test]
    fn unread_activity_older_than_the_baseline_does_not_wake_the_row() {
        let state = PrState::default();
        sync_one(&state, pr("acme/widgets", 7, &[]), SNOOZED_AT - 2);
        let unread = sync_one(
            &state,
            pr("acme/widgets", 7, &[BEFORE_SNOOZE]),
            SNOOZED_AT - 1,
        );
        assert_eq!(unread_counts(&unread.snapshot), [1]);
        state.snooze(KEY, SNOOZED_AT, EXPIRES_AT);

        let effects = sync_one(
            &state,
            pr("acme/widgets", 7, &[BEFORE_SNOOZE]),
            SNOOZED_AT + 1,
        );

        assert!(is_snoozed(&effects));
        assert_eq!(unread_counts(&effects.snapshot), [1]);
    }

    #[test]
    fn snoozing_does_not_mark_the_row_read() {
        let state = state_with_one_unread();

        let effects = state.snooze(KEY, SNOOZED_AT, EXPIRES_AT);

        assert_eq!(unread_counts(&effects.snapshot), [1, 0]);
        assert_eq!(effects.save_read, None);
    }

    #[test]
    fn a_review_request_arriving_after_the_snooze_wakes_the_row() {
        let state = snoozed_state(pr("acme/widgets", 7, &[]));

        let effects = sync_one(
            &state,
            requested(pr("acme/widgets", 7, &[])),
            SNOOZED_AT + 1,
        );

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.tray_count, Some(1));
    }

    #[test]
    fn a_review_request_already_outstanding_at_the_snooze_does_not_wake_the_row() {
        let state = snoozed_state(requested(pr("acme/widgets", 7, &[])));

        let effects = sync_one(
            &state,
            requested(pr("acme/widgets", 7, &[])),
            SNOOZED_AT + 1,
        );

        assert!(is_snoozed(&effects));
        assert_eq!(effects.tray_count, Some(0));
    }

    #[test]
    fn a_re_request_after_reviewing_wakes_the_row() {
        let state = snoozed_state(requested(pr("acme/widgets", 7, &[])));
        // You reviewed: the request is gone, and that alone is not news.
        let reviewed = sync_one(&state, pr("acme/widgets", 7, &[]), SNOOZED_AT + 1);
        assert!(is_snoozed(&reviewed));

        let effects = sync_one(
            &state,
            requested(pr("acme/widgets", 7, &[])),
            SNOOZED_AT + 2,
        );

        assert!(effects.snapshot.snoozed.is_empty());
    }

    /// The popover opening and the sync loop's wait both run this, so no
    /// sync has to succeed for morning to arrive.
    #[test]
    fn an_expiry_that_passes_with_no_sync_wakes_the_row() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));
        state.sync_failed("GitHub rate limit reached.".into());
        assert_eq!(state.next_snooze_expiry(), Some(EXPIRES_AT));

        let early = state.wake_expired(EXPIRES_AT - 1);
        assert!(is_snoozed(&early));
        assert_eq!(early.save_snoozes, None);
        assert_eq!(early.tray_count, None);

        let effects = state.wake_expired(EXPIRES_AT);

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.tray_count, Some(1));
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
        assert_eq!(state.next_snooze_expiry(), None);
    }

    #[test]
    fn a_sync_after_the_expiry_wakes_the_row() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));

        let effects = sync_one(&state, yours(pr("acme/widgets", 7, &[])), EXPIRES_AT);

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.tray_count, Some(1));
    }

    #[test]
    fn the_snoozed_set_survives_a_restart() {
        let before = PrState::default();
        sync_one(&before, yours(pr("acme/widgets", 7, &[])), SNOOZED_AT - 1);
        let saved = before
            .snooze(KEY, SNOOZED_AT, EXPIRES_AT)
            .save_snoozes
            .expect("snoozes need saving");

        let after = PrState::default();
        after.restore(HashMap::new(), vec![], None, saved);
        let effects = sync_one(&after, yours(pr("acme/widgets", 7, &[])), SNOOZED_AT + 1);

        assert!(is_snoozed(&effects));
        assert_eq!(effects.tray_count, Some(0));
    }

    #[test]
    fn a_snoozed_pr_that_merges_drops_its_entry_and_appears_in_merged() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));
        let departed = state.departed(&[], &repos());
        let merged_at = HashMap::from([(KEY.to_string(), T2.to_string())]);

        let effects = state.sync_succeeded(
            vec![],
            merged_entries(&departed, &merged_at),
            SNOOZED_AT + 1,
        );

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
        assert_eq!(effects.snapshot.merged.len(), 1);
        assert_eq!(effects.snapshot.merged[0].number, 7);
    }

    #[test]
    fn waking_by_hand_removes_the_entry_and_restores_the_row_to_the_tray_count() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));

        let effects = state.wake(KEY);

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.tray_count, Some(1));
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
    }

    #[test]
    fn a_row_in_all_cannot_be_snoozed() {
        let state = PrState::default();
        let browse_only = PullRequest {
            section: Section::All,
            ..pr("acme/widgets", 7, &[])
        };
        sync_one(&state, browse_only, SNOOZED_AT - 1);

        let effects = state.snooze(KEY, SNOOZED_AT, EXPIRES_AT);

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.save_snoozes, None);
    }

    /// You left the conversation: the row is now browse-only, where nothing
    /// is snoozed, so it must not stay folded or keep an entry.
    #[test]
    fn a_snoozed_pr_that_moves_to_all_drops_its_entry() {
        let state = snoozed_state(pr("acme/widgets", 7, &[]));

        let browse_only = PullRequest {
            section: Section::All,
            ..pr("acme/widgets", 7, &[])
        };
        let effects = sync_one(&state, browse_only, SNOOZED_AT + 1);

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
    }

    #[test]
    fn an_unconfigured_sync_drops_the_snoozes_with_the_prs() {
        let state = snoozed_state(yours(pr("acme/widgets", 7, &[])));

        let effects = state.unconfigured();

        assert!(effects.snapshot.snoozed.is_empty());
        assert_eq!(effects.save_snoozes, Some(HashMap::new()));
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
            None,
            HashMap::new(),
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
            None,
            HashMap::new(),
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
            None,
            HashMap::new(),
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
            None,
            HashMap::new(),
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
                failing_checks: vec!["e2e-safari".into()],
                failing_checks_truncated: true,
                checks_running: true,
                auto_merge: true,
                ready_to_merge: true,
                turn: Some(Turn::Yours),
                turn_since: Some(1_784_205_000_000),
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
            snoozed: vec!["acme/widgets#7".into()],
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
                    "failing_checks": ["e2e-safari"],
                    "failing_checks_truncated": true,
                    "checks_running": true,
                    "auto_merge": true,
                    "ready_to_merge": true,
                    "turn": "yours",
                    "turn_since": 1_784_205_000_000u64,
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
                "snoozed": ["acme/widgets#7"],
                "has_synced": true,
                "last_sync_at": 1_784_205_296_000u64,
                "sync_error": "Could not reach GitHub."
            })
        );
    }

    /// The row reads null as "start unknown" and keeps the updated time, so
    /// the field must serialize as null rather than being omitted.
    #[test]
    fn an_unknown_turn_start_serializes_as_null() {
        let value = serde_json::to_value(pr("acme/widgets", 7, &[])).unwrap();
        assert_eq!(value["turn_since"], json!(null));
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
