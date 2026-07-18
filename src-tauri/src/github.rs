use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const GRAPHQL_URL: &str = "https://api.github.com/graphql";

/// A sync-path failure, split by what the sync loop should do about it: rate
/// limiting waits for the reset, anything else retries with backoff.
/// `Display` renders the message the popover and settings view show.
#[derive(Debug, Clone, PartialEq)]
pub enum GithubError {
    RateLimited {
        /// `None` when GitHub did not say when the window resets.
        reset_epoch_secs: Option<u64>,
    },
    Other(String),
}

impl std::fmt::Display for GithubError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GithubError::RateLimited { .. } => {
                write!(f, "GitHub rate limit reached. Waiting for it to reset.")
            }
            GithubError::Other(message) => write!(f, "{message}"),
        }
    }
}

impl From<GithubError> for String {
    fn from(e: GithubError) -> String {
        e.to_string()
    }
}

/// GraphQL rate-limit budget reported alongside a successful sync; lets the
/// loop slow down before GitHub starts refusing.
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimit {
    pub remaining: u64,
    pub reset_epoch_secs: Option<u64>,
}

/// PRs per page. GitHub caps connections at 100; 50 keeps the multi-repo
/// query well under the node limit while rarely paginating.
const PAGE_SIZE: usize = 50;

/// Recent comments/reviews fetched per PR for unread counting. Older activity
/// cannot be counted, so badges cap out on unusually busy PRs — acceptable,
/// since the watermark is baselined at first sight anyway.
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

/// One reason a PR is blocked, for the row's indicator dot. Declaration order
/// is the fixed tooltip order (conflict, then CI, then review); the frontend
/// renders the list verbatim and never re-derives or re-sorts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockedReason {
    Conflict,
    Ci,
    Review,
}

#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub author: String,
    /// Author's GitHub avatar URL; empty when the account was deleted.
    pub avatar_url: String,
    /// Repo owner's (organization or user) avatar URL, used as the org badge
    /// on the row. Empty when GitHub returns no owner; the UI shows no badge.
    pub owner_avatar_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub section: Section,
    /// Why the PR is blocked, in tooltip order; empty means no dot. Composed
    /// here rather than shipping GitHub's raw enums so the frontend renders
    /// one dot plus tooltip instead of re-deriving the logic. A property of
    /// the PR, never an unread event: it does not feed [`collect_activity`]
    /// and must never move a badge or the tray count.
    pub blocked_reasons: Vec<BlockedReason>,
    /// Whether the PR is a draft. Renders as a neutral pill, and suppresses
    /// the blocked dot (see [`blocked_reasons_for`]). Same property-not-event
    /// rule as `blocked_reasons`.
    pub is_draft: bool,
    /// Activity newer than the PR's last-read watermark; filled in by the
    /// unread engine after fetch, always 0 out of this module.
    pub unread_count: u64,
    /// Recent comment/review timestamps (ISO-8601 UTC, ascending) by people
    /// other than the viewer. Input to the unread computation.
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

async fn graphql(
    token: &str,
    query: &str,
    variables: Value,
) -> Result<GraphQlResponse, GithubError> {
    Ok(graphql_with_scopes(token, query, variables).await?.0)
}

/// Like [`graphql`], but also reports the raw `X-OAuth-Scopes` header: `None`
/// when GitHub sent none (fine-grained PATs and App tokens have permissions,
/// not scopes), `Some` — possibly empty — for classic tokens.
async fn graphql_with_scopes(
    token: &str,
    query: &str,
    variables: Value,
) -> Result<(GraphQlResponse, Option<String>), GithubError> {
    let client = reqwest::Client::builder()
        .user_agent("grapevine")
        .build()
        .map_err(|e| GithubError::Other(format!("cannot build HTTP client: {e}")))?;
    let response = client
        .post(GRAPHQL_URL)
        .bearer_auth(token)
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|_| {
            GithubError::Other("Could not reach GitHub. Check your network connection.".into())
        })?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(GithubError::Other(
            "GitHub rejected the token. Check that it is valid and not expired.".into(),
        ));
    }
    // Both are read as quota refusals: the primary limit answers 403, the
    // secondary limit 429.
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(GithubError::RateLimited {
            reset_epoch_secs: rate_limit_reset(response.headers()),
        });
    }
    if !status.is_success() {
        return Err(GithubError::Other(format!(
            "GitHub returned an error (HTTP {}).",
            status.as_u16()
        )));
    }
    let scopes = response
        .headers()
        .get("x-oauth-scopes")
        .map(|v| v.to_str().unwrap_or_default().to_string());
    let parsed = response
        .json::<GraphQlResponse>()
        .await
        .map_err(|_| GithubError::Other("GitHub returned an unexpected response.".into()))?;
    Ok((parsed, scopes))
}

