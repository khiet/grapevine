use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// PRs per page. GitHub caps connection page sizes at 100; 50 keeps the
/// multi-repo query well under the node limit while rarely paginating.
const PAGE_SIZE: usize = 50;

/// Recent comments/reviews fetched per PR for unread counting. Activity
/// older than these windows can't be counted, so a badge caps out rather
/// than being exact on unusually busy PRs — an acceptable trade against
/// query size, since the watermark is baselined at first sight anyway.
const COMMENT_PAGE: usize = 20;
const REVIEW_PAGE: usize = 20;
const REVIEW_COMMENT_PAGE: usize = 10;

/// Which popover section a PR belongs to, from the viewer's perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Mine,
    Participated,
    All,
}

#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub author: String,
    pub created_at: String,
    pub section: Section,
    /// Comments/reviews newer than the PR's last-read watermark; computed by
    /// the unread engine after fetch, always 0 straight out of this module.
    pub unread_count: u64,
    /// Timestamps (ISO-8601 UTC, ascending) of recent comment/review
    /// activity by people other than the viewer. Input to the unread
    /// computation, not part of the frontend wire payload.
    #[serde(skip)]
    pub activity: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GraphQlError {
    message: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphQlResponse {
    data: Option<Value>,
    #[serde(default)]
    errors: Vec<GraphQlError>,
}

/// Errors are user-facing strings: they surface as inline messages in the
/// settings view, so they must stand on their own without context.
async fn graphql(token: &str, query: &str, variables: Value) -> Result<GraphQlResponse, String> {
    let client = reqwest::Client::builder()
        .user_agent("grapevine")
        .build()
        .map_err(|e| format!("cannot build HTTP client: {e}"))?;
    let response = client
        .post(GRAPHQL_URL)
        .bearer_auth(token)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Check your network connection.".to_string())?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("GitHub rejected the token. Check that it is valid and not expired.".into());
    }
    if !response.status().is_success() {
        return Err(format!(
            "GitHub returned an error (HTTP {}).",
            response.status().as_u16()
        ));
    }
    response
        .json::<GraphQlResponse>()
        .await
        .map_err(|_| "GitHub returned an unexpected response.".to_string())
}

/// Validates the token by asking GitHub who it belongs to. Returns the login.
pub async fn validate_token(token: &str) -> Result<String, String> {
    let response = graphql(token, "query { viewer { login } }", json!({})).await?;
    if let Some(login) = response
        .data
        .as_ref()
        .and_then(|d| d.pointer("/viewer/login"))
        .and_then(Value::as_str)
    {
        return Ok(login.to_string());
    }
    Err(response
        .errors
        .into_iter()
        .next()
        .map(|e| format!("GitHub error: {}", e.message))
        .unwrap_or_else(|| "GitHub returned an unexpected response.".into()))
}

/// Checks that `owner/name` exists and is accessible with this token.
/// Returns the canonical `nameWithOwner` (fixes user-typed casing).
pub async fn validate_repo(token: &str, owner: &str, name: &str) -> Result<String, String> {
    let response = graphql(
        token,
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { nameWithOwner } }",
        json!({ "owner": owner, "name": name }),
    )
    .await?;
    if let Some(full_name) = response
        .data
        .as_ref()
        .and_then(|d| d.pointer("/repository/nameWithOwner"))
        .and_then(Value::as_str)
    {
        return Ok(full_name.to_string());
    }
    if response
        .errors
        .iter()
        .any(|e| e.kind.as_deref() == Some("NOT_FOUND"))
    {
        return Err(format!(
            "{owner}/{name} was not found or is not accessible with this token."
        ));
    }
    Err(response
        .errors
        .into_iter()
        .next()
        .map(|e| format!("GitHub error: {}", e.message))
        .unwrap_or_else(|| "GitHub returned an unexpected response.".into()))
}

