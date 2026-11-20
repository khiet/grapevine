use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{github, github::PullRequest, unread};

/// A PR that merged in a watched repo. Shown in the popover's Merged section
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
    /// The deploy of the merge commit. `None` until a deploy run
    /// is found, which is forever for a repo that has none, and for entries
    /// saved before this field existed.
    #[serde(default)]
    pub deploy: Option<Deploy>,
    /// The Conventional Commit type the title opens with, as written
    /// (see [`change_kind`]). `None` for a title with no such prefix, and for
    /// entries saved before this field existed. Like the summary, taken when
    /// the merge is first seen and never refreshed.
    #[serde(default)]
    pub kind: Option<String>,
    /// Whether the title's prefix marks a breaking change. Never true
    /// without a `kind`.
    #[serde(default)]
    pub breaking: bool,
    /// The first prose paragraph of the PR description (see [`summary`]).
    /// `None` for a bot's PR, a description with no prose, and entries saved
    /// before this field existed.
    #[serde(default)]
    pub summary: Option<String>,
}

/// The kind of change a PR title names with a Conventional Commit prefix,
/// `^([a-z]+)(\([^)]*\))?(!)?: `, and whether the prefix marks it breaking.
/// The kind is kept as written, so any lowercase word counts, not only the
/// well-known types. `None` for a title with no such prefix.
pub fn change_kind(title: &str) -> Option<(String, bool)> {
    let kind_len = title.bytes().take_while(u8::is_ascii_lowercase).count();
    if kind_len == 0 {
        return None;
    }
    let (kind, mut rest) = title.split_at(kind_len);
    if let Some(scoped) = rest.strip_prefix('(') {
        rest = &scoped[scoped.find(')')? + 1..];
    }
    let breaking = rest.starts_with('!');
    rest = rest.strip_prefix('!').unwrap_or(rest);
    rest.starts_with(": ").then(|| (kind.to_string(), breaking))
}

/// The most characters a summary keeps.
const SUMMARY_CHARS: usize = 200;

/// One line saying what a PR does: the first prose paragraph of its markdown
/// description, reduced to plain text and cut to [`SUMMARY_CHARS`]. Headings,
/// HTML comments, rules and lines holding only an image or HTML tags are
/// passed over on the way to it. `None` when no prose is left.
pub fn summary(body: &str) -> Option<String> {
    let body = without_html_comments(body);
    let mut lines = body
        .lines()
        .map(str::trim)
        .skip_while(|line| !is_prose(line));
    let first = lines.next()?;
    let mut paragraph = list_item_text(first).unwrap_or(first).to_string();
    // A list item ends where the next one starts, so the summary is one
    // item and not the whole list run together.
    for line in lines.take_while(|line| is_prose(line) && list_item_text(line).is_none()) {
        paragraph.push(' ');
        paragraph.push_str(line);
    }
    let text = inline_text(&paragraph);
    let words: Vec<&str> = text.split_whitespace().collect();
    let cut: String = words.join(" ").chars().take(SUMMARY_CHARS).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// `body` without its `<!-- -->` comments. An unclosed comment runs to the
/// end, as GitHub renders it.
fn without_html_comments(body: &str) -> String {
    let mut kept = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        kept.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    kept.push_str(rest);
    kept
}

/// Whether a trimmed line can open or continue a summary: not blank, and not
/// a heading, a horizontal rule, or a line of nothing but images and HTML
/// tags.
fn is_prose(line: &str) -> bool {
    !(line.is_empty() || is_heading(line) || is_rule(line) || is_only_markup(line))
}

fn is_heading(line: &str) -> bool {
    let text = line.trim_start_matches('#');
    let hashes = line.len() - text.len();
    (1..=6).contains(&hashes) && (text.is_empty() || text.starts_with(' '))
}

fn is_rule(line: &str) -> bool {
    ['-', '*', '_'].into_iter().any(|mark| {
        line.chars().filter(|&c| c == mark).count() >= 3
            && line.chars().all(|c| c == mark || c == ' ')
    })
}

/// Whether nothing is left of the line once its images and HTML tags are
/// taken out. A badge is an image inside a link, so link wrappers go too.
fn is_only_markup(line: &str) -> bool {
    let mut rest = line;
    let mut found = false;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            return found;
        }
        let after = if rest.starts_with('<') {
            rest.find('>').map(|end| &rest[end + 1..])
        } else if let Some(wrapped) = rest.strip_prefix("[!") {
            // "[![alt](image)](target)": the image, then the link's target.
            link_end(wrapped)
                .and_then(|(_, after)| after.strip_prefix(']'))
                .and_then(|after| after.strip_prefix('('))
                .and_then(|target| target.find(')').map(|end| &target[end + 1..]))
        } else if let Some(image) = rest.strip_prefix('!') {
            link_end(image).map(|(_, after)| after)
        } else {
            None
        };
        match after {
            Some(after) => {
                rest = after;
                found = true;
            }
            None => return false,
        }
    }
}