/// When the current quota window resets. `retry-after` counts seconds from
/// now; `x-ratelimit-reset` is already an absolute epoch.
fn rate_limit_reset(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    if let Some(seconds) = value("retry-after") {
        return Some(now_epoch_secs() + seconds);
    }
    value("x-ratelimit-reset")
}

pub fn now_epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Folds a GraphQL-level error list into one typed error. GitHub tags quota
/// refusals with `"type": "RATE_LIMITED"`.
fn error_from_graphql(errors: Vec<GraphQlError>) -> GithubError {
    if errors
        .iter()
        .any(|e| e.kind.as_deref() == Some("RATE_LIMITED"))
    {
        return GithubError::RateLimited {
            reset_epoch_secs: None,
        };
    }
    GithubError::Other(
        errors
            .into_iter()
            .next()
            .map(|e| format!("GitHub error: {}", e.message))
            .unwrap_or_else(|| "GitHub returned an unexpected response.".into()),
    )
}

/// Parses GitHub's fixed ISO-8601 UTC form ("2026-07-16T12:00:00Z") to epoch
/// seconds; `None` on anything else. Hand-rolled to avoid a date-time crate
/// for one field: GitHub never sends offsets or fractional seconds here.
fn rfc3339_utc_to_epoch_secs(iso: &str) -> Option<u64> {
    let bytes = iso.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if bytes[10] != b'T' || bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| iso[range].parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, minute, second) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Days-from-civil (Howard Hinnant): days since 1970-01-01 for a
    // proleptic Gregorian date.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

/// Outcome of a successful token validation: who the token belongs to, plus
/// an optional user-facing warning about the scopes it was granted.
pub struct ValidatedToken {
    pub login: String,
    pub scope_warning: Option<String>,
}

/// Warns when a token's granted scopes cannot see private repositories.
///
/// `header` is the raw `X-OAuth-Scopes` response header, where absent and
/// empty mean opposite things: absent means the token has permissions rather
/// than scopes (fine-grained PAT, App token) and may well read private repos,
/// so warning would call a working setup broken; empty means a classic token
/// minted with no scopes ticked, which is the case this exists to catch.
fn scope_warning(header: Option<&str>) -> Option<String> {
    let scopes = header?;
    if scopes.split(',').any(|scope| scope.trim() == "repo") {
        return None;
    }
    Some("This token has no repo scope, so private repositories won't be visible.".into())
}

/// Validates the token by asking GitHub who it belongs to.
pub async fn validate_token(token: &str) -> Result<ValidatedToken, String> {
    let (response, scopes) = graphql_with_scopes(token, "query { viewer { login } }", json!({}))
        .await
        .map_err(String::from)?;
    if let Some(login) = response
        .data
        .as_ref()
        .and_then(|d| d.pointer("/viewer/login"))
        .and_then(Value::as_str)
    {
        return Ok(ValidatedToken {
            login: login.to_string(),
            scope_warning: scope_warning(scopes.as_deref()),
        });
    }
    Err(error_from_graphql(response.errors).into())
}

/// Checks that `owner/name` exists and is accessible with this token.
/// Returns the canonical `nameWithOwner` (fixes user-typed casing).
pub async fn validate_repo(token: &str, owner: &str, name: &str) -> Result<String, String> {
    let response = graphql(
        token,
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { nameWithOwner } }",
        json!({ "owner": owner, "name": name }),
    )
    .await
    .map_err(String::from)?;
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
    Err(error_from_graphql(response.errors).into())
}

/// A completed PR fetch plus the rate-limit budget GitHub reported with it.
pub struct FetchedPrs {
    pub prs: Vec<PullRequest>,
    pub rate_limit: Option<RateLimit>,
}

