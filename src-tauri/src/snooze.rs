use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    github::{self, AgentTask, PullRequest, Section},
    unread,
};

/// One parked PR: what it looked like when it was snoozed, and when the
/// snooze runs out regardless, if it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snooze {
    /// The activity baseline: epoch ms of the last successful sync before
    /// the snooze, which dates the row the viewer was looking at. Human
    /// activity after it wakes the row, so a comment that landed between
    /// that sync and the click is not lost. Unread is not the signal, since
    /// unread activity may predate the snooze.
    pub baseline_at: u64,
    /// Whether a review request naming the viewer was outstanding when last
    /// looked at. Starts as the state at snooze time and follows a request
    /// that clears, so a re-request after reviewing reads as fresh.
    pub request_outstanding: bool,
    /// Whether the PR's Copilot task was waiting for the viewer when a task
    /// was last seen on it, kept like `request_outstanding` so a task that
    /// stops for them again reads as fresh. A sync that shows no task leaves
    /// it alone: the tasks may only have gone unread, and a wait the viewer
    /// snoozed knowingly must not come back as news. Absent from snoozes
    /// stored before tasks were read, none of which had one.
    #[serde(default)]
    pub task_waiting: bool,
    /// Epoch ms at which the row wakes with or without news. `None` is a
    /// snooze with no limit, which only news ends. Absent from the file for
    /// such a snooze; one stored before the limit became a choice always
    /// has it, and keeps it.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// The snoozed PRs, keyed by `owner/repo#number` like the read watermarks.
pub type Snoozes = HashMap<String, Snooze>;

/// Whether `pr` may be snoozed: only rows the viewer has a part in. All is a
/// browse list with nothing to park.
pub fn can_snooze(pr: &PullRequest) -> bool {
    pr.section != Section::All
}

pub fn entry(pr: &PullRequest, baseline_at: u64, expires_at: Option<u64>) -> Snooze {
    Snooze {
        baseline_at,
        request_outstanding: pr.review_requested,
        task_waiting: task_waiting(pr),
        expires_at,
    }
}

fn task_waiting(pr: &PullRequest) -> bool {
    pr.agent_task == Some(AgentTask::NeedsYou)
}

fn has_expired(snooze: &Snooze, now_epoch_ms: u64) -> bool {
    snooze.expires_at.is_some_and(|at| at <= now_epoch_ms)
}

/// Drops every entry whose expiry has passed. Returns true when the state
/// changed and needs persisting.
pub fn wake_expired(snoozes: &mut Snoozes, now_epoch_ms: u64) -> bool {
    let before = snoozes.len();
    snoozes.retain(|_, snooze| !has_expired(snooze, now_epoch_ms));
    snoozes.len() != before
}

/// The earliest expiry among the entries: when the clock next has a row to
/// wake. `None` when no entry has a limit.
pub fn next_expiry(snoozes: &Snoozes) -> Option<u64> {
    snoozes
        .values()
        .filter_map(|snooze| snooze.expires_at)
        .min()
}

/// Brings `snoozes` level with a fetch: an entry is dropped when its PR left
/// the list or can no longer be snoozed, and when the row wakes, which is the
/// first of human activity after the baseline, a fresh review request naming
/// the viewer, a Copilot task newly waiting for the viewer, or the expiry
/// passing where the snooze has one. Bot activity never wakes a row. Returns true when the state
/// changed and needs persisting.
pub fn apply(prs: &[PullRequest], snoozes: &mut Snoozes, now_epoch_ms: u64) -> bool {
    let mut next = Snoozes::new();
    for pr in prs {
        let key = unread::key(pr);
        let Some(snooze) = snoozes.get(&key) else {
            continue;
        };
        let fresh_request = pr.review_requested && !snooze.request_outstanding;
        let fresh_wait = task_waiting(pr) && !snooze.task_waiting;
        if !can_snooze(pr)
            || has_expired(snooze, now_epoch_ms)
            || fresh_request
            || fresh_wait
            || has_activity_after(pr, snooze.baseline_at)
        {
            continue;
        }
        next.insert(
            key,
            Snooze {
                request_outstanding: pr.review_requested,
                task_waiting: match pr.agent_task {
                    Some(_) => task_waiting(pr),
                    None => snooze.task_waiting,
                },
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