/// Fetches open PRs for every `owner/name` in `repos` and classifies each
/// into its popover section. All repos go into one query (as aliased
/// `repository` fields); follow-up queries are issued only for repos whose
/// PR list spills past [`PAGE_SIZE`]. Repos that have vanished or become
/// inaccessible are skipped rather than failing the whole sync.
pub async fn fetch_open_prs(token: &str, repos: &[String]) -> Result<Vec<PullRequest>, String> {
    // (owner, name, resume cursor); repos drop out once fully fetched.
    let mut pending: Vec<(String, String, Option<String>)> = repos
        .iter()
        .filter_map(|full| full.split_once('/'))
        .map(|(owner, name)| (owner.to_string(), name.to_string(), None))
        .collect();
    let mut prs = Vec::new();

    let mut rounds = 0;
    while !pending.is_empty() {
        rounds += 1;
        if rounds > 20 {
            return Err("GitHub returned too many pages of pull requests.".into());
        }
        let mut query = String::from("query { viewer { login } ");
        for (i, (owner, name, cursor)) in pending.iter().enumerate() {
            query.push_str(&repo_field(
                &format!("r{i}"),
                owner,
                name,
                cursor.as_deref(),
            ));
        }
        query.push('}');

        let response = graphql(token, &query, json!({})).await?;
        let Some(data) = response.data else {
            return Err(response
                .errors
                .into_iter()
                .next()
                .map(|e| format!("GitHub error: {}", e.message))
                .unwrap_or_else(|| "GitHub returned an unexpected response.".into()));
        };
        let viewer = data
            .pointer("/viewer/login")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        let mut still_pending = Vec::new();
        for (i, (owner, name, _)) in pending.iter().enumerate() {
            let alias = format!("r{i}");
            let Some(repo) = data.get(alias.as_str()).filter(|v| !v.is_null()) else {
                continue;
            };
            if let Some(cursor) = collect_repo_prs(repo, &viewer, &mut prs) {
                still_pending.push((owner.clone(), name.clone(), Some(cursor)));
            }
        }
        pending = still_pending;
    }

    // Newest first across all repos; ISO-8601 UTC timestamps sort lexically.
    prs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(prs)
}

fn repo_field(alias: &str, owner: &str, name: &str, after: Option<&str>) -> String {
    let after = after
        .map(|c| format!(", after: {}", json!(c)))
        .unwrap_or_default();
    format!(
        "{alias}: repository(owner: {owner}, name: {name}) {{ \
           nameWithOwner \
           pullRequests(states: OPEN, first: {PAGE_SIZE}{after}, \
                        orderBy: {{field: CREATED_AT, direction: DESC}}) {{ \
             pageInfo {{ hasNextPage endCursor }} \
             nodes {{ \
               number title url createdAt viewerDidAuthor \
               author {{ login }} \
               participants(first: 50) {{ nodes {{ login }} }} \
               reviewRequests(first: 50) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }} \
               comments(last: {COMMENT_PAGE}) {{ nodes {{ createdAt author {{ login }} }} }} \
               reviews(last: {REVIEW_PAGE}) {{ nodes {{ state submittedAt author {{ login }} \
                 comments(last: {REVIEW_COMMENT_PAGE}) {{ nodes {{ createdAt author {{ login }} }} }} }} }} \
             }} \
           }} \
         }} ",
        owner = json!(owner),
        name = json!(name),
    )
}

/// Appends one repo's page of PRs to `out`. Returns the cursor to resume
/// from when the repo has further pages.
fn collect_repo_prs(repo: &Value, viewer: &str, out: &mut Vec<PullRequest>) -> Option<String> {
    let repo_name = repo
        .pointer("/nameWithOwner")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let nodes = repo
        .pointer("/pullRequests/nodes")
        .and_then(Value::as_array);
    for node in nodes.into_iter().flatten() {
        out.push(PullRequest {
            number: node.get("number").and_then(Value::as_u64).unwrap_or(0),
            title: node
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            url: node
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            repo: repo_name.to_string(),
            // A null author means the account was deleted; GitHub shows these
            // as "ghost".
            author: node
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("ghost")
                .to_string(),
            created_at: node
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            section: section_for(node, viewer),
            unread_count: 0,
            activity: collect_activity(node, viewer),
        });
    }
    let page = repo.pointer("/pullRequests/pageInfo")?;
    if page.get("hasNextPage").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    page.get("endCursor")
        .and_then(Value::as_str)
        .map(String::from)
}

