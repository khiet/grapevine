// The popover's side of Fix with Copilot: which rows have had the fix asked
// for, and the two clicks it takes to ask. The requests live here, outside
// any component, so they outlast a collapsed section, a filter and a visit to
// Settings. They last as long as the webview does and are never stored.

import { useSyncExternalStore } from "react";
import { fixConflict } from "./backend";
import type { FixRequest, FixRequests } from "./popoverView";

// Replaced, never mutated: useSyncExternalStore compares by identity.
let requests: FixRequests = new Map();
const listeners = new Set<() => void>();

function record(key: string, state: FixRequest | null): void {
  const next = new Map(requests);
  if (state === null) next.delete(key);
  else next.set(key, state);
  requests = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export const fixRequests = (): FixRequests => requests;

/** The requests, re-rendering the caller when one changes. */
export function useFixRequests(): FixRequests {
  return useSyncExternalStore(subscribe, fixRequests);
}

/** Forgets every request. For tests, which share this module's state. */
export function resetFixRequests(): void {
  requests = new Map();
}

/** Asks Copilot to fix the conflict on the PR filed under `key`. Resolves to
 * null once the comment is posted, or to the message to show on the row when
 * it is not. A key already asked for, or being asked for, posts nothing more:
 * that is what keeps one PR to one comment however the control is clicked. */
export async function requestFix(key: string): Promise<string | null> {
  if (requests.has(key)) return null;
  record(key, "posting");
  try {
    await fixConflict(key);
    record(key, "requested");
    return null;
  } catch (error) {
    // A failure leaves no trace, so the control comes back for another try.
    record(key, null);
    // The backend rejects with the message itself.
    return typeof error === "string" ? error : "Copilot could not be asked.";
  }
}

/** A click on the control of the PR filed under `key`. The first only arms
 * it, and the second posts: the comment goes out under the user's name, so
 * one stray click must never send it. Returns whether the control is armed
 * now, and the request when this click made one (see requestFix for what it
 * resolves to). The caller holds `armed` and stands it down on its own
 * schedule. */
export function clickFix(
  key: string,
  armed: boolean,
): { armed: boolean; request: Promise<string | null> | null } {
  return armed
    ? { armed: false, request: requestFix(key) }
    : { armed: true, request: null };
}