/// Fetches open PRs for every `owner/name` in `repos` and classifies each into
/// its popover section. All repos share one query as aliased `repository`
/// fields; only repos spilling past [`PAGE_SIZE`] need a follow-up query.
/// Vanished or inaccessible repos are skipped rather than failing the sync.
pub async fn fetch_open_prs(token: &str, repos: &[String]) -> Result<FetchedPrs, GithubError> {
    // (owner, name, resume cursor); repos drop out once fully fetched.
    let mut pending: Vec<(String, String, Option<String>)> = repos
        .iter()
        .filter_map(|full| full.split_once('/'))
        .map(|(owner, name)| (owner.to_string(), name.to_string(), None))
        .collect();
    let mut prs = Vec::new();
    let mut rate_limit = None;

    let mut rounds = 0;
    while !pending.is_empty() {
        rounds += 1;
        if rounds > 20 {
            return Err(GithubError::Other(
                "GitHub returned too many pages of pull requests.".into(),
            ));
        }
        let mut query = String::from("query { viewer { login } rateLimit { remaining resetAt } ");
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
            return Err(error_from_graphql(response.errors));
        };
        // Later pages overwrite: the budget after the final request is the
        // one the sync loop plans around.
        if let Some(limit) = collect_rate_limit(&data) {
            rate_limit = Some(limit);
        }
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

    // Most recently updated first across all repos; ISO-8601 UTC timestamps
    // sort lexically.
    prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(FetchedPrs { prs, rate_limit })
}

/// Reads the `rateLimit` field out of a response; `None` rather than failing
/// the sync when it is missing or malformed.
fn collect_rate_limit(data: &Value) -> Option<RateLimit> {
    let remaining = data
        .pointer("/rateLimit/remaining")
        .and_then(Value::as_u64)?;
    let reset_epoch_secs = data
        .pointer("/rateLimit/resetAt")
        .and_then(Value::as_str)
        .and_then(rfc3339_utc_to_epoch_secs);
    Some(RateLimit {
        remaining,
        reset_epoch_secs,
    })
}

/// Asks GitHub which of the given `owner/repo#number` PRs are merged, in one
/// aliased query. Keys absent from the result were closed without merging,
/// deleted, or are no longer accessible with this token.
pub async fn fetch_merged_status(
    token: &str,
    keys: &[String],
) -> Result<HashMap<String, String>, GithubError> {
    let mut query = String::from("query { ");
    // Keys come from PRs the app itself fetched, so skipping a malformed one
    // drops corrupt data, not a real PR.
    let mut targets: Vec<(String, String)> = Vec::new();
    for key in keys {
        let Some((owner, name, number)) = parse_pr_key(key) else {
            continue;
        };
        let alias = format!("p{}", targets.len());
        query.push_str(&merged_status_field(&alias, owner, name, number));
        targets.push((alias, key.clone()));
    }
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    query.push('}');

    let response = graphql(token, &query, json!({})).await?;
    let Some(data) = response.data else {
        return Err(error_from_graphql(response.errors));
    };
    Ok(collect_merged(&data, &targets))
}

fn parse_pr_key(key: &str) -> Option<(&str, &str, u64)> {
    let (repo, number) = key.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    Some((owner, name, number.parse().ok()?))
}

fn merged_status_field(alias: &str, owner: &str, name: &str, number: u64) -> String {
    format!(
        "{alias}: repository(owner: {owner}, name: {name}) {{ \
           pullRequest(number: {number}) {{ merged mergedAt }} \
         }} ",
        owner = json!(owner),
        name = json!(name),
    )
}

/// Reads the merged aliases back out of the response. A vanished repository
/// or PR and an unmerged PR both mean "not merged" and are left out.
fn collect_merged(data: &Value, targets: &[(String, String)]) -> HashMap<String, String> {
    let mut merged = HashMap::new();
    for (alias, key) in targets {
        let Some(pr) = data
            .pointer(&format!("/{alias}/pullRequest"))
            .filter(|v| !v.is_null())
        else {
            continue;
        };
        if pr.get("merged").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        if let Some(t) = pr.get("mergedAt").and_then(Value::as_str) {
            merged.insert(key.clone(), t.to_string());
        }
    }
    merged
}

