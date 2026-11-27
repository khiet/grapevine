// What the UI knows about the Rust backend's commands and events: the wire
// types, the PR key, one typed function per command, and the Snapshot
// subscription. Components import from here and never from @tauri-apps/api.
// The updater plugins are the one exception; updater.ts owns those.
//
// The wire types are hand-written copies of the Rust structs that serialize
// them (github.rs, merged.rs, pr_state.rs, commands.rs, settings.rs). A
// field added or
// renamed there must be mirrored here, and the reverse.

import { tauriAdapter } from "./tauri";

export interface PullRequest {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  /** The author is a machine account; renders the "bot" tag on the avatar. */
  author_is_bot: boolean;
  avatar_url: string;
  owner_avatar_url: string;
  created_at: string;
  updated_at: string;
  section: Section;
  /** Why the PR is blocked, already in severity order; empty means not blocked.
   * Composed at the Rust boundary; the row only maps keys to words. */
  blocked_reasons: BlockedReason[];
  /** Drafts say "Draft" in the neutral tone; the backend suppresses their blocked
   * reasons and review states. */
  is_draft: boolean;
  /** The viewer's review is requested and not yet acted on; reads "Review
   * requested". Backend-computed, suppressed on drafts, and self-clearing
   * once the viewer reviews. */
  review_requested: boolean;
  /** A review is outstanding from one of the viewer's teams, with no request
   * naming the viewer; reads "Team review". Never set
   * beside review_requested or awaiting_review, suppressed on drafts,
   * self-clearing once a team member reviews, and never part of the unread
   * count. */
  team_review_requested: boolean;
  /** One of the viewer's own PRs is waiting on a reviewer; reads "Awaiting
   * review". The Mine-only mirror of review_requested
   * (the two never share a row), suppressed on drafts, and self-clearing as
   * reviewers submit. */
  awaiting_review: boolean;
  /** The review decision is an explicit APPROVED; reads "Approved".
   * Backend-computed and suppressed on drafts, like the review states. */
  approved: boolean;
  /** 1-based position in the repo's merge queue as GitHub reports it; null
   * when the PR is not queued. Reads "Queued 2nd". */
  merge_queue_position: number | null;
  /** When GitHub expects the queued PR to merge, epoch ms: the time GitHub
   * answered the sync plus its estimate. Null when not queued or GitHub has
   * no estimate. Reads "Queued 2nd · ETA 21m", counting down from now. */
  merge_queue_eta: number | null;
  /** Names of the failed checks behind the "ci" reason, GitHub's order; the
   * state's tooltip names the first. Empty when "ci" is not raised, and when it is
   * raised with no check to name. */
  failing_checks: string[];
  /** The PR has more checks than the backend read, so failing_checks may be
   * short: say "and more", never a count. */
  failing_checks_truncated: boolean;
  /** A check that gates the merge is still running and none has failed;
   * reads "Checks running". Suppressed on drafts. */
  checks_running: boolean;
  /** Auto-merge is armed; reads "Auto-merge on". Suppressed on drafts. */
  auto_merge: boolean;
  /** A review thread is unresolved and the base branch does not require
   * resolution; reads "Unresolved threads" in the neutral tone. Never set
   * beside the "threads" blocked reason, which is the same thread where
   * resolution is required. Suppressed on drafts. */
  threads_open: boolean;
  /** Nothing is left but pressing merge; reads "Mergeable".
   * Backend-computed and only ever set in Mine. */
  ready_to_merge: boolean;
  /** Whose move the PR waits on; null when there is no verdict, always so on
   * a draft and in All. Backend-computed. A "yours" row sits in Todo and
   * leads with the reason for the turn, and the tray shows how many there
   * are; "theirs" renders nothing of its own. Never part of the unread count. */
  turn: Turn | null;
  /** Epoch ms of the sync that first saw the turn become yours; the row shows
   * it as "waiting 2d". Null when the turn is not yours, and when nobody
   * watched the turn begin (it was already yours the first time the PR was
   * seen, for one), so the start is unknown. */
  turn_since: number | null;
  /** The row may offer to ask Copilot to resolve the merge conflict.
   * Backend-computed: a Mine, non-draft, conflicting PR that is not from a
   * fork, in a repo where Copilot's cloud agent is enabled. The backend
   * checks it again before posting, so this only decides what is offered. */
  can_fix_conflict: boolean;
  /** Copilot has been asked to resolve the conflict, within the last hour,
   * and no sync has shown the conflict gone since; reads "Copilot fixing",
   * in place of the fix control. Backend-kept, so it survives a restart.
   * Such a PR waits on Copilot: its turn is never "yours" unless a review is
   * asked of the viewer by name. */
  fix_requested: boolean;
  /** Where the viewer's latest Copilot task on this PR stands; null when
   * they have none among the tasks the backend read. Backend-computed, and
   * the backend moves `turn` to match: "working" waits on Copilot as a
   * requested fix does, "needs_you" is always the viewer's turn, dated from
   * when the task stopped for them. Never set beside fix_requested: a task
   * that changed after the fix was asked for replaces the request, and an
   * older one gives way to it. */
  agent_task: AgentTask | null;
  /** What the row may offer to do to the PR on GitHub, in the order it lists
   * them. Backend-computed from what GitHub lets the viewer do: only ever
   * set in Mine, never on a draft, and "queue" stands in for "merge" where
   * the base branch has a merge queue. The backend checks it again before
   * writing, so this only decides what is offered. */
  actions: PrAction[];
  /** What the viewer did to the PR from its row that no sync has read back
   * yet; null otherwise. Backend-set once GitHub has taken the write, beside
   * an empty `actions`, and cleared by the first sync that began after it
   * and landed. The row says it until then ("Queued"). */
  acted: PrAction | null;
  /** How "merge" and "enable_auto_merge" merge in this PR's repo: rebase
   * where the repo allows it, otherwise the repo's default for the viewer.
   * Named in the confirm hint; there is no picker. */
  merge_method: MergeMethod;
  /** The branch the PR merges into, named in the confirm hint. */
  base_branch: string;
  /** The head commit this row was built from. Never shown: it goes back
   * with a write, so the backend can tell a row the user confirmed from a
   * PR that has moved since. */
  head_oid: string;
  /** New activity from people. The only count that badges a row or a
   * collapsed section. It does not feed the tray. */
  unread_count: number;
  /** New activity from bots, past the same read point; renders the row's muted
   * note and never a badge. Counts the recent activity the backend fetched,
   * not a lifetime total. */
  bot_unread_count: number;
}

