use std::{sync::Mutex, time::Duration};

use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{
    github,
    github::{GithubError, PullRequest, RateLimit},
    keychain, merged,
    pr_state::{Effects, PrState, Snapshot},
    settings,
    store::{self, Store},
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

#[derive(Default)]
pub struct SyncState {
    pr_state: PrState,
    /// Held from a transition until its effects are delivered; see
    /// [`transition`].
    delivery: Mutex<()>,
    /// Wakes the poll loop early, e.g. right after settings change.
    pub wake: Notify,
    /// Tells the poll loop's wait that the snoozes changed, so the expiry it
    /// is timing may no longer be the earliest.
    snoozes_changed: Notify,
}

impl SyncState {
    pub fn snapshot(&self) -> Snapshot {
        self.pr_state.snapshot()
    }
}

/// Spawns the background loop: sync now, then again after a delay picked from
/// the outcome, or whenever [`SyncState::wake`] is notified, whichever comes
/// first.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    {
        // No document here fails on a corrupt file, so the defaults only
        // guard against that policy changing.
        let store = app.state::<Store>();
        app.state::<SyncState>().pr_state.restore(
            store.load(&store::READ_STATE).unwrap_or_default(),
            store.load(&store::MERGED).unwrap_or_default(),
            store.load(&store::TURN_STARTS).unwrap_or_default(),
            store.load(&store::SNOOZES).unwrap_or_default(),
        );
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
            wait_for_next_sync(&app, delay).await;
        }
    });
}

/// Longest the wait goes without re-reading the wall clock. The timer under
/// `tokio::time::sleep` stops while the machine sleeps, so a single sleep
/// until 08:00 would fire hours late after a night with the lid closed.
const EXPIRY_RECHECK: Duration = Duration::from_secs(60);

/// Sleeps until the next sync is due or [`SyncState::wake`] asks for one.
/// Snoozes that run out in the meantime are woken on time: the wait can be a
/// long rate-limit backoff, and that must not hold a row past its expiry.
async fn wait_for_next_sync(app: &AppHandle, delay: Duration) {
    let state = app.state::<SyncState>();
    let due = tokio::time::sleep(delay);
    tokio::pin!(due);
    loop {
        // Read afresh each round: a snooze made during the wait has to be
        // timed too, and `snoozes_changed` ends the round when one is.
        let expiry = async {
            match state.pr_state.next_snooze_expiry() {
                Some(at) => {
                    let wait = Duration::from_millis(at.saturating_sub(now_epoch_ms()));
                    tokio::time::sleep(wait.min(EXPIRY_RECHECK)).await
                }
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            _ = &mut due => return,
            _ = state.wake.notified() => return,
            _ = state.snoozes_changed.notified() => {}
            _ = expiry => {
                let now = now_epoch_ms();
                if state.pr_state.next_snooze_expiry().is_some_and(|at| at <= now) {
                    wake_expired_snoozes(app);
                }
            }
        }
    }
}

/// Wakes the rows whose snooze has run out by the local clock, with no sync
/// involved.
pub fn wake_expired_snoozes(app: &AppHandle) {
    if let Err(e) = transition(app, |state| state.wake_expired(now_epoch_ms())) {
        eprintln!("cannot deliver woken snoozes: {e}");
    }
}

/// Reads the poll interval fresh each round, so a settings change takes
/// effect from the next scheduling decision without a restart.
fn poll_interval(app: &AppHandle) -> Duration {
    let secs = settings::load(&app.state::<Store>())
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

/// The tray title for `total`, the number of PRs waiting on the viewer. Zero
/// yields an empty string rather than `None`:
/// tray-icon's macOS `set_title` skips the button entirely on `None`, stranding
/// the last count on the icon so a cleared badge never disappears.
fn tray_title(total: u64) -> String {
    if total > 0 {
        total.to_string()
    } else {
        String::new()
    }
}

/// Runs one [`PrState`] transition and pays what it owes the outside world:
/// changed state goes to disk, the tray shows the new count, and the popover
/// gets the Snapshot. This is the only way to run a transition, and it
/// delivers the effects before the next transition may start, so a sync and a
/// command can never publish or save out of order.
///
/// A failed save does not hold back the tray or the popover, which must keep
/// agreeing with the state in memory; the first failure is returned once
/// everything has been attempted.
pub fn transition(app: &AppHandle, apply: impl FnOnce(&PrState) -> Effects) -> Result<(), String> {
    let state = app.state::<SyncState>();
    let _delivering = state.delivery.lock().unwrap();
    let effects = apply(&state.pr_state);
    let store = app.state::<Store>();
    let saved_read = effects.save_read.map_or(Ok(()), |read_state| {
        store.save(&store::READ_STATE, &read_state)
    });
    let saved_turns = effects.save_turns.map_or(Ok(()), |turn_starts| {
        store.save(&store::TURN_STARTS, &Some(turn_starts))
    });
    let saved_merged = effects
        .save_merged
        .map_or(Ok(()), |merged| store.save(&store::MERGED, &merged));
    let saved_snoozes = effects.save_snoozes.map_or(Ok(()), |snoozes| {
        state.snoozes_changed.notify_one();
        store.save(&store::SNOOZES, &snoozes)
    });
    if let Some(total) = effects.tray_count {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_title(Some(tray_title(total)));
        }
    }
    let emitted = app
        .emit("prs-updated", effects.snapshot)
        .map_err(|e| e.to_string());
    saved_read
        .and(saved_turns)
        .and(saved_merged)
        .and(saved_snoozes)
        .and(emitted)
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

pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

async fn sync_once(app: &AppHandle) -> Outcome {
    let result = fetch(app).await;
    let (delivered, outcome) = match result {
        Ok(Some(FetchResult {
            prs,
            newly_merged,
            rate_limit,
        })) => (
            transition(app, |state| {
                state.sync_succeeded(prs, newly_merged, now_epoch_ms())
            }),
            Outcome::Synced { rate_limit },
        ),
        Ok(None) => (
            transition(app, PrState::unconfigured),
            Outcome::Unconfigured,
        ),
        Err(e) => {
            eprintln!("sync failed: {e}");
            let delivered = transition(app, |state| state.sync_failed(e.to_string()));
            let (rate_limited, reset_epoch_secs) = match e {
                GithubError::RateLimited { reset_epoch_secs } => (true, reset_epoch_secs),
                GithubError::Other(_) => (false, None),
            };
            (
                delivered,
                Outcome::Failed {
                    rate_limited,
                    reset_epoch_secs,
                },
            )
        }
    };
    if let Err(e) = delivered {
        eprintln!("cannot deliver sync effects: {e}");
    }
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
    let repos = settings::load(&app.state::<Store>())
        .map_err(GithubError::Other)?
        .repos;
    if repos.is_empty() {
        return Ok(None);
    }
    let fetched = github::fetch_open_prs(&token, &repos).await?;
    let newly_merged = app
        .state::<SyncState>()
        .pr_state
        .detect_merged(&token, &repos, &fetched.prs)
        .await?;
    Ok(Some(FetchResult {
        prs: fetched.prs,
        newly_merged,
        rate_limit: fetched.rate_limit,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Section;
    use serde_json::json;

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
