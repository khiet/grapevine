// The popover's copy of the snooze limit setting. A row reads it at the
// moment it is snoozed, and Settings changes it, so it lives here, outside
// any component: the list and Settings are never mounted together.

import { useSyncExternalStore } from "react";
import {
  DEFAULT_SNOOZE_LIMIT,
  getSnoozeLimit,
  setSnoozeLimit,
  type SnoozeLimit,
} from "./backend";

// Replaced, never mutated: useSyncExternalStore compares by identity.
let current: SnoozeLimit = DEFAULT_SNOOZE_LIMIT;
// Set by a choice, so a stored value that loads late cannot undo it.
let chosen = false;
const listeners = new Set<() => void>();

function set(next: SnoozeLimit): void {
  current = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The limit as last loaded or chosen; the default until then. */
export const snoozeLimit = (): SnoozeLimit => current;

/** The limit, re-rendering the caller when it changes. */
export function useSnoozeLimit(): SnoozeLimit {
  // The third argument is for a render with no store to subscribe to, which
  // is how the tests draw a row.
  return useSyncExternalStore(subscribe, snoozeLimit, snoozeLimit);
}

/** Brings in the stored limit. Never rejects: settings that cannot be read
 * leave the default, which is what the backend would load them as. */
export function loadSnoozeLimit(): Promise<void> {
  return getSnoozeLimit()
    .then((stored) => {
      if (!chosen) set(stored);
    })
    .catch(() => {});
}

/** Stores the limit, then makes it the one the next snooze reads. Rejects,
 * with nothing changed, if the backend refuses. Resolves to the limit as
 * stored. */
export async function chooseSnoozeLimit(limit: SnoozeLimit): Promise<SnoozeLimit> {
  const stored = await setSnoozeLimit(limit);
  chosen = true;
  set(stored);
  return stored;
}

/** Back to the default with nothing chosen. For tests, which share this
 * module's state. */
export function resetSnoozeLimit(): void {
  current = DEFAULT_SNOOZE_LIMIT;
  chosen = false;
}