/// The text of a trimmed line that is a list item, without its bullet or
/// number and without a task checkbox. `None` for any other line.
fn list_item_text(line: &str) -> Option<&str> {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    let item = if digits > 0 {
        let numbered = &line[digits..];
        numbered
            .strip_prefix(". ")
            .or_else(|| numbered.strip_prefix(") "))?
    } else {
        ["- ", "* ", "+ "]
            .into_iter()
            .find_map(|bullet| line.strip_prefix(bullet))?
    };
    let item = item.trim_start();
    let task = ["[ ] ", "[x] ", "[X] "]
        .into_iter()
        .find_map(|checkbox| item.strip_prefix(checkbox));
    Some(task.unwrap_or(item).trim_start())
}

/// For text starting at a markdown link's `[`: the label and what follows
/// the closing `)` of its target. `None` when no `[label](target)` starts
/// there. Brackets and parentheses nest, so a label holding a link and a
/// target holding parentheses both close where they should.
fn link_end(text: &str) -> Option<(&str, &str)> {
    let close = matching(text, '[', ']')?;
    let target = &text[close + 1..];
    let end = matching(target, '(', ')')?;
    Some((&text[1..close], &target[end + 1..]))
}

/// The byte index of the `close` that pairs with the `open` that `text`
/// starts with.
fn matching(text: &str, open: char, close: char) -> Option<usize> {
    if !text.starts_with(open) {
        return None;
    }
    let mut depth = 0usize;
    for (at, c) in text.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return Some(at);
            }
        }
    }
    None
}

/// Inline markdown as the text a reader sees: a link is its label, an image
/// its alt text, and backticks and emphasis markers are gone. Code keeps its
/// characters, so `snake_case` in backticks survives, and an underscore
/// inside a word is never read as emphasis.
fn inline_text(markdown: &str) -> String {
    let mut text = String::new();
    let mut rest = markdown;
    let mut in_code = false;
    let mut previous: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        let next = after.chars().next();
        if c == '`' {
            in_code = !in_code;
            rest = after;
            continue;
        }
        if !in_code {
            let link = match c {
                '[' => link_end(rest),
                '!' if next == Some('[') => link_end(after),
                _ => None,
            };
            if let Some((label, after_link)) = link {
                text.push_str(&inline_text(label));
                previous = Some(']');
                rest = after_link;
                continue;
            }
            let spaced = |side: Option<char>| side.is_none_or(char::is_whitespace);
            let in_word = |side: Option<char>| side.is_some_and(char::is_alphanumeric);
            let emphasis = match c {
                // Alone between spaces it is a bullet or a multiplication sign.
                '*' => !(spaced(previous) && spaced(next)),
                '_' => !(in_word(previous) && in_word(next)) && !(spaced(previous) && spaced(next)),
                _ => false,
            };
            if emphasis {
                previous = Some(c);
                rest = after;
                continue;
            }
        }
        text.push(c);
        previous = Some(c);
        rest = after;
    }
    text
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

