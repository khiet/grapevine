//! What a PR's own text says about its change: the kind its title names and
//! a one-line summary of its markdown description.

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
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let underlined = |at: usize| lines.get(at + 1).is_some_and(|next| is_underline(next));
    // A line with an underline beneath it is a heading too.
    let start = (0..lines.len()).find(|&at| is_prose(lines[at]) && !underlined(at))?;
    let item = list_item_text(lines[start]);
    let mut paragraph = item.unwrap_or(lines[start]).to_string();
    for line in lines[start + 1..].iter().take_while(|line| is_prose(line)) {
        // A list item ends where the next one starts, so the summary is one
        // item and not the whole list run together. In a plain paragraph a
        // wrapped line may well open with "- ", and stays.
        if item.is_some() && list_item_text(line).is_some() {
            break;
        }
        paragraph.push(' ');
        paragraph.push_str(line);
    }
    let text = inline_text(&paragraph);
    let words: Vec<&str> = text.split_whitespace().collect();
    let cut: String = words.join(" ").chars().take(SUMMARY_CHARS).collect();
    let cut = cut.trim_end();
    // Leftover marks alone, such as the "-" of an empty template bullet,
    // say nothing.
    cut.chars()
        .any(char::is_alphanumeric)
        .then(|| cut.to_string())
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
    !(line.is_empty()
        || is_heading(line)
        || is_rule(line)
        || is_underline(line)
        || is_only_markup(line))
}

fn is_heading(line: &str) -> bool {
    let text = line.trim_start_matches('#');
    let hashes = line.len() - text.len();
    (1..=6).contains(&hashes) && (text.is_empty() || text.starts_with(' '))
}

