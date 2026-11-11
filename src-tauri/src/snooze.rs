use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    github::{self, PullRequest, Section},
    unread,
};

/// One parked PR: what it looked like when it was snoozed, and when the
/// snooze runs out regardless.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snooze {
    /// Epoch ms of the snooze: the activity baseline. Human activity dated
    /// after it wakes the row. Unread is not the signal, since unread
    /// activity may predate the snooze.
    pub snoozed_at: u64,
    /// Whether a review request naming the viewer was outstanding when last
    /// looked at. Starts as the state at snooze time and follows a request
    /// that clears, so a re-request after reviewing reads as fresh.
    pub request_outstanding: bool,
    /// Epoch ms at which the row wakes with or without news.
    pub expires_at: u64,
}

/// The snoozed PRs, keyed by `owner/repo#number` like the read watermarks.
pub type Snoozes = HashMap<String, Snooze>;

/// Whether `pr` may be snoozed: only rows the viewer has a part in. All is a
/// browse list with nothing to park.
pub fn can_snooze(pr: &PullRequest) -> bool {
    pr.section != Section::All
}

pub fn entry(pr: &PullRequest, now_epoch_ms: u64, expires_at: u64) -> Snooze {
    Snooze {
        snoozed_at: now_epoch_ms,
        request_outstanding: pr.review_requested,
        expires_at,
    }
}

/// Drops every entry whose expiry has passed. Returns true when the state
/// changed and needs persisting.
pub fn wake_expired(snoozes: &mut Snoozes, now_epoch_ms: u64) -> bool {
    let before = snoozes.len();
    snoozes.retain(|_, snooze| snooze.expires_at > now_epoch_ms);
    snoozes.len() != before
}

/// The earliest expiry among the entries: when the clock next has a row to
/// wake.
pub fn next_expiry(snoozes: &Snoozes) -> Option<u64> {
    snoozes.values().map(|snooze| snooze.expires_at).min()
}

/// Brings `snoozes` level with a fetch: an entry is dropped when its PR left
/// the list or can no longer be snoozed, and when the row wakes, which is the
/// first of human activity after the baseline, a fresh review request naming
/// the viewer, or the expiry passing. Bot activity never wakes a row. Returns
/// true when the state changed and needs persisting.
pub fn apply(prs: &[PullRequest], snoozes: &mut Snoozes, now_epoch_ms: u64) -> bool {
    let mut next = Snoozes::new();
    for pr in prs {
        let key = unread::key(pr);
        let Some(snooze) = snoozes.get(&key) else {
            continue;
        };
        let fresh_request = pr.review_requested && !snooze.request_outstanding;
        if !can_snooze(pr)
            || snooze.expires_at <= now_epoch_ms
            || fresh_request
            || has_activity_after(pr, snooze.snoozed_at)
        {
            continue;
        }
        next.insert(
            key,
            Snooze {
                request_outstanding: pr.review_requested,
                ..snooze.clone()
            },
        );
    }
    let changed = *snoozes != next;
    *snoozes = next;
    changed
}

/// An activity time this cannot parse counts as old: a row wakes on news, not
/// on a date nobody could read.
fn has_activity_after(pr: &PullRequest, baseline_epoch_ms: u64) -> bool {
    pr.activity.iter().any(|at| {
        github::rfc3339_utc_to_epoch_secs(at).is_some_and(|secs| secs * 1000 > baseline_epoch_ms)
    })
}
