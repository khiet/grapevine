use std::collections::{HashMap, HashSet};

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
/// GitHub caps a PR at ten assignees, so one page always holds them all.
const ASSIGNEE_PAGE: usize = 10;
/// Search hits per page; 100 is GitHub's maximum, and a viewer with more
/// team requests than that across the watched repos is paged.
const TEAM_SEARCH_PAGE: usize = 100;
/// GitHub search serves at most 1000 results for one query.
const TEAM_SEARCH_MAX_PAGES: usize = 10;

/// The login of Copilot's cloud agent, as GitHub lists it among a repo's
/// assignable actors when the agent is enabled there.
const COPILOT_LOGIN: &str = "copilot-swe-agent";
/// Room for other accounts the actor filter returns: it is a search on
/// [`COPILOT_LOGIN`], not an exact match, and the bot is picked out after.
const COPILOT_ACTOR_PAGE: usize = 5;
/// The whole comment that asks Copilot for the fix. Fixed here so nothing
/// outside this module can choose what Grapevine writes to a PR.
const CONFLICT_FIX_COMMENT: &str = "@copilot resolve the merge conflicts on this PR.";

/// Which popover section a PR belongs to, from the viewer's perspective.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Section {
    Mine,
    Participated,
    All,
}

/// One reason a PR is blocked, for the row's reason pills. Declaration order
/// is the fixed severity order (conflict, CI, review, threads, behind); the
/// frontend shows the first as the primary pill and renders the list
/// verbatim, never re-deriving or re-sorting it. The order tracks how much
/// work clearing each takes: a conflict needs a manual rebase, CI an
/// investigation, review new code, threads an answer, and behind often just
/// one update-branch click.
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BlockedReason {
    Conflict,
    Ci,
    Review,
    Threads,
    Behind,
}

/// Whose move an open PR waits on, from the viewer's side: `Yours` when the
/// next step is the viewer's, `Theirs` when it is a reviewer's, a check's or
/// GitHub's own. A PR with no verdict carries `None` (see [`turn_for`]).
///
/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Turn {
    Yours,
    Theirs,
}

/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Debug, Clone, Serialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub repo: String,
    pub author: String,
    /// Whether the author is a machine account (see [`is_bot`]). Renders as
    /// the "bot" tag on the avatar.
    pub author_is_bot: bool,
    /// Author's GitHub avatar URL; empty when the account was deleted.
    pub avatar_url: String,
    /// Repo owner's (organization or user) avatar URL, used as the org badge
    /// on the row. Empty when GitHub returns no owner; the UI shows no badge.
    pub owner_avatar_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub section: Section,
    /// Why the PR is blocked, in severity order; empty means no pills. Composed
    /// here rather than shipping GitHub's raw enums so the frontend renders
    /// the reason pills instead of re-deriving the logic. A property of
    /// the PR, never an unread event: it does not feed [`collect_activity`]
    /// and must never move a badge or the tray count.
    pub blocked_reasons: Vec<BlockedReason>,
    /// Whether the PR is a draft. Renders as a neutral grey mark, and
    /// suppresses the blocked pills (see [`blocked_reasons_for`]). Same
    /// property-not-event rule as `blocked_reasons`.
    pub is_draft: bool,
    /// Whether the viewer's review is directly requested and not yet acted
    /// on. Renders as a neutral grey mark: a call to act, since we may be
    /// blocking the author, but not a "stuck" state. GitHub drops the viewer
    /// from `reviewRequests` once they submit a review, so this clears itself
    /// and the row falls back to plain participation. Suppressed on drafts,
    /// like the blocked pills, so a not-ready PR never nags for a review; the
    /// request still counts toward Participated membership (see
    /// [`section_for`]). Same property-not-event rule as `blocked_reasons`;
    /// team requests never set it (see [`review_requested_for`]); they set
    /// `team_review_requested` instead.
    pub review_requested: bool,
    /// Whether a review is outstanding from a team the viewer belongs to,
    /// with no request naming the viewer. Kept apart from `review_requested`
    /// because the obligation differs: a request by name is the viewer's, a
    /// team request is the team's. It shares the row's one review slot, so it
    /// is never set beside `review_requested` or `awaiting_review`, and never
    /// on a PR the viewer authored. GitHub drops the team from the pending
    /// list once a member reviews, so this clears itself. Suppressed on
    /// drafts like the other review marks. It never changes the section (see
    /// [`section_for`]) and, by the property-not-event rule of
    /// `blocked_reasons`, never reaches a badge or the tray count. Out of
    /// [`collect_repo_prs`] it means only "could be one" (see
    /// [`could_be_team_request`]); [`Client::fetch_open_prs`] keeps it for
    /// the PRs the team-request search confirms.
    pub team_review_requested: bool,
    /// Whether one of the viewer's own PRs is waiting on a reviewer: it has at
    /// least one outstanding requested reviewer. The outgoing mirror of
    /// `review_requested` (see [`has_pending_review_request`]), set only on
    /// `Mine` PRs and never beside `review_requested`: an agent's PR in Mine
    /// can ask the viewer for review, and then the incoming mark wins. Renders
    /// as the glasses
    /// glyph with an outgoing arrow, a neutral property ("the ball is in a
    /// reviewer's court"), not a "stuck" state. GitHub drops a reviewer from
    /// `reviewRequests` once they submit, so this clears itself as reviews
    /// arrive. Suppressed on drafts, matching `review_requested` and the
    /// blocked pills. Unlike `review_requested`, team requests count: it is an
    /// existence check, with no `login` to match. Same property-not-event rule
    /// as `blocked_reasons`.
    pub awaiting_review: bool,
    /// Whether the review decision is an explicit APPROVED. Renders as the
    /// green check in the row's marker cluster: the one positive mark, so
    /// green means "the required approvals are in", not merely "nothing is
    /// stuck". A null reviewDecision stays false (repos without required reviews never
    /// show the check). Shown even beside a review glyph: a later re-request
    /// does not unmeet the requirement. Suppressed on drafts like the other
    /// markers. Same property-not-event rule as `blocked_reasons`.
    pub approved: bool,
    /// The PR's 1-based position in its repo's merge queue, as GitHub reports
    /// it; `None` when it is not queued. Renders as the stack mark with the
    /// number beside it. Not gated on anything else on the row: blocked
    /// reasons are computed as usual and may sit beside it. Same
    /// property-not-event rule as `blocked_reasons`, so a position change
    /// never counts as unread.
    pub merge_queue_position: Option<u64>,
    /// Names of the failed checks behind the `Ci` reason, in GitHub's order;
    /// the CI pill names the first. Only the checks that gate the merge are
    /// listed (see [`checks_for`]). Empty whenever `Ci` is not raised, and
    /// also when it is raised from the rollup alone, with no check to name.
    pub failing_checks: Vec<String>,
    /// Whether the PR has more checks than the one page read, so
    /// `failing_checks` may be short of the full list. The pill then says
    /// "+ more" and never a count.
    pub failing_checks_truncated: bool,
    /// Whether a check that gates the merge is still running and none has
    /// failed. Renders as the neutral Running mark. Suppressed on drafts.
    /// Same property-not-event rule as `blocked_reasons`.
    pub checks_running: bool,
    /// Whether auto-merge is armed: GitHub merges the PR itself once its
    /// requirements are met. Renders as the Auto mark. Suppressed on drafts.
    /// Same property-not-event rule as `blocked_reasons`.
    pub auto_merge: bool,
    /// Whether nothing stands between the PR and its merge but the viewer
    /// pressing the button (see [`ready_to_merge`]). Only ever set in Mine.
    /// Renders as the Ready to merge pill. Same property-not-event rule as
    /// `blocked_reasons`.
    pub ready_to_merge: bool,
    /// Whose move the PR waits on (see [`turn_for`]); `None` when there is no
    /// verdict, always so on a draft and in All. A your-turn row leads its
    /// section, tints its mark and is what the tray counts. Same
    /// property-not-event rule as `blocked_reasons`: it never reaches an
    /// unread badge.
    pub turn: Option<Turn>,
    /// Epoch ms of the sync that first saw the turn become yours; filled in
    /// by the turn tracker after fetch, always `None` out of this module.
    /// `None` also when the turn is not yours or its start is unknown (see
    /// [`crate::turns::TurnStarts`]). Deliberately not derived from
    /// `updated_at`, which any unrelated comment moves.
    pub turn_since: Option<u64>,
    /// Whether the row may offer to ask Copilot to resolve the PR's merge
    /// conflict (see [`can_fix_conflict`]). The one field that gates a write
    /// to GitHub: [`Client::request_conflict_fix`] refuses a PR without it.
    /// Same property-not-event rule as `blocked_reasons`.
    pub can_fix_conflict: bool,
    /// GitHub's global node id, which the comment mutation addresses the PR
    /// by. Stays on this side of the wire: the frontend names a PR by its key
    /// and never supplies what gets written to.
    #[serde(skip)]
    pub node_id: String,
    /// Human activity newer than the PR's last-read watermark; filled in by
    /// the unread engine after fetch, always 0 out of this module.
    pub unread_count: u64,
    /// Bot activity newer than the same watermark, filled in the same way.
    /// Renders as the row's muted note and never reaches a badge or the tray
    /// count. It covers the fetched window only, so it is not a lifetime
    /// total.
    pub bot_unread_count: u64,
    /// Recent comment/review timestamps (ISO-8601 UTC, ascending) by people
    /// other than the viewer. Input to the unread computation.
    #[serde(skip)]
    pub activity: Vec<String>,
    /// The same, by bots. Kept apart so it can be counted without badging.
    #[serde(skip)]
    pub bot_activity: Vec<String>,
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

/// What GitHub answered, before any interpretation.
struct HttpResponse {
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: String,
}

/// The seam between this module and the wire: a GraphQL payload goes in, the
/// raw answer comes out. Implementations do no interpretation; mapping a
/// status or body to a [`GithubError`] belongs to [`Client`], so every adapter
/// gets the same rules. `Err` is reserved for "no answer at all".
trait Transport {
    async fn post(&self, token: &str, payload: &Value) -> Result<HttpResponse, GithubError>;
}

/// The production adapter: GitHub's GraphQL endpoint over `reqwest`.
struct ReqwestTransport;

impl Transport for ReqwestTransport {
    async fn post(&self, token: &str, payload: &Value) -> Result<HttpResponse, GithubError> {
        let client = reqwest::Client::builder()
            .user_agent("grapevine")
            .build()
            .map_err(|e| GithubError::Other(format!("cannot build HTTP client: {e}")))?;
        let response = client
            .post(GRAPHQL_URL)
            .bearer_auth(token)
            .json(payload)
            .send()
            .await
            .map_err(|_| {
                GithubError::Other("Could not reach GitHub. Check your network connection.".into())
            })?;
        Ok(HttpResponse {
            status: response.status(),
            headers: response.headers().clone(),
            // A body that fails mid-read must not hide the status and headers
            // already received: a refusal is classified from those alone, and
            // an empty body on a success reads as an unexpected response.
            body: response.text().await.unwrap_or_default(),
        })
    }
}

/// Everything this module asks of GitHub, over a [`Transport`]. The public
/// functions below run it against [`ReqwestTransport`]; tests run it against
/// canned responses.
struct Client<'a, T> {
    transport: &'a T,
    token: &'a str,
    /// Read only after a response arrives, so a relative `retry-after` is
    /// counted from then and not from before the request.
    now_epoch_secs: fn() -> u64,
}

impl<'a> Client<'a, ReqwestTransport> {
    fn live(token: &'a str) -> Self {
        Client {
            transport: &ReqwestTransport,
            token,
            now_epoch_secs,
        }
    }
}

