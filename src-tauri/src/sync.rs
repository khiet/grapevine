use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{
    github,
    github::{GithubError, PullRequest, RateLimit},
    keychain, merged, settings, unread,
};

/// Remaining GraphQL budget under which the loop stops syncing until the
/// window resets. A sync costs a handful of points out of 5000/hour, so
/// this leaves the rest of the budget to whatever else shares the token.
const RATE_LIMIT_RESERVE: u64 = 100;
/// First retry delay after a failed sync; doubles per consecutive failure.
const BACKOFF_BASE: Duration = Duration::from_secs(15);
/// Ceiling for error backoff, so an extended outage still gets retried
/// every few minutes rather than ever more rarely.
const BACKOFF_CAP: Duration = Duration::from_secs(900);
/// Floor for every rate-limited retry: GitHub asks for at least a minute of
/// quiet after a secondary rate limit.
const RATE_LIMIT_MIN_WAIT: Duration = Duration::from_secs(60);
/// Margin past the documented reset moment, absorbing clock skew.
const RESET_BUFFER: Duration = Duration::from_secs(5);

/// Latest sync result. A failed sync leaves the previous snapshot's list in
/// place so the popover keeps showing the last known PRs instead of
/// blanking; only the error field changes.
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

#[derive(Default)]
pub struct SyncState {
    pub snapshot: Mutex<Snapshot>,
    /// Both are authoritative in memory and mirrored to disk on every change,
    /// so unread badges and the Merged section survive restarts.
    pub read_state: Mutex<unread::ReadState>,
    pub merged: Mutex<merged::MergedState>,
    /// Wakes the poll loop early, e.g. right after settings change.
    pub wake: Notify,
}

/// Spawns the background loop: sync now, then again after a delay picked from
/// the outcome, or whenever [`SyncState::wake`] is notified, whichever comes
/// first.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    *app.state::<SyncState>().read_state.lock().unwrap() = unread::load(&app);
    {
        // Seed the initial snapshot too, so the Merged section is visible
        // before the first sync completes.
        let loaded = merged::load(&app);
        let state = app.state::<SyncState>();
        state.snapshot.lock().unwrap().merged = loaded.clone();
        *state.merged.lock().unwrap() = loaded;
    }
    tauri::async_runtime::spawn(async move {
        let mut consecutive_failures: u32 = 0;
        loop {
            let outcome = sync_once(&app).await;
            let now = github::now_epoch_secs();
            let delay = match outcome {
                Outcome::Synced { rate_limit } => {
                    consecutive_failures = 0;
                    delay_after_success(poll_interval(&app), rate_limit.as_ref(), now)
                }
                Outcome::Unconfigured => {
                    consecutive_failures = 0;
                    poll_interval(&app)
                }
                Outcome::Failed {
                    rate_limited,
                    reset_epoch_secs,
                } => {
                    consecutive_failures += 1;
                    delay_after_failure(consecutive_failures, rate_limited, reset_epoch_secs, now)
                }
            };
            let state = app.state::<SyncState>();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = state.wake.notified() => {}
            }
        }
    });
}

/// Reads the poll interval fresh each round, so a settings change takes
/// effect from the next scheduling decision without a restart.
fn poll_interval(app: &AppHandle) -> Duration {
    let secs = settings::load(app)
        .map(|s| s.poll_interval_secs)
        .unwrap_or(settings::DEFAULT_POLL_SECS);
    Duration::from_secs(secs)
}

/// How long past the documented reset moment to stay quiet; zero when the
/// reset is unknown or already behind us.
fn until_reset(reset_epoch_secs: Option<u64>, now_epoch_secs: u64) -> Duration {
    match reset_epoch_secs {
        Some(reset) if reset > now_epoch_secs => {
            Duration::from_secs(reset - now_epoch_secs) + RESET_BUFFER
        }
        _ => Duration::ZERO,
    }
}

/// After a successful sync: the poll interval, unless the reported budget is
/// nearly spent — then wait out the current rate-limit window instead.
fn delay_after_success(
    poll: Duration,
    rate_limit: Option<&RateLimit>,
    now_epoch_secs: u64,
) -> Duration {
    let Some(limit) = rate_limit else { return poll };
    if limit.remaining >= RATE_LIMIT_RESERVE {
        return poll;
    }
    match limit.reset_epoch_secs {
        // An exhausted budget with no usable reset time: back off hard
        // rather than keep spending the last few points.
        None => poll.max(BACKOFF_CAP),
        _ => poll.max(until_reset(limit.reset_epoch_secs, now_epoch_secs)),
    }
}

