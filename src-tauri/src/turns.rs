use std::collections::HashMap;

use crate::{
    github::{PullRequest, Turn},
    unread::{self, ReadState},
};

/// When each turn that is the viewer's began, keyed by `owner/repo#number`
/// like the read watermarks. A PR has an entry exactly while its turn is
/// yours. The value is the epoch ms of the sync that first observed the turn,
/// or `None` when the turn was already under way the first time the PR was
/// seen, so its real start is unknowable.
pub type TurnStarts = HashMap<String, Option<u64>>;

/// How many PRs wait on the viewer: the tray count.
pub fn yours_count(prs: &[PullRequest]) -> u64 {
    prs.iter().filter(|pr| is_yours(pr)).count() as u64
}

fn is_yours(pr: &PullRequest) -> bool {
    pr.turn == Some(Turn::Yours)
}

/// Fills each PR's `turn_since` and brings `starts` level with `prs`: a turn
/// that is still yours keeps its start, one that began since the last sync
/// starts at `now_epoch_ms`, and entries for turns that ended or PRs that
/// left the list are dropped. Returns true when the state changed and needs
/// persisting.
///
/// `starts` is `None` until turns have been tracked at all (a fresh install,
/// an upgrade, a lost file). Every turn under way then gets an unknown start
/// instead of `now_epoch_ms`.
///
/// `seen_before` must be the read watermarks as they were before this sync's
/// PRs were baselined into them: a key in there is how a PR counts as seen.
pub fn apply(
    prs: &mut [PullRequest],
    starts: &mut Option<TurnStarts>,
    seen_before: &ReadState,
    now_epoch_ms: u64,
) -> bool {
    let mut next = TurnStarts::new();
    for pr in prs.iter_mut() {
        pr.turn_since = None;
        if !is_yours(pr) {
            continue;
        }
        let key = unread::key(pr);
        let since = match starts.as_ref() {
            Some(known) => match known.get(&key) {
                Some(since) => *since,
                None => seen_before.contains_key(&key).then_some(now_epoch_ms),
            },
            None => None,
        };
        pr.turn_since = since;
        next.insert(key, since);
    }
    let changed = starts.as_ref() != Some(&next);
    *starts = Some(next);
    changed
}
