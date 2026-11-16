// The popover's side of a row's writes to GitHub (the Copilot fix, the merge,
// the queue, the branch update, auto-merge on and off): the two clicks each
// takes, and which rows have one on its way. Those live here, outside any
// component, so they outlast a collapsed section, a filter and a visit to
// Settings. Nothing here remembers a write that got there: the backend shows
// it, on the PR in the Snapshot it pushes.

import { useSyncExternalStore } from "react";
import { actOnPr, fixConflict } from "./backend";
import type { WriteId, WritesPending } from "./popoverView";

// Replaced, never mutated: useSyncExternalStore compares by identity.
let pending: WritesPending = new Map();
const listeners = new Set<() => void>();

function setPending(key: string, id: WriteId | null): void {
  const next = new Map(pending);
  if (id === null) next.delete(key);
  else next.set(key, id);
  pending = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export const writesPending = (): WritesPending => pending;

/** The writes on their way, re-rendering the caller when one changes. */
export function useWritesPending(): WritesPending {
  return useSyncExternalStore(subscribe, writesPending);
}

/** Forgets every write on its way. For tests, which share this module's
 * state. */
export function resetWritesPending(): void {
  pending = new Map();
}

/** Sends the write `id` for the PR filed under `key`. Resolves to null once
 * GitHub has taken it, or to the message to show on the row when it has not.
 * A PR with a write already on its way sends nothing more: what it offers is
 * about to change. Past that, the backend is what refuses a write the PR no
 * longer offers. */
export async function requestWrite(
  key: string,
  id: WriteId,
): Promise<string | null> {
  if (pending.has(key)) return null;
  setPending(key, id);
  try {
    await (id === "fix" ? fixConflict(key) : actOnPr(key, id));
    return null;
  } catch (error) {
    // The backend rejects with the message itself.
    if (typeof error === "string") return error;
    return id === "fix"
      ? "Copilot could not be asked."
      : "GitHub could not be asked.";
  } finally {
    // Either the Snapshot now shows what the write did, or the control
    // comes back for another try.
    setPending(key, null);
  }
}

/** A click on the control of write `id` on the PR filed under `key`, where
 * `armed` is the write that row has armed, if any. The first click on a
 * write only arms it, and the second sends it: every one of them acts on
 * GitHub under the user's name, so one stray click must never send it.
 * Clicking a different write stands the armed one down and arms that one.
 * Returns the write armed now, and the request when this click made one (see
 * requestWrite for what it resolves to). The caller holds `armed` and stands
 * it down on its own schedule. */
export function clickWrite(
  key: string,
  id: WriteId,
  armed: WriteId | null,
): { armed: WriteId | null; request: Promise<string | null> | null } {
  return armed === id
    ? { armed: null, request: requestWrite(key, id) }
    : { armed: id, request: null };
}
