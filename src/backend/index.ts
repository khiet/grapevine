// What the UI knows about the Rust backend's commands and events: the wire
// types, the PR key, one typed function per command, and the Snapshot
// subscription. Components import from here and never from @tauri-apps/api.
// The updater plugins are the one exception; updater.ts owns those.
//
// The wire types are hand-written copies of the Rust structs that serialize
// them (github.rs, merged.rs, pr_state.rs, commands.rs). A field added or
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
  /** Why the PR is blocked, already in severity order; empty means no pill.
   * Composed at the Rust boundary; the row only maps keys to labels. */
  blocked_reasons: BlockedReason[];
  /** Drafts render a neutral pill; the backend suppresses their blocked
   * reasons and review markers. */
  is_draft: boolean;
  /** The viewer's review is requested and not yet acted on; renders the
   * glasses glyph with an incoming arrow. Backend-computed, suppressed on
   * drafts, and self-clearing once the viewer reviews. */
  review_requested: boolean;
  /** A review is outstanding from one of the viewer's teams, with no request
   * naming the viewer; renders the glasses glyph with the team glyph. Never
   * set beside review_requested or awaiting_review, suppressed on drafts,
   * self-clearing once a team member reviews, and never part of the unread
   * count. */
  team_review_requested: boolean;
  /** One of the viewer's own PRs is waiting on a reviewer; renders the glasses
   * glyph with an outgoing arrow. The Mine-only mirror of review_requested
   * (the two never share a row), suppressed on drafts, and self-clearing as
   * reviewers submit. */
  awaiting_review: boolean;
  /** The review decision is an explicit APPROVED; renders the green check in
   * the marker cluster. Backend-computed and suppressed on drafts, like the
   * review markers. */
  approved: boolean;
  /** 1-based position in the repo's merge queue as GitHub reports it; null
   * when the PR is not queued. Renders the stack mark with the number. */
  merge_queue_position: number | null;
  /** Names of the failed checks behind the "ci" reason, GitHub's order; the
   * CI pill names the first. Empty when "ci" is not raised, and when it is
   * raised with no check to name. */
  failing_checks: string[];
  /** The PR has more checks than the backend read, so failing_checks may be
   * short: say "+ more", never a count. */
  failing_checks_truncated: boolean;
  /** A check that gates the merge is still running and none has failed;
   * renders the neutral Running mark. Suppressed on drafts. */
  checks_running: boolean;
  /** Auto-merge is armed; renders the Auto mark. Suppressed on drafts. */
  auto_merge: boolean;
  /** Nothing is left but pressing merge; renders the Ready to merge pill.
   * Backend-computed and only ever set in Mine. */
  ready_to_merge: boolean;
  /** Whose move the PR waits on; null when there is no verdict, always so on
   * a draft and in All. Backend-computed. A "yours" row sits in Your turn
   * and tints its mark, and the tray shows how many there are; "theirs" renders
   * nothing of its own. Never part of the unread count. */
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
   * and no sync has shown the conflict gone since; renders the Copilot fixing
   * mark in place of the fix control. Backend-kept, so it survives a restart.
   * Such a PR waits on Copilot: its turn is never "yours" unless a review is
   * asked of the viewer by name. */
  fix_requested: boolean;
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

// The backend answers each of these by pushing a fresh Snapshot.
export const markRead = (key: string) => adapter.invoke<void>("mark_read", { key });
export const markAllRead = () => adapter.invoke<void>("mark_all_read");
export const dismissMerged = (key: string) =>
  adapter.invoke<void>("dismiss_merged", { key });
export const clearMerged = () => adapter.invoke<void>("clear_merged");
/** Parks a Mine or Participated row until it has news or morning comes. The
 * expiry is worked out here because the webview knows the local time zone. */
export const snooze = (key: string) =>
  adapter.invoke<void>("snooze", {
    key,
    expiresAt: nextLocalMorning(new Date()),
  });
export const wake = (key: string) => adapter.invoke<void>("wake", { key });

/** Asks Copilot to resolve the PR's merge conflict, with a comment posted on
 * GitHub as the user: the one command that writes there. Only the key goes
 * over; the backend picks the PR and the words. Rejects with the message to
 * show on the row. A posted comment is answered with a Snapshot in which the
 * PR has fix_requested set; the backend refuses a second one while that
 * lasts. */
export const fixConflict = (key: string) =>
  adapter.invoke<void>("fix_conflict", { key });

/** Epoch ms of the next local 08:00 strictly after `now`: when a snooze runs
 * out on its own. */
export function nextLocalMorning(now: Date): number {
  const morning = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 8);
  // Calendar arithmetic, not +24h: a DST day is not 24 hours long.
  if (morning.getTime() <= now.getTime()) morning.setDate(morning.getDate() + 1);
  return morning.getTime();
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
export const getLaunchAtLogin = () => adapter.invoke<boolean>("get_launch_at_login");
export const setLaunchAtLogin = (enabled: boolean) =>
  adapter.invoke<void>("set_launch_at_login", { enabled });

export const openUrl = (url: string) => adapter.openUrl(url);
export const appVersion = () => adapter.appVersion();