impl<T: Transport> Client<'_, T> {
    async fn graphql(&self, query: &str, variables: Value) -> Result<GraphQlResponse, GithubError> {
        Ok(self.graphql_with_scopes(query, variables).await?.0)
    }

    /// Like [`Client::graphql`], but also reports the raw `X-OAuth-Scopes`
    /// header: `None` when GitHub sent none (fine-grained PATs and App tokens
    /// have permissions, not scopes), `Some` (possibly empty) for classic
    /// tokens.
    async fn graphql_with_scopes(
        &self,
        query: &str,
        variables: Value,
    ) -> Result<(GraphQlResponse, Option<String>), GithubError> {
        let response = self
            .transport
            .post(
                self.token,
                &json!({ "query": query, "variables": variables }),
            )
            .await?;

        if response.status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubError::Other(
                "GitHub rejected the token. Check that it is valid and not expired.".into(),
            ));
        }
        // Both are read as quota refusals: the primary limit answers 403, the
        // secondary limit 429.
        if response.status == reqwest::StatusCode::FORBIDDEN
            || response.status == reqwest::StatusCode::TOO_MANY_REQUESTS
        {
            return Err(GithubError::RateLimited {
                reset_epoch_secs: rate_limit_reset(&response.headers, (self.now_epoch_secs)()),
            });
        }
        if !response.status.is_success() {
            return Err(GithubError::Other(format!(
                "GitHub returned an error (HTTP {}).",
                response.status.as_u16()
            )));
        }
        let scopes = response
            .headers
            .get("x-oauth-scopes")
            .map(|v| v.to_str().unwrap_or_default().to_string());
        let parsed = serde_json::from_str::<GraphQlResponse>(&response.body)
            .map_err(|_| GithubError::Other("GitHub returned an unexpected response.".into()))?;
        Ok((parsed, scopes))
    }

    async fn validate_token(&self) -> Result<ValidatedToken, String> {
        let (response, scopes) = self
            .graphql_with_scopes("query { viewer { login } }", json!({}))
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

    async fn validate_repo(&self, owner: &str, name: &str) -> Result<String, String> {
        let response = self
            .graphql(
                "query($owner: String!, $name: String!) { \
                   repository(owner: $owner, name: $name) { nameWithOwner } \
                 }",
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

    async fn fetch_affiliated_repos(&self) -> Result<Vec<String>, GithubError> {
        let mut repos = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..10 {
            let response = self
                .graphql(AFFILIATED_REPOS_QUERY, json!({ "cursor": cursor }))
                .await?;
            let Some(data) = response.data else {
                return Err(error_from_graphql(response.errors));
            };
            cursor = collect_affiliated_page(&data, &mut repos);
            if cursor.is_none() {
                break;
            }
        }
        Ok(repos)
    }

    async fn fetch_open_prs(&self, repos: &[String]) -> Result<FetchedPrs, GithubError> {
        // (owner, name, resume cursor); repos drop out once fully fetched.
        let mut pending: Vec<(String, String, Option<String>)> = repos
            .iter()
            .filter_map(|full| full.split_once('/'))
            .map(|(owner, name)| (owner.to_string(), name.to_string(), None))
            .collect();
        let mut prs = Vec::new();
        let mut rate_limit = None;
        // The team-request search rides along with the repo pages. Outer
        // `Some` while it has a page left to ask for; the inner value is the
        // cursor to resume from. Nothing watched means nothing to search.
        let mut team_search: Option<Option<String>> = (!pending.is_empty()).then_some(None);
        let mut team_search_pages = 0;
        let mut team_requested: HashSet<String> = HashSet::new();
        // Whether the repo pages still ask if Copilot can be assigned.
        let mut ask_actors = true;
        let searched_repos: Vec<String> = pending
            .iter()
            .map(|(owner, name, _)| format!("{owner}/{name}"))
            .collect();

        let mut rounds = 0;
        while !pending.is_empty() || team_search.is_some() {
            rounds += 1;
            if rounds > 20 {
                return Err(GithubError::Other(
                    "GitHub returned too many pages of pull requests.".into(),
                ));
            }
            let mut query =
                String::from("query { viewer { login } rateLimit { remaining resetAt } ");
            for (i, (owner, name, cursor)) in pending.iter().enumerate() {
                query.push_str(&repo_field(
                    &format!("r{i}"),
                    owner,
                    name,
                    cursor.as_deref(),
                    ask_actors,
                ));
            }
            if let Some(cursor) = &team_search {
                query.push_str(&team_search_field(&searched_repos, cursor.as_deref()));
            }
            query.push('}');

            let response = self.graphql(&query, json!({})).await?;
            let Some(mut data) = response.data else {
                let error = error_from_graphql(response.errors);
                // GitHub refuses a whole query over one field it will not
                // serve. The round is asked again without the extras, one at
                // a time, so a token that cannot search or cannot list a
                // repo's actors still syncs: first the search, then the
                // Copilot actors. A quota refusal is not about either, and
                // asking again would only spend more of it.
                if matches!(error, GithubError::RateLimited { .. }) {
                    return Err(error);
                }
                if team_search.is_some() {
                    team_search = None;
                    team_requested.clear();
                    continue;
                }
                if ask_actors {
                    ask_actors = false;
                    continue;
                }
                return Err(error);
            };
            if team_search.is_some() {
                team_search_pages += 1;
                // Stops at what GitHub search will page through at all, so
                // the search can never run the fetch into the round limit.
                team_search = collect_team_requested(&data, &mut team_requested)
                    .filter(|_| team_search_pages < TEAM_SEARCH_MAX_PAGES)
                    .map(Some);
            }
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
                let Some(repo) = data.get_mut(alias.as_str()).filter(|v| !v.is_null()) else {
                    continue;
                };
                self.add_check_detail(repo, owner, name, &mut rate_limit)
                    .await?;
                if let Some(cursor) = collect_repo_prs(repo, &viewer, &mut prs) {
                    still_pending.push((owner.clone(), name.clone(), Some(cursor)));
                }
            }
            pending = still_pending;
        }

        for pr in &mut prs {
            pr.team_review_requested =
                pr.team_review_requested && team_requested.contains(&pr_key(&pr.repo, pr.number));
            // One glyph per row: an agent's PR in Mine that asks the viewer's
            // team shows the incoming mark, as it does when it asks the
            // viewer by name.
            if pr.team_review_requested {
                pr.awaiting_review = false;
            }
        }

        // Most recently updated first across all repos; ISO-8601 UTC timestamps
        // sort lexically.
        prs.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(FetchedPrs { prs, rate_limit })
    }

    /// Completes one repo's page of PR nodes in place: each PR whose rollup
    /// leaves a question (see [`needs_check_detail`]) gets its head commit
    /// replaced by one that also lists the individual checks, so everything
    /// reading the node afterwards sees them as if the page had carried them.
    /// One query per such PR; this is the sync's only cost that grows with
    /// how many PRs are red or running.
    ///
    /// The whole `commits` field is replaced, rollup state included, so the
    /// state and the checks always describe the same commit even when a push
    /// landed between the two queries.
    ///
    /// A query GitHub declines leaves the node as the page gave it, and the
    /// rollup state answers alone (see [`checks_for`]). Running out of quota
    /// is the exception and fails the fetch like any other request would.
    async fn add_check_detail(
        &self,
        repo: &mut Value,
        owner: &str,
        name: &str,
        rate_limit: &mut Option<RateLimit>,
    ) -> Result<(), GithubError> {
        let nodes = repo
            .pointer_mut("/pullRequests/nodes")
            .and_then(Value::as_array_mut);
        for node in nodes.into_iter().flatten() {
            if !needs_check_detail(node) {
                continue;
            }
            let Some(number) = node.get("number").and_then(Value::as_u64) else {
                continue;
            };
            let response = self
                .graphql(
                    CHECK_DETAIL_QUERY,
                    json!({ "owner": owner, "name": name, "number": number }),
                )
                .await?;
            let Some(mut detail) = response.data else {
                let error = error_from_graphql(response.errors);
                if matches!(error, GithubError::RateLimited { .. }) {
                    return Err(error);
                }
                continue;
            };
            if let Some(limit) = collect_rate_limit(&detail) {
                *rate_limit = Some(limit);
            }
            if let Some(commits) = detail
                .pointer_mut("/repository/pullRequest/commits")
                .filter(|c| !c.is_null())
            {
                node["commits"] = commits.take();
            }
        }
        Ok(())
    }

    /// Asks Copilot to resolve `pr`'s merge conflict by commenting on it as
    /// the viewer. The only write this module makes. It posts nothing for a
    /// PR the eligibility rule turns down, so no caller can widen what
    /// Grapevine writes to.
    async fn request_conflict_fix(&self, pr: &PullRequest) -> Result<(), GithubError> {
        if !pr.can_fix_conflict || pr.node_id.is_empty() {
            return Err(GithubError::Other(
                "Nothing here for Copilot to fix.".into(),
            ));
        }
        let response = self
            .graphql(
                "mutation($subject: ID!, $body: String!) { \
                   addComment(input: { subjectId: $subject, body: $body }) { subject { id } } \
                 }",
                json!({ "subject": pr.node_id, "body": CONFLICT_FIX_COMMENT }),
            )
            .await?;
        let posted = response
            .data
            .as_ref()
            .and_then(|d| d.pointer("/addComment/subject/id"))
            .is_some_and(|id| !id.is_null());
        if posted {
            Ok(())
        } else {
            Err(error_from_graphql(response.errors))
        }
    }

    async fn fetch_merged_status(
        &self,
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

        let response = self.graphql(&query, json!({})).await?;
        let Some(data) = response.data else {
            return Err(error_from_graphql(response.errors));
        };
        Ok(collect_merged(&data, &targets))
    }
}

/// When the current quota window resets. `retry-after` counts seconds from
/// `now_epoch_secs`; `x-ratelimit-reset` is already an absolute epoch.
fn rate_limit_reset(headers: &reqwest::header::HeaderMap, now_epoch_secs: u64) -> Option<u64> {
    let value = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
    };
    if let Some(seconds) = value("retry-after") {
        return Some(now_epoch_secs + seconds);
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
pub(crate) fn rfc3339_utc_to_epoch_secs(iso: &str) -> Option<u64> {
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
    Client::live(token).validate_token().await
}

/// Checks that `owner/name` exists and is accessible with this token.
/// Returns the canonical `nameWithOwner` (fixes user-typed casing).
pub async fn validate_repo(token: &str, owner: &str, name: &str) -> Result<String, String> {
    Client::live(token).validate_repo(owner, name).await
}

/// Both affiliation arguments matter: `ownerAffiliations` defaults to
/// [OWNER, COLLABORATOR], so setting `affiliations` alone would silently
/// drop org-member repos from the intersection. Deliberately no `rateLimit`
/// field: this query runs outside the sync loop, and its budget report would
/// only confuse the loop's planning.
const AFFILIATED_REPOS_QUERY: &str = "\
query($cursor: String) { \
  viewer { \
    repositories(first: 100, after: $cursor, \
                 affiliations: [OWNER, ORGANIZATION_MEMBER], \
                 ownerAffiliations: [OWNER, ORGANIZATION_MEMBER], \
                 orderBy: {field: NAME, direction: ASC}) { \
      pageInfo { hasNextPage endCursor } \
      nodes { nameWithOwner isArchived } \
    } \
  } \
}";

/// Every repo the viewer owns or shares an org with, as canonical
/// `owner/name` strings, archived repos excluded. Capped at 10 pages
/// (1000 repos); past the cap the partial list is returned rather than an
/// error, since the settings view unions in watched repos anyway and a
/// truncated browse list degrades more gracefully than a blank one.
pub async fn fetch_affiliated_repos(token: &str) -> Result<Vec<String>, GithubError> {
    Client::live(token).fetch_affiliated_repos().await
}

/// Appends one page of non-archived repo names to `out`. Returns the cursor
/// to resume from when more pages remain.
fn collect_affiliated_page(data: &Value, out: &mut Vec<String>) -> Option<String> {
    let nodes = data
        .pointer("/viewer/repositories/nodes")
        .and_then(Value::as_array);
    for node in nodes.into_iter().flatten() {
        if node.get("isArchived").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if let Some(name) = node.get("nameWithOwner").and_then(Value::as_str) {
            out.push(name.to_string());
        }
    }
    let page = data.pointer("/viewer/repositories/pageInfo")?;
    if page.get("hasNextPage").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    page.get("endCursor")
        .and_then(Value::as_str)
        .map(String::from)
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
    Client::live(token).fetch_open_prs(repos).await
}

/// Asks Copilot to resolve `pr`'s merge conflict, with a comment posted as the
/// token's user. Fails without posting when `pr` is not eligible.
pub async fn request_conflict_fix(token: &str, pr: &PullRequest) -> Result<(), GithubError> {
    Client::live(token).request_conflict_fix(pr).await
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
    Client::live(token).fetch_merged_status(keys).await
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

fn pr_key(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

/// The search for open PRs in `repos` whose pending reviewers include the
/// viewer, by name or through one of their teams. Search resolves the team
/// membership on GitHub's side, which a `repo`-scoped token cannot do in
/// the PR query: every GraphQL `Team` field needs `read:org`, and asking for
/// one fails the whole query. Never add a `Team` selection here or there.
fn team_search_field(repos: &[String], after: Option<&str>) -> String {
    let mut search = String::from("is:open is:pr review-requested:@me");
    for repo in repos {
        search.push_str(" repo:");
        search.push_str(repo);
    }
    let after = after
        .map(|c| format!(", after: {}", json!(c)))
        .unwrap_or_default();
    format!(
        "search(query: {search}, type: ISSUE, first: {TEAM_SEARCH_PAGE}{after}) {{ \
           pageInfo {{ hasNextPage endCursor }} \
           nodes {{ ... on PullRequest {{ number repository {{ nameWithOwner }} }} }} \
         }} ",
        search = json!(search),
    )
}

/// Adds one page of search hits to `out`, keyed like [`pr_key`]. Returns the
/// cursor to resume from when more pages remain. A search GitHub declined to
/// answer reads as its last page: the team mark is an extra, and must never
/// cost a sync the PRs it would otherwise report.
fn collect_team_requested(data: &Value, out: &mut HashSet<String>) -> Option<String> {
    for node in list(data, "/search/nodes") {
        let repo = node
            .pointer("/repository/nameWithOwner")
            .and_then(Value::as_str);
        let number = node.get("number").and_then(Value::as_u64);
        if let (Some(repo), Some(number)) = (repo, number) {
            out.insert(pr_key(repo, number));
        }
    }
    let page = data.pointer("/search/pageInfo")?;
    if page.get("hasNextPage").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    page.get("endCursor")
        .and_then(Value::as_str)
        .map(String::from)
}

/// One PR's head commit with its individual checks, in the shape the paged
/// query gives `commits` plus `contexts`. It is a query of its own because
/// `isRequired` must be told which PR it is asked about (required checks
/// follow the base branch), and the paged query cannot bind a number per row.
///
/// One page of 100 is all that is read: past that the row says "+ more"
/// rather than paging a monorepo's checks every poll.
///
/// The rollup mixes two kinds of check, GitHub Actions-style `CheckRun`s and
/// the older commit `StatusContext`s, so both fragments are needed; dropping
/// either makes its checks invisible rather than passing.
const CHECK_DETAIL_QUERY: &str = "\
query($owner: String!, $name: String!, $number: Int!) { \
  rateLimit { remaining resetAt } \
  repository(owner: $owner, name: $name) { \
    pullRequest(number: $number) { \
      commits(last: 1) { nodes { commit { statusCheckRollup { \
        state \
        contexts(first: 100) { \
          pageInfo { hasNextPage } \
          nodes { \
            ... on CheckRun { name status conclusion isRequired(pullRequestNumber: $number) } \
            ... on StatusContext { context state isRequired(pullRequestNumber: $number) } \
          } \
        } \
      } } } } \
    } \
  } \
}";

/// One repo's page of open PRs. `copilot` adds the question of whether
/// Copilot's cloud agent can be assigned in the repo; it is asked on every
/// page, since each page's rows are built from that page alone.
fn repo_field(alias: &str, owner: &str, name: &str, after: Option<&str>, copilot: bool) -> String {
    let after = after
        .map(|c| format!(", after: {}", json!(c)))
        .unwrap_or_default();
    let copilot = if copilot {
        format!(
            "suggestedActors(capabilities: [CAN_BE_ASSIGNED], query: {login}, \
                             first: {COPILOT_ACTOR_PAGE}) {{ nodes {{ __typename login }} }} ",
            login = json!(COPILOT_LOGIN),
        )
    } else {
        String::new()
    };
    format!(
        "{alias}: repository(owner: {owner}, name: {name}) {{ \
           nameWithOwner \
           owner {{ avatarUrl }} \
           {copilot}\
           pullRequests(states: OPEN, first: {PAGE_SIZE}{after}, \
                        orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ \
             pageInfo {{ hasNextPage endCursor }} \
             nodes {{ \
               id number title url createdAt updatedAt viewerDidAuthor viewerSubscription \
               isCrossRepository isDraft mergeable mergeStateStatus reviewDecision \
               viewerLatestReview {{ state }} \
               mergeQueueEntry {{ position }} \
               autoMergeRequest {{ enabledAt }} \
               author {{ __typename login avatarUrl }} \
               assignees(first: {ASSIGNEE_PAGE}) {{ nodes {{ login }} }} \
               commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ state }} }} }} }} \
               reviewRequests(first: 50) {{ nodes {{ requestedReviewer {{ ... on User {{ login }} }} }} }} \
               reviewThreads(last: 10) {{ nodes {{ isResolved }} }} \
               comments(last: {COMMENT_PAGE}) {{ nodes {{ createdAt author {{ __typename login }} }} }} \
               reviews(last: {REVIEW_PAGE}) {{ nodes {{ state submittedAt author {{ __typename login }} \
                 comments(last: {REVIEW_COMMENT_PAGE}) {{ nodes {{ createdAt author {{ __typename login }} }} }} }} }} \
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
    let copilot_enabled = copilot_enabled(repo);
    let nodes = repo
        .pointer("/pullRequests/nodes")
        .and_then(Value::as_array);
    for node in nodes.into_iter().flatten() {
        let is_draft = node.get("isDraft").and_then(Value::as_bool) == Some(true);
        // Computed once and reused: `awaiting_review` is gated to Mine, so the
        // outgoing marker never lights up on a PR the viewer merely reviews.
        let section = section_for(node, viewer);
        let review_requested = !is_draft && review_requested_for(node, viewer);
        let activity = collect_activity(node, viewer);
        let checks = checks_for(node);
        let blocked_reasons = blocked_reasons_for(node, &checks);
        let ready_to_merge = ready_to_merge(node, section, &blocked_reasons, &checks);
        let turn = turn_for(
            node,
            viewer,
            section,
            &blocked_reasons,
            ready_to_merge,
            &checks,
        );
        // The names belong to the CI pill, so they go only where it shows: a
        // draft or an unsettled mergeability suppresses both together.
        let ci_raised = blocked_reasons.contains(&BlockedReason::Ci);
        let can_fix_conflict =
            can_fix_conflict(node, section, is_draft, &blocked_reasons, copilot_enabled);
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
            author_is_bot: is_bot(node),
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
            section,
            blocked_reasons,
            is_draft,
            // Suppressed on drafts, matching the blocked pills: a draft is the
            // author's not-ready choice, so the row never nags you to act on
            // it. Only the marker is hidden; `section_for` still counts the
            // request, so the PR keeps its Participated membership.
            review_requested,
            team_review_requested: !is_draft && !review_requested && could_be_team_request(node),
            // The outgoing mirror: your own PR waiting on a reviewer. Gated to
            // Mine so it cannot fire on a PR you merely participate in, and
            // suppressed on drafts like `review_requested`. It also yields to
            // `review_requested`: only an agent's PR can be in Mine and ask
            // for your review, and there the ball is in your court.
            awaiting_review: section == Section::Mine
                && !is_draft
                && !review_requested
                && has_pending_review_request(node),
            // Suppressed on drafts like the markers above; an early approval
            // on a not-ready PR would misread as "ship it". Deliberately not
            // suppressed beside the review glyphs: GitHub keeps APPROVED once
            // the required approvals are in even while more reviewers are
            // listed, and the met requirement is worth showing regardless.
            approved: !is_draft && is_approved(node),
            merge_queue_position: node
                .pointer("/mergeQueueEntry/position")
                .and_then(Value::as_u64),
            failing_checks_truncated: ci_raised && checks.truncated,
            checks_running: !is_draft && checks.pending && !checks.failed,
            failing_checks: if ci_raised {
                checks.failing
            } else {
                Vec::new()
            },
            auto_merge: !is_draft && is_set(node, "autoMergeRequest"),
            ready_to_merge,
            turn,
            turn_since: None,
            can_fix_conflict,
            node_id: node
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            unread_count: 0,
            bot_unread_count: 0,
            activity: activity.human,
            bot_activity: activity.bot,
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

/// Whether Copilot's cloud agent can be assigned in the repo, which is how
/// GitHub shows that the agent is enabled there. An actor list that is
/// missing or null (not asked for, or not readable with this token) reads as
/// not enabled.
fn copilot_enabled(repo: &Value) -> bool {
    list(repo, "/suggestedActors/nodes").iter().any(|actor| {
        actor.get("__typename").and_then(Value::as_str) == Some("Bot")
            && actor.get("login").and_then(Value::as_str) == Some(COPILOT_LOGIN)
    })
}

/// Whether the row may offer to ask Copilot for a conflict fix. Copilot
/// answers by pushing to the PR's branch, so the offer stays with branches of
/// the repo itself, and it answers only where its cloud agent is enabled.
/// Mine keeps the offer to PRs whose conflict is the viewer's to clear.
fn can_fix_conflict(
    node: &Value,
    section: Section,
    is_draft: bool,
    blocked_reasons: &[BlockedReason],
    copilot_enabled: bool,
) -> bool {
    section == Section::Mine
        && !is_draft
        && blocked_reasons.contains(&BlockedReason::Conflict)
        && node.get("isCrossRepository").and_then(Value::as_bool) == Some(false)
        && copilot_enabled
}

/// A PR's unread-eligible activity as ascending timestamps, split by who
/// wrote it: only the human list may badge.
#[derive(Debug, Default, PartialEq)]
struct Activity {
    human: Vec<String>,
    bot: Vec<String>,
}

/// Whether `item`'s author is a machine account: a GitHub App acting as
/// itself, which GraphQL types as `Bot`. A person's account driven by
/// automation is a `User` and is not caught. `item` is anything carrying an
/// `author`: a PR, a comment or a review.
fn is_bot(item: &Value) -> bool {
    item.pointer("/author/__typename").and_then(Value::as_str) == Some("Bot")
}

/// Gathers the timestamps of what counts as an "update" for unread purposes:
/// issue comments, submitted reviews, and review comments — never commits or
/// CI. The query does fetch the head commit's CI rollup for the row indicator,
/// but that is a property of the PR, not an unread event, so it is read
/// elsewhere and never enters these lists. The viewer's own activity is
/// excluded from both (you have read what you wrote), as are PENDING reviews
/// (invisible to everyone but their author until submitted).
fn collect_activity(node: &Value, viewer: &str) -> Activity {
    let mut activity = Activity::default();
    let by_other =
        |item: &Value| item.pointer("/author/login").and_then(Value::as_str) != Some(viewer);
    let mut push = |item: &Value, time_field: &str| {
        if by_other(item) {
            if let Some(t) = item.get(time_field).and_then(Value::as_str) {
                let times = if is_bot(item) {
                    &mut activity.bot
                } else {
                    &mut activity.human
                };
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
    activity.human.sort();
    activity.bot.sort();
    activity
}

fn list<'a>(node: &'a Value, pointer: &str) -> &'a [Value] {
    node.pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// Composes the row's blocked indicator from mergeability, the head commit's
/// checks (see [`checks_for`]), the review decision, unresolved review
/// threads, and the merge-state status, in the fixed severity order. Only
/// negative states
/// light it: a mergeable, in-flight, or quiet PR yields an empty list so an
/// undecorated row keeps meaning "nothing is stuck".
///
/// Suppressed entirely for drafts (the author has not declared readiness) and
/// while `mergeable` is `UNKNOWN`: GitHub computes mergeability lazily and
/// reports `UNKNOWN` until it settles, which must read as "no flag yet", not
/// as a blocked state; it resolves on the next poll.
///
/// `BEHIND` never coexists with a conflict: `mergeStateStatus` is a single
/// value and a conflicting head reports `DIRTY` instead. GitHub also only
/// reports `BEHIND` on repos whose branch protection requires branches to be
/// up to date before merging; elsewhere a stale-but-clean head stays `CLEAN`
/// and rightly shows no pill, since being behind does not block it.
///
/// `mergeStateStatus` is matched on `BEHIND` alone, never on its other
/// variants, which is what makes it safe to read here at all: GitHub computes
/// it reliably only for tokens with push access and answers `UNKNOWN`
/// otherwise, and its `BLOCKED`/`UNSTABLE` variants conflate wait-states with
/// act-states. An exact `BEHIND` match degrades to no pill on a repo we
/// cannot see into, rather than to a wrong one; the cost is that the behind
/// signal is simply absent for PRs in repos the viewer cannot push to.
///
/// `checks` must be `checks_for(node)`; it is a parameter only so the caller
/// reads the checks once for everything that needs them.
fn blocked_reasons_for(node: &Value, checks: &Checks) -> Vec<BlockedReason> {
    let field = |name: &str| node.get(name).and_then(Value::as_str);
    if node.get("isDraft").and_then(Value::as_bool) == Some(true)
        || field("mergeable") == Some("UNKNOWN")
    {
        return Vec::new();
    }
    let mut reasons = Vec::new();
    if field("mergeable") == Some("CONFLICTING") {
        reasons.push(BlockedReason::Conflict);
    }
    if checks.failed {
        reasons.push(BlockedReason::Ci);
    }
    if field("reviewDecision") == Some("CHANGES_REQUESTED") {
        reasons.push(BlockedReason::Review);
    }
    // Unresolved threads flag independently of the other signals: an open
    // thread needs an answer whether or not reviews or CI are done, so the
    // pill coexists with any other reason (and with the green check). Only
    // the draft/UNKNOWN suppression above gates it, as it does every reason
    // here, so the pill sits out the poll after a push while GitHub
    // recomputes mergeability. Reads the
    // newest 10 threads (the query pages from the end, where the still-open
    // threads skew; a full-history page would also bloat the batched query's
    // node budget); a PR whose unresolved threads all sit deeper than that is
    // beyond what one page can see.
    let unresolved = list(node, "/reviewThreads/nodes")
        .iter()
        .any(|t| t.get("isResolved").and_then(Value::as_bool) == Some(false));
    if unresolved {
        reasons.push(BlockedReason::Threads);
    }
    if field("mergeStateStatus") == Some("BEHIND") {
        reasons.push(BlockedReason::Behind);
    }
    reasons
}

/// What the head commit's checks say about merging the PR.
#[derive(Debug, Default, PartialEq)]
struct Checks {
    /// Names of the failed checks that gate the merge, in GitHub's order.
    failing: Vec<String>,
    /// Whether a gating check failed. True with `failing` empty when only
    /// the rollup says so and there is no check to name.
    failed: bool,
    /// Whether a gating check has yet to finish. Independent of `failed`.
    pending: bool,
    /// Whether GitHub holds more checks than the page read.
    truncated: bool,
}

enum CheckOutcome {
    Failed,
    Pending,
    Passed,
}

/// Normalizes one rollup context. The two kinds GitHub mixes in the list
/// spell their verdict differently: a `CheckRun` in `conclusion`, which is
/// null until the run completes, and a `StatusContext` in `state`. Anything
/// that is neither a pass nor a failure is still on its way, a missing
/// conclusion included.
fn check_outcome(context: &Value) -> CheckOutcome {
    let verdict = ["conclusion", "state"]
        .iter()
        .find_map(|field| context.get(*field).and_then(Value::as_str));
    match verdict {
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => CheckOutcome::Passed,
        Some(
            "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE",
        ) => CheckOutcome::Failed,
        _ => CheckOutcome::Pending,
    }
}

/// Reads the head commit's checks, counting only the ones that gate the
/// merge: the required checks, or every check when none of those read is
/// marked required, where the rollup's "any failure fails" is all there is
/// to go on. An optional check listed beside a required one never counts,
/// failed or running.
///
/// The individual checks are on the node only once
/// [`Client::add_check_detail`] has put them there. Without them the rollup
/// state answers alone, as it also does for whatever a truncated page leaves
/// unseen: a failing or pending rollup that the page cannot account for is
/// believed rather than read as a pass.
fn checks_for(node: &Value) -> Checks {
    let rollup = node.pointer("/commits/nodes/0/commit/statusCheckRollup");
    let state = rollup.and_then(|r| r.get("state")).and_then(Value::as_str);
    let rollup_failed = matches!(state, Some("FAILURE" | "ERROR"));
    let rollup_pending = matches!(state, Some("PENDING" | "EXPECTED"));
    let Some(contexts) = rollup
        .and_then(|r| r.get("contexts"))
        .filter(|c| !c.is_null())
    else {
        return Checks {
            failed: rollup_failed,
            pending: rollup_pending,
            ..Checks::default()
        };
    };

    let truncated = contexts
        .pointer("/pageInfo/hasNextPage")
        .and_then(Value::as_bool)
        == Some(true);
    let required = |c: &Value| c.get("isRequired").and_then(Value::as_bool) == Some(true);
    let nodes = list(contexts, "/nodes");
    let any_required = nodes.iter().any(required);
    let mut failing = Vec::new();
    let mut failed = false;
    let mut pending = false;
    for context in nodes.iter().filter(|c| !any_required || required(c)) {
        match check_outcome(context) {
            CheckOutcome::Failed => {
                failed = true;
                // A `CheckRun` is named by `name`, a `StatusContext` by
                // `context`. A failure with neither still counts; it just
                // has nothing to put on the pill.
                let name = ["name", "context"]
                    .iter()
                    .find_map(|field| context.get(*field).and_then(Value::as_str));
                failing.extend(name.filter(|n| !n.is_empty()).map(String::from));
            }
            CheckOutcome::Pending => pending = true,
            CheckOutcome::Passed => {}
        }
    }
    Checks {
        failed: failed || (truncated && rollup_failed),
        pending: pending || (truncated && rollup_pending),
        failing,
        truncated,
    }
}

/// Whether the PR's rollup leaves a question only its individual checks
/// answer: which check failed and whether it is required, or whether the
/// ones still running are. A passing or absent rollup leaves none, and a
/// draft shows no check marks at all.
fn needs_check_detail(node: &Value) -> bool {
    let state = node
        .pointer("/commits/nodes/0/commit/statusCheckRollup/state")
        .and_then(Value::as_str);
    node.get("isDraft").and_then(Value::as_bool) != Some(true)
        && matches!(state, Some("FAILURE" | "ERROR" | "PENDING" | "EXPECTED"))
}

/// Whether `node` carries a non-null `name` field: GitHub answers null for
/// an object field with nothing behind it, such as a PR that is not queued.
fn is_set(node: &Value, name: &str) -> bool {
    node.get(name).is_some_and(|v| !v.is_null())
}

/// Whether the merge button is all that is left: the PR is the viewer's,
/// GitHub can merge it, nothing blocks it, every gating check has passed,
/// and review is settled. Review is settled by an explicit approval, or, on
/// a repo that requires none (a null decision), by nobody still being asked.
///
/// Mergeability must be an explicit `MERGEABLE`: `UNKNOWN` empties the
/// blocked reasons (see [`blocked_reasons_for`]) and so would otherwise read
/// as ready for the one poll after every push.
///
/// A PR in the merge queue or with auto-merge armed is not ready: the merge
/// is already under way and needs nothing from the viewer.
///
/// A `mergeStateStatus` of `CLEAN` would be the obvious shortcut. It is not
/// read, for the reasons recorded on [`blocked_reasons_for`].
///
/// `reasons` and `checks` must be the ones read off this `node`.
fn ready_to_merge(
    node: &Value,
    section: Section,
    reasons: &[BlockedReason],
    checks: &Checks,
) -> bool {
    let field = |name: &str| node.get(name).and_then(Value::as_str);
    let review_settled = match field("reviewDecision") {
        Some("APPROVED") => true,
        None => !has_pending_review_request(node),
        Some(_) => false,
    };
    section == Section::Mine
        && node.get("isDraft").and_then(Value::as_bool) != Some(true)
        && field("mergeable") == Some("MERGEABLE")
        // A failed check is among the reasons; one still running is not.
        && reasons.is_empty()
        && !checks.pending
        && review_settled
        && !is_set(node, "mergeQueueEntry")
        && !is_set(node, "autoMergeRequest")
}

/// Whose move the PR waits on. The first matching rule decides, so the order
/// below is the contract:
///
/// 1. A draft, or a PR in All, has no verdict.
/// 2. A review requested from the viewer by name is the viewer's to give, in
///    any section, whatever else is true of the PR: a re-request after an
///    approval lands here too.
/// 3. In Mine, any blocked reason is the viewer's to clear, even with a
///    merge under way.
/// 4. In Mine, a queued PR or one with auto-merge armed waits on GitHub.
/// 5. In Mine, a ready PR waits on the viewer pressing merge.
/// 6. In Mine, a PR with a reviewer still asked, or with gating checks
///    running and none failed, waits on them.
/// 7. In Participated, a PR the viewer has reviewed waits on its author.
/// 8. Anything else has no verdict.
///
/// Rule 3 needs no guard for an `UNKNOWN` mergeability: the reasons are
/// already empty then (see [`blocked_reasons_for`]), so the PR falls through
/// for that one poll rather than reading as the viewer's.
///
/// A team request never makes a turn. It is the team's obligation, not the
/// viewer's, and has its own mark.
///
/// `reasons`, `ready` and `checks` must be the ones read off this `node`.
fn turn_for(
    node: &Value,
    viewer: &str,
    section: Section,
    reasons: &[BlockedReason],
    ready: bool,
    checks: &Checks,
) -> Option<Turn> {
    if node.get("isDraft").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match section {
        Section::All => None,
        _ if review_requested_for(node, viewer) => Some(Turn::Yours),
        Section::Mine => {
            if !reasons.is_empty() {
                Some(Turn::Yours)
            } else if is_set(node, "mergeQueueEntry") || is_set(node, "autoMergeRequest") {
                Some(Turn::Theirs)
            } else if ready {
                Some(Turn::Yours)
            } else if has_pending_review_request(node) || (checks.pending && !checks.failed) {
                Some(Turn::Theirs)
            } else {
                None
            }
        }
        Section::Participated => viewer_has_reviewed(node).then_some(Turn::Theirs),
    }
}

/// Whether the viewer has a review standing on the PR. A review still being
/// written is `PENDING`: nobody else can see it, so it has handed nothing
/// back to the author. A `DISMISSED` one no longer counts either.
fn viewer_has_reviewed(node: &Value) -> bool {
    matches!(
        node.pointer("/viewerLatestReview/state")
            .and_then(Value::as_str),
        Some("APPROVED" | "CHANGES_REQUESTED" | "COMMENTED")
    )
}

/// Whether the viewer appears among the PR's requested reviewers. Team
/// requests carry no `login` (GitHub only exposes it via `... on User`), so
/// they never match: a viewer whose team was asked is flagged through
/// `team_review_requested` instead.
fn review_requested_for(node: &Value, viewer: &str) -> bool {
    list(node, "/reviewRequests/nodes").iter().any(|n| {
        n.pointer("/requestedReviewer/login")
            .and_then(Value::as_str)
            == Some(viewer)
    })
}

/// Whether a team request to the viewer is possible on this PR: someone else
/// wrote it, and a pending reviewer carries no `login`, which is all a
/// `repo`-scoped token sees of a team. It cannot say whose team; the search
/// in [`team_search_field`] does. Checking the live list as well ties the
/// mark's clearing to the PR itself, not to how fresh the search index is.
fn could_be_team_request(node: &Value) -> bool {
    node.get("viewerDidAuthor").and_then(Value::as_bool) != Some(true)
        && list(node, "/reviewRequests/nodes").iter().any(|n| {
            n.pointer("/requestedReviewer/login")
                .and_then(Value::as_str)
                .is_none()
        })
}

/// Whether the PR has any outstanding requested reviewer. The outgoing
/// counterpart to [`review_requested_for`]: it asks only whether a request
/// exists, not who was asked, so unlike that helper it counts team requests
/// too (a team node carries no `login`, but it is still a pending request).
/// GitHub lists only reviewers who have not yet submitted, so an empty list
/// means the reviews are in.
fn has_pending_review_request(node: &Value) -> bool {
    !list(node, "/reviewRequests/nodes").is_empty()
}

/// Whether the review decision is an explicit APPROVED, i.e. the repo's
/// required approvals are all in. Null is not approved: GitHub reports a null
/// reviewDecision both when the repo requires no reviews and while a
/// comment-only review is in progress, so only the explicit verdict counts.
fn is_approved(node: &Value) -> bool {
    node.get("reviewDecision").and_then(Value::as_str) == Some("APPROVED")
}

/// Whether GitHub considers the viewer involved enough to notify them about
/// this PR. GitHub subscribes them the moment they comment, review, are
/// mentioned, are assigned, or have a review requested, including through a
/// team, whose membership nothing else in the response resolves. Unlike
/// `participants`, it is unpaged and survives an org's member-privacy setting,
/// so involvement can neither fall out of a page window nor be hidden; unlike
/// watching a repo, it is per-thread. Muting a thread clears it, by design:
/// the section decides what may badge (see `unread::apply`), and a thread the
/// viewer muted should stop badging them.
fn viewer_subscribed(node: &Value) -> bool {
    node.get("viewerSubscription").and_then(Value::as_str) == Some("SUBSCRIBED")
}

fn assigned_to(node: &Value, viewer: &str) -> bool {
    list(node, "/assignees/nodes")
        .iter()
        .any(|n| n.get("login").and_then(Value::as_str) == Some(viewer))
}

/// Mine beats Participated: authors are subscribed to their own PRs, so order
/// matters. A bot-authored PR assigned to the viewer is Mine too: a coding
/// agent opens the PR under its own account, so `viewerDidAuthor` is false
/// and the assignment is what marks the work as the viewer's. Assignment to
/// a person's PR stays ordinary involvement.
///
/// A pending review request is checked alongside the subscription
/// rather than trusted to it, so muting a thread cannot hide a review the
/// viewer was named for. That cover stops at named requests: a team request
/// carries no `login` for [`review_requested_for`] to match, so a muted
/// CODEOWNERS thread falls to All until the viewer re-subscribes.
fn section_for(node: &Value, viewer: &str) -> Section {
    if node.get("viewerDidAuthor").and_then(Value::as_bool) == Some(true)
        || (is_bot(node) && assigned_to(node, viewer))
    {
        return Section::Mine;
    }
    if viewer_subscribed(node) || review_requested_for(node, viewer) {
        return Section::Participated;
    }
    Section::All
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr_state::{Effects, PrState};

    /// `node` with the top-level fields of `overrides` laid over it.
    fn with(mut node: Value, overrides: Value) -> Value {
        node.as_object_mut()
            .unwrap()
            .extend(overrides.as_object().unwrap().clone());
        node
    }

    /// The blocked reasons of a node, as `collect_repo_prs` derives them.
    fn blocked_reasons(node: &Value) -> Vec<BlockedReason> {
        blocked_reasons_for(node, &checks_for(node))
    }

    fn pr_node(overrides: Value) -> Value {
        let node = json!({
            "number": 7,
            "title": "Fix the thing",
            "url": "https://github.com/acme/widgets/pull/7",
            "createdAt": "2026-07-10T12:00:00Z",
            "updatedAt": "2026-07-11T09:30:00Z",
            "viewerDidAuthor": false,
            "author": { "login": "someone", "avatarUrl": "https://avatars.example/someone" },
            "viewerSubscription": "UNSUBSCRIBED",
            "reviewRequests": { "nodes": [] }
        });
        with(node, overrides)
    }

    /// Wraps PR nodes in the single-page repo scaffold `collect_repo_prs`
    /// expects, so a test states only the node fields it is about.
    fn repo_with(nodes: Vec<Value>) -> Value {
        json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": nodes
            }
        })
    }

    #[test]
    fn authored_prs_are_mine_even_though_the_author_is_subscribed() {
        let node = pr_node(json!({
            "viewerDidAuthor": true,
            "viewerSubscription": "SUBSCRIBED"
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Mine);
    }

    #[test]
    fn subscribed_prs_are_participated() {
        // GitHub subscribes the viewer when they comment, review, are
        // mentioned, or are assigned, so one field answers all of those.
        let node = pr_node(json!({ "viewerSubscription": "SUBSCRIBED" }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn an_ignored_thread_is_not_involvement() {
        // Muting a thread is its own subscription state, not the absence of
        // one: only SUBSCRIBED counts. Reading the field as merely present
        // would sweep every browsed PR into Participated and badge it.
        let node = pr_node(json!({ "viewerSubscription": "IGNORED" }));
        assert_eq!(section_for(&node, "khiet"), Section::All);
    }

    #[test]
    fn review_requested_prs_are_participated() {
        // `pr_node` defaults to UNSUBSCRIBED, so the request alone places this
        // PR: one GitHub never subscribed the viewer to still counts.
        let node = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn muting_a_thread_does_not_hide_a_review_you_were_named_for() {
        // A mute must not outrank a review the viewer was asked for by name,
        // so IGNORED loses to the request rather than winning outright.
        let node = pr_node(json!({
            "viewerSubscription": "IGNORED",
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn review_requested_flags_only_when_the_viewer_is_a_requested_reviewer() {
        // The viewer is one of several requested reviewers: flagged.
        let requested = pr_node(json!({
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "login": "someone" } },
                { "requestedReviewer": { "login": "khiet" } }
            ] }
        }));
        assert!(review_requested_for(&requested, "khiet"));
        // Someone else's review is requested, not the viewer's: not flagged.
        // The common case, and what keeps the glyph off PRs awaiting others.
        let other = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "someone" } }] }
        }));
        assert!(!review_requested_for(&other, "khiet"));
        // The viewer takes part but was never asked to review: not flagged.
        let involved = pr_node(json!({ "viewerSubscription": "SUBSCRIBED" }));
        assert!(!review_requested_for(&involved, "khiet"));
    }

    #[test]
    fn a_team_review_request_does_not_flag_the_viewer() {
        // Team reviewers have no `login` field, so no user can match one.
        let node = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] }
        }));
        assert!(!review_requested_for(&node, "khiet"));
    }

    #[test]
    fn awaiting_review_tracks_any_outstanding_request() {
        // No requested reviewers: the reviews are in (or none were asked).
        let none = pr_node(json!({ "reviewRequests": { "nodes": [] } }));
        assert!(!has_pending_review_request(&none));
        // A named reviewer is still pending: waiting on them.
        let user = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "someone" } }] }
        }));
        assert!(has_pending_review_request(&user));
        // A team request counts too, unlike the incoming direction: the check
        // is existence, and a team node needs no `login`.
        let team = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] }
        }));
        assert!(has_pending_review_request(&team));
    }

    #[test]
    fn a_draft_review_request_still_lands_in_participated() {
        // Suppressing the draft's row marker must not drop the PR out of
        // Participated: the viewer is still a requested reviewer.
        let node = pr_node(json!({
            "isDraft": true,
            "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
    }

    #[test]
    fn a_team_review_request_alone_leaves_the_pr_in_all() {
        // A team request names no user, so on its own it places nothing. This
        // is what stops `section_for` from being widened to
        // `has_pending_review_request` to "fix" team requests, which would
        // sweep every PR still waiting on any reviewer into Participated.
        let node = pr_node(json!({
            "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::All);
    }

    #[test]
    fn a_team_review_request_reaches_the_viewer_through_the_subscription() {
        // GitHub subscribes the team's members, which is the only way the app
        // learns a request aimed at a team is the viewer's.
        let node = pr_node(json!({
            "viewerSubscription": "SUBSCRIBED",
            "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] }
        }));
        assert_eq!(section_for(&node, "khiet"), Section::Participated);
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

    /// A `commits` connection whose head commit rolls up to `state`, for
    /// embedding in a `pr_node` overrides literal.
    fn ci_commits(state: &str) -> Value {
        json!({ "nodes": [{ "commit": { "statusCheckRollup": { "state": state } } }] })
    }

    /// A `reviewThreads` connection with one thread per flag, for embedding
    /// in a `pr_node` overrides literal.
    fn review_threads(resolved: &[bool]) -> Value {
        let nodes: Vec<Value> = resolved
            .iter()
            .map(|r| json!({ "isResolved": r }))
            .collect();
        json!({ "nodes": nodes })
    }

    #[test]
    fn each_blocked_trigger_lights_the_marker_on_its_own() {
        let reasons = |overrides: Value| blocked_reasons(&pr_node(overrides));
        assert_eq!(
            reasons(json!({ "mergeable": "CONFLICTING" })),
            vec![BlockedReason::Conflict]
        );
        assert_eq!(
            reasons(json!({ "commits": ci_commits("FAILURE") })),
            vec![BlockedReason::Ci]
        );
        assert_eq!(
            reasons(json!({ "commits": ci_commits("ERROR") })),
            vec![BlockedReason::Ci]
        );
        assert_eq!(
            reasons(json!({ "reviewDecision": "CHANGES_REQUESTED" })),
            vec![BlockedReason::Review]
        );
        assert_eq!(
            reasons(json!({ "mergeStateStatus": "BEHIND" })),
            vec![BlockedReason::Behind]
        );
        assert_eq!(
            reasons(json!({ "reviewThreads": review_threads(&[true, false]) })),
            vec![BlockedReason::Threads]
        );
    }

    /// Threads are their own signal: an open thread needs an answer whether
    /// or not reviews or CI are done, so no other state gates it; the fixed
    /// severity order still lists harder reasons first.
    #[test]
    fn unresolved_threads_flag_independently_of_other_signals() {
        let reasons = |overrides: Value| blocked_reasons(&pr_node(overrides));
        let unresolved = || review_threads(&[false]);
        assert_eq!(
            reasons(json!({
                "reviewDecision": "CHANGES_REQUESTED",
                "reviewThreads": unresolved()
            })),
            vec![BlockedReason::Review, BlockedReason::Threads]
        );
        assert_eq!(
            reasons(json!({
                "commits": ci_commits("FAILURE"),
                "reviewThreads": unresolved()
            })),
            vec![BlockedReason::Ci, BlockedReason::Threads]
        );
        // Mid-review, no-required-review (null decision), and CI-in-flight
        // PRs all flag: the thread, not the surrounding state, is the signal.
        assert_eq!(
            reasons(json!({
                "reviewDecision": "REVIEW_REQUIRED",
                "reviewThreads": unresolved()
            })),
            vec![BlockedReason::Threads]
        );
        assert_eq!(
            reasons(json!({ "reviewThreads": unresolved() })),
            vec![BlockedReason::Threads]
        );
        assert_eq!(
            reasons(json!({
                "commits": ci_commits("PENDING"),
                "reviewThreads": unresolved()
            })),
            vec![BlockedReason::Threads]
        );
        // Fully resolved threads never flag, however done the PR is.
        assert_eq!(
            reasons(json!({
                "reviewDecision": "APPROVED",
                "commits": ci_commits("SUCCESS"),
                "reviewThreads": review_threads(&[true, true])
            })),
            Vec::new()
        );
        // An approved, green PR with an open thread is the reason's home case.
        assert_eq!(
            reasons(json!({
                "reviewDecision": "APPROVED",
                "commits": ci_commits("SUCCESS"),
                "reviewThreads": unresolved()
            })),
            vec![BlockedReason::Threads]
        );
    }

    #[test]
    fn mergeable_in_flight_and_quiet_prs_stay_undecorated() {
        let reasons = |overrides: Value| blocked_reasons(&pr_node(overrides));
        // The pr_node fixture has no CI, mergeability, or review fields at all.
        assert_eq!(reasons(json!({})), Vec::<BlockedReason>::new());
        assert_eq!(
            reasons(json!({ "mergeable": "MERGEABLE", "reviewDecision": "APPROVED" })),
            Vec::new()
        );
        // In-flight states (CI pending, awaiting review) are progress, not
        // blockage.
        assert_eq!(
            reasons(json!({ "commits": ci_commits("PENDING") })),
            Vec::new()
        );
        assert_eq!(
            reasons(json!({ "commits": ci_commits("EXPECTED") })),
            Vec::new()
        );
        assert_eq!(
            reasons(json!({ "commits": ci_commits("SUCCESS") })),
            Vec::new()
        );
        assert_eq!(
            reasons(json!({ "reviewDecision": "REVIEW_REQUIRED" })),
            Vec::new()
        );
        // Every non-BEHIND merge state is either fine (CLEAN), covered by a
        // dedicated reason (DIRTY -> conflict), or a state we deliberately do
        // not flag (BLOCKED: branch protection, e.g. missing approvals).
        assert_eq!(reasons(json!({ "mergeStateStatus": "CLEAN" })), Vec::new());
        assert_eq!(
            reasons(json!({ "mergeStateStatus": "BLOCKED" })),
            Vec::new()
        );
    }

    /// Fixed severity order regardless of which fields say what: conflict,
    /// then CI, then review, then threads, then behind. The conflict+behind
    /// combination is synthetic (a conflicting head reports `DIRTY`, not
    /// `BEHIND`), but it pins the ordering contract for every pair that can
    /// occur.
    #[test]
    fn concurrent_triggers_list_every_reason_in_fixed_order() {
        let node = pr_node(json!({
            "mergeable": "CONFLICTING",
            "commits": ci_commits("FAILURE"),
            "reviewDecision": "CHANGES_REQUESTED",
            "reviewThreads": review_threads(&[false]),
            "mergeStateStatus": "BEHIND"
        }));
        assert_eq!(
            blocked_reasons(&node),
            vec![
                BlockedReason::Conflict,
                BlockedReason::Ci,
                BlockedReason::Review,
                BlockedReason::Threads,
                BlockedReason::Behind
            ]
        );
    }

    /// The author has not declared readiness, so "needs action now" does not
    /// apply — whatever the CI, conflict, or review state says.
    #[test]
    fn drafts_suppress_the_marker_entirely() {
        let node = pr_node(json!({
            "isDraft": true,
            "mergeable": "CONFLICTING",
            "commits": ci_commits("FAILURE"),
            "reviewDecision": "CHANGES_REQUESTED",
            "reviewThreads": review_threads(&[false]),
            "mergeStateStatus": "BEHIND"
        }));
        assert_eq!(blocked_reasons(&node), Vec::new());
    }

    /// GitHub computes `mergeable` lazily and answers `UNKNOWN` until it
    /// settles; that transient must render as no marker, not as a blocked state.
    #[test]
    fn an_unknown_mergeability_suppresses_the_marker_until_the_next_poll() {
        let node = pr_node(json!({
            "mergeable": "UNKNOWN",
            "commits": ci_commits("FAILURE"),
            "reviewDecision": "CHANGES_REQUESTED",
            "reviewThreads": review_threads(&[false]),
            "mergeStateStatus": "BEHIND"
        }));
        assert_eq!(blocked_reasons(&node), Vec::new());
    }

    fn check_run(name: &str, conclusion: Option<&str>, required: bool) -> Value {
        let status = if conclusion.is_some() {
            "COMPLETED"
        } else {
            "IN_PROGRESS"
        };
        json!({ "name": name, "status": status, "conclusion": conclusion, "isRequired": required })
    }

    fn status_context(context: &str, state: &str, required: bool) -> Value {
        json!({ "context": context, "state": state, "isRequired": required })
    }

    /// A `commits` connection whose head commit rolls up to `state` and
    /// lists `contexts`, as a node reads once its checks are filled in.
    /// `more` says GitHub holds further checks past this page.
    fn checked_commits(state: &str, contexts: Vec<Value>, more: bool) -> Value {
        json!({ "nodes": [{ "commit": { "statusCheckRollup": {
            "state": state,
            "contexts": { "pageInfo": { "hasNextPage": more }, "nodes": contexts }
        } } }] })
    }

    /// The row `collect_repo_prs` builds for one PR node.
    fn row(overrides: Value) -> PullRequest {
        let mut out = Vec::new();
        collect_repo_prs(&repo_with(vec![pr_node(overrides)]), "khiet", &mut out);
        out.remove(0)
    }

    #[test]
    fn a_failed_required_check_raises_ci_and_is_the_one_named() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                check_run("lint-optional", Some("FAILURE"), false),
                check_run("e2e-safari", Some("FAILURE"), true),
                check_run("build", Some("SUCCESS"), true),
            ],
            false,
        ) }));
        assert_eq!(pr.blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(pr.failing_checks, vec!["e2e-safari"]);
        assert!(!pr.failing_checks_truncated);
    }

    #[test]
    fn failures_in_optional_checks_alone_raise_nothing() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                check_run("lint-optional", Some("FAILURE"), false),
                check_run("build", Some("SUCCESS"), true),
            ],
            false,
        ) }));
        assert_eq!(pr.blocked_reasons, Vec::new());
        assert_eq!(pr.failing_checks, Vec::<String>::new());
        assert!(!pr.checks_running);
    }

    /// With nothing marked required there is no gate to read, so the rollup's
    /// meaning stands: any failure is a failure.
    #[test]
    fn any_failed_check_counts_when_the_repo_requires_none() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                check_run("build", Some("SUCCESS"), false),
                check_run("lint", Some("TIMED_OUT"), false),
            ],
            false,
        ) }));
        assert_eq!(pr.blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(pr.failing_checks, vec!["lint"]);
    }

    /// A pill reading "CI: " would name nothing; the plain label is honest.
    #[test]
    fn a_failed_check_without_a_name_raises_ci_and_names_nothing() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![json!({ "conclusion": "FAILURE", "isRequired": true })],
            false,
        ) }));
        assert_eq!(pr.blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(pr.failing_checks, Vec::<String>::new());
    }

    #[test]
    fn check_runs_and_commit_statuses_are_both_read_as_checks() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                status_context("ci/legacy", "ERROR", true),
                check_run("e2e-safari", Some("FAILURE"), true),
                status_context("coverage", "SUCCESS", true),
            ],
            false,
        ) }));
        assert_eq!(pr.failing_checks, vec!["ci/legacy", "e2e-safari"]);
    }

    #[test]
    fn a_check_run_without_a_conclusion_is_still_running() {
        let pr = row(json!({ "commits": checked_commits(
            "PENDING",
            vec![
                check_run("build", Some("SUCCESS"), true),
                check_run("e2e-safari", None, true),
            ],
            false,
        ) }));
        assert!(pr.checks_running);
        assert_eq!(pr.blocked_reasons, Vec::new());
    }

    #[test]
    fn a_pending_commit_status_is_running_too() {
        let pr = row(json!({ "commits": checked_commits(
            "PENDING",
            vec![status_context("ci/legacy", "PENDING", true)],
            false,
        ) }));
        assert!(pr.checks_running);
    }

    /// Running means "nothing has failed yet"; once a required check fails
    /// the CI pill says so and the running mark would only add noise.
    #[test]
    fn a_failed_check_outranks_the_ones_still_running() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                check_run("build", Some("FAILURE"), true),
                check_run("e2e-safari", None, true),
            ],
            false,
        ) }));
        assert_eq!(pr.blocked_reasons, vec![BlockedReason::Ci]);
        assert!(!pr.checks_running);
    }

    #[test]
    fn checks_past_the_fetched_page_mark_the_failing_list_truncated() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![check_run("e2e-safari", Some("FAILURE"), true)],
            true,
        ) }));
        assert_eq!(pr.failing_checks, vec!["e2e-safari"]);
        assert!(pr.failing_checks_truncated);
    }

    /// The failure sits past the page, so whether it is required is unknown;
    /// the rollup's verdict stands rather than being read as a pass.
    #[test]
    fn a_failing_rollup_whose_failure_is_past_the_page_still_raises_ci() {
        let pr = row(json!({ "commits": checked_commits(
            "FAILURE",
            vec![check_run("build", Some("SUCCESS"), true)],
            true,
        ) }));
        assert_eq!(pr.blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(pr.failing_checks, Vec::<String>::new());
    }

    /// The names belong to the CI pill; where the pill is held back, a
    /// check name with nothing to label would only invite a second reading.
    #[test]
    fn failing_check_names_are_held_back_with_the_ci_pill() {
        let failed = || {
            checked_commits(
                "FAILURE",
                vec![check_run("e2e-safari", Some("FAILURE"), true)],
                true,
            )
        };
        for held_back in [
            json!({ "mergeable": "UNKNOWN", "commits": failed() }),
            json!({ "isDraft": true, "commits": failed() }),
        ] {
            let pr = row(held_back);
            assert_eq!(pr.blocked_reasons, Vec::new());
            assert_eq!(pr.failing_checks, Vec::<String>::new());
            assert!(!pr.failing_checks_truncated);
        }
    }

    #[test]
    fn a_pending_rollup_without_check_detail_reads_as_running() {
        assert!(row(json!({ "commits": ci_commits("PENDING") })).checks_running);
        assert!(!row(json!({ "commits": ci_commits("SUCCESS") })).checks_running);
    }

    #[test]
    fn a_draft_shows_no_running_or_auto_merge_mark() {
        let pr = row(json!({
            "isDraft": true,
            "commits": ci_commits("PENDING"),
            "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" },
        }));
        assert!(!pr.checks_running);
        assert!(!pr.auto_merge);
    }

    #[test]
    fn armed_auto_merge_is_read_onto_the_row() {
        let armed = json!({ "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" } });
        assert!(row(armed).auto_merge);
        assert!(!row(json!({ "autoMergeRequest": null })).auto_merge);
    }

    /// One of the viewer's PRs with nothing between it and the merge button.
    fn ready_node(overrides: Value) -> Value {
        with(
            json!({
                "viewerDidAuthor": true,
                "mergeable": "MERGEABLE",
                "reviewDecision": "APPROVED",
                "commits": ci_commits("SUCCESS"),
            }),
            overrides,
        )
    }

    #[test]
    fn an_approved_mergeable_pr_of_the_viewers_with_nothing_in_the_way_is_ready() {
        assert!(row(ready_node(json!({}))).ready_to_merge);
    }

    #[test]
    fn unknown_mergeability_is_never_ready() {
        assert!(!row(ready_node(json!({ "mergeable": "UNKNOWN" }))).ready_to_merge);
    }

    /// A null decision means the repo asks for no review; an outstanding
    /// request shows the author asked for one anyway.
    #[test]
    fn without_required_reviews_a_pr_is_ready_once_no_request_is_outstanding() {
        assert!(row(ready_node(json!({ "reviewDecision": null }))).ready_to_merge);
        let asked = ready_node(json!({
            "reviewDecision": null,
            "reviewRequests": review_requests(&["someone"]),
        }));
        assert!(!row(asked).ready_to_merge);
        assert!(!row(ready_node(json!({ "reviewDecision": "REVIEW_REQUIRED" }))).ready_to_merge);
    }

    #[test]
    fn any_blocked_reason_keeps_an_approved_pr_from_ready() {
        let unresolved = ready_node(json!({ "reviewThreads": review_threads(&[false]) }));
        assert!(!row(unresolved).ready_to_merge);
        let behind = ready_node(json!({ "mergeStateStatus": "BEHIND" }));
        assert!(!row(behind).ready_to_merge);
    }

    #[test]
    fn a_required_check_still_running_keeps_a_pr_from_ready() {
        let running = ready_node(json!({ "commits": checked_commits(
            "PENDING",
            vec![check_run("e2e-safari", None, true)],
            false,
        ) }));
        assert!(!row(running).ready_to_merge);
    }

    /// Optional checks do not gate the merge, so neither their failure nor
    /// their progress holds a PR back.
    #[test]
    fn optional_checks_do_not_keep_a_pr_from_ready() {
        let optional = ready_node(json!({ "commits": checked_commits(
            "FAILURE",
            vec![
                check_run("build", Some("SUCCESS"), true),
                check_run("lint-optional", Some("FAILURE"), false),
                check_run("preview", None, false),
            ],
            false,
        ) }));
        assert!(row(optional).ready_to_merge);
    }

    /// Queued or armed, the merge is already under way: there is nothing
    /// left for the viewer to press.
    #[test]
    fn a_queued_or_auto_merging_pr_is_not_ready() {
        let queued = ready_node(json!({ "mergeQueueEntry": { "position": 1 } }));
        assert!(!row(queued).ready_to_merge);
        let armed = ready_node(json!({
            "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" }
        }));
        assert!(!row(armed).ready_to_merge);
    }

    #[test]
    fn only_a_pr_in_mine_is_ever_ready() {
        let participated = ready_node(json!({
            "viewerDidAuthor": false,
            "viewerSubscription": "SUBSCRIBED",
        }));
        assert!(!row(participated).ready_to_merge);
        assert!(!row(ready_node(json!({ "viewerDidAuthor": false }))).ready_to_merge);
    }

    #[test]
    fn a_draft_is_never_ready() {
        assert!(!row(ready_node(json!({ "isDraft": true }))).ready_to_merge);
    }

    /// The viewer's own PR, as the paged query returns one with nothing
    /// decided yet.
    fn mine_node(overrides: Value) -> Value {
        with(
            json!({ "viewerDidAuthor": true, "mergeable": "MERGEABLE" }),
            overrides,
        )
    }

    /// Someone else's PR the viewer is subscribed to.
    fn participated_node(overrides: Value) -> Value {
        with(json!({ "viewerSubscription": "SUBSCRIBED" }), overrides)
    }

    fn turn(overrides: Value) -> Option<Turn> {
        row(overrides).turn
    }

    #[test]
    fn a_review_requested_from_the_viewer_by_name_is_the_viewers_turn_in_any_section() {
        let asked = json!({ "reviewRequests": review_requests(&["khiet"]) });
        assert_eq!(turn(participated_node(asked.clone())), Some(Turn::Yours));
        // An agent's PR in Mine can ask the viewer too.
        assert_eq!(turn(mine_node(asked)), Some(Turn::Yours));
    }

    #[test]
    fn a_draft_has_no_turn() {
        let blocked = mine_node(json!({ "isDraft": true, "mergeable": "CONFLICTING" }));
        assert_eq!(turn(blocked), None);
    }

    #[test]
    fn a_draft_asking_the_viewer_by_name_has_no_turn() {
        let asked = participated_node(json!({
            "isDraft": true,
            "reviewRequests": review_requests(&["khiet"]),
        }));
        assert_eq!(turn(asked), None);
    }

    #[test]
    fn a_pr_in_all_has_no_turn() {
        let reviewed = json!({ "viewerLatestReview": { "state": "APPROVED" } });
        assert_eq!(row(reviewed.clone()).section, Section::All);
        assert_eq!(turn(reviewed), None);
    }

    #[test]
    fn a_review_requested_from_the_viewers_team_alone_leaves_all_without_a_turn() {
        let team = json!({ "reviewRequests": { "nodes": [{ "requestedReviewer": {} }] } });
        assert_eq!(row(team.clone()).section, Section::All);
        assert_eq!(turn(team), None);
    }

    #[test]
    fn a_blocked_pr_of_the_viewers_is_the_viewers_turn() {
        let conflict = mine_node(json!({ "mergeable": "CONFLICTING" }));
        assert_eq!(turn(conflict), Some(Turn::Yours));
        let changes = mine_node(json!({ "reviewDecision": "CHANGES_REQUESTED" }));
        assert_eq!(turn(changes), Some(Turn::Yours));
        let behind = mine_node(json!({ "mergeStateStatus": "BEHIND" }));
        assert_eq!(turn(behind), Some(Turn::Yours));
        let failing = mine_node(json!({ "commits": ci_commits("FAILURE") }));
        assert_eq!(turn(failing), Some(Turn::Yours));
        let threads = mine_node(json!({ "reviewThreads": review_threads(&[false]) }));
        assert_eq!(turn(threads), Some(Turn::Yours));
    }

    #[test]
    fn a_queued_or_auto_merging_pr_of_the_viewers_waits_on_others() {
        let queued = ready_node(json!({ "mergeQueueEntry": { "position": 1 } }));
        assert_eq!(turn(queued), Some(Turn::Theirs));
        let armed = ready_node(json!({
            "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" }
        }));
        assert_eq!(turn(armed), Some(Turn::Theirs));
    }

    #[test]
    fn a_ready_pr_is_the_viewers_turn() {
        assert_eq!(turn(ready_node(json!({}))), Some(Turn::Yours));
    }

    #[test]
    fn a_pr_of_the_viewers_with_a_reviewer_asked_or_checks_running_waits_on_others() {
        let asked = mine_node(json!({ "reviewRequests": review_requests(&["someone"]) }));
        assert_eq!(turn(asked), Some(Turn::Theirs));
        let running = mine_node(json!({ "commits": ci_commits("PENDING") }));
        assert_eq!(turn(running), Some(Turn::Theirs));
    }

    #[test]
    fn a_pr_the_viewer_reviewed_with_no_new_request_waits_on_others() {
        let approved = participated_node(json!({
            "reviewDecision": "APPROVED",
            "viewerLatestReview": { "state": "APPROVED" },
        }));
        assert_eq!(turn(approved), Some(Turn::Theirs));
        let commented = participated_node(json!({
            "viewerLatestReview": { "state": "COMMENTED" },
        }));
        assert_eq!(turn(commented), Some(Turn::Theirs));
    }

    #[test]
    fn a_pr_the_viewer_approved_that_asks_them_again_is_the_viewers_turn() {
        let asked_again = participated_node(json!({
            "reviewDecision": "APPROVED",
            "viewerLatestReview": { "state": "APPROVED" },
            "reviewRequests": review_requests(&["khiet"]),
        }));
        assert_eq!(turn(asked_again), Some(Turn::Yours));
    }

    /// Nobody else can see a review until it is submitted, and a dismissed
    /// one no longer stands.
    #[test]
    fn a_review_the_viewer_has_not_submitted_or_had_dismissed_hands_nothing_back() {
        let drafting = participated_node(json!({
            "viewerLatestReview": { "state": "PENDING" },
        }));
        assert_eq!(turn(drafting), None);
        let dismissed = participated_node(json!({
            "viewerLatestReview": { "state": "DISMISSED" },
        }));
        assert_eq!(turn(dismissed), None);
    }

    #[test]
    fn a_pr_with_nothing_decided_has_no_turn() {
        // A review is required and nobody has been asked for one.
        let unasked = mine_node(json!({ "reviewDecision": "REVIEW_REQUIRED" }));
        assert_eq!(turn(unasked), None);
        assert_eq!(turn(participated_node(json!({}))), None);
    }

    /// A blocked reason outranks a merge under way.
    #[test]
    fn a_blocked_pr_is_the_viewers_turn_even_with_its_merge_under_way() {
        let failing_and_armed = mine_node(json!({
            "commits": checked_commits(
                "FAILURE",
                vec![check_run("e2e", Some("FAILURE"), true)],
                false,
            ),
            "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" },
        }));
        assert_eq!(turn(failing_and_armed), Some(Turn::Yours));
        let conflicting_and_queued = mine_node(json!({
            "mergeable": "CONFLICTING",
            "mergeQueueEntry": { "position": 1 },
        }));
        assert_eq!(turn(conflicting_and_queued), Some(Turn::Yours));
    }

    /// GitHub has not settled mergeability, so the reasons are withheld and
    /// with them the verdict they would give.
    #[test]
    fn an_unknown_mergeability_withholds_the_turn_a_blocked_reason_would_give() {
        let unsettled = mine_node(json!({
            "mergeable": "UNKNOWN",
            "reviewDecision": "CHANGES_REQUESTED",
            "reviewThreads": review_threads(&[false]),
        }));
        assert_eq!(turn(unsettled), None);
    }

    /// The open thread makes it the viewer's, not the approval: the row is
    /// not ready, so its blocked pill is the mark that says why.
    #[test]
    fn an_approved_pr_with_an_unresolved_thread_is_the_viewers_turn_as_blocked() {
        let pr = row(ready_node(
            json!({ "reviewThreads": review_threads(&[false]) }),
        ));
        assert_eq!(pr.turn, Some(Turn::Yours));
        assert_eq!(pr.blocked_reasons, [BlockedReason::Threads]);
        assert!(!pr.ready_to_merge);
    }

    #[test]
    fn the_paged_query_asks_for_the_viewers_latest_review() {
        assert!(repo_field("r0", "acme", "widgets", None, true)
            .contains("viewerLatestReview { state }"));
    }

    #[test]
    fn turn_variants_serialize_lowercase() {
        assert_eq!(serde_json::to_value(Turn::Yours).unwrap(), json!("yours"));
        assert_eq!(serde_json::to_value(Turn::Theirs).unwrap(), json!("theirs"));
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
            (BlockedReason::Threads, "threads"),
            (BlockedReason::Behind, "behind"),
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
                    pr_node(json!({ "commits": ci_commits("FAILURE") })),
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
    fn a_requested_review_is_read_onto_the_row() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [
                    pr_node(json!({
                        "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
                    })),
                    pr_node(json!({})),
                ]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].review_requested);
        assert!(!out[1].review_requested);
    }

    #[test]
    fn a_draft_pr_never_flags_review_requested_on_the_row() {
        // The request still stands, but a draft is the author's not-ready
        // choice, so its row marker is suppressed like the blocked pills.
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [pr_node(json!({
                    "isDraft": true,
                    "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "khiet" } }] }
                }))]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].is_draft);
        assert!(!out[0].review_requested);
    }

    #[test]
    fn an_awaiting_review_is_read_onto_a_mine_row() {
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [
                    pr_node(json!({
                        "viewerDidAuthor": true,
                        "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "someone" } }] }
                    })),
                    pr_node(json!({ "viewerDidAuthor": true })),
                ]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].awaiting_review);
        assert!(!out[1].awaiting_review);
    }

    #[test]
    fn awaiting_review_is_gated_to_your_own_prs() {
        // A PR the viewer merely takes part in still carries an outstanding
        // request (for someone else), so the helper matches, but the outgoing
        // marker is for your own PRs and must stay off here. The viewer is
        // subscribed, not the requested reviewer, so no incoming glyph either:
        // this isolates the Mine gate on its own.
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [pr_node(json!({
                    "viewerSubscription": "SUBSCRIBED",
                    "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "someone" } }] }
                }))]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(out[0].section, Section::Participated);
        assert!(!out[0].review_requested);
        assert!(!out[0].awaiting_review);
    }

    #[test]
    fn a_draft_mine_pr_never_flags_awaiting_review() {
        // Matches review_requested and the blocked pills: a draft is the author's
        // not-ready choice, so its row marker is suppressed while the request
        // still stands.
        let repo = json!({
            "nameWithOwner": "acme/widgets",
            "pullRequests": {
                "pageInfo": { "hasNextPage": false, "endCursor": null },
                "nodes": [pr_node(json!({
                    "viewerDidAuthor": true,
                    "isDraft": true,
                    "reviewRequests": { "nodes": [{ "requestedReviewer": { "login": "someone" } }] }
                }))]
            }
        });
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].is_draft);
        assert!(!out[0].awaiting_review);
    }

    #[test]
    fn an_explicit_approval_is_read_onto_the_row() {
        // Only APPROVED sets the flag: a null decision (repo without required
        // reviews, or review still in progress) stays unmarked.
        let repo = repo_with(vec![
            pr_node(json!({ "reviewDecision": "APPROVED" })),
            pr_node(json!({})),
            pr_node(json!({ "reviewDecision": "REVIEW_REQUIRED" })),
        ]);
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].approved);
        assert!(!out[1].approved);
        assert!(!out[2].approved);
    }

    #[test]
    fn an_approval_shows_the_check_even_beside_a_pending_request() {
        // GitHub keeps reviewDecision at APPROVED once the required approvals
        // are in, even while more reviewers are still listed; the met
        // requirement shows regardless, so the check and a glasses glyph can
        // share a row. Both directions: the viewer's own APPROVED PR still
        // waiting on another reviewer, and an APPROVED PR the viewer is still
        // asked to review.
        let repo = repo_with(vec![
            pr_node(json!({
                "viewerDidAuthor": true,
                "reviewDecision": "APPROVED",
                "reviewRequests": { "nodes": [
                    { "requestedReviewer": { "login": "other" } }
                ] }
            })),
            pr_node(json!({
                "reviewDecision": "APPROVED",
                "reviewRequests": { "nodes": [
                    { "requestedReviewer": { "login": "khiet" } }
                ] }
            })),
        ]);
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].awaiting_review);
        assert!(out[0].approved);
        assert!(out[1].review_requested);
        assert!(out[1].approved);
    }

    #[test]
    fn a_queued_pr_carries_its_merge_queue_position() {
        // GitHub reports a 1-based position and a null entry for a PR that
        // is not queued; an absent field reads the same as null.
        let repo = repo_with(vec![
            pr_node(json!({ "mergeQueueEntry": { "position": 3 } })),
            pr_node(json!({ "mergeQueueEntry": null })),
            pr_node(json!({})),
        ]);
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert_eq!(out[0].merge_queue_position, Some(3));
        assert_eq!(out[1].merge_queue_position, None);
        assert_eq!(out[2].merge_queue_position, None);
    }

    #[test]
    fn a_draft_never_shows_the_approved_check() {
        // Matches the other markers: an approval left on a PR later flipped
        // back to draft must not read as "ship it" while the author says
        // not-ready.
        let repo = repo_with(vec![pr_node(json!({
            "isDraft": true,
            "reviewDecision": "APPROVED"
        }))]);
        let mut out = Vec::new();
        collect_repo_prs(&repo, "khiet", &mut out);
        assert!(out[0].is_draft);
        assert!(!out[0].approved);
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
            collect_activity(&node, "khiet").human,
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
            collect_activity(&node, "khiet").human,
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
            collect_activity(&node, "khiet").human,
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
            collect_activity(&node, "khiet").human,
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
            collect_activity(&node, "khiet").human,
            vec!["2026-07-11T08:00:00Z", "2026-07-12T09:00:00Z"]
        );
    }

    #[test]
    fn commits_and_status_changes_never_reach_the_activity_list() {
        // CI, mergeability, review decision, and draftness are all fetched
        // (for the row's blocked pills and draft mark) but deliberately excluded
        // from activity: they are properties of the PR, not unread events. A
        // node with neither a comment nor a review yields nothing, whatever
        // those fields say.
        let node = pr_node(json!({
            "isDraft": false,
            "mergeable": "CONFLICTING",
            "commits": ci_commits("FAILURE"),
            "reviewDecision": "CHANGES_REQUESTED"
        }));
        assert_eq!(collect_activity(&node, "khiet"), Activity::default());
    }

    #[test]
    fn repo_field_quotes_arguments_and_resumes_from_a_cursor() {
        let field = repo_field("r0", "acme", "widgets", Some("abc"), true);
        assert!(field.contains(r#"repository(owner: "acme", name: "widgets")"#));
        assert!(field.contains(r#"after: "abc""#));
        assert!(!repo_field("r0", "acme", "widgets", None, true).contains("after:"));
    }

    /// `ownerAffiliations` defaults to [OWNER, COLLABORATOR]; if the query
    /// ever drops the explicit argument, org-member repos silently vanish
    /// from the browse list.
    #[test]
    fn the_affiliated_query_sets_both_affiliation_arguments() {
        assert!(AFFILIATED_REPOS_QUERY.contains("affiliations: [OWNER, ORGANIZATION_MEMBER]"));
        assert!(AFFILIATED_REPOS_QUERY.contains("ownerAffiliations: [OWNER, ORGANIZATION_MEMBER]"));
        assert!(AFFILIATED_REPOS_QUERY.contains("isArchived"));
        assert!(AFFILIATED_REPOS_QUERY.contains("nameWithOwner"));
    }

    fn repo_page(nodes: Value, page_info: Value) -> Value {
        json!({ "viewer": { "repositories": { "nodes": nodes, "pageInfo": page_info } } })
    }

    #[test]
    fn affiliated_page_collects_names_and_skips_archived() {
        let data = repo_page(
            json!([
                { "nameWithOwner": "acme/widgets", "isArchived": false },
                { "nameWithOwner": "acme/attic", "isArchived": true },
                // Missing isArchived is treated as live, not dropped.
                { "nameWithOwner": "khietle/dotfiles" }
            ]),
            json!({ "hasNextPage": false, "endCursor": "c1" }),
        );
        let mut out = Vec::new();
        assert_eq!(collect_affiliated_page(&data, &mut out), None);
        assert_eq!(out, vec!["acme/widgets", "khietle/dotfiles"]);
    }

    #[test]
    fn affiliated_page_returns_the_resume_cursor_only_when_more_pages_remain() {
        let more = repo_page(json!([]), json!({ "hasNextPage": true, "endCursor": "c2" }));
        let mut out = Vec::new();
        assert_eq!(
            collect_affiliated_page(&more, &mut out),
            Some("c2".to_string())
        );
        let last = repo_page(
            json!([]),
            json!({ "hasNextPage": false, "endCursor": "c2" }),
        );
        assert_eq!(collect_affiliated_page(&last, &mut out), None);
    }

    #[test]
    fn affiliated_page_tolerates_malformed_responses() {
        let mut out = Vec::new();
        // No pageInfo: no cursor, and no panic.
        let no_page = json!({ "viewer": { "repositories": {
            "nodes": [{ "nameWithOwner": "acme/widgets" }]
        } } });
        assert_eq!(collect_affiliated_page(&no_page, &mut out), None);
        assert_eq!(out, vec!["acme/widgets"]);
        // Null nodes and a node with no name: skipped.
        let sparse = repo_page(json!(null), json!({ "hasNextPage": false }));
        assert_eq!(collect_affiliated_page(&sparse, &mut out), None);
        assert_eq!(out.len(), 1);
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
        assert_eq!(
            rate_limit_reset(&headers(&[("retry-after", "60")]), 1_000),
            Some(1_060)
        );
        assert_eq!(
            rate_limit_reset(
                &headers(&[("retry-after", "60"), ("x-ratelimit-reset", "1784205296")]),
                1_000
            ),
            Some(1_060)
        );
    }

    #[test]
    fn the_reset_header_is_read_as_an_absolute_epoch() {
        assert_eq!(
            rate_limit_reset(&headers(&[("x-ratelimit-reset", "1784205296")]), 1_000),
            Some(1_784_205_296)
        );
    }

    /// No reset time means the caller falls back to its own backoff.
    #[test]
    fn missing_or_unparsable_reset_headers_report_no_reset_time() {
        assert_eq!(rate_limit_reset(&headers(&[]), 1_000), None);
        assert_eq!(
            rate_limit_reset(&headers(&[("x-ratelimit-reset", "soon")]), 1_000),
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

    // Fetch-level tests: each operation runs on a `Client` over canned
    // responses, so pagination, status mapping and the query itself are
    // under test together. Only `ReqwestTransport` and the one-line public
    // wrappers stay outside them.

    /// Stands in for GitHub: answers each request with the next canned
    /// response and records the payload it was sent. Like GitHub, it answers
    /// the query it was asked (see [`keep_asked`]), so a field or window
    /// dropped from a query changes the answer and fails the test reading it.
    struct Canned {
        responses: std::sync::Mutex<std::collections::VecDeque<HttpResponse>>,
        payloads: std::sync::Mutex<Vec<Value>>,
    }

    impl Canned {
        fn new(responses: Vec<HttpResponse>) -> Self {
            Canned {
                responses: std::sync::Mutex::new(responses.into()),
                payloads: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn client(&self) -> Client<'_, Canned> {
            Client {
                transport: self,
                token: "token",
                now_epoch_secs: || 1_000,
            }
        }

        fn request_count(&self) -> usize {
            self.payloads.lock().unwrap().len()
        }

        fn query(&self, index: usize) -> String {
            self.payloads.lock().unwrap()[index]["query"]
                .as_str()
                .unwrap()
                .to_string()
        }

        fn variables(&self, index: usize) -> Value {
            self.payloads.lock().unwrap()[index]["variables"].clone()
        }
    }

    impl Transport for Canned {
        async fn post(&self, _token: &str, payload: &Value) -> Result<HttpResponse, GithubError> {
            let mut response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("a request beyond the canned responses");
            if let Ok(mut body) = serde_json::from_str::<Value>(&response.body) {
                if let Some(data) = body.get_mut("data") {
                    keep_asked(data, &parse_query(payload["query"].as_str().unwrap()));
                }
                response.body = body.to_string();
            }
            self.payloads.lock().unwrap().push(payload.clone());
            Ok(response)
        }
    }

    /// What a query asks of one field: the window its arguments put on a
    /// connection, and its sub-selection keyed by response name (the alias
    /// when the field has one).
    #[derive(Default)]
    struct Asked {
        window: Option<Window>,
        fields: HashMap<String, Asked>,
    }

    enum Window {
        First(usize),
        Last(usize),
    }

    /// Cuts canned `data` down to what the query selects: a key survives only
    /// where the query asks for it, and a connection's `nodes` are windowed
    /// by its `first:` or `last:` argument. Other arguments are ignored;
    /// canned data is already the right repository and page.
    fn keep_asked(value: &mut Value, asked: &Asked) {
        match value {
            Value::Object(map) => {
                map.retain(|key, _| asked.fields.contains_key(key));
                for (key, child) in map.iter_mut() {
                    keep_asked(child, &asked.fields[key]);
                }
                if let Some(Value::Array(nodes)) = map.get_mut("nodes") {
                    match asked.window {
                        Some(Window::First(n)) => nodes.truncate(n),
                        Some(Window::Last(n)) => {
                            nodes.drain(..nodes.len().saturating_sub(n));
                        }
                        None => {}
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| keep_asked(item, asked)),
            _ => {}
        }
    }

    type QueryChars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

    /// Reads the selection out of the GraphQL subset this module writes:
    /// one anonymous `query` or `mutation`, aliases, arguments, and inline
    /// fragments, whose fields count as the enclosing field's own.
    fn parse_query(query: &str) -> Asked {
        let mut chars = query.chars().peekable();
        skip_space(&mut chars);
        let operation = word(&mut chars);
        assert!(
            operation == "query" || operation == "mutation",
            "not an operation: {query}"
        );
        skip_space(&mut chars);
        if chars.peek() == Some(&'(') {
            arguments(&mut chars);
            skip_space(&mut chars);
        }
        assert_eq!(chars.next(), Some('{'), "query has no selection: {query}");
        Asked {
            window: None,
            fields: selection(&mut chars),
        }
    }

    /// Parses up to the `}` closing a selection whose `{` is already read.
    fn selection(chars: &mut QueryChars) -> HashMap<String, Asked> {
        let mut fields = HashMap::new();
        loop {
            skip_space(chars);
            match chars.peek() {
                None => panic!("query ended inside a selection"),
                Some('}') => {
                    chars.next();
                    return fields;
                }
                Some('.') => {
                    while chars.next_if_eq(&'.').is_some() {}
                    skip_space(chars);
                    assert_eq!(word(chars), "on");
                    skip_space(chars);
                    word(chars);
                    skip_space(chars);
                    assert_eq!(chars.next(), Some('{'));
                    fields.extend(selection(chars));
                }
                Some(_) => {
                    let response_name = word(chars);
                    assert!(!response_name.is_empty(), "unexpected {:?}", chars.peek());
                    skip_space(chars);
                    if chars.next_if_eq(&':').is_some() {
                        skip_space(chars);
                        word(chars);
                        skip_space(chars);
                    }
                    let window = if chars.peek() == Some(&'(') {
                        window(&arguments(chars))
                    } else {
                        None
                    };
                    skip_space(chars);
                    let fields_below = if chars.next_if_eq(&'{').is_some() {
                        selection(chars)
                    } else {
                        HashMap::new()
                    };
                    fields.insert(
                        response_name,
                        Asked {
                            window,
                            fields: fields_below,
                        },
                    );
                }
            }
        }
    }

    fn skip_space(chars: &mut QueryChars) {
        while chars.next_if(|c| c.is_whitespace() || *c == ',').is_some() {}
    }

    fn word(chars: &mut QueryChars) -> String {
        let mut word = String::new();
        while let Some(c) = chars.next_if(|c| c.is_ascii_alphanumeric() || *c == '_') {
            word.push(c);
        }
        word
    }

    /// Consumes an argument list starting at its `(` and returns its text.
    fn arguments(chars: &mut QueryChars) -> String {
        let mut text = String::new();
        let (mut depth, mut quoted) = (0, false);
        for c in chars.by_ref() {
            match c {
                '"' => quoted = !quoted,
                '(' if !quoted => depth += 1,
                ')' if !quoted => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            text.push(c);
        }
        text
    }

    fn window(arguments: &str) -> Option<Window> {
        let count = |name: &str| {
            let after = &arguments[arguments.find(name)? + name.len()..];
            let digits: String = after
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse::<usize>().ok()
        };
        count("first:")
            .map(Window::First)
            .or_else(|| count("last:").map(Window::Last))
    }

    fn run<F: std::future::Future>(future: F) -> F::Output {
        tauri::async_runtime::block_on(future)
    }

    fn answer(status: u16, header_pairs: &[(&str, &str)], body: Value) -> HttpResponse {
        HttpResponse {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            headers: headers(header_pairs),
            body: body.to_string(),
        }
    }

    fn ok(data: Value) -> HttpResponse {
        answer(200, &[], json!({ "data": data }))
    }

    /// One repo's page of PRs; `next` is the cursor of the page after it.
    fn repo_named(name: &str, nodes: Vec<Value>, next: Option<&str>) -> Value {
        let mut repo = repo_with(nodes);
        repo["nameWithOwner"] = json!(name);
        repo["pullRequests"]["pageInfo"] =
            json!({ "hasNextPage": next.is_some(), "endCursor": next });
        repo
    }

    /// The `data` of one open-PR round: `repos` answer aliases `r0`, `r1`, ...
    /// in order; a `Value::Null` entry is a repository GitHub could not
    /// resolve.
    fn prs_round(remaining: u64, repos: Vec<Value>) -> Value {
        let mut data = json!({
            "viewer": { "login": "khiet" },
            "rateLimit": { "remaining": remaining, "resetAt": "1970-01-01T01:00:00Z" }
        });
        for (i, repo) in repos.into_iter().enumerate() {
            data[format!("r{i}")] = repo;
        }
        data
    }

    fn fetch_open(github: &Canned, repos: &[&str]) -> Result<FetchedPrs, GithubError> {
        let repos: Vec<String> = repos.iter().map(|r| r.to_string()).collect();
        run(github.client().fetch_open_prs(&repos))
    }

    /// (repo, number) of each fetched PR, in the order returned.
    fn rows(fetched: &FetchedPrs) -> Vec<(&str, u64)> {
        fetched
            .prs
            .iter()
            .map(|pr| (pr.repo.as_str(), pr.number))
            .collect()
    }

    /// Aliases are renumbered over the repos still pending, so in the second
    /// round `r0` is the repo that was `r1` in the first. Resuming the wrong
    /// repo, or the right one without its cursor, would refetch page one.
    #[test]
    fn a_repo_with_more_pages_is_resumed_alone_from_its_cursor() {
        let github = Canned::new(vec![
            ok(prs_round(
                5000,
                vec![
                    repo_named(
                        "acme/widgets",
                        vec![pr_node(
                            json!({ "number": 1, "updatedAt": "2026-07-13T00:00:00Z" }),
                        )],
                        None,
                    ),
                    repo_named(
                        "acme/gadgets",
                        vec![pr_node(
                            json!({ "number": 2, "updatedAt": "2026-07-12T00:00:00Z" }),
                        )],
                        Some("g1"),
                    ),
                ],
            )),
            ok(prs_round(
                4999,
                vec![repo_named(
                    "acme/gadgets",
                    vec![pr_node(
                        json!({ "number": 3, "updatedAt": "2026-07-11T00:00:00Z" }),
                    )],
                    None,
                )],
            )),
        ]);

        let fetched = fetch_open(&github, &["acme/widgets", "acme/gadgets"]).unwrap();

        assert_eq!(
            rows(&fetched),
            vec![
                ("acme/widgets", 1),
                ("acme/gadgets", 2),
                ("acme/gadgets", 3)
            ]
        );
        assert_eq!(github.request_count(), 2);
        let first = github.query(0);
        assert!(first.contains(r#"r0: repository(owner: "acme", name: "widgets")"#));
        assert!(first.contains(r#"r1: repository(owner: "acme", name: "gadgets")"#));
        assert!(!first.contains("after:"));
        let second = github.query(1);
        assert!(second.contains(r#"r0: repository(owner: "acme", name: "gadgets")"#));
        assert!(second.contains(r#"after: "g1""#));
        assert!(!second.contains("widgets"));
    }

    /// A repo that never stops reporting another page must not keep the sync
    /// spinning: the fetch gives up after 20 requests.
    #[test]
    fn a_fetch_that_never_reaches_a_last_page_stops_after_twenty_rounds() {
        let endless = || {
            ok(prs_round(
                5000,
                vec![repo_named(
                    "acme/widgets",
                    vec![pr_node(json!({}))],
                    Some("more"),
                )],
            ))
        };
        let github = Canned::new((0..20).map(|_| endless()).collect());

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::Other(
                "GitHub returned too many pages of pull requests.".into()
            ))
        );
        assert_eq!(github.request_count(), 20);
    }

    /// GitHub answers a deleted or newly private repository with a null alias
    /// and a NOT_FOUND error beside the data it could resolve.
    #[test]
    fn a_watched_repo_that_no_longer_exists_is_skipped() {
        let github = Canned::new(vec![answer(
            200,
            &[],
            json!({
                "data": prs_round(
                    5000,
                    vec![
                        Value::Null,
                        repo_named("acme/gadgets", vec![pr_node(json!({ "number": 2 }))], None),
                    ],
                ),
                "errors": [{ "type": "NOT_FOUND", "message": "Could not resolve to a Repository." }]
            }),
        )]);

        let fetched = fetch_open(&github, &["acme/gone", "acme/gadgets"]).unwrap();

        assert_eq!(rows(&fetched), vec![("acme/gadgets", 2)]);
        assert_eq!(github.request_count(), 1);
    }

    /// The sync loop plans its next poll around the budget left after the
    /// whole fetch, which is what the final request reported.
    #[test]
    fn the_rate_limit_reported_is_the_one_from_the_last_page() {
        let github = Canned::new(vec![
            ok(prs_round(
                4000,
                vec![repo_named(
                    "acme/widgets",
                    vec![pr_node(json!({}))],
                    Some("w1"),
                )],
            )),
            ok(prs_round(
                3999,
                vec![repo_named("acme/widgets", vec![pr_node(json!({}))], None)],
            )),
        ]);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(
            fetched.rate_limit,
            Some(RateLimit {
                remaining: 3999,
                reset_epoch_secs: Some(3600),
            })
        );
    }

    #[test]
    fn prs_from_several_repos_come_back_most_recently_updated_first() {
        let updated = |number: u64, at: &str| pr_node(json!({ "number": number, "updatedAt": at }));
        let github = Canned::new(vec![ok(prs_round(
            5000,
            vec![
                repo_named(
                    "acme/widgets",
                    vec![
                        updated(1, "2026-07-11T11:00:00Z"),
                        updated(2, "2026-07-11T09:00:00Z"),
                    ],
                    None,
                ),
                repo_named(
                    "acme/gadgets",
                    vec![
                        updated(3, "2026-07-11T12:00:00Z"),
                        updated(4, "2026-07-11T10:00:00Z"),
                    ],
                    None,
                ),
            ],
        ))]);

        let fetched = fetch_open(&github, &["acme/widgets", "acme/gadgets"]).unwrap();

        assert_eq!(
            rows(&fetched),
            vec![
                ("acme/gadgets", 3),
                ("acme/widgets", 1),
                ("acme/gadgets", 4),
                ("acme/widgets", 2),
            ]
        );
    }

    /// The marker fields ride the PR query the sync already makes, so the row
    /// only shows them if that query asks for each one.
    #[test]
    fn a_fetched_row_carries_its_queue_position_and_blocked_state() {
        let github = Canned::new(vec![
            ok(prs_round(
                5000,
                vec![repo_named(
                    "acme/widgets",
                    vec![
                        pr_node(json!({
                            "number": 1,
                            "updatedAt": "2026-07-11T03:00:00Z",
                            "mergeQueueEntry": { "position": 3 },
                            "mergeable": "CONFLICTING",
                            "commits": ci_commits("FAILURE"),
                            "reviewDecision": "CHANGES_REQUESTED",
                            "reviewThreads": review_threads(&[false]),
                        })),
                        pr_node(json!({
                            "number": 2,
                            "updatedAt": "2026-07-11T02:00:00Z",
                            "mergeStateStatus": "BEHIND",
                        })),
                        pr_node(json!({
                            "number": 3,
                            "updatedAt": "2026-07-11T01:00:00Z",
                            "isDraft": true,
                        })),
                    ],
                    None,
                )],
            )),
            ok(checks_answer(4999, ci_commits("FAILURE"))),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].merge_queue_position, Some(3));
        assert_eq!(
            prs[0].blocked_reasons,
            vec![
                BlockedReason::Conflict,
                BlockedReason::Ci,
                BlockedReason::Review,
                BlockedReason::Threads,
            ]
        );
        assert_eq!(prs[1].blocked_reasons, vec![BlockedReason::Behind]);
        assert!(prs[2].is_draft);
    }

    /// The CI rollup is per commit, so the query must window `commits` to the
    /// head: an older commit's failure says nothing about the PR now.
    #[test]
    fn ci_is_read_off_the_head_commit() {
        let history = |states: &[&str]| {
            let nodes: Vec<Value> = states
                .iter()
                .map(|state| json!({ "commit": { "statusCheckRollup": { "state": state } } }))
                .collect();
            json!({ "nodes": nodes })
        };
        let github = Canned::new(vec![
            ok(prs_round(
                5000,
                vec![repo_named(
                    "acme/widgets",
                    vec![
                        pr_node(json!({
                            "number": 1,
                            "updatedAt": "2026-07-11T02:00:00Z",
                            "commits": history(&["SUCCESS", "FAILURE"]),
                        })),
                        pr_node(json!({
                            "number": 2,
                            "updatedAt": "2026-07-11T01:00:00Z",
                            "commits": history(&["FAILURE", "SUCCESS"]),
                        })),
                    ],
                    None,
                )],
            )),
            ok(checks_answer(4999, ci_commits("FAILURE"))),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(prs[1].blocked_reasons, vec![]);
    }

    /// The `data` answering one PR's check-detail query: its head commit
    /// with the individual checks.
    fn checks_answer(remaining: u64, commits: Value) -> Value {
        json!({
            "rateLimit": { "remaining": remaining, "resetAt": "1970-01-01T01:00:00Z" },
            "repository": { "pullRequest": { "commits": commits } }
        })
    }

    /// One open-PR round holding `nodes` in acme/widgets.
    fn widgets_round(nodes: Vec<Value>) -> HttpResponse {
        ok(prs_round(
            5000,
            vec![repo_named("acme/widgets", nodes, None)],
        ))
    }

    /// The rollup says only that something failed. Which check, and whether
    /// it is required, takes a query that can name the PR.
    #[test]
    fn a_failing_rollup_is_followed_up_for_the_checks_behind_it() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({
                "number": 12,
                "commits": ci_commits("FAILURE"),
            }))]),
            ok(checks_answer(
                4999,
                checked_commits(
                    "FAILURE",
                    vec![
                        check_run("lint-optional", Some("FAILURE"), false),
                        status_context("ci/legacy", "FAILURE", true),
                        check_run("e2e-safari", Some("FAILURE"), true),
                    ],
                    false,
                ),
            )),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(github.request_count(), 2);
        assert_eq!(
            github.variables(1),
            json!({ "owner": "acme", "name": "widgets", "number": 12 })
        );
        assert!(github
            .query(1)
            .contains("isRequired(pullRequestNumber: $number)"));
        assert_eq!(prs[0].blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(prs[0].failing_checks, vec!["ci/legacy", "e2e-safari"]);
        assert!(!prs[0].failing_checks_truncated);
    }

    #[test]
    fn a_pending_rollup_is_followed_up_for_whether_a_required_check_is_running() {
        let pending = |number: u64, updated: &str| {
            pr_node(json!({
                "number": number,
                "updatedAt": updated,
                "commits": ci_commits("PENDING"),
            }))
        };
        let github = Canned::new(vec![
            widgets_round(vec![
                pending(1, "2026-07-11T02:00:00Z"),
                pending(2, "2026-07-11T01:00:00Z"),
            ]),
            ok(checks_answer(
                4999,
                checked_commits("PENDING", vec![check_run("build", None, true)], false),
            )),
            ok(checks_answer(
                4998,
                checked_commits(
                    "PENDING",
                    vec![
                        check_run("build", Some("SUCCESS"), true),
                        check_run("preview", None, false),
                    ],
                    false,
                ),
            )),
        ]);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(github.variables(1)["number"], json!(1));
        assert_eq!(github.variables(2)["number"], json!(2));
        assert!(fetched.prs[0].checks_running);
        assert!(!fetched.prs[1].checks_running);
        // The budget the sync loop plans around is the one after the last
        // request, and the follow-ups come last.
        assert_eq!(fetched.rate_limit.unwrap().remaining, 4998);
    }

    /// Follow-ups are the sync's variable cost, so they go only where the
    /// rollup leaves a question: not to a passing PR, one with no checks, or
    /// a draft, which shows no check marks whatever they say.
    #[test]
    fn a_pr_whose_rollup_settles_it_costs_no_follow_up() {
        let github = Canned::new(vec![widgets_round(vec![
            pr_node(json!({ "number": 1, "commits": ci_commits("SUCCESS") })),
            pr_node(json!({ "number": 2 })),
            pr_node(json!({
                "number": 3,
                "isDraft": true,
                "commits": ci_commits("FAILURE"),
            })),
        ])]);

        fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(github.request_count(), 1);
    }

    #[test]
    fn a_pr_with_more_checks_than_one_page_reports_its_failures_truncated() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({ "commits": ci_commits("FAILURE") }))]),
            ok(checks_answer(
                4999,
                checked_commits(
                    "FAILURE",
                    vec![check_run("e2e-safari", Some("FAILURE"), true)],
                    true,
                ),
            )),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].failing_checks, vec!["e2e-safari"]);
        assert!(prs[0].failing_checks_truncated);
    }

    /// The check names are an extra on top of the rollup; a token GitHub will
    /// not show them to still gets the PR and the pill it had before.
    #[test]
    fn a_follow_up_github_declines_costs_only_the_check_detail() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({ "commits": ci_commits("FAILURE") }))]),
            answer(
                200,
                &[],
                json!({ "data": null, "errors": [{ "message": "Resource not accessible" }] }),
            ),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].blocked_reasons, vec![BlockedReason::Ci]);
        assert_eq!(prs[0].failing_checks, Vec::<String>::new());
    }

    /// A push can land between the page and the follow-up. The row then
    /// describes the newer commit throughout, not the old verdict beside
    /// the new checks.
    #[test]
    fn check_detail_for_a_newer_commit_replaces_the_pages_verdict() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({ "commits": ci_commits("FAILURE") }))]),
            ok(checks_answer(
                4999,
                checked_commits(
                    "SUCCESS",
                    vec![check_run("e2e-safari", Some("SUCCESS"), true)],
                    false,
                ),
            )),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].blocked_reasons, Vec::new());
    }

    /// Merged or closed between the two queries: there is nothing to add,
    /// and the row keeps what the page said until the next sync drops it.
    #[test]
    fn a_pr_gone_by_the_follow_up_keeps_the_rollups_reading() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({ "commits": ci_commits("FAILURE") }))]),
            ok(json!({
                "rateLimit": { "remaining": 4999, "resetAt": "1970-01-01T01:00:00Z" },
                "repository": { "pullRequest": null }
            })),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].blocked_reasons, vec![BlockedReason::Ci]);
    }

    /// Out of quota is not a refusal to skip past: the sync loop has to hear
    /// of it to back off.
    #[test]
    fn a_rate_limited_follow_up_fails_the_fetch_as_rate_limited() {
        let github = Canned::new(vec![
            widgets_round(vec![pr_node(json!({ "commits": ci_commits("FAILURE") }))]),
            answer(
                200,
                &[],
                json!({
                    "data": null,
                    "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }]
                }),
            ),
        ]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::RateLimited {
                reset_epoch_secs: None
            })
        );
    }

    /// Auto-merge rides the paged query, so the row shows it only if that
    /// query asks.
    #[test]
    fn a_fetched_row_carries_armed_auto_merge() {
        let github = Canned::new(vec![widgets_round(vec![pr_node(json!({
            "autoMergeRequest": { "enabledAt": "2026-07-11T09:00:00Z" },
        }))])]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert!(prs[0].auto_merge);
    }

    /// The query asks for the newest ten threads, where the open ones skew;
    /// an unresolved thread older than that is out of sight.
    #[test]
    fn only_the_newest_ten_review_threads_can_flag_a_row() {
        let mut out_of_window = vec![false];
        out_of_window.extend([true; 10]);
        let mut in_window = vec![true, false];
        in_window.extend([true; 9]);
        let github = Canned::new(vec![ok(prs_round(
            5000,
            vec![repo_named(
                "acme/widgets",
                vec![
                    pr_node(json!({
                        "number": 1,
                        "updatedAt": "2026-07-11T02:00:00Z",
                        "reviewThreads": review_threads(&out_of_window),
                    })),
                    pr_node(json!({
                        "number": 2,
                        "updatedAt": "2026-07-11T01:00:00Z",
                        "reviewThreads": review_threads(&in_window),
                    })),
                ],
                None,
            )],
        ))]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(prs[0].blocked_reasons, vec![]);
        assert_eq!(prs[1].blocked_reasons, vec![BlockedReason::Threads]);
    }

    /// Sectioning reads involvement off the node, so a query that stopped
    /// asking for it would collapse every PR into All and stop unread badges.
    #[test]
    fn a_fetched_row_is_sectioned_by_the_viewers_involvement() {
        let github = Canned::new(vec![ok(prs_round(
            5000,
            vec![repo_named(
                "acme/widgets",
                vec![
                    pr_node(json!({
                        "number": 1,
                        "updatedAt": "2026-07-11T04:00:00Z",
                        "viewerDidAuthor": true,
                    })),
                    pr_node(json!({
                        "number": 2,
                        "updatedAt": "2026-07-11T03:00:00Z",
                        "viewerSubscription": "SUBSCRIBED",
                    })),
                    pr_node(json!({
                        "number": 3,
                        "updatedAt": "2026-07-11T02:00:00Z",
                        "reviewRequests": {
                            "nodes": [{ "requestedReviewer": { "login": "khiet" } }]
                        },
                    })),
                    pr_node(json!({ "number": 4, "updatedAt": "2026-07-11T01:00:00Z" })),
                ],
                None,
            )],
        ))]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert_eq!(
            prs.iter().map(|pr| pr.section).collect::<Vec<_>>(),
            vec![
                Section::Mine,
                Section::Participated,
                Section::Participated,
                Section::All,
            ]
        );
        assert!(prs[2].review_requested);
    }

    // Sync-level tests: a fetch over canned responses feeds the transition
    // that counts unread, so the query, the human/bot split and the
    // watermarks are under test together.

    /// A comment node as the PR query returns it. `kind` is the author's
    /// `__typename`: "User" for a person, "Bot" for a machine.
    fn comment(at: &str, login: &str, kind: &str) -> Value {
        json!({ "createdAt": at, "author": { "__typename": kind, "login": login } })
    }

    /// One fetch of a single watched repo whose open PRs are `nodes`.
    fn fetch_nodes(nodes: Vec<Value>) -> Vec<PullRequest> {
        let github = Canned::new(vec![ok(prs_round(
            5000,
            vec![repo_named("acme/widgets", nodes, None)],
        ))]);
        fetch_open(&github, &["acme/widgets"]).unwrap().prs
    }

    /// One sync of the same repo: the fetch, then the unread transition.
    fn sync(state: &PrState, nodes: Vec<Value>) -> Effects {
        state.sync_succeeded(fetch_nodes(nodes), vec![], 0)
    }

    /// A PR the viewer takes part in, so one that can carry unread activity.
    fn joined(overrides: Value) -> Value {
        with(
            pr_node(json!({ "viewerSubscription": "SUBSCRIBED" })),
            overrides,
        )
    }

    /// Eight inline comments from a review bot, wrapped in the one COMMENTED
    /// review GitHub files them under.
    fn bot_review() -> Value {
        let inline: Vec<Value> = (0..8)
            .map(|i| comment(&format!("2026-07-12T10:00:0{i}Z"), "review-bot", "Bot"))
            .collect();
        json!({
            "state": "COMMENTED",
            "submittedAt": "2026-07-12T10:00:00Z",
            "author": { "__typename": "Bot", "login": "review-bot" },
            "comments": { "nodes": inline }
        })
    }

    #[test]
    fn bot_activity_is_counted_apart_from_unread_and_the_viewers_own_in_neither() {
        let state = PrState::default();
        sync(&state, vec![joined(json!({}))]);

        let effects = sync(
            &state,
            vec![joined(json!({
                "comments": { "nodes": [
                    comment("2026-07-12T09:00:00Z", "alice", "User"),
                    comment("2026-07-12T09:01:00Z", "bob", "User"),
                    comment("2026-07-12T09:02:00Z", "khiet", "User"),
                ] },
                "reviews": { "nodes": [
                    bot_review(),
                    {
                        "state": "APPROVED",
                        "submittedAt": "2026-07-12T11:00:00Z",
                        "author": { "__typename": "User", "login": "carol" },
                        "comments": { "nodes": [] }
                    },
                ] },
            }))],
        );

        let pr = &effects.snapshot.prs[0];
        assert_eq!(pr.unread_count, 3);
        assert_eq!(pr.bot_unread_count, 8);
    }

    #[test]
    fn a_pr_with_only_new_bot_activity_carries_no_unread() {
        let state = PrState::default();
        sync(&state, vec![joined(json!({}))]);

        let effects = sync(
            &state,
            vec![joined(json!({ "reviews": { "nodes": [bot_review()] } }))],
        );

        let pr = &effects.snapshot.prs[0];
        assert_eq!(pr.unread_count, 0);
        assert_eq!(pr.bot_unread_count, 8);
    }

    #[test]
    fn a_pr_first_seen_with_only_bot_activity_is_baselined_as_read() {
        let state = PrState::default();
        let with_bots = || vec![joined(json!({ "reviews": { "nodes": [bot_review()] } }))];

        let first = sync(&state, with_bots());
        assert_eq!(first.snapshot.prs[0].bot_unread_count, 0);

        // The baseline sits at the newest bot comment, so the same activity
        // stays read on the next sync.
        let second = sync(&state, with_bots());
        assert_eq!(second.snapshot.prs[0].bot_unread_count, 0);
    }

    #[test]
    fn marking_a_pr_read_clears_both_counts() {
        let state = PrState::default();
        sync(&state, vec![joined(json!({}))]);
        let busy = || {
            vec![joined(json!({
                "comments": { "nodes": [comment("2026-07-12T09:00:00Z", "alice", "User")] },
                "reviews": { "nodes": [bot_review()] },
            }))]
        };
        sync(&state, busy());

        let marked = state.mark_read("acme/widgets#7");

        let pr = &marked.snapshot.prs[0];
        assert_eq!((pr.unread_count, pr.bot_unread_count), (0, 0));
        // The watermark moved past the bot comments too, so they stay read.
        let next = sync(&state, busy());
        let pr = &next.snapshot.prs[0];
        assert_eq!((pr.unread_count, pr.bot_unread_count), (0, 0));
    }

    #[test]
    fn mark_all_read_clears_the_bot_count_of_a_pr_with_nothing_else_new() {
        let state = PrState::default();
        sync(&state, vec![joined(json!({}))]);
        let with_bots = || vec![joined(json!({ "reviews": { "nodes": [bot_review()] } }))];
        sync(&state, with_bots());

        let marked = state.mark_all_read();

        assert_eq!(marked.snapshot.prs[0].bot_unread_count, 0);
        assert_eq!(
            sync(&state, with_bots()).snapshot.prs[0].bot_unread_count,
            0
        );
    }

    /// A PR opened by a coding agent: the author is a `Bot` account.
    fn agent_pr(overrides: Value) -> Value {
        let node = pr_node(json!({
            "author": {
                "__typename": "Bot",
                "login": "copilot",
                "avatarUrl": "https://avatars.example/copilot"
            },
        }));
        with(node, overrides)
    }

    fn assignees(login: &str) -> Value {
        json!({ "nodes": [{ "login": login }] })
    }

    fn review_requests(logins: &[&str]) -> Value {
        let nodes: Vec<Value> = logins
            .iter()
            .map(|login| json!({ "requestedReviewer": { "login": login } }))
            .collect();
        json!({ "nodes": nodes })
    }

    #[test]
    fn a_bot_authored_pr_assigned_to_the_viewer_is_mine() {
        let prs = fetch_nodes(vec![
            agent_pr(json!({
                "number": 1,
                "updatedAt": "2026-07-11T05:00:00Z",
                "viewerSubscription": "SUBSCRIBED",
                "assignees": assignees("khiet"),
            })),
            // Unassigned, or assigned to someone else: sectioned by the
            // viewer's involvement, like a person's PR.
            agent_pr(json!({
                "number": 2,
                "updatedAt": "2026-07-11T04:00:00Z",
                "viewerSubscription": "SUBSCRIBED",
            })),
            agent_pr(json!({
                "number": 3,
                "updatedAt": "2026-07-11T03:00:00Z",
                "assignees": assignees("someone"),
            })),
            // Being assigned a person's PR does not make it yours.
            pr_node(json!({
                "number": 4,
                "updatedAt": "2026-07-11T02:00:00Z",
                "viewerSubscription": "SUBSCRIBED",
                "assignees": assignees("khiet"),
            })),
        ]);

        assert_eq!(
            prs.iter().map(|pr| pr.section).collect::<Vec<_>>(),
            vec![
                Section::Mine,
                Section::Participated,
                Section::All,
                Section::Participated,
            ]
        );
        assert_eq!(
            prs.iter().map(|pr| pr.author_is_bot).collect::<Vec<_>>(),
            vec![true, true, true, false]
        );
    }

    /// An agent's PR is the one kind in Mine that can ask the viewer for a
    /// review; the row shows the incoming mark alone.
    #[test]
    fn an_agent_pr_in_mine_that_requests_the_viewers_review_shows_only_the_incoming_mark() {
        let prs = fetch_nodes(vec![
            agent_pr(json!({
                "number": 1,
                "updatedAt": "2026-07-11T03:00:00Z",
                "assignees": assignees("khiet"),
                "reviewRequests": review_requests(&["khiet"]),
            })),
            agent_pr(json!({
                "number": 2,
                "updatedAt": "2026-07-11T02:00:00Z",
                "assignees": assignees("khiet"),
                "reviewRequests": review_requests(&["khiet", "someone"]),
            })),
        ]);

        for pr in &prs {
            assert_eq!(pr.section, Section::Mine, "#{}", pr.number);
            assert!(pr.review_requested, "#{}", pr.number);
            assert!(!pr.awaiting_review, "#{}", pr.number);
        }
    }

    #[test]
    fn a_human_authored_pr_in_mine_with_another_reviewer_pending_is_awaiting_review() {
        let prs = fetch_nodes(vec![pr_node(json!({
            "viewerDidAuthor": true,
            "reviewRequests": review_requests(&["someone"]),
        }))]);

        assert!(prs[0].awaiting_review);
        assert!(!prs[0].review_requested);
    }

    /// What the PR query reports for a team request to a `repo`-scoped token:
    /// a pending reviewer GitHub will not name.
    fn team_request() -> Value {
        json!({ "nodes": [{ "requestedReviewer": null }] })
    }

    /// `round` with the review-request search answering `hits`, each the
    /// (repo, number) of a PR asking the viewer or one of their teams.
    fn with_search(mut round: Value, hits: &[(&str, u64)], next: Option<&str>) -> Value {
        let nodes: Vec<Value> = hits
            .iter()
            .map(|(repo, number)| json!({ "number": number, "repository": { "nameWithOwner": repo } }))
            .collect();
        round["search"] = json!({
            "pageInfo": { "hasNextPage": next.is_some(), "endCursor": next },
            "nodes": nodes
        });
        round
    }

    /// One fetch of `acme/widgets` whose open PRs are `nodes`, with the
    /// review-request search answering the PR numbers in `hits`.
    fn fetch_with_search(nodes: Vec<Value>, hits: &[u64]) -> Vec<PullRequest> {
        let hits: Vec<(&str, u64)> = hits.iter().map(|n| ("acme/widgets", *n)).collect();
        let github = Canned::new(vec![ok(with_search(
            prs_round(5000, vec![repo_named("acme/widgets", nodes, None)]),
            &hits,
            None,
        ))]);
        fetch_open(&github, &["acme/widgets"]).unwrap().prs
    }

    #[test]
    fn a_request_to_one_of_the_viewers_teams_is_read_onto_the_row() {
        let prs = fetch_with_search(
            vec![
                pr_node(json!({
                    "number": 1,
                    "updatedAt": "2026-07-11T02:00:00Z",
                    "reviewRequests": team_request(),
                })),
                // Asked of a team the viewer is not in: the PR query reports
                // the same unnamed reviewer, and only the search tells them
                // apart.
                pr_node(json!({
                    "number": 2,
                    "updatedAt": "2026-07-11T01:00:00Z",
                    "reviewRequests": team_request(),
                })),
            ],
            &[1],
        );

        assert!(prs[0].team_review_requested);
        assert!(!prs[0].review_requested);
        assert!(!prs[1].team_review_requested);
    }

    /// The search matches a request by name too, so a hit alone is not a team
    /// request: the viewer's own name on the PR wins.
    #[test]
    fn a_pr_asking_the_viewer_and_their_team_carries_only_the_named_request() {
        let prs = fetch_with_search(
            vec![pr_node(json!({
                "reviewRequests": { "nodes": [
                    { "requestedReviewer": { "login": "khiet" } },
                    { "requestedReviewer": null },
                ] },
            }))],
            &[7],
        );

        assert!(prs[0].review_requested);
        assert!(!prs[0].team_review_requested);
    }

    #[test]
    fn a_draft_never_flags_a_team_review_request() {
        let prs = fetch_with_search(
            vec![pr_node(json!({
                "isDraft": true,
                "reviewRequests": team_request(),
            }))],
            &[7],
        );

        assert!(!prs[0].team_review_requested);
    }

    /// An agent's PR in Mine can ask the viewer's team, as it can ask the
    /// viewer; the incoming mark takes the review slot either way.
    #[test]
    fn an_agent_pr_in_mine_that_asks_the_viewers_team_shows_only_the_team_mark() {
        let prs = fetch_with_search(
            vec![agent_pr(json!({
                "assignees": assignees("khiet"),
                "reviewRequests": team_request(),
            }))],
            &[7],
        );

        assert_eq!(prs[0].section, Section::Mine);
        assert!(prs[0].team_review_requested);
        assert!(!prs[0].awaiting_review);
    }

    /// The search index can trail the PR. Once the team has reviewed, GitHub
    /// drops it from the pending list, and the mark goes with it even while
    /// the search still reports the PR.
    #[test]
    fn a_stale_search_hit_does_not_keep_the_mark_after_the_team_reviewed() {
        let prs = fetch_with_search(
            vec![pr_node(
                json!({ "reviewRequests": review_requests(&["someone"]) }),
            )],
            &[7],
        );

        assert!(!prs[0].team_review_requested);
    }

    /// GitHub refuses a whole query over a field it will not serve the
    /// token. The fetch asks again without the search and reports every PR.
    #[test]
    fn a_query_refused_with_the_search_in_it_is_asked_again_without_it() {
        let round = || {
            prs_round(
                5000,
                vec![repo_named(
                    "acme/widgets",
                    vec![pr_node(json!({ "reviewRequests": team_request() }))],
                    None,
                )],
            )
        };
        let github = Canned::new(vec![
            answer(
                200,
                &[],
                json!({
                    "data": null,
                    "errors": [{ "type": "INSUFFICIENT_SCOPES", "message": "No search for you." }]
                }),
            ),
            ok(with_search(round(), &[("acme/widgets", 7)], None)),
        ]);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(rows(&fetched), vec![("acme/widgets", 7)]);
        assert!(!fetched.prs[0].team_review_requested);
        assert_eq!(github.request_count(), 2);
        assert!(github.query(0).contains("search("));
        assert!(!github.query(1).contains("search("));
    }

    /// GitHub search stops at 1000 results; the fetch stops asking there
    /// and keeps what it has, never failing the sync over the search.
    #[test]
    fn a_search_that_never_reaches_a_last_page_stops_without_failing_the_sync() {
        let mut responses = vec![ok(with_search(
            prs_round(
                5000,
                vec![repo_named(
                    "acme/widgets",
                    vec![pr_node(json!({ "reviewRequests": team_request() }))],
                    None,
                )],
            ),
            &[("acme/widgets", 7)],
            Some("more"),
        ))];
        responses
            .extend((0..9).map(|_| ok(with_search(prs_round(5000, vec![]), &[], Some("more")))));
        let github = Canned::new(responses);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert!(fetched.prs[0].team_review_requested);
        assert_eq!(github.request_count(), 10);
    }

    /// The viewer's own PR keeps the outgoing mark in the review slot.
    #[test]
    fn a_team_request_on_the_viewers_own_pr_stays_awaiting_review() {
        let prs = fetch_with_search(
            vec![pr_node(json!({
                "viewerDidAuthor": true,
                "reviewRequests": team_request(),
            }))],
            &[7],
        );

        assert!(prs[0].awaiting_review);
        assert!(!prs[0].team_review_requested);
    }

    #[test]
    fn a_team_request_leaves_the_section_where_involvement_put_it() {
        let nodes = || {
            vec![
                pr_node(json!({
                    "number": 1,
                    "updatedAt": "2026-07-11T02:00:00Z",
                    "viewerSubscription": "SUBSCRIBED",
                    "reviewRequests": team_request(),
                })),
                // A muted thread: the team request does not pull it back.
                pr_node(json!({
                    "number": 2,
                    "updatedAt": "2026-07-11T01:00:00Z",
                    "viewerSubscription": "IGNORED",
                    "reviewRequests": team_request(),
                })),
            ]
        };
        let sections = |prs: Vec<PullRequest>| prs.iter().map(|pr| pr.section).collect::<Vec<_>>();

        let with_request = fetch_with_search(nodes(), &[1, 2]);
        assert!(with_request.iter().all(|pr| pr.team_review_requested));
        assert_eq!(
            sections(with_request),
            vec![Section::Participated, Section::All]
        );
        assert_eq!(
            sections(fetch_with_search(nodes(), &[])),
            vec![Section::Participated, Section::All]
        );
    }

    #[test]
    fn a_team_request_adds_nothing_to_the_tray() {
        let node = || vec![joined(json!({ "reviewRequests": team_request() }))];
        let sync_with = |state: &PrState, hits: &[u64]| {
            state.sync_succeeded(fetch_with_search(node(), hits), vec![], 0)
        };
        let state = PrState::default();
        sync_with(&state, &[]);

        let effects = sync_with(&state, &[7]);

        assert!(effects.snapshot.prs[0].team_review_requested);
        assert_eq!(effects.snapshot.prs[0].unread_count, 0);
        assert_eq!(effects.tray_count, Some(0));
    }

    /// The search is scoped to the watched repos and travels in the same
    /// request as their PRs, so it costs no extra round.
    #[test]
    fn the_team_request_search_rides_the_first_round_scoped_to_the_watched_repos() {
        let github = Canned::new(vec![ok(with_search(
            prs_round(
                5000,
                vec![
                    repo_named("acme/widgets", vec![], None),
                    repo_named("acme/gadgets", vec![], None),
                ],
            ),
            &[],
            None,
        ))]);

        fetch_open(&github, &["acme/widgets", "acme/gadgets"]).unwrap();

        assert_eq!(github.request_count(), 1);
        assert!(github.query(0).contains(
            r#"search(query: "is:open is:pr review-requested:@me repo:acme/widgets repo:acme/gadgets", type: ISSUE"#
        ));
    }

    /// The repos are done after one round; only the search goes back for
    /// more, from its own cursor.
    #[test]
    fn a_search_with_more_pages_is_resumed_alone_from_its_cursor() {
        let node =
            |number: u64| pr_node(json!({ "number": number, "reviewRequests": team_request() }));
        let github = Canned::new(vec![
            ok(with_search(
                prs_round(
                    5000,
                    vec![repo_named("acme/widgets", vec![node(1), node(2)], None)],
                ),
                &[("acme/widgets", 1)],
                Some("s1"),
            )),
            ok(with_search(
                prs_round(4999, vec![]),
                &[("acme/widgets", 2)],
                None,
            )),
        ]);

        let prs = fetch_open(&github, &["acme/widgets"]).unwrap().prs;

        assert!(prs.iter().all(|pr| pr.team_review_requested));
        assert_eq!(github.request_count(), 2);
        let second = github.query(1);
        assert!(second.contains(r#"after: "s1""#));
        assert!(!second.contains("repository(owner:"));
    }

    /// A hit is matched on repo and number together: the same number in
    /// another watched repo is a different PR.
    #[test]
    fn a_search_hit_marks_only_the_pr_in_its_own_repo() {
        let node = || pr_node(json!({ "number": 5, "reviewRequests": team_request() }));
        let github = Canned::new(vec![ok(with_search(
            prs_round(
                5000,
                vec![
                    repo_named("acme/widgets", vec![node()], None),
                    repo_named("acme/gadgets", vec![node()], None),
                ],
            ),
            &[("acme/gadgets", 5)],
            None,
        ))]);

        let prs = fetch_open(&github, &["acme/widgets", "acme/gadgets"])
            .unwrap()
            .prs;

        let marked: Vec<&str> = prs
            .iter()
            .filter(|pr| pr.team_review_requested)
            .map(|pr| pr.repo.as_str())
            .collect();
        assert_eq!(marked, vec!["acme/gadgets"]);
    }

    /// GitHub answers a field it cannot resolve with null and an error beside
    /// the rest of the data. The sync reports every PR and drops only the
    /// team marks.
    #[test]
    fn a_search_github_declines_to_answer_costs_only_the_team_marks() {
        let mut data = prs_round(
            5000,
            vec![repo_named(
                "acme/widgets",
                vec![pr_node(json!({ "reviewRequests": team_request() }))],
                None,
            )],
        );
        data["search"] = Value::Null;
        let github = Canned::new(vec![answer(
            200,
            &[],
            json!({
                "data": data,
                "errors": [{ "type": "FORBIDDEN", "message": "Resource not accessible." }]
            }),
        )]);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(rows(&fetched), vec![("acme/widgets", 7)]);
        assert!(!fetched.prs[0].team_review_requested);
        assert_eq!(github.request_count(), 1);
    }

    // Fix with Copilot: the eligibility the sync reads onto a row, and the
    // one comment the app writes.

    fn copilot_actors() -> Value {
        json!({ "nodes": [{ "__typename": "Bot", "login": "copilot-swe-agent" }] })
    }

    /// A PR of the viewer's with a merge conflict, on a branch of the repo
    /// itself.
    fn conflicted(overrides: Value) -> Value {
        with(
            pr_node(json!({
                "id": "PR_kwDOconflict7",
                "viewerDidAuthor": true,
                "mergeable": "CONFLICTING",
                "isCrossRepository": false,
            })),
            overrides,
        )
    }

    /// One fetch of a single watched repo that answers `actors` for its
    /// assignable Copilot actors.
    fn fetch_with_actors(actors: Value, nodes: Vec<Value>) -> Vec<PullRequest> {
        let mut repo = repo_named("acme/widgets", nodes, None);
        repo["suggestedActors"] = actors;
        let github = Canned::new(vec![ok(prs_round(5000, vec![repo]))]);
        fetch_open(&github, &["acme/widgets"]).unwrap().prs
    }

    fn can_fix(actors: Value, overrides: Value) -> bool {
        fetch_with_actors(actors, vec![conflicted(overrides)])[0].can_fix_conflict
    }

    #[test]
    fn a_conflicting_pr_of_the_viewers_in_a_repo_with_copilot_can_ask_for_the_fix() {
        assert!(can_fix(copilot_actors(), json!({})));
    }

    #[test]
    fn the_fix_is_not_offered_when_any_one_condition_fails() {
        let participated = json!({ "viewerDidAuthor": false, "viewerSubscription": "SUBSCRIBED" });
        assert!(!can_fix(copilot_actors(), participated));
        assert!(!can_fix(copilot_actors(), json!({ "isDraft": true })));
        assert!(!can_fix(
            copilot_actors(),
            json!({ "mergeable": "MERGEABLE" })
        ));
        assert!(!can_fix(
            copilot_actors(),
            json!({ "isCrossRepository": true })
        ));
        assert!(!can_fix(json!({ "nodes": [] }), json!({})));
    }

    /// The actor filter is a search, so it can return accounts that are not
    /// the agent.
    #[test]
    fn only_the_copilot_bot_itself_counts_as_copilot() {
        let lookalikes = json!({ "nodes": [
            { "__typename": "User", "login": "copilot-swe-agent" },
            { "__typename": "Bot", "login": "copilot-swe-agent-fan" },
        ] });
        assert!(!can_fix(lookalikes, json!({})));
    }

    /// GitHub answers a field it will not serve with null beside the rest of
    /// the data.
    #[test]
    fn a_repo_whose_actors_cannot_be_read_syncs_with_the_fix_withheld() {
        let prs = fetch_with_actors(Value::Null, vec![conflicted(json!({}))]);

        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].blocked_reasons, vec![BlockedReason::Conflict]);
        assert!(!prs[0].can_fix_conflict);
    }

    /// The other way GitHub declines: the whole query, with no data. Search
    /// is given up first, then the actors, and the PRs still arrive.
    #[test]
    fn a_query_refused_over_the_actors_is_asked_again_without_them() {
        let refused = || {
            answer(
                200,
                &[],
                json!({
                    "data": null,
                    "errors": [{ "type": "FORBIDDEN", "message": "No actors for you." }]
                }),
            )
        };
        let mut repo = repo_named("acme/widgets", vec![conflicted(json!({}))], None);
        repo["suggestedActors"] = copilot_actors();
        let github = Canned::new(vec![refused(), refused(), ok(prs_round(5000, vec![repo]))]);

        let fetched = fetch_open(&github, &["acme/widgets"]).unwrap();

        assert_eq!(rows(&fetched), vec![("acme/widgets", 7)]);
        assert!(!fetched.prs[0].can_fix_conflict);
        assert!(github.query(1).contains("suggestedActors("));
        assert!(!github.query(1).contains("search("));
        assert!(!github.query(2).contains("suggestedActors("));
    }

    fn fixable() -> PullRequest {
        fetch_with_actors(copilot_actors(), vec![conflicted(json!({}))]).remove(0)
    }

    #[test]
    fn asking_for_the_fix_posts_one_comment_on_the_pr_with_the_fixed_body() {
        let github = Canned::new(vec![ok(json!({
            "addComment": { "subject": { "id": "PR_kwDOconflict7" } }
        }))]);

        run(github.client().request_conflict_fix(&fixable())).unwrap();

        assert_eq!(github.request_count(), 1);
        assert!(github.query(0).starts_with("mutation"));
        assert!(github
            .query(0)
            .contains("addComment(input: { subjectId: $subject, body: $body })"));
        assert_eq!(
            github.variables(0),
            json!({
                "subject": "PR_kwDOconflict7",
                "body": "@copilot resolve the merge conflicts on this PR."
            })
        );
    }

    #[test]
    fn a_pr_that_is_not_eligible_is_refused_and_nothing_is_posted() {
        let github = Canned::new(vec![]);
        let quiet = fetch_with_actors(
            copilot_actors(),
            vec![conflicted(json!({ "mergeable": "MERGEABLE" }))],
        )
        .remove(0);

        let refused = run(github.client().request_conflict_fix(&quiet));

        assert_eq!(
            refused,
            Err(GithubError::Other(
                "Nothing here for Copilot to fix.".into()
            ))
        );
        assert_eq!(github.request_count(), 0);
    }

    /// A mutation GitHub turns down answers with a null payload and the
    /// reason beside it.
    #[test]
    fn a_comment_github_turns_down_fails_with_githubs_message() {
        let github = Canned::new(vec![answer(
            200,
            &[],
            json!({
                "data": { "addComment": null },
                "errors": [{ "type": "FORBIDDEN", "message": "Resource not accessible." }]
            }),
        )]);

        let failed = run(github.client().request_conflict_fix(&fixable()));

        assert_eq!(
            failed,
            Err(GithubError::Other(
                "GitHub error: Resource not accessible.".into()
            ))
        );
    }

    #[test]
    fn a_rejected_token_fails_the_fetch_with_a_message_about_the_token() {
        let github = Canned::new(vec![answer(
            401,
            &[],
            json!({ "message": "Bad credentials" }),
        )]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::Other(
                "GitHub rejected the token. Check that it is valid and not expired.".into()
            ))
        );
    }

    /// The primary limit answers 403 with the absolute reset epoch.
    #[test]
    fn a_403_is_rate_limiting_until_the_reset_header() {
        let github = Canned::new(vec![answer(
            403,
            &[("x-ratelimit-reset", "1784205296")],
            json!({ "message": "API rate limit exceeded" }),
        )]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::RateLimited {
                reset_epoch_secs: Some(1_784_205_296)
            })
        );
    }

    /// The secondary limit answers 429 with a wait in seconds; the canned
    /// clock reads 1000.
    #[test]
    fn a_429_is_rate_limiting_until_retry_after_has_passed() {
        let github = Canned::new(vec![answer(
            429,
            &[("retry-after", "60")],
            json!({ "message": "You have exceeded a secondary rate limit" }),
        )]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::RateLimited {
                reset_epoch_secs: Some(1_060)
            })
        );
    }

    #[test]
    fn other_http_failures_report_the_status() {
        let github = Canned::new(vec![answer(502, &[], json!({}))]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::Other(
                "GitHub returned an error (HTTP 502).".into()
            ))
        );
    }

    /// A proxy or captive portal can answer 200 with a page that is not
    /// GitHub's JSON at all.
    #[test]
    fn a_successful_status_with_an_unreadable_body_is_an_unexpected_response() {
        let github = Canned::new(vec![HttpResponse {
            status: reqwest::StatusCode::OK,
            headers: headers(&[]),
            body: "<html>Sign in to the network</html>".into(),
        }]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::Other(
                "GitHub returned an unexpected response.".into()
            ))
        );
    }

    /// GraphQL quota refusals arrive as HTTP 200 with no data, so the sync
    /// loop must still see them as rate limiting rather than a retryable
    /// failure.
    #[test]
    fn a_graphql_rate_limit_refusal_fails_the_fetch_as_rate_limited() {
        let github = Canned::new(vec![answer(
            200,
            &[],
            json!({
                "data": null,
                "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }]
            }),
        )]);

        let result = fetch_open(&github, &["acme/widgets"]);

        assert_eq!(
            result.err(),
            Some(GithubError::RateLimited {
                reset_epoch_secs: None
            })
        );
    }

    #[test]
    fn a_valid_token_reports_its_login() {
        let github = Canned::new(vec![answer(
            200,
            &[("x-oauth-scopes", "repo, read:org")],
            json!({ "data": { "viewer": { "login": "khiet" } } }),
        )]);

        let validated = run(github.client().validate_token()).unwrap();

        assert_eq!(validated.login, "khiet");
        assert_eq!(validated.scope_warning, None);
    }

    #[test]
    fn a_valid_classic_token_without_repo_scope_carries_a_warning() {
        let github = Canned::new(vec![answer(
            200,
            &[("x-oauth-scopes", "read:org")],
            json!({ "data": { "viewer": { "login": "khiet" } } }),
        )]);

        let validated = run(github.client().validate_token()).unwrap();

        assert_eq!(validated.login, "khiet");
        assert_eq!(
            validated.scope_warning.as_deref(),
            Some("This token has no repo scope, so private repositories won't be visible.")
        );
    }

    #[test]
    fn a_rejected_token_fails_validation() {
        let github = Canned::new(vec![answer(
            401,
            &[],
            json!({ "message": "Bad credentials" }),
        )]);

        let result = run(github.client().validate_token());

        assert_eq!(
            result.err().as_deref(),
            Some("GitHub rejected the token. Check that it is valid and not expired.")
        );
    }

    /// The canonical name fixes whatever casing the user typed.
    #[test]
    fn an_accessible_repo_validates_to_its_canonical_name() {
        let github = Canned::new(vec![ok(json!({
            "repository": { "nameWithOwner": "Acme/Widgets" }
        }))]);

        let result = run(github.client().validate_repo("acme", "widgets"));

        assert_eq!(result.as_deref(), Ok("Acme/Widgets"));
        assert_eq!(
            github.variables(0),
            json!({ "owner": "acme", "name": "widgets" })
        );
    }

    #[test]
    fn a_missing_or_inaccessible_repo_fails_validation_by_name() {
        let github = Canned::new(vec![answer(
            200,
            &[],
            json!({
                "data": { "repository": null },
                "errors": [{ "type": "NOT_FOUND", "message": "Could not resolve to a Repository." }]
            }),
        )]);

        let result = run(github.client().validate_repo("acme", "nope"));

        assert_eq!(
            result.err().as_deref(),
            Some("acme/nope was not found or is not accessible with this token.")
        );
    }

    #[test]
    fn affiliated_repos_are_gathered_across_pages() {
        let github = Canned::new(vec![
            ok(repo_page(
                json!([{ "nameWithOwner": "acme/gadgets", "isArchived": false }]),
                json!({ "hasNextPage": true, "endCursor": "c1" }),
            )),
            ok(repo_page(
                json!([
                    { "nameWithOwner": "acme/old", "isArchived": true },
                    { "nameWithOwner": "acme/widgets", "isArchived": false }
                ]),
                json!({ "hasNextPage": false, "endCursor": null }),
            )),
        ]);

        let repos = run(github.client().fetch_affiliated_repos()).unwrap();

        assert_eq!(repos, vec!["acme/gadgets", "acme/widgets"]);
        assert_eq!(github.variables(0), json!({ "cursor": null }));
        assert_eq!(github.variables(1), json!({ "cursor": "c1" }));
    }

    /// Malformed keys take no alias, so the aliases in the answer line up
    /// with the well-formed keys only.
    #[test]
    fn merged_status_maps_each_answer_back_to_its_pr_key() {
        let github = Canned::new(vec![ok(json!({
            "p0": { "pullRequest": { "merged": false, "mergedAt": null } },
            "p1": { "pullRequest": { "merged": true, "mergedAt": "2026-07-12T08:00:00Z" } }
        }))]);
        let keys = ["acme/widgets#1", "not a key", "acme/gadgets#2"].map(String::from);

        let merged = run(github.client().fetch_merged_status(&keys)).unwrap();

        assert_eq!(
            merged,
            HashMap::from([(
                "acme/gadgets#2".to_string(),
                "2026-07-12T08:00:00Z".to_string()
            )])
        );
    }
}