/// Per watched repo, the newest `mergedAt` already taken from GitHub's list
/// of that repo's merged PRs, so GitHub's clock and never the local one. A
/// repo is here from the first time its list is read; `None` is a repo read
/// with no merge in it. Persisted, so merges made while the app was quit are
/// caught up at the next sync.
pub type Watermarks = HashMap<String, Option<String>>;

/// What one sync read from GitHub's merged lists.
#[derive(Debug, Default)]
pub struct CatchUp {
    /// The PRs merged since the watermarks the read started from.
    pub merged: Vec<MergedPr>,
    /// The watermarks after the read. They replace the old ones, so a repo
    /// no longer watched has none.
    pub watermarks: Watermarks,
}

/// Folds the merges one sync read into the section: a reopened PR leaves
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
pub mod testing {
    use super::*;

    /// A Merged section entry for `repo#number`.
    pub fn entry(repo: &str, number: u64, merged_at: &str) -> MergedPr {
        MergedPr {
            number,
            title: "Fix the thing".into(),
            url: format!("https://github.com/{repo}/pull/{number}"),
            repo: repo.into(),
            author: "someone".into(),
            avatar_url: "https://avatars.example/someone".into(),
            owner_avatar_url: "https://avatars.example/acme".into(),
            merged_at: merged_at.into(),
            deploy: None,
            kind: None,
            breaking: false,
            summary: None,
        }
    }