/** Which popover section a PR belongs to, from the viewer's perspective. */
export type Section = "mine" | "participated" | "all";

export type Turn = "yours" | "theirs";

/** "working" reads "Copilot working", "needs_you" "Copilot needs you" and
 * "failed" "Copilot failed". "settled" is a task that is over or idle, and
 * reads nothing. */
export type AgentTask = "working" | "needs_you" | "failed" | "settled";

export type PrAction =
  | "merge"
  | "queue"
  | "update_branch"
  | "enable_auto_merge"
  | "disable_auto_merge";

export type MergeMethod = "merge" | "squash" | "rebase";

export type BlockedReason = "conflict" | "ci" | "review" | "threads" | "behind";

export interface MergedPr {
  number: number;
  title: string;
  url: string;
  repo: string;
  author: string;
  avatar_url: string;
  owner_avatar_url: string;
  merged_at: string;
  /** The deploy of the merge commit; null when no deploy run has
   * been found, which is forever for a repo that has none. */
  deploy: Deploy | null;
}

/** Every state but "deploying" is final. */
export type DeployState = "deploying" | "deployed" | "failed" | "cancelled";

export interface Deploy {
  state: DeployState;
  /** When the run behind `state` finished (ISO-8601 UTC); null while
   * deploying. */
  completed_at: string | null;
  /** The workflow run behind `state`. */
  url: string;
}