/// After a failed sync: exponential backoff on consecutive failures, raised
/// to GitHub's documented reset (or a one-minute floor) when the failure was
/// rate limiting.
fn delay_after_failure(
    consecutive_failures: u32,
    rate_limited: bool,
    reset_epoch_secs: Option<u64>,
    now_epoch_secs: u64,
) -> Duration {
    // Exponent capped so the shift cannot overflow; the cap dominates anyway.
    let exponent = consecutive_failures.saturating_sub(1).min(6);
    let mut delay = BACKOFF_CAP.min(BACKOFF_BASE * 2u32.pow(exponent));
    if rate_limited {
        delay = delay
            .max(RATE_LIMIT_MIN_WAIT)
            .max(until_reset(reset_epoch_secs, now_epoch_secs));
    }
    delay
}

pub fn total_unread(prs: &[PullRequest]) -> u64 {
    prs.iter().map(|pr| pr.unread_count).sum()
}

/// The tray title for `total`. Zero yields an empty string rather than `None`:
/// tray-icon's macOS `set_title` skips the button entirely on `None`, stranding
/// the last count on the icon so a cleared badge never disappears.
fn tray_title(total: u64) -> String {
    if total > 0 {
        total.to_string()
    } else {
        String::new()
    }
}

pub fn update_tray_count(app: &AppHandle, total: u64) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_title(Some(tray_title(total)));
    }
}

/// What one sync attempt means for scheduling the next one.
enum Outcome {
    Synced {
        rate_limit: Option<RateLimit>,
    },
    Unconfigured,
    Failed {
        rate_limited: bool,
        reset_epoch_secs: Option<u64>,
    },
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn sync_once(app: &AppHandle) -> Outcome {
    let result = fetch(app).await;
    let state = app.state::<SyncState>();
    let (new_snapshot, outcome) = match result {
        Ok(Some(FetchResult {
            mut prs,
            newly_merged,
            rate_limit,
        })) => {
            let merged = {
                let mut merged_state = state.merged.lock().unwrap();
                if merged::apply(&prs, newly_merged, &mut merged_state) {
                    if let Err(e) = merged::save(app, &merged_state) {
                        eprintln!("cannot persist merged state: {e}");
                    }
                }
                merged_state.clone()
            };
            let mut read_state = state.read_state.lock().unwrap();
            if unread::apply(&mut prs, &mut read_state) {
                if let Err(e) = unread::save(app, &read_state) {
                    eprintln!("cannot persist unread state: {e}");
                }
            }
            (
                Snapshot {
                    prs,
                    merged,
                    has_synced: true,
                    last_sync_at: Some(now_epoch_ms()),
                    sync_error: None,
                },
                Outcome::Synced { rate_limit },
            )
        }
        // Unconfigured: clear any list left over from previous settings,
        // but keep the read watermarks and merged history for when a token
        // comes back.
        Ok(None) => (Snapshot::default(), Outcome::Unconfigured),
        // Failure: keep the last known list and its sync time, surface only
        // the error, and skip the tray update — a stale count beats a
        // vanishing one.
        Err(e) => {
            eprintln!("sync failed: {e}");
            let snapshot = {
                let mut snapshot = state.snapshot.lock().unwrap();
                snapshot.sync_error = Some(e.to_string());
                snapshot.clone()
            };
            let _ = app.emit("prs-updated", snapshot);
            let (rate_limited, reset_epoch_secs) = match e {
                GithubError::RateLimited { reset_epoch_secs } => (true, reset_epoch_secs),
                GithubError::Other(_) => (false, None),
            };
            return Outcome::Failed {
                rate_limited,
                reset_epoch_secs,
            };
        }
    };
    let snapshot = {
        let mut snapshot = state.snapshot.lock().unwrap();
        *snapshot = new_snapshot;
        snapshot.clone()
    };
    update_tray_count(app, total_unread(&snapshot.prs));
    let _ = app.emit("prs-updated", snapshot);
    outcome
}

/// One sync's fetch: the open PRs, any tracked PRs that turned out to have
/// merged since the previous sync, and the budget GitHub reported.
struct FetchResult {
    prs: Vec<PullRequest>,
    newly_merged: Vec<merged::MergedPr>,
    rate_limit: Option<RateLimit>,
}

/// `Ok(None)` means the app is not configured (no token or no repos).
async fn fetch(app: &AppHandle) -> Result<Option<FetchResult>, GithubError> {
    let Some(token) = keychain::load().map_err(GithubError::Other)? else {
        return Ok(None);
    };
    let repos = settings::load(app).map_err(GithubError::Other)?.repos;
    if repos.is_empty() {
        return Ok(None);
    }
    let fetched = github::fetch_open_prs(&token, &repos).await?;
    let newly_merged = detect_merged(app, &token, &repos, &fetched.prs).await?;
    Ok(Some(FetchResult {
        prs: fetched.prs,
        newly_merged,
        rate_limit: fetched.rate_limit,
    }))
}

/// Finds tracked PRs that left the open set since the previous sync and asks
/// GitHub which of them merged. A failed status query fails the whole sync,
/// keeping the old snapshot so the departure is re-detected next round — a
/// network blip never silently drops a merge. Merges that happen while the app
/// is quit are missed entirely: the previous open set lives only in memory.
async fn detect_merged(
    app: &AppHandle,
    token: &str,
    repos: &[String],
    live_prs: &[PullRequest],
) -> Result<Vec<merged::MergedPr>, GithubError> {
    let departed = {
        let state = app.state::<SyncState>();
        let snapshot = state.snapshot.lock().unwrap();
        departed_prs(&snapshot.prs, live_prs, repos)
    };
    if departed.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = departed.iter().map(unread::key).collect();
    let merged_at = github::fetch_merged_status(token, &keys).await?;
    Ok(merged_entries(&departed, &merged_at))
}

/// The tracked PRs in `previous` that are gone from the newly fetched
/// `live_prs` — the candidates for having merged.
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
    use crate::github::Section;
    use serde_json::json;

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

