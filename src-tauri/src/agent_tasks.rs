//! Copilot's cloud agent tasks, as GitHub's Agent tasks REST API lists them
//! (a public preview), and what the task behind a PR means for its row. No
//! I/O here: [`crate::github`] makes the request and hands the body over.

use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
};

use serde_json::Value;

use crate::{
    fix_requests,
    github::{rfc3339_utc_to_epoch_secs, AgentTask, PullRequest, Turn},
};

/// The newest 100 tasks the token can see, which is all that is ever read: a
/// task further back than that speaks for no PR.
pub const URL: &str =
    "https://api.github.com/agents/tasks?per_page=100&sort=updated_at&direction=desc";
pub const API_VERSION: &str = "2026-03-10";
/// How long the tasks call stays away after a rate limit answer that does not
/// say when the window resets. GitHub asks for at least a minute.
pub const UNDATED_RATE_LIMIT_WAIT_SECS: u64 = 60;

/// One task that has a pull request.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    state: AgentTask,
    /// GitHub's database id of the account that started the task.
    creator_id: u64,
    /// GitHub's database id of the task's PR.
    pull_id: u64,
    /// Epoch ms; `None` when GitHub's time could not be read, which ranks
    /// the task behind every dated one.
    updated_at: Option<u64>,
}

/// Reads a page of tasks out of a response body; `None` when the body is not
/// a task list at all. A task with no PR is left out, as is one too malformed
/// to tell whose it is or which PR it has.
pub fn parse(body: &str) -> Option<Vec<Task>> {
    let page: Value = serde_json::from_str(body).ok()?;
    let tasks = page.get("tasks")?.as_array()?;
    Some(tasks.iter().filter_map(task).collect())
}

fn task(task: &Value) -> Option<Task> {
    let pull = task.get("artifacts")?.as_array()?.iter().find(|artifact| {
        artifact.get("provider").and_then(Value::as_str) == Some("github")
            && artifact.get("type").and_then(Value::as_str) == Some("pull")
    })?;
    Some(Task {
        state: state(task.get("state")?.as_str()?),
        creator_id: task.pointer("/creator/id")?.as_u64()?,
        pull_id: pull.pointer("/data/id")?.as_u64()?,
        updated_at: task
            .get("updated_at")
            .and_then(Value::as_str)
            .and_then(epoch_ms),
    })
}

/// A state this does not know, which a preview API may grow, reads as
/// settled: the task still outranks older ones, and says nothing itself.
fn state(state: &str) -> AgentTask {
    match state {
        "queued" | "in_progress" => AgentTask::Working,
        "waiting_for_user" => AgentTask::NeedsYou,
        "failed" | "timed_out" => AgentTask::Failed,
        _ => AgentTask::Settled,
    }
}

/// Task times carry fractional seconds ("2026-11-18T09:44:29.271460483Z"),
/// unlike every other time GitHub sends this app.
fn epoch_ms(iso: &str) -> Option<u64> {
    let (whole, fraction) = match iso.split_once('.') {
        Some((whole, fraction)) => (format!("{whole}Z"), fraction.strip_suffix('Z')?),
        None => (iso.to_string(), ""),
    };
    if !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis: u64 = format!("{fraction:0<3}")[..3].parse().ok()?;
    Some(rfc3339_utc_to_epoch_secs(&whole)? * 1000 + millis)
}

/// Gives each PR the state of the viewer's latest task on it, and the turn
/// that state calls for (see [`decide`]). Only a task `viewer_id` started
/// counts, and only on a PR in `prs`; among several on one PR the one updated
/// last decides, and GitHub's order, newest first, settles a tie.
pub fn assign(prs: &mut [PullRequest], tasks: &[Task], viewer_id: u64) {
    let mut latest: HashMap<u64, &Task> = HashMap::new();
    for task in tasks.iter().filter(|task| task.creator_id == viewer_id) {
        let known = latest.entry(task.pull_id).or_insert(task);
        if task.updated_at > known.updated_at {
            *known = task;
        }
    }
    for pr in prs {
        if let Some(task) = pr.database_id.and_then(|id| latest.get(&id)) {
            decide(pr, task.state, task.updated_at);
        }
    }
}

/// Marks `pr` with its task's state and moves its turn to match. A working
/// task takes the PR off the viewer's hands as a requested conflict fix does
/// (see [`fix_requests::wait_on_copilot`]). A task waiting for the viewer makes the turn theirs
/// whatever GitHub's state said, a draft included: the task goes nowhere
/// until they answer. Any other state leaves the turn alone.
pub fn decide(pr: &mut PullRequest, state: AgentTask, updated_at: Option<u64>) {
    pr.agent_task = Some(state);
    pr.agent_task_updated_at = updated_at;
    match state {
        AgentTask::Working => fix_requests::wait_on_copilot(pr),
        AgentTask::NeedsYou => pr.turn = Some(Turn::Yours),
        AgentTask::Failed | AgentTask::Settled => {}
    }
}

/// Whether the tasks call may be made. Kept in memory only, so a restart
/// opens it again.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Gate {
    /// A fingerprint of the token GitHub would not serve tasks to. The call
    /// stays unmade while the token in use is that one: asking again would
    /// get the same answer. Forgotten once another token is in use, so
    /// going back to the first asks afresh.
    refused: Option<u64>,
    /// Epoch secs before which the call stays unmade, after a rate limit.
    wait_until: Option<u64>,
}

impl Gate {
    pub fn allows(&mut self, token: &str, now_epoch_secs: u64) -> bool {
        if self
            .refused
            .is_some_and(|refused| refused != fingerprint(token))
        {
            self.refused = None;
        }
        self.refused.is_none() && self.wait_until.is_none_or(|until| until <= now_epoch_secs)
    }

    /// GitHub answered that `token` may not read tasks, or that there is no
    /// such thing to read.
    pub fn refuse(&mut self, token: &str) {
        self.refused = Some(fingerprint(token));
    }

    /// GitHub answered that the budget is spent until `reset_epoch_secs`.
    pub fn wait_until(&mut self, reset_epoch_secs: u64) {
        self.wait_until = Some(reset_epoch_secs);
    }
}

/// Stands in for the token so the gate holds no secret. Compared within one
/// run of the app only, which is all the default hasher is stable for.
fn fingerprint(token: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    token.hash(&mut hasher);
    hasher.finish()
}