export interface Snapshot {
  prs: PullRequest[];
  merged: MergedPr[];
  /** Keys (see prKey) of the snoozed PRs. Each names a PR in `prs`; the
   * popover folds those rows away and the tray leaves them out. */
  snoozed: string[];
  has_synced: boolean;
  /** Epoch ms of the last successful sync; null before the first one. */
  last_sync_at: number | null;
  /** User-facing message of the most recent failure; cleared by a success. */
  sync_error: string | null;
  /** A sync is under way or has been asked for; Sync now is busy and
   * sync_now does nothing while this is set. */
  syncing: boolean;
}

export interface TokenStatus {
  has_token: boolean;
  login: string | null;
}

export interface SavedToken {
  login: string;
  /** Advisory only: the token is already stored when this is set. */
  scope_warning: string | null;
}

/** What the UI holds until the backend first replies; equal to the Rust
 * `Snapshot::default()`. */
export const EMPTY_SNAPSHOT: Snapshot = {
  prs: [],
  merged: [],
  snoozed: [],
  has_synced: false,
  last_sync_at: null,
  sync_error: null,
  syncing: false,
};

/** The key the backend files unread, snoozed and merged state under. The Rust
 * side builds the same string; the two must agree or mark_read, snooze and
 * dismiss_merged silently miss. */
export function prKey(pr: { repo: string; number: number }): string {
  return `${pr.repo}#${pr.number}`;
}

/** The transport under the typed functions below. Tauri in the app; the fake
 * in the demo and in tests. */
export interface Adapter {
  invoke<T>(command: string, args?: Record<string, unknown>): Promise<T>;
  /** Resolves to a function that stops the listener. */
  listen<T>(event: string, handler: (payload: T) => void): Promise<() => void>;
  openUrl(url: string): Promise<void>;
  appVersion(): Promise<string>;
}

let adapter: Adapter = tauriAdapter;

/** Swaps the transport. Call before the first render: calls already in
 * flight stay on the adapter they started with. */
export function setAdapter(next: Adapter): void {
  adapter = next;
}

/** The event the backend pushes each new Snapshot on. */
export const SNAPSHOT_EVENT = "prs-updated";

/** Delivers the current Snapshot, then every one the backend pushes. Returns
 * the unsubscribe function; nothing is delivered after it runs. */
export function subscribeSnapshot(
  onSnapshot: (snapshot: Snapshot) => void,
): () => void {
  let active = true;
  // A push that lands first is at least as new as the get_prs reply it raced,
  // so the reply is dropped rather than allowed to overwrite it.
  let pushed = false;
  const stop = adapter.listen<Snapshot>(SNAPSHOT_EVENT, (snapshot) => {
    if (!active) return;
    pushed = true;
    onSnapshot(snapshot);
  });
  // A failed registration leaves the get_prs reply as the only delivery.
  stop.catch(() => {});
  adapter
    .invoke<Snapshot>("get_prs")
    .then((snapshot) => {
      if (active && !pushed) onSnapshot(snapshot);
    })
    .catch(() => {});
  return () => {
    active = false;
    stop.then((unlisten) => unlisten()).catch(() => {});
  };
}

/** Starts a sync straight away, without changing the poll interval. The
 * backend answers with a Snapshot that has `syncing` set, then with the
 * sync's result; it does nothing while a sync is already under way. */
export const syncNow = () => adapter.invoke<void>("sync_now");

// The backend answers each of these by pushing a fresh Snapshot.
export const markRead = (key: string) => adapter.invoke<void>("mark_read", { key });
export const markAllRead = () => adapter.invoke<void>("mark_all_read");
export const dismissMerged = (key: string) =>
  adapter.invoke<void>("dismiss_merged", { key });
export const dismissAllMerged = () => adapter.invoke<void>("dismiss_all_merged");
/** Parks a Mine or Participated row until it has news, or until the limit
 * of `days` runs out if that comes first; null is no limit. The expiry is
 * worked out here because the webview knows the local time zone. */
export const snooze = (key: string, days: number | null) =>
  adapter.invoke<void>("snooze", {
    key,
    expiresAt: days === null ? null : snoozeExpiry(new Date(), days),
  });
export const wake = (key: string) => adapter.invoke<void>("wake", { key });