    #[test]
    fn a_pr_gone_from_the_open_list_has_departed() {
        let previous = vec![pr("acme/widgets", 7), pr("acme/widgets", 8)];
        let live = vec![pr("acme/widgets", 8)];
        let repos = vec!["acme/widgets".to_string()];
        let departed = departed_prs(&previous, &live, &repos);
        assert_eq!(departed.len(), 1);
        assert_eq!(departed[0].number, 7);
    }

    #[test]
    fn prs_still_open_have_not_departed() {
        let prs = vec![pr("acme/widgets", 7)];
        let repos = vec!["acme/widgets".to_string()];
        assert!(departed_prs(&prs, &prs, &repos).is_empty());
    }

    /// Un-watching a repo drops its PRs from the fetch, so without this guard
    /// removing a repo would spray its open PRs into the Merged section.
    #[test]
    fn prs_of_an_unwatched_repo_never_count_as_departed() {
        let previous = vec![pr("acme/widgets", 7), pr("acme/gadgets", 1)];
        let repos = vec!["acme/widgets".to_string()];
        let departed = departed_prs(&previous, &[], &repos);
        assert_eq!(departed.len(), 1);
        assert_eq!(departed[0].repo, "acme/widgets");
    }

    /// The watchlist stores GitHub's canonical casing while a PR's repo name
    /// comes back from the API; a case mismatch must not read as un-watched.
    #[test]
    fn the_watchlist_check_ignores_repo_name_casing() {
        let previous = vec![pr("acme/widgets", 7)];
        let repos = vec!["Acme/Widgets".to_string()];
        assert_eq!(departed_prs(&previous, &[], &repos).len(), 1);
    }

