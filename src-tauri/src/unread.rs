use std::collections::{HashMap, HashSet};

use crate::github::{PullRequest, Section};

/// Per-PR last-read watermarks, keyed by `owner/repo#number`. A PR's unread
/// count is its activity strictly newer than the watermark; ISO-8601 UTC
/// timestamps make plain string comparison correct.
pub type ReadState = HashMap<String, String>;

pub fn key(pr: &PullRequest) -> String {
    format!("{}#{}", pr.repo, pr.number)
}

/// The watermark that makes `pr` fully read: its newest known activity, human
/// or bot, or its creation time when there is no discussion yet. Anchoring to
/// GitHub's own timestamps rather than the local clock keeps this immune to
/// clock skew.
pub fn read_watermark(pr: &PullRequest) -> String {
    pr.activity
        .last()
        .max(pr.bot_activity.last())
        .cloned()
        .unwrap_or_else(|| pr.created_at.clone())
}

/// Computes each PR's unread counts, human and bot, from the watermarks,
/// mutating both sides. One watermark serves both counts, so reading a PR
/// clears them together.
/// PRs seen for the first time are baselined as read at their newest activity,
/// so a fresh install (or a lost state file) never flags historical discussion;
/// watermarks for PRs that left the list are dropped. Returns true when the
/// state changed and needs persisting.
///
/// Only PRs you authored or take part in can carry unread activity: the All
/// section is a browse list, and activity on a PR you have no part in is not a
/// signal you owe anything to.
pub fn apply(prs: &mut [PullRequest], state: &mut ReadState) -> bool {
    let live: HashSet<String> = prs.iter().map(key).collect();
    let before = state.len();
    state.retain(|k, _| live.contains(k));
    let mut changed = state.len() != before;

    for pr in prs.iter_mut() {
        let key = key(pr);
        match state.get(&key) {
            Some(watermark) if pr.section != Section::All => {
                let newer = |times: &[String]| {
                    times
                        .iter()
                        .filter(|t| t.as_str() > watermark.as_str())
                        .count() as u64
                };
                pr.unread_count = newer(&pr.activity);
                pr.bot_unread_count = newer(&pr.bot_activity);
            }
            // First sight of any PR, and every sync of a browse-only one.
            // Re-baselining browse-only PRs rather than freezing them means
            // joining one later counts from that moment instead of dumping the
            // backlog as unread. The watermark only moves forward: a deleted
            // comment shrinks the activity list, and rewinding would resurface
            // that backlog.
            current => {
                let fresh = read_watermark(pr);
                pr.unread_count = 0;
                pr.bot_unread_count = 0;
                if current.is_none_or(|w| w.as_str() < fresh.as_str()) {
                    state.insert(key, fresh);
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Section;

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
            unread_count: 0,
            bot_unread_count: 0,
            activity: activity.iter().map(|s| s.to_string()).collect(),
            bot_activity: vec![],
        }
    }

    /// A PR you neither authored nor take part in: the All section.
    fn browse_only(repo: &str, number: u64, activity: &[&str]) -> PullRequest {
        PullRequest {
            section: Section::All,
            ..pr(repo, number, activity)
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

    /// The rule the tray badge rests on. Mine and Participated share a branch
    /// today, and every other test here uses Participated, so an edit that
    /// narrowed the badge to that section alone would pass all of them.
    #[test]
    fn only_prs_you_authored_or_take_part_in_accrue_unread() {
        for (section, expected) in [
            (Section::Mine, 1),
            (Section::Participated, 1),
            (Section::All, 0),
        ] {
            let mut prs = vec![PullRequest {
                section,
                ..pr("acme/widgets", 7, &["2026-07-12T09:30:00Z"])
            }];
            let mut state = ReadState::from([(
                "acme/widgets#7".to_string(),
                "2026-07-10T12:00:00Z".to_string(),
            )]);
            apply(&mut prs, &mut state);
            assert_eq!(prs[0].unread_count, expected, "{section:?}");
        }
    }

    /// The bot note follows the badge's rule: a browse-only PR carries
    /// neither, and its watermark keeps level with bot activity too, so
    /// joining it later does not surface the bots' backlog.
    #[test]
    fn bot_activity_accrues_only_on_prs_you_authored_or_take_part_in() {
        let with_bot_comment = |section| PullRequest {
            section,
            bot_activity: vec!["2026-07-12T09:30:00Z".into()],
            ..pr("acme/widgets", 7, &[])
        };
        let read_until = |watermark: &str| {
            ReadState::from([("acme/widgets#7".to_string(), watermark.to_string())])
        };

        for section in [Section::Mine, Section::Participated] {
            let mut state = read_until("2026-07-10T12:00:00Z");
            let mut prs = vec![with_bot_comment(section)];
            apply(&mut prs, &mut state);
            assert_eq!(
                (prs[0].unread_count, prs[0].bot_unread_count),
                (0, 1),
                "{section:?}"
            );
        }

        let mut state = read_until("2026-07-10T12:00:00Z");
        let mut prs = vec![with_bot_comment(Section::All)];
        apply(&mut prs, &mut state);
        assert_eq!(prs[0].bot_unread_count, 0);
        assert_eq!(state["acme/widgets#7"], "2026-07-12T09:30:00Z");
    }

    #[test]
    fn a_browse_only_pr_keeps_its_watermark_level_with_its_activity() {
        let mut prs = vec![browse_only(
            "acme/widgets",
            7,
            &["2026-07-10T12:00:00Z", "2026-07-12T09:30:00Z"],
        )];
        let mut state = ReadState::from([(
            "acme/widgets#7".to_string(),
            "2026-07-10T12:00:00Z".to_string(),
        )]);
        let changed = apply(&mut prs, &mut state);
        assert!(changed);
        assert_eq!(
            state.get("acme/widgets#7").map(String::as_str),
            Some("2026-07-12T09:30:00Z")
        );
    }

    /// A deleted comment shrinks the activity list, dropping `read_watermark`
    /// back to the creation time. Following it down would resurface the whole
    /// discussion the moment you were pulled into the PR.
    #[test]
    fn a_browse_only_watermark_never_rewinds_when_activity_disappears() {
        let mut prs = vec![browse_only("acme/widgets", 7, &[])];
        let mut state = ReadState::from([(
            "acme/widgets#7".to_string(),
            "2026-07-12T09:30:00Z".to_string(),
        )]);
        assert!(!apply(&mut prs, &mut state));
        assert_eq!(
            state.get("acme/widgets#7").map(String::as_str),
            Some("2026-07-12T09:30:00Z")
        );
    }

    #[test]
    fn a_quiet_browse_only_pr_reports_nothing_to_persist() {
        let mut prs = vec![browse_only("acme/widgets", 7, &["2026-07-12T09:30:00Z"])];
        let mut state = ReadState::from([(
            "acme/widgets#7".to_string(),
            "2026-07-12T09:30:00Z".to_string(),
        )]);
        assert!(!apply(&mut prs, &mut state));
    }

    #[test]
    fn joining_a_browse_only_pr_counts_only_activity_after_you_joined() {
        let mut state = ReadState::new();
        let mut prs = vec![browse_only("acme/widgets", 7, &["2026-07-10T12:00:00Z"])];
        apply(&mut prs, &mut state);

        // The team discusses it while you are only watching.
        prs[0].activity.push("2026-07-12T09:30:00Z".into());
        apply(&mut prs, &mut state);

        // Someone requests your review, then comments. Only that comment is
        // unread; the backlog you were never part of is not.
        prs[0].section = Section::Participated;
        prs[0].activity.push("2026-07-13T10:00:00Z".into());
        apply(&mut prs, &mut state);
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