/** Asks Copilot to resolve the PR's merge conflict, with a comment posted on
 * GitHub as the user. Only the key goes
 * over; the backend picks the PR and the words. Rejects with the message to
 * show on the row. A posted comment is answered with a Snapshot in which the
 * PR has fix_requested set; the backend refuses a second one while that
 * lasts. */
export const fixConflict = (key: string) =>
  adapter.invoke<void>("fix_conflict", { key });

/** Does `action` to the PR on GitHub as the user: with fixConflict, the
 * commands that write there. The key and the action's name go over, with
 * `head`, the head_oid of the row the user confirmed; the backend picks the
 * PR and the merge method, and refuses an action the PR does not list, a
 * snoozed PR, and a PR that is no longer at `head`. Rejects with the message
 * to show on the row, GitHub's own when GitHub turned the write down. A
 * write that went through is answered with a Snapshot in which the PR
 * offers no actions. Either way a sync follows. */
export const actOnPr = (key: string, action: PrAction, head: string) =>
  adapter.invoke<void>("act_on_pr", { key, action, head });

/** Epoch ms at which a snooze made at `now` with a limit of `days` runs out
 * on its own: 08:00 local on the day that many days later, whatever the
 * hour of the snooze. */
export function snoozeExpiry(now: Date, days: number): number {
  // Calendar arithmetic, not multiples of 24h: a DST day is not 24 hours
  // long, and the hour must not move.
  return new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() + days,
    8,
  ).getTime();
}

export const tokenStatus = () => adapter.invoke<TokenStatus>("token_status");
export const saveToken = (token: string) =>
  adapter.invoke<SavedToken>("save_token", { token });
export const clearToken = () => adapter.invoke<void>("clear_token");

// add and remove resolve to the authoritative watched list.
export const listRepos = () => adapter.invoke<string[]>("list_repos");
export const addRepo = (name: string) =>
  adapter.invoke<string[]>("add_repo", { name });
export const removeRepo = (name: string) =>
  adapter.invoke<string[]>("remove_repo", { name });
export const listAffiliatedRepos = () =>
  adapter.invoke<string[]>("list_affiliated_repos");

export const getPollInterval = () => adapter.invoke<number>("get_poll_interval");
/** Resolves to the interval the backend stored, which it may have clamped. */
export const setPollInterval = (secs: number) =>
  adapter.invoke<number>("set_poll_interval", { secs });
/** The palette the user picked. "system" follows the desktop's style. */
export type Appearance = "system" | "light" | "dark";
export const getAppearance = () => adapter.invoke<Appearance>("get_appearance");
/** Stores the choice and resolves to it. Showing it is appearance.ts's job. */
export const setAppearance = (appearance: Appearance) =>
  adapter.invoke<Appearance>("set_appearance", { appearance });

/** The lengths a snooze limit can be, in days, shortest first. A month is 30
 * days. The backend refuses any other. */
export const SNOOZE_LIMIT_DAYS = [1, 3, 7, 14, 30, 90] as const;

/** How long a snooze may last. A change applies to snoozes made after it. */
export interface SnoozeLimit {
  /** Off means a snooze has no expiry and only news wakes the row. */
  enabled: boolean;
  /** The first, second and third duration offered when snoozing, each one
   * of SNOOZE_LIMIT_DAYS. Kept while `enabled` is off. Two may be alike. */
  days: [number, number, number];
}

/** What a settings file that has never held a limit loads as; equal to the
 * Rust `SnoozeLimit::default()`. */
export const DEFAULT_SNOOZE_LIMIT: SnoozeLimit = {
  enabled: true,
  days: [1, 7, 30],
};

export const getSnoozeLimit = () => adapter.invoke<SnoozeLimit>("get_snooze_limit");
/** Stores the limit and resolves to it as stored. */
export const setSnoozeLimit = (limit: SnoozeLimit) =>
  adapter.invoke<SnoozeLimit>("set_snooze_limit", { limit });
export const getLaunchAtLogin = () => adapter.invoke<boolean>("get_launch_at_login");
export const setLaunchAtLogin = (enabled: boolean) =>
  adapter.invoke<void>("set_launch_at_login", { enabled });

export const openUrl = (url: string) => adapter.openUrl(url);
export const appVersion = () => adapter.appVersion();
