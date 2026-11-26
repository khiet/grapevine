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

// The read is held open until the choice is stored, then answers with what
// the file held before it.
test("a load that lands after a choice does not undo it", async () => {
  const fake = createFakeBackend(SNAPSHOT).adapter;
  let release = () => {};
  setAdapter({
    ...fake,
    invoke: (command, args) =>
      command === "get_snooze_limit"
        ? new Promise((resolve) => {
            release = () => resolve(DEFAULT as never);
          })
        : fake.invoke(command, args),
  });

  const loading = loadSnoozeLimit();
  await chooseSnoozeLimit(CHOSEN);
  release();
  await loading;

  expect(snoozeLimit()).toEqual(CHOSEN);
});

// Settings shows the limit from here, so the second dropdown changed must
// not send the first one's old value back.
test("a second choice made before the first is stored builds on the first", async () => {
  const [, second, third] = snoozeLimit().days;
  const first = chooseSnoozeLimit({ enabled: true, days: [3, second, third] });
  const shown = snoozeLimit().days;
  const next = chooseSnoozeLimit({ enabled: true, days: [shown[0], 14, shown[2]] });
  await Promise.all([first, next]);

  resetSnoozeLimit();
  await loadSnoozeLimit();
  expect(snoozeLimit()).toEqual({ enabled: true, days: [3, 14, 30] });
});

test("settings that cannot be read leave the default in place", async () => {
  setAdapter({
    ...createFakeBackend(SNAPSHOT).adapter,
    invoke: () => Promise.reject("cannot read settings.json"),
  });
  await loadSnoozeLimit();
  expect(snoozeLimit()).toEqual(DEFAULT);
});

test("a choice the backend refuses leaves the limit as last stored", async () => {
  await chooseSnoozeLimit({ enabled: true, days: [3, 7, 30] });
  setAdapter({
    ...createFakeBackend(SNAPSHOT).adapter,
    invoke: () => Promise.reject("That is not a snooze length on offer."),
  });
  await expect(chooseSnoozeLimit(CHOSEN)).rejects.toBe(
    "That is not a snooze length on offer.",
  );
  expect(snoozeLimit()).toEqual({ enabled: true, days: [3, 7, 30] });
});