/// Gathers the timestamps of activity that counts as an "update" for unread
/// purposes: issue comments, submitted reviews, and review comments — never
/// commits or CI, which the query doesn't even fetch. The viewer's own
/// activity is excluded (you have read what you wrote), as are PENDING
/// reviews (invisible to everyone but their author until submitted).
fn collect_activity(node: &Value, viewer: &str) -> Vec<String> {
    let mut times = Vec::new();
    let by_other =
        |item: &Value| item.pointer("/author/login").and_then(Value::as_str) != Some(viewer);
    let mut push = |item: &Value, time_field: &str| {
        if by_other(item) {
            if let Some(t) = item.get(time_field).and_then(Value::as_str) {
                times.push(t.to_string());
            }
        }
    };

    for comment in list(node, "/comments/nodes") {
        push(comment, "createdAt");
    }
    for review in list(node, "/reviews/nodes") {
        // A null submittedAt means the review is still PENDING.
        if review.get("submittedAt").and_then(Value::as_str).is_none() {
            continue;
        }
        let comments = list(review, "/comments/nodes");
        // A COMMENTED review is often just the wrapper GitHub creates around
        // inline comments; counting both would double-count a single reply.
        let is_wrapper = review.get("state").and_then(Value::as_str) == Some("COMMENTED")
            && !comments.is_empty();
        if !is_wrapper {
            push(review, "submittedAt");
        }
        for comment in comments {
            push(comment, "createdAt");
        }
    }
    times.sort();
    times
}

fn list<'a>(node: &'a Value, pointer: &str) -> &'a [Value] {
    node.pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Mine beats Participated: the author is always a participant, so order
