import { afterEach, beforeEach, expect, test } from "vitest";
import { setAdapter, setSnoozeLimit, type SnoozeLimit } from "./backend";
import { createFakeBackend } from "./backend/fake";
import { tauriAdapter } from "./backend/tauri";
import {
  chooseSnoozeLimit,
  loadSnoozeLimit,
  resetSnoozeLimit,
  snoozeLimit,
} from "./snoozeLimit";

const SNAPSHOT = {
  prs: [],
  merged: [],
  snoozed: [],
  has_synced: true,
  last_sync_at: null,
  sync_error: null,
  syncing: false,
};
const DEFAULT: SnoozeLimit = { enabled: true, days: [1, 7, 30] };
const CHOSEN: SnoozeLimit = { enabled: false, days: [3, 14, 90] };

beforeEach(() => {
  resetSnoozeLimit();
  setAdapter(createFakeBackend(SNAPSHOT).adapter);
});
afterEach(() => setAdapter(tauriAdapter));

test("before the stored limit arrives, the limit is the default", () => {
  expect(snoozeLimit()).toEqual(DEFAULT);
});

test("loading brings in the stored limit", async () => {
  await setSnoozeLimit(CHOSEN);
  await loadSnoozeLimit();
  expect(snoozeLimit()).toEqual(CHOSEN);
});

test("a choice is stored, and is the limit the next snooze reads", async () => {
  expect(await chooseSnoozeLimit(CHOSEN)).toEqual(CHOSEN);
  expect(snoozeLimit()).toEqual(CHOSEN);

  resetSnoozeLimit();
  await loadSnoozeLimit();
  expect(snoozeLimit()).toEqual(CHOSEN);
});

// The stored value was read before the choice was made.
test("a load that lands after a choice does not undo it", async () => {
  const loading = loadSnoozeLimit();
  const choosing = chooseSnoozeLimit(CHOSEN);
  await Promise.all([loading, choosing]);
  expect(snoozeLimit()).toEqual(CHOSEN);
});

test("settings that cannot be read leave the default in place", async () => {
  setAdapter({
    ...createFakeBackend(SNAPSHOT).adapter,
    invoke: () => Promise.reject("cannot read settings.json"),
  });
  await loadSnoozeLimit();
  expect(snoozeLimit()).toEqual(DEFAULT);
});

test("a choice the backend refuses changes nothing", async () => {
  setAdapter({
    ...createFakeBackend(SNAPSHOT).adapter,
    invoke: () => Promise.reject("That is not a snooze length on offer."),
  });
  await expect(chooseSnoozeLimit(CHOSEN)).rejects.toBe(
    "That is not a snooze length on offer.",
  );
  expect(snoozeLimit()).toEqual(DEFAULT);
});