/// The line under a heading written as text over a row of `=` or `-`.
fn is_underline(line: &str) -> bool {
    !line.is_empty() && (line.chars().all(|c| c == '=') || line.chars().all(|c| c == '-'))
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
            // "<https://example.com>" is a link written out, not a tag.
            rest.find('>')
                .filter(|&end| !rest[..end].contains("://"))
                .map(|end| &rest[end + 1..])
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

/// The text of a trimmed line that is a bulleted list item, without its
/// bullet and without a task checkbox. `None` for any other line.
fn list_item_text(line: &str) -> Option<&str> {
    let item = ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|bullet| line.strip_prefix(bullet))?
        .trim_start();
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

/// A stretch of inline markdown on its way to plain text.
enum Piece {
    Text(String),
    /// A run of `*` or `_` that may be emphasis.
    Marks {
        run: String,
        can_open: bool,
        can_close: bool,
    },
}

/// Inline markdown as the text a reader sees: a link is its label, an image
/// its alt text, and backticks and emphasis markers are gone. A `*` or `_`
/// goes only as one of a pair around text, so `src/*.rs`, `2 * n` and
/// `snake_case` keep theirs. Code keeps every character.
fn inline_text(markdown: &str) -> String {
    let mut pieces = inline_pieces(markdown);
    // Each closing run takes the nearest open run of the same marks; both
    // become empty text.
    let mut open: Vec<usize> = Vec::new();
    for at in 0..pieces.len() {
        let Piece::Marks {
            run,
            can_open,
            can_close,
        } = &pieces[at]
        else {
            continue;
        };
        let same_run = |opener: &usize| matches!(&pieces[*opener], Piece::Marks { run: opened, .. } if opened == run);
        let partner = can_close.then(|| open.iter().rposition(same_run)).flatten();
        let can_open = *can_open;
        match partner {
            Some(position) => {
                pieces[open[position]] = Piece::Text(String::new());
                pieces[at] = Piece::Text(String::new());
                open.truncate(position);
            }
            None if can_open => open.push(at),
            None => {}
        }
    }
    pieces
        .into_iter()
        .map(|piece| match piece {
            Piece::Text(text) => text,
            Piece::Marks { run, .. } => run,
        })
        .collect()
}

fn inline_pieces(markdown: &str) -> Vec<Piece> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = markdown;
    let mut in_code = false;
    let mut previous: Option<char> = None;
    while let Some(c) = rest.chars().next() {
        let after = &rest[c.len_utf8()..];
        if c == '`' {
            in_code = !in_code;
            previous = Some(c);
            rest = after;
            continue;
        }
        if !in_code {
            let link = match c {
                '[' => link_end(rest),
                '!' if after.starts_with('[') => link_end(after),
                _ => None,
            };
            if let Some((label, after_link)) = link {
                text.push_str(&inline_text(label));
                previous = Some(')');
                rest = after_link;
                continue;
            }
            if c == '*' || c == '_' {
                let after_run = rest.trim_start_matches(c);
                let run = &rest[..rest.len() - after_run.len()];
                let next = after_run.chars().next();
                // CommonMark's flanking rules: a run opens emphasis when
                // text follows it and closes when text precedes it, and an
                // underscore inside a word does neither.
                let space = |side: Option<char>| side.is_none_or(char::is_whitespace);
                let word = |side: Option<char>| side.is_some_and(char::is_alphanumeric);
                let opens = !space(next) && (word(next) || !word(previous));
                let closes = !space(previous) && (word(previous) || !word(next));
                let in_word = c == '_' && word(previous) && word(next);
                pieces.push(Piece::Text(std::mem::take(&mut text)));
                pieces.push(Piece::Marks {
                    run: run.to_string(),
                    can_open: opens && !in_word,
                    can_close: closes && !in_word,
                });
                previous = Some(c);
                rest = after_run;
                continue;
            }
        }
        text.push(c);
        previous = Some(c);
        rest = after;
    }
    pieces.push(Piece::Text(text));
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_conventional_commit_prefix_names_the_kind() {
        assert_eq!(change_kind("feat: add X"), Some(("feat".into(), false)));
        assert_eq!(change_kind("fix(sync): Y"), Some(("fix".into(), false)));
        // Kept as written: any lowercase word is a kind.
        assert_eq!(change_kind("wip: thing"), Some(("wip".into(), false)));
    }

    #[test]
    fn a_bang_before_the_colon_marks_the_change_breaking() {
        assert_eq!(change_kind("refactor!: Z"), Some(("refactor".into(), true)));
        assert_eq!(
            change_kind("feat(api)!: drop v1"),
            Some(("feat".into(), true))
        );
    }

    #[test]
    fn a_title_without_the_prefix_has_no_kind() {
        assert_eq!(change_kind("Add X"), None);
        assert_eq!(change_kind("WIP: thing"), None);
        assert_eq!(change_kind("Feat: add X"), None);
        assert_eq!(change_kind("feat:add X"), None);
        assert_eq!(change_kind("feat add X"), None);
        assert_eq!(change_kind("feat(sync: add X"), None);
        assert_eq!(change_kind(": add X"), None);
        assert_eq!(change_kind(""), None);
    }

    #[test]
    fn the_summary_is_the_first_paragraph_on_one_line() {
        assert_eq!(
            summary("Caches the rates\r\nfor   an hour.\r\n\r\nSecond paragraph.").as_deref(),
            Some("Caches the rates for an hour.")
        );
    }

    #[test]
    fn a_heading_before_the_first_paragraph_is_passed_over() {
        let body = "## Summary\n\nRegenerates the README demo gif so the Merged header \
                    action matches the app after #150.\n\n## Testing\n\nNone.";
        assert_eq!(
            summary(body).as_deref(),
            Some("Regenerates the README demo gif so the Merged header action matches the app after #150.")
        );
        // A heading ends the paragraph above it, blank line or not.
        assert_eq!(
            summary("Adds the thing.\n## Why").as_deref(),
            Some("Adds the thing.")
        );
        // "#150" opens no heading.
        assert_eq!(
            summary("#150 follow-up.").as_deref(),
            Some("#150 follow-up.")
        );
    }

    #[test]
    fn an_html_comment_template_is_removed() {
        let body = "<!--\nDescribe your change.\nLink the issue.\n-->\n\nFixes the retry loop.";
        assert_eq!(summary(body).as_deref(), Some("Fixes the retry loop."));
        assert_eq!(
            summary("Fixes <!-- which? --> the retry loop.").as_deref(),
            Some("Fixes the retry loop.")
        );
        assert_eq!(
            summary("<!-- never closed\nFixes the retry loop.").as_deref(),
            None
        );
    }

    #[test]
    fn lines_of_only_images_tags_or_rules_are_passed_over() {
        assert_eq!(
            summary("![before](https://img.example/a.png)\n\nMoves the button.").as_deref(),
            Some("Moves the button.")
        );
        assert_eq!(
            summary(
                "[![ci](https://img.example/badge.svg)](https://ci.example/run)\nMoves the button."
            )
            .as_deref(),
            Some("Moves the button.")
        );
        assert_eq!(
            summary("<img src=\"a.png\" width=\"200\">\n<details>\n---\n***\nMoves the button.")
                .as_deref(),
            Some("Moves the button.")
        );
        // Tags around words leave the words, so the line is prose.
        assert_eq!(
            summary("<p>Moves the button.</p>").as_deref(),
            Some("<p>Moves the button.</p>")
        );
    }

    #[test]
    fn a_list_item_gives_its_text_without_the_marker() {
        assert_eq!(
            summary("- Adds the cache\n- Drops the old one").as_deref(),
            Some("Adds the cache")
        );
        assert_eq!(
            summary("* [x] Adds the cache").as_deref(),
            Some("Adds the cache")
        );
        assert_eq!(
            summary("- [ ] Adds the cache").as_deref(),
            Some("Adds the cache")
        );
        // An item wrapped over two lines is still one item.
        assert_eq!(
            summary("- Adds the cache\n  for rates\n- More").as_deref(),
            Some("Adds the cache for rates")
        );
    }

    #[test]
    fn inline_markdown_is_reduced_to_its_text() {
        assert_eq!(
            summary("Moves `is_bot` to [the **sync** module](https://example.com/a_(b)) and *renames* __it__.").as_deref(),
            Some("Moves is_bot to the sync module and renames it.")
        );
        // An image inside prose leaves its alt text.
        assert_eq!(
            summary("See ![the chart](https://img.example/c.png) below.").as_deref(),
            Some("See the chart below.")
        );
        // Not emphasis: an underscore inside a word, a star between spaces,
        // and brackets that are no link.
        assert_eq!(
            summary("Sets max_retries to 2 * n [sic].").as_deref(),
            Some("Sets max_retries to 2 * n [sic].")
        );
    }

    #[test]
    fn a_star_or_underscore_with_no_partner_is_kept() {
        for literal in [
            "Lints src/**/*.ts files.",
            "Preserve *.json files.",
            "Raises the limit to 2*n.",
            "Renames _unused.",
        ] {
            assert_eq!(summary(literal).as_deref(), Some(literal));
        }
        assert_eq!(
            summary("Keeps *.json and **drops** the rest.").as_deref(),
            Some("Keeps *.json and drops the rest.")
        );
    }

    #[test]
    fn a_wrapped_line_that_opens_like_a_list_item_stays_in_its_paragraph() {
        assert_eq!(
            summary("Raises the retry limit from 5 to\n10. Larger batches need it.").as_deref(),
            Some("Raises the retry limit from 5 to 10. Larger batches need it.")
        );
        assert_eq!(
            summary("Moves the cache\n- the old one - out of the way.").as_deref(),
            Some("Moves the cache - the old one - out of the way.")
        );
    }

    #[test]
    fn a_heading_written_as_underlined_text_is_passed_over() {
        assert_eq!(
            summary("Summary\n=======\n\nFixes retries.").as_deref(),
            Some("Fixes retries.")
        );
        assert_eq!(
            summary("Summary\n---\nFixes retries.").as_deref(),
            Some("Fixes retries.")
        );
    }

    #[test]
    fn a_link_written_out_in_angle_brackets_is_prose() {
        assert_eq!(
            summary("<https://example.org/notes>").as_deref(),
            Some("<https://example.org/notes>")
        );
    }

    #[test]
    fn a_body_with_no_prose_has_no_summary() {
        assert_eq!(summary("").as_deref(), None);
        assert_eq!(summary("  \n\n").as_deref(), None);
        assert_eq!(
            summary("## Summary\n\n<!-- fill me in -->\n\n---").as_deref(),
            None
        );
        assert_eq!(summary("****").as_deref(), None);
        // An empty bullet left over from a template.
        assert_eq!(summary("## Changes\n-").as_deref(), None);
        assert_eq!(summary("- [ ] ").as_deref(), None);
    }

    #[test]
    fn a_long_summary_is_cut_to_200_characters_not_bytes() {
        // 300 two-byte characters, then 300 four-byte ones.
        assert_eq!(summary(&"\u{e9}".repeat(300)), Some("\u{e9}".repeat(200)));
        assert_eq!(
            summary(&"\u{20000}".repeat(300)),
            Some("\u{20000}".repeat(200))
        );
        // The cut never leaves a trailing space.
        let words = format!("{} tail", "a".repeat(199));
        assert_eq!(summary(&words), Some("a".repeat(199)));
    }
}
