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
  unread_count: number;
}

/** Which popover section a PR belongs to, from the viewer's perspective. */
export type Section = "mine" | "participated" | "all";

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
}

export interface Snapshot {
  prs: PullRequest[];
  merged: MergedPr[];
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
  has_synced: false,
  last_sync_at: null,
  sync_error: null,
};

/** The key the backend files unread and merged state under. The Rust side
 * builds the same string; the two must agree or mark_read and dismiss_merged
 * silently miss. */
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