    #[test]
    fn departed_prs_that_merged_become_section_entries() {
        let departed = vec![pr("acme/widgets", 7)];
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
        let departed = vec![pr("acme/widgets", 7)];
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
                author: "someone".into(),
                avatar_url: "https://avatars.example/someone".into(),
                owner_avatar_url: "https://avatars.example/acme".into(),
                created_at: "2026-07-10T12:00:00Z".into(),
                updated_at: "2026-07-11T09:30:00Z".into(),
                section: Section::Mine,
                unread_count: 3,
                // Internal working data; must not leak into the payload.
                activity: vec!["2026-07-10T12:00:00Z".into()],
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
                    "author": "someone",
                    "avatar_url": "https://avatars.example/someone",
                    "owner_avatar_url": "https://avatars.example/acme",
                    "created_at": "2026-07-10T12:00:00Z",
                    "updated_at": "2026-07-11T09:30:00Z",
                    "section": "mine",
                    "unread_count": 3
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

    #[test]
    fn a_healthy_sync_schedules_the_plain_poll_interval() {
        let poll = Duration::from_secs(180);
        assert_eq!(delay_after_success(poll, None, 1_000), poll);
        let limit = RateLimit {
            remaining: 4_900,
            reset_epoch_secs: Some(2_000),
        };
        assert_eq!(delay_after_success(poll, Some(&limit), 1_000), poll);
    }

    #[test]
    fn a_depleted_budget_waits_for_the_reset() {
        let poll = Duration::from_secs(180);
        let limit = RateLimit {
            remaining: RATE_LIMIT_RESERVE - 1,
            reset_epoch_secs: Some(2_000),
        };
        // 1000 seconds until reset, plus the buffer.
        assert_eq!(
            delay_after_success(poll, Some(&limit), 1_000),
            Duration::from_secs(1_000) + RESET_BUFFER
        );
    }

    #[test]
    fn a_depleted_budget_with_an_imminent_reset_still_waits_the_poll_interval() {
        let poll = Duration::from_secs(180);
        let limit = RateLimit {
            remaining: 0,
            reset_epoch_secs: Some(1_010),
        };
        assert_eq!(delay_after_success(poll, Some(&limit), 1_000), poll);
    }

    #[test]
    fn a_depleted_budget_without_a_reset_time_backs_off_hard() {
        let poll = Duration::from_secs(180);
        let limit = RateLimit {
            remaining: 0,
            reset_epoch_secs: None,
        };
        assert_eq!(delay_after_success(poll, Some(&limit), 1_000), BACKOFF_CAP);
    }

    #[test]
    fn failures_back_off_exponentially_up_to_the_cap() {
        let delay = |failures| delay_after_failure(failures, false, None, 1_000);
        assert_eq!(delay(1), Duration::from_secs(15));
        assert_eq!(delay(2), Duration::from_secs(30));
        assert_eq!(delay(3), Duration::from_secs(60));
        assert_eq!(delay(6), Duration::from_secs(480));
        assert_eq!(delay(7), BACKOFF_CAP);
        assert_eq!(delay(100), BACKOFF_CAP);
    }

    /// The first-failure backoff alone would retry after 15s.
    #[test]
    fn a_rate_limited_failure_waits_at_least_a_minute() {
        assert_eq!(
            delay_after_failure(1, true, None, 1_000),
            RATE_LIMIT_MIN_WAIT
        );
    }

    #[test]
    fn a_rate_limited_failure_waits_for_a_documented_reset() {
        assert_eq!(
            delay_after_failure(1, true, Some(1_500), 1_000),
            Duration::from_secs(500) + RESET_BUFFER
        );
        // A reset already behind us falls back to the minimum wait.
        assert_eq!(
            delay_after_failure(1, true, Some(900), 1_000),
            RATE_LIMIT_MIN_WAIT
        );
    }

    #[test]
    fn a_zero_total_clears_the_tray_title_instead_of_leaving_it() {
        assert_eq!(tray_title(0), "");
        assert_eq!(tray_title(1), "1");
        assert_eq!(tray_title(42), "42");
    }

    #[test]
    fn the_tray_total_sums_unread_across_prs() {
        let pr = |unread_count: u64| PullRequest {
            number: 7,
            title: "Fix the thing".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            repo: "acme/widgets".into(),
            author: "someone".into(),
            avatar_url: String::new(),
            owner_avatar_url: String::new(),
            created_at: "2026-07-10T12:00:00Z".into(),
            updated_at: "2026-07-11T09:30:00Z".into(),
            section: Section::All,
            unread_count,
            activity: vec![],
        };
        assert_eq!(total_unread(&[]), 0);
        assert_eq!(total_unread(&[pr(2), pr(0), pr(5)]), 7);
    }

    /// `PrList.tsx` filters rows with `pr.section === key`; a mismatched
    /// value drops PRs from the popover without any error.
    #[test]
    fn every_section_variant_serializes_to_its_frontend_key() {
        for (section, expected) in [
            (Section::Mine, "mine"),
            (Section::Participated, "participated"),
            (Section::All, "all"),
        ] {
            assert_eq!(serde_json::to_value(section).unwrap(), json!(expected));
        }
    }
}
