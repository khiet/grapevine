// The popover's side of Fix with Copilot: the two clicks it takes to ask, and
// which rows have a request on its way to GitHub. Those live here, outside
// any component, so they outlast a collapsed section, a filter and a visit to
// Settings. Nothing here remembers a request that got there: the backend
// does, and says so on the PR in the Snapshot it pushes.

import { useSyncExternalStore } from "react";
import { fixConflict } from "./backend";
import type { FixesPosting } from "./popoverView";

// Replaced, never mutated: useSyncExternalStore compares by identity.
let posting: FixesPosting = new Set();
const listeners = new Set<() => void>();

function setPosting(key: string, on: boolean): void {
  const next = new Set(posting);
  if (on) next.add(key);
  else next.delete(key);
  posting = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export const fixesPosting = (): FixesPosting => posting;

/** The requests on their way, re-rendering the caller when one changes. */
export function useFixesPosting(): FixesPosting {
  return useSyncExternalStore(subscribe, fixesPosting);
}

/** Forgets every request on its way. For tests, which share this module's
 * state. */
export function resetFixesPosting(): void {
  posting = new Set();
}

/** Asks Copilot to fix the conflict on the PR filed under `key`. Resolves to
 * null once the comment is posted, or to the message to show on the row when
 * it is not. A key already being asked for posts nothing more; once the
 * comment is out, the backend is what keeps the PR to one. */
export async function requestFix(key: string): Promise<string | null> {
  if (posting.has(key)) return null;
  setPosting(key, true);
  try {
    await fixConflict(key);
    return null;
  } catch (error) {
    // The backend rejects with the message itself.
    return typeof error === "string" ? error : "Copilot could not be asked.";
  } finally {
    // Either the Snapshot now shows the PR waiting on Copilot, or the
    // control comes back for another try.
    setPosting(key, false);
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