    /// Watermarks from `(repo, newest merge taken)` pairs.
    pub fn watermarks(entries: &[(&str, Option<&str>)]) -> Watermarks {
        entries
            .iter()
            .map(|(repo, at)| (repo.to_string(), at.map(String::from)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{testing::entry, *};
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
            threads_open: false,
            ready_to_merge: false,
            turn: None,
            turn_since: None,
            can_fix_conflict: false,
            fix_requested: false,
            actions: Vec::new(),
            merge_method: crate::github::MergeMethod::Merge,
            base_branch: String::new(),
            head_oid: String::new(),
            mergeability_unknown: false,
            node_id: String::new(),
            unread_count: 0,
            bot_unread_count: 0,
            activity: vec![],
            bot_activity: vec![],
        }
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

    fn kind(title: &str) -> Option<(String, bool)> {
        change_kind(title)
    }

    #[test]
    fn a_conventional_commit_prefix_names_the_kind() {
        assert_eq!(kind("feat: add X"), Some(("feat".into(), false)));
        assert_eq!(kind("fix(sync): Y"), Some(("fix".into(), false)));
        // Kept as written: any lowercase word is a kind.
        assert_eq!(kind("wip: thing"), Some(("wip".into(), false)));
    }

    #[test]
    fn a_bang_before_the_colon_marks_the_change_breaking() {
        assert_eq!(kind("refactor!: Z"), Some(("refactor".into(), true)));
        assert_eq!(kind("feat(api)!: drop v1"), Some(("feat".into(), true)));
    }

    #[test]
    fn a_title_without_the_prefix_has_no_kind() {
        assert_eq!(kind("Add X"), None);
        assert_eq!(kind("WIP: thing"), None);
        assert_eq!(kind("Feat: add X"), None);
        assert_eq!(kind("feat:add X"), None);
        assert_eq!(kind("feat add X"), None);
        assert_eq!(kind("feat(sync: add X"), None);
        assert_eq!(kind(": add X"), None);
        assert_eq!(kind(""), None);
    }

    fn summarized(body: &str) -> Option<&'static str> {
        summary(body).map(|text| &*text.leak())
    }

    #[test]
    fn the_summary_is_the_first_paragraph_on_one_line() {
        assert_eq!(
            summarized("Caches the rates\r\nfor   an hour.\r\n\r\nSecond paragraph."),
            Some("Caches the rates for an hour.")
        );
    }

    #[test]
    fn a_heading_before_the_first_paragraph_is_passed_over() {
        let body = "## Summary\n\nRegenerates the README demo gif so the Merged header \
                    action matches the app after #150.\n\n## Testing\n\nNone.";
        assert_eq!(
            summarized(body),
            Some("Regenerates the README demo gif so the Merged header action matches the app after #150.")
        );
        // A heading ends the paragraph above it, blank line or not.
        assert_eq!(
            summarized("Adds the thing.\n## Why"),
            Some("Adds the thing.")
        );
        // "#150" opens no heading.
        assert_eq!(summarized("#150 follow-up."), Some("#150 follow-up."));
    }

    #[test]
    fn an_html_comment_template_is_removed() {
        let body = "<!--\nDescribe your change.\nLink the issue.\n-->\n\nFixes the retry loop.";
        assert_eq!(summarized(body), Some("Fixes the retry loop."));
        assert_eq!(
            summarized("Fixes <!-- which? --> the retry loop."),
            Some("Fixes the retry loop.")
        );
        assert_eq!(summarized("<!-- never closed\nFixes the retry loop."), None);
    }

    #[test]
    fn lines_of_only_images_tags_or_rules_are_passed_over() {
        assert_eq!(
            summarized("![before](https://img.example/a.png)\n\nMoves the button."),
            Some("Moves the button.")
        );
        assert_eq!(
            summarized(
                "[![ci](https://img.example/badge.svg)](https://ci.example/run)\nMoves the button."
            ),
            Some("Moves the button.")
        );
        assert_eq!(
            summarized("<img src=\"a.png\" width=\"200\">\n<details>\n---\n***\nMoves the button."),
            Some("Moves the button.")
        );
        // Tags around words leave the words, so the line is prose.
        assert_eq!(
            summarized("<p>Moves the button.</p>"),
            Some("<p>Moves the button.</p>")
        );
    }

    #[test]
    fn a_list_item_gives_its_text_without_the_marker() {
        assert_eq!(
            summarized("- Adds the cache\n- Drops the old one"),
            Some("Adds the cache")
        );
        assert_eq!(summarized("* [x] Adds the cache"), Some("Adds the cache"));
        assert_eq!(summarized("- [ ] Adds the cache"), Some("Adds the cache"));
        assert_eq!(
            summarized("1. Adds the cache\n2. More"),
            Some("Adds the cache")
        );
        // An item wrapped over two lines is still one item.
        assert_eq!(
            summarized("- Adds the cache\n  for rates\n- More"),
            Some("Adds the cache for rates")
        );
    }

    #[test]
    fn inline_markdown_is_reduced_to_its_text() {
        assert_eq!(
            summarized("Moves `is_bot` to [the **sync** module](https://example.com/a_(b)) and *renames* __it__."),
            Some("Moves is_bot to the sync module and renames it.")
        );
        // An image inside prose leaves its alt text.
        assert_eq!(
            summarized("See ![the chart](https://img.example/c.png) below."),
            Some("See the chart below.")
        );
        // Not emphasis: an underscore inside a word, a star between spaces,
        // and brackets that are no link.
        assert_eq!(
            summarized("Sets max_retries to 2 * n [sic]."),
            Some("Sets max_retries to 2 * n [sic].")
        );
    }

    #[test]
    fn a_body_with_no_prose_has_no_summary() {
        assert_eq!(summarized(""), None);
        assert_eq!(summarized("  \n\n"), None);
        assert_eq!(summarized("## Summary\n\n<!-- fill me in -->\n\n---"), None);
        assert_eq!(summarized("****"), None);
    }

    #[test]
    fn a_long_summary_is_cut_to_200_characters_not_bytes() {
        // 300 two-byte characters, then 300 four-byte ones.
        assert_eq!(summary(&"\u{e9}".repeat(300)), Some("\u{e9}".repeat(200)));
        assert_eq!(
            summary(&"\u{1f347}".repeat(300)),
            Some("\u{1f347}".repeat(200))
        );
        // The cut never leaves a trailing space.
        let words = format!("{} tail", "a".repeat(199));
        assert_eq!(summary(&words), Some("a".repeat(199)));
    }
}