/// matters. Participants covers reviewers, commenters, and mentions;
/// reviewRequests covers pending review requests, which don't count as
/// participation until acted on.
fn section_for(node: &Value, viewer: &str) -> Section {
    if node.get("viewerDidAuthor").and_then(Value::as_bool) == Some(true) {
        return Section::Mine;
    }
    let login_matches = |list: Option<&Value>, login_path: &str| {
        list.and_then(Value::as_array).is_some_and(|nodes| {
            nodes
                .iter()
                .any(|n| n.pointer(login_path).and_then(Value::as_str) == Some(viewer))
        })
    };
    if login_matches(node.pointer("/participants/nodes"), "/login")
        || login_matches(
            node.pointer("/reviewRequests/nodes"),
            "/requestedReviewer/login",
        )
    {
        return Section::Participated;
    }
    Section::All
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr_node(overrides: Value) -> Value {
        let mut node = json!({
            "number": 7,
            "title": "Fix the thing",
            "url": "https://github.com/acme/widgets/pull/7",
            "createdAt": "2026-07-10T12:00:00Z",
            "viewerDidAuthor": false,
            "author": { "login": "someone" },
            "participants": { "nodes": [{ "login": "someone" }] },
            "reviewRequests": { "nodes": [] }
        });
        node.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        node
    }

    #[test]
    fn authored_prs_are_mine_even_though_the_author_participates() {
        let node = pr_node(json!({
            "viewerDidAuthor": true,
            "participants": { "nodes": [{ "login": "khiet" }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Mine);
    }

    #[test]
    fn participant_prs_are_participated() {
        let node = pr_node(json!({
            "participants": { "nodes": [{ "login": "other" }, { "login": "khiet" }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn review_requested_prs_are_participated() {
        let node = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn team_review_requests_do_not_match_the_viewer() {
        // Team reviewers have no `login` field in the response.
        let node = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::All);
    }

    #[test]
    fn unrelated_prs_land_in_all() {
        assert_eq!(section_for(&pr_node(json!({})), "khiet"), Section::All);
    }

    #[test]
    fn collects_rows_and_reports_no_cursor_on_the_last_page() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": "abc" },
                "nodes": [pr_node(json!({}))]
            }
        });
        let mut out = Vec::new();
        let cursor = collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(cursor, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].repo, "acme/widgets");
        assert_eq!(out[0].number, 7);
        assert_eq!(out[0].author, "someone");
        assert_eq!(out[0].created_at, "2026-07-10T12:00:00Z");
    }

    #[test]
    fn reports_the_resume_cursor_when_more_pages_remain() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": true, "endCursor": "abc" },
                "nodes": []
            }
        });
        let mut out = Vec::new();
        assert_eq!(
            collect_repo_prs(&repo, "khiet", &mut out),
            Some("abc".to_string())
        );
    }

    #[test]
    fn deleted_authors_render_as_ghost() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [pr_node(json!({ "author": null }))]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(out[0].author, "ghost");
    }

    #[test]
    fn issue_comments_by_others_count_as_activity_but_the_viewers_do_not() {
        let node = pr_node(json!({
            "comments": { "nodes": [
                { "createdAt": "2026-07-11T08:00:00Z", "author": { "login": "other" } },
                { "createdAt": "2026-07-12T09:00:00Z", "author": { "login": "khiet" } }
            ] }
        }));
        assert_eq!(
            collect_activity(&node, "khiet"),
            vec!["2026-07-11T08:00:00Z"]
        );
    }

    #[test]
    fn submitted_reviews_count_but_pending_ones_do_not() {
        let node = pr_node(json!({
            "reviews": { "nodes": [
                { "state": "APPROVED", "submittedAt": "2026-07-11T08:00:00Z",
                  "author": { "login": "other" }, "comments": { "nodes": [] } },
                { "state": "PENDING", "submittedAt": null,
                  "author": { "login": "other" }, "comments": { "nodes": [] } }
            ] }
        }));
        assert_eq!(
            collect_activity(&node, "khiet"),
            vec!["2026-07-11T08:00:00Z"]
        );
    }

    #[test]
    fn a_commented_review_wrapping_inline_comments_counts_only_the_comments() {
        let node = pr_node(json!({
            "reviews": { "nodes": [{
                "state": "COMMENTED", "submittedAt": "2026-07-11T08:00:00Z",
                "author": { "login": "other" },
                "comments": { "nodes": [
                    { "createdAt": "2026-07-11T08:00:00Z", "author": { "login": "other" } },
                    { "createdAt": "2026-07-11T08:00:05Z", "author": { "login": "other" } }
                ] }
            }] }
        }));
        assert_eq!(
            collect_activity(&node, "khiet"),
            vec!["2026-07-11T08:00:00Z", "2026-07-11T08:00:05Z"]
        );
    }

    #[test]
    fn a_review_with_a_verdict_counts_alongside_its_inline_comments() {
        let node = pr_node(json!({
            "reviews": { "nodes": [{
                "state": "CHANGES_REQUESTED", "submittedAt": "2026-07-11T08:00:00Z",
                "author": { "login": "other" },
                "comments": { "nodes": [
                    { "createdAt": "2026-07-11T07:59:00Z", "author": { "login": "other" } }
                ] }
            }] }
        }));
        assert_eq!(
            collect_activity(&node, "khiet"),
            vec!["2026-07-11T07:59:00Z", "2026-07-11T08:00:00Z"]
        );
    }

    #[test]
    fn activity_is_sorted_across_comments_and_reviews() {
        let node = pr_node(json!({
            "comments": { "nodes": [
                { "createdAt": "2026-07-12T09:00:00Z", "author": { "login": "other" } }
            ] },
            "reviews": { "nodes": [{
                "state": "APPROVED", "submittedAt": "2026-07-11T08:00:00Z",
                "author": { "login": "other" }, "comments": { "nodes": [] }
            }] }
        }));
        assert_eq!(
            collect_activity(&node, "khiet"),
            vec!["2026-07-11T08:00:00Z", "2026-07-12T09:00:00Z"]
        );
    }

    #[test]
    fn commits_and_status_changes_never_reach_the_activity_list() {
        // The query only asks for comments and reviews, so a node with
        // neither yields no activity regardless of anything else on the PR.
        assert!(collect_activity(&pr_node(json!({})), "khiet").is_empty());
    }

    #[test]
    fn repo_field_quotes_arguments_and_resumes_from_a_cursor() {
        let field = repo_field("r0", "acme", "widgets", Some("abc"));
        assert!(field.contains(r#"repository(owner: "acme", name: "widgets")"#));
        assert!(field.contains(r#"after: "abc""#));
        assert!(!repo_field("r0", "acme", "widgets", None).contains("after:"));
    }
}
