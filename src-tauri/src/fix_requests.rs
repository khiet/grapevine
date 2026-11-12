use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    github::{BlockedReason, PullRequest, Turn},
    unread,
};

/// How long a requested fix keeps its PR waiting on Copilot. Nothing tells
/// the app that Copilot never picked the request up or gave up on it, so the
/// PR goes back to the viewer once this has passed with the conflict still
/// there.
const LIFETIME_MS: u64 = 3_600_000;

/// One conflict fix asked of Copilot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixRequest {
    /// Epoch ms at which the request was posted.
    pub requested_at: u64,
}

/// The fixes asked for and not yet settled, keyed by `owner/repo#number` like
/// the read watermarks.
pub type FixRequests = HashMap<String, FixRequest>;

/// The turn GitHub's state gave each PR that is handed to Copilot, kept so
/// the PR can be given back between syncs. Never stored: a sync refills it.
pub type DisplacedTurns = HashMap<String, Option<Turn>>;

pub fn entry(now_epoch_ms: u64) -> FixRequest {
    FixRequest {
        requested_at: now_epoch_ms,
    }
}

fn expires_at(request: &FixRequest) -> u64 {
    request.requested_at.saturating_add(LIFETIME_MS)
}

/// Marks `pr` as waiting on Copilot and returns the turn it had. A review
/// asked of the viewer by name outranks the handover, as it outranks every
/// other turn rule: that row stays the viewer's.
pub fn hand_over(pr: &mut PullRequest) -> Option<Turn> {
    let displaced = pr.turn;
    pr.fix_requested = true;
    if !pr.review_requested {
        pr.turn = Some(Turn::Theirs);
    }
    displaced
}

/// Undoes [`hand_over`], given the turn it returned.
pub fn take_back(pr: &mut PullRequest, displaced: Option<Turn>) {
    pr.fix_requested = false;
    pr.turn = displaced;
}

/// The earliest expiry among the entries: when the clock next has a PR to
/// give back.
pub fn next_expiry(requests: &FixRequests) -> Option<u64> {
    requests.values().map(expires_at).min()
}

/// Drops every entry whose lifetime has passed and returns their keys.
pub fn expire(requests: &mut FixRequests, now_epoch_ms: u64) -> Vec<String> {
    let expired: Vec<String> = requests
        .iter()
        .filter(|(_, request)| expires_at(request) <= now_epoch_ms)
        .map(|(key, _)| key.clone())
        .collect();
    for key in &expired {
        requests.remove(key);
    }
    expired
}

/// Brings `requests` level with a fetch and hands each PR that still has one
/// to Copilot (see [`hand_over`]), refilling `displaced` to match. An entry
/// is dropped when its PR left the list, when the conflict is gone, or when
/// its lifetime has passed. Returns true when the entries changed and need
/// persisting.
///
/// Must run before the turns are tracked, so a handed-over PR is not counted
/// as the viewer's.
pub fn apply(
    prs: &mut [PullRequest],
    requests: &mut FixRequests,
    displaced: &mut DisplacedTurns,
    now_epoch_ms: u64,
) -> bool {
    let mut next = FixRequests::new();
    displaced.clear();
    for pr in prs.iter_mut() {
        let key = unread::key(pr);
        let Some(request) = requests.get(&key) else {
            continue;
        };
        if expires_at(request) <= now_epoch_ms || conflict_cleared(pr) {
            continue;
        }
        displaced.insert(key.clone(), hand_over(pr));
        next.insert(key, request.clone());
    }
    let changed = *requests != next;
    *requests = next;
    changed
}

/// A poll that could not tell mergeability reports no blocked reasons at all,
/// which says nothing about the conflict.
fn conflict_cleared(pr: &PullRequest) -> bool {
    !pr.mergeability_unknown && !pr.blocked_reasons.contains(&BlockedReason::Conflict)
}