fn repo_field(alias: &str, owner: &str, name: &str, after: Option<&str>) -> String {
    let after = after
        .map(|c| format!(", after: {}", json!(c)))
        .unwrap_or_default();
    format!(
        "{alias}: repository(owner: {owner}, name: {name}) {{ \
           nameWithOwner \
           owner {{ avatarUrl }} \
           pullRequests(states: OPEN, first: {PAGE_SIZE}{after}, \
                        orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ \
             pageInfo {{ hasNextPage endCursor }} \
             nodes {{ \
               number title url createdAt updatedAt viewerDidAuthor \
               isDraft mergeable reviewDecision \
               author {{ login avatarUrl }} \
               commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} \
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
    // Read once per repo: the owner is a repository-level field, shared by
    // every PR in the repo.
    let owner_avatar_url = repo
        .pointer("/owner/avatarUrl")
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
            // A null author means a deleted account, which GitHub calls
            // "ghost".
            author: node
                .pointer("/author/login")
                .and_then(Value::as_str)
                .unwrap_or("ghost")
                .to_string(),
            avatar_url: node
                .pointer("/author/avatarUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            owner_avatar_url: owner_avatar_url.to_string(),
            created_at: node
                .get("createdAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            updated_at: node
                .get("updatedAt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            section: section_for(node, viewer),
            blocked_reasons: blocked_reasons_for(node),
            is_draft: node.get("isDraft").and_then(Value::as_bool) == Some(true),
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

/// Gathers the timestamps of what counts as an "update" for unread purposes:
/// issue comments, submitted reviews, and review comments — never commits or
/// CI. The query does fetch the head commit's CI rollup for the row indicator,
/// but that is a property of the PR, not an unread event, so it is read
/// elsewhere and never enters this list. The viewer's own activity is excluded
/// (you have read what you wrote), as are PENDING reviews (invisible to
/// everyone but their author until submitted).
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

/// Composes the row's blocked indicator from mergeability, the head commit's
/// CI rollup, and the review decision, in the fixed tooltip order. Only
/// negative states light it: a mergeable, in-flight, or quiet PR yields an
/// empty list so an undecorated row keeps meaning "nothing is stuck".
///
/// Suppressed entirely for drafts (the author has not declared readiness) and
/// while `mergeable` is `UNKNOWN`: GitHub computes mergeability lazily and
/// reports `UNKNOWN` until it settles, which must read as "no flag yet", not
/// as a blocked state; it resolves on the next poll.
fn blocked_reasons_for(node: &Value) -> Vec<BlockedReason> {
    let field = |name: &str| node.get(name).and_then(Value::as_str);
    if node.get("isDraft").and_then(Value::as_bool) == Some(true)
        || field("mergeable") == Some("UNKNOWN")
    {
        return Vec::new();
    }
    let ci = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(Value::as_str);
    let mut reasons = Vec::new();
    if field("mergeable") == Some("CONFLICTING") {
        reasons.push(BlockedReason::Conflict);
    }
    if matches!(ci, Some("FAILURE") | Some("ERROR")) {
        reasons.push(BlockedReason::Ci);
    }
    if field("reviewDecision") == Some("CHANGES_REQUESTED") {
        reasons.push(BlockedReason::Review);
    }
    reasons
}

/// Mine beats Participated: the author is always a participant, so order
/// matters. Participants covers reviewers, commenters, and mentions;
/// reviewRequests covers requests not yet acted on, which GitHub does not
/// count as participation.
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
            "updatedAt": "2026-07-11T09:30:00Z",
            "viewerDidAuthor": false,
            "author": { "login": "someone", "avatarUrl": "https://avatars.example/someone" },
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
        assert_eq!(out[0].avatar_url, "https://avatars.example/someone");
        assert_eq!(out[0].created_at, "2026-07-10T12:00:00Z");
        assert_eq!(out[0].updated_at, "2026-07-11T09:30:00Z");
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
        assert_eq!(out[0].avatar_url, "");
    }

    #[test]
    fn the_repo_owner_avatar_is_copied_onto_every_pr() {
        // The owner avatar is a repository-level field read once and shared by
        // every PR in the repo; it becomes the org badge on each row.
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "owner": { "avatarUrl": "https://avatars.example/acme" },
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [pr_node(json!({ "number": 7 })), pr_node(json!({ "number": 8 }))]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(out[0].owner_avatar_url, "https://avatars.example/acme");
        assert_eq!(out[1].owner_avatar_url, "https://avatars.example/acme");
    }

    fn ci_node(state: &str) -> Value {
        json!({ "commits": { "nodes": [{ "commit": {
            "statusCheckRollup": { "state": state }
        } }] } })
    }

    #[test]
    fn each_blocked_trigger_lights_the_dot_on_its_own() {
        let reasons = |overrides: Value| blocked_reasons_for(&pr_node(overrides));
        assert_eq!(
            reasons(json!({ "mergeable": "CONFLICTING" })),
            vec![BlockedReason::Conflict]
        );
        assert_eq!(reasons(ci_node("FAILURE")), vec![BlockedReason::Ci]);
        assert_eq!(reasons(ci_node("ERROR")), vec![BlockedReason::Ci]);
        assert_eq!(
            reasons(json!({ "reviewDecision": "CHANGES_REQUESTED" })),
            vec![BlockedReason::Review]
        );
    }

    #[test]
    fn mergeable_in_flight_and_quiet_prs_stay_undecorated() {
        let reasons = |overrides: Value| blocked_reasons_for(&pr_node(overrides));
        // The pr_node fixture has no CI, mergeability, or review fields at all.
        assert_eq!(reasons(json!({})), Vec::<BlockedReason>::new());
        assert_eq!(
            reasons(json!({ "mergeable": "MERGEABLE", "reviewDecision": "APPROVED" })),
            Vec::new()
        );
        // In-flight states (CI pending, awaiting review) are progress, not
        // blockage.
        assert_eq!(reasons(ci_node("PENDING")), Vec::new());
        assert_eq!(reasons(ci_node("EXPECTED")), Vec::new());
        assert_eq!(reasons(ci_node("SUCCESS")), Vec::new());
        assert_eq!(
            reasons(json!({ "reviewDecision": "REVIEW_REQUIRED" })),
            Vec::new()
        );
    }

    /// Fixed tooltip order regardless of which fields say what: conflict,
    /// then CI, then review.
    #[test]
    fn concurrent_triggers_list_every_reason_in_fixed_order() {
        let mut node = pr_node(json!({
            "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED"
        }));
        node.as_object_mut()
            .unwrap()
            .extend(ci_node("FAILURE").as_object().unwrap().clone());
        assert_eq!(
            blocked_reasons_for(&node),
            vec![
                BlockedReason::Conflict,
                BlockedReason::Ci,
                BlockedReason::Review
            ]
        );
    }

    /// The author has not declared readiness, so "needs action now" does not
    /// apply — whatever the CI, conflict, or review state says.
    #[test]
    fn drafts_suppress_the_dot_entirely() {
        let mut node = pr_node(json!({
            "isDraft": true,
            "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED"
        }));
        node.as_object_mut()
            .unwrap()
            .extend(ci_node("FAILURE").as_object().unwrap().clone());
        assert_eq!(blocked_reasons_for(&node), Vec::new());
    }

    /// GitHub computes `mergeable` lazily and answers `UNKNOWN` until it
    /// settles; that transient must render as no dot, not as a blocked state.
    #[test]
    fn an_unknown_mergeability_suppresses_the_dot_until_the_next_poll() {
        let mut node = pr_node(json!({
            "mergeable": "UNKNOWN",
            "reviewDecision": "CHANGES_REQUESTED"
        }));
        node.as_object_mut()
            .unwrap()
            .extend(ci_node("FAILURE").as_object().unwrap().clone());
        assert_eq!(blocked_reasons_for(&node), Vec::new());
    }

    /// `PrList.tsx` maps these exact strings to tooltip labels, so every
    /// variant's wire form is a contract; a rename or a dropped `rename_all`
    /// breaks the row silently.
    #[test]
    fn every_blocked_reason_serializes_to_its_frontend_key() {
        for (reason, expected) in [
            (BlockedReason::Conflict, "conflict"),
            (BlockedReason::Ci, "ci"),
            (BlockedReason::Review, "review"),
        ] {
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(expected));
        }
    }

    #[test]
    fn blocked_state_and_draftness_are_read_onto_the_row() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [
                    pr_node(ci_node("FAILURE")),
                    pr_node(json!({ "isDraft": true }))
                ]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(out[0].blocked_reasons, vec![BlockedReason::Ci]);
        assert!(!out[0].is_draft);
        assert_eq!(out[1].blocked_reasons, Vec::new());
        assert!(out[1].is_draft);
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
        // CI, mergeability, review decision, and draftness are all fetched
        // (for the row's blocked dot and draft pill) but deliberately excluded
        // from activity: they are properties of the PR, not unread events. A
        // node with neither a comment nor a review yields nothing, whatever
        // those fields say.
        let mut node = pr_node(json!({
            "isDraft": false,
            "mergeable": "CONFLICTING",
            "reviewDecision": "CHANGES_REQUESTED"
        }));
        node.as_object_mut()
            .unwrap()
            .extend(ci_node("FAILURE").as_object().unwrap().clone());
        assert!(collect_activity(&node, "khiet").is_empty());
    }

    #[test]
    fn repo_field_quotes_arguments_and_resumes_from_a_cursor() {
        let field = repo_field("r0", "acme", "widgets", Some("abc"));
        assert!(field.contains(r#"repository(owner: "acme", name: "widgets")"#));
        assert!(field.contains(r#"after: "abc""#));
        assert!(!repo_field("r0", "acme", "widgets", None).contains("after:"));
    }

    /// Everything the blocked indicator needs rides the existing PR query
    /// rather than a new request. The CI rollup must be read off the head
    /// commit (`commits(last: 1)`) since the rollup is per-commit.
    #[test]
    fn repo_field_asks_for_the_blocked_indicator_fields() {
        let field = repo_field("r0", "acme", "widgets", None);
        assert!(field.contains("commits(last: 1)"));
        assert!(field.contains("statusCheckRollup { state }"));
        assert!(field.contains("isDraft"));
        assert!(field.contains("mergeable"));
        assert!(field.contains("reviewDecision"));
    }

    #[test]
    fn merged_status_field_quotes_arguments() {
        let field = merged_status_field("p0", "acme", "widgets", 7);
        assert!(field.contains(r#"p0: repository(owner: "acme", name: "widgets")"#));
        assert!(field.contains("pullRequest(number: 7)"));
    }

    #[test]
    fn pr_keys_parse_into_query_arguments() {
        assert_eq!(parse_pr_key("acme/widgets#7"), Some(("acme", "widgets", 7)));
        assert_eq!(parse_pr_key("no-separator"), None);
        assert_eq!(parse_pr_key("acme#7"), None);
        assert_eq!(parse_pr_key("acme/widgets#seven"), None);
    }

    #[test]
    fn collect_merged_keeps_only_merged_prs() {
        let data = json!({
            "p0": { "pullRequest": { "merged": true, "mergedAt": "2026-07-12T10:00:00Z" } },
            "p1": { "pullRequest": { "merged": false, "mergedAt": null } }
        });
        let targets = [
            ("p0".to_string(), "acme/widgets#7".to_string()),
            ("p1".to_string(), "acme/widgets#8".to_string()),
        ];
        let merged = collect_merged(&data, &targets);
        assert_eq!(
            merged.get("acme/widgets#7").map(String::as_str),
            Some("2026-07-12T10:00:00Z")
        );
        assert!(!merged.contains_key("acme/widgets#8"));
    }

    #[test]
    fn iso_timestamps_parse_to_epoch_seconds() {
        assert_eq!(rfc3339_utc_to_epoch_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_utc_to_epoch_secs("2026-07-16T12:34:56Z"),
            Some(1_784_205_296)
        );
        assert_eq!(
            rfc3339_utc_to_epoch_secs("2024-02-29T00:00:00Z"),
            Some(1_709_164_800)
        );
    }

    #[test]
    fn malformed_timestamps_parse_to_none() {
        for input in [
            "",
            "2026-07-16",
            "2026-07-16 12:34:56Z",
            "2026-07-16T12:34:56",
            "2026-07-16T12:34:56.000Z",
            "2026-13-01T00:00:00Z",
            "2026-00-10T00:00:00Z",
            "2026-01-32T00:00:00Z",
            "2026-01-01T24:00:00Z",
            "yyyy-mm-ddThh:mm:ssZ",
        ] {
            assert_eq!(rfc3339_utc_to_epoch_secs(input), None, "input: {input:?}");
        }
    }

    #[test]
    fn a_rate_limited_graphql_error_is_typed_as_rate_limited() {
        let errors = vec![
            GraphQlError {
                message: "something else".into(),
                kind: None,
            },
            GraphQlError {
                message: "API rate limit exceeded".into(),
                kind: Some("RATE_LIMITED".into()),
            },
        ];
        assert_eq!(
            error_from_graphql(errors),
            GithubError::RateLimited {
                reset_epoch_secs: None
            }
        );
    }

    #[test]
    fn other_graphql_errors_surface_the_first_message() {
        let errors = vec![GraphQlError {
            message: "Something exploded".into(),
            kind: Some("INTERNAL".into()),
        }];
        assert_eq!(
            error_from_graphql(errors),
            GithubError::Other("GitHub error: Something exploded".into())
        );
        assert_eq!(
            error_from_graphql(Vec::new()),
            GithubError::Other("GitHub returned an unexpected response.".into())
        );
    }

    #[test]
    fn a_classic_token_with_repo_gets_no_scope_warning() {
        // The live header format is comma-space separated.
        assert_eq!(scope_warning(Some("read:user, repo")), None);
        assert_eq!(scope_warning(Some("repo")), None);
    }

    /// Scopes are matched exactly after splitting on commas: a substring test
    /// for `repo` would pass exactly the token the warning exists for.
    #[test]
    fn public_repo_does_not_count_as_repo() {
        assert!(scope_warning(Some("public_repo")).is_some());
        assert!(scope_warning(Some("public_repo, read:user")).is_some());
    }

    /// A classic token minted with no scopes ticked.
    #[test]
    fn an_empty_scopes_header_warns() {
        assert!(scope_warning(Some("")).is_some());
    }

    /// A fine-grained or App token, which may read private repos just fine.
    #[test]
    fn an_absent_scopes_header_does_not_warn() {
        assert_eq!(scope_warning(None), None);
    }

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                value.parse().unwrap(),
            );
        }
        map
    }

    /// `retry-after` is GitHub's instruction for this particular response, so
    /// it outranks the standing `x-ratelimit-reset`.
    #[test]
    fn retry_after_is_read_as_seconds_from_now_and_outranks_the_reset_header() {
        let assert_roughly_a_minute_out = |pairs: &[(&str, &str)]| {
            let before = now_epoch_secs();
            let reset = rate_limit_reset(&headers(pairs)).unwrap();
            assert!(
                (before + 60..=now_epoch_secs() + 60).contains(&reset),
                "expected roughly 60s from now, got {reset} for {pairs:?}"
            );
        };
        assert_roughly_a_minute_out(&[("retry-after", "60")]);
        assert_roughly_a_minute_out(&[("retry-after", "60"), ("x-ratelimit-reset", "1784205296")]);
    }

    #[test]
    fn the_reset_header_is_read_as_an_absolute_epoch() {
        assert_eq!(
            rate_limit_reset(&headers(&[("x-ratelimit-reset", "1784205296")])),
            Some(1_784_205_296)
        );
    }

    /// No reset time means the caller falls back to its own backoff.
    #[test]
    fn missing_or_unparsable_reset_headers_report_no_reset_time() {
        assert_eq!(rate_limit_reset(&headers(&[])), None);
        assert_eq!(
            rate_limit_reset(&headers(&[("x-ratelimit-reset", "soon")])),
            None
        );
    }

    /// The popover shows this verbatim, so it must not leak the reset epoch.
    #[test]
    fn the_rate_limited_error_renders_a_user_facing_message() {
        let error = GithubError::RateLimited {
            reset_epoch_secs: Some(1_784_205_296),
        };
        assert_eq!(
            error.to_string(),
            "GitHub rate limit reached. Waiting for it to reset."
        );
    }

    #[test]
    fn the_rate_limit_budget_is_read_from_the_response() {
        let data = json!({
            "rateLimit": { "remaining": 4321, "resetAt": "1970-01-01T01:00:00Z" }
        });
        assert_eq!(
            collect_rate_limit(&data),
            Some(RateLimit {
                remaining: 4321,
                reset_epoch_secs: Some(3600),
            })
        );
        assert_eq!(collect_rate_limit(&json!({})), None);
    }

    #[test]
    fn a_missing_or_malformed_reset_time_still_reports_the_budget() {
        let data = json!({ "rateLimit": { "remaining": 12, "resetAt": "not a time" } });
        assert_eq!(
            collect_rate_limit(&data),
            Some(RateLimit {
                remaining: 12,
                reset_epoch_secs: None,
            })
        );
    }

    #[test]
    fn vanished_repos_and_prs_do_not_count_as_merged() {
        let data = json!({
            "p0": null,
            "p1": { "pullRequest": null }
        });
        let targets = [
            ("p0".to_string(), "acme/widgets#7".to_string()),
            ("p1".to_string(), "acme/widgets#8".to_string()),
        ];
        assert!(collect_merged(&data, &targets).is_empty());
    }
}
