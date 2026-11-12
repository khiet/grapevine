import { afterEach, expect, test } from "vitest";
import {
  EMPTY_SNAPSHOT,
  setAdapter,
  type Adapter,
  type PullRequest,
  type Snapshot,
} from "./backend";
import { createFakeBackend } from "./backend/fake";
import { tauriAdapter } from "./backend/tauri";
import {
  clickFix,
  fixRequests,
  forgetCleared,
  requestFix,
  resetFixRequests,
} from "./fixConflict";

afterEach(() => {
  setAdapter(tauriAdapter);
  resetFixRequests();
});

const KEY = "acme/widgets#7";

const conflicted: PullRequest = {
  number: 7,
  title: "Fix the thing",
  url: "https://github.com/acme/widgets/pull/7",
  repo: "acme/widgets",
  author: "someone",
  author_is_bot: false,
  avatar_url: "",
  owner_avatar_url: "",
  created_at: "2026-07-10T12:00:00Z",
  updated_at: "2026-07-11T09:30:00Z",
  section: "mine",
  blocked_reasons: ["conflict"],
  is_draft: false,
  review_requested: false,
  team_review_requested: false,
  awaiting_review: false,
  approved: false,
  merge_queue_position: null,
  failing_checks: [],
  failing_checks_truncated: false,
  checks_running: false,
  auto_merge: false,
  ready_to_merge: false,
  turn: "yours",
  turn_since: null,
  can_fix_conflict: true,
  unread_count: 0,
  bot_unread_count: 0,
};

const synced = (prs: PullRequest[]): Snapshot => ({
  ...EMPTY_SNAPSHOT,
  prs,
  has_synced: true,
});

/** Installs a backend holding one fixable PR and returns the keys the fix
 * was asked for, in order. `fail` makes the backend turn each request down. */
function backend(fail: string | null = null): string[] {
  const fake = createFakeBackend({
    ...EMPTY_SNAPSHOT,
    prs: [conflicted],
    has_synced: true,
  });
  const asked: string[] = [];
  const adapter: Adapter = {
    ...fake.adapter,
    async invoke<T>(command: string, args?: Record<string, unknown>) {
      if (command === "fix_conflict") {
        asked.push(args?.key as string);
        if (fail !== null) throw fail;
      }
      return fake.adapter.invoke<T>(command, args);
    },
  };
  setAdapter(adapter);
  return asked;
}

/** A control as the row holds it: the armed flag, carried from click to
 * click. */
function control(key: string) {
  let armed = false;
  return {
    async click(): Promise<string | null> {
      const next = clickFix(key, armed);
      armed = next.armed;
      return next.request;
    },
  };
}

test("one click never posts", async () => {
  const asked = backend();
  await control(KEY).click();
  expect(asked).toEqual([]);
  expect(fixRequests().has(KEY)).toBe(false);
});

test("two clicks post exactly once, and the row reads as requested", async () => {
  const asked = backend();
  const fix = control(KEY);
  await fix.click();
  await fix.click();
  expect(asked).toEqual([KEY]);
  expect(fixRequests().get(KEY)).toBe("requested");
});

test("clicking on after the request is made posts nothing more", async () => {
  const asked = backend();
  const fix = control(KEY);
  for (let clicks = 0; clicks < 6; clicks++) await fix.click();
  expect(asked).toEqual([KEY]);
});

test("a second request made while the first is on its way posts nothing", async () => {
  const asked = backend();
  const first = requestFix(KEY);
  expect(fixRequests().get(KEY)).toBe("posting");
  await Promise.all([first, requestFix(KEY)]);
  expect(asked).toEqual([KEY]);
});

test("a failed post hands back the backend's message and leaves the row free to ask again", async () => {
  const asked = backend("GitHub error: Resource not accessible.");
  expect(await requestFix(KEY)).toBe("GitHub error: Resource not accessible.");
  expect(fixRequests().has(KEY)).toBe(false);
  await requestFix(KEY);
  expect(asked).toEqual([KEY, KEY]);
});

test("the backend refuses a PR that is not fixable", async () => {
  backend();
  expect(await requestFix("acme/widgets#8")).toBe(
    "Nothing here for Copilot to fix.",
  );
});

test("a request stands while a sync still shows the conflict", async () => {
  backend();
  await requestFix(KEY);
  forgetCleared(synced([conflicted]));
  expect(fixRequests().get(KEY)).toBe("requested");
});

test("a request is forgotten once a sync shows the row no longer fixable, so a later conflict can ask again", async () => {
  const asked = backend();
  await requestFix(KEY);
  forgetCleared(
    synced([{ ...conflicted, blocked_reasons: [], can_fix_conflict: false }]),
  );
  expect(fixRequests().has(KEY)).toBe(false);
  await requestFix(KEY);
  expect(asked).toEqual([KEY, KEY]);
});

test("a request is forgotten when its PR leaves the list", async () => {
  backend();
  await requestFix(KEY);
  forgetCleared(synced([]));
  expect(fixRequests().has(KEY)).toBe(false);
});

test("a request still on its way is not forgotten by a sync that lands meanwhile", async () => {
  const asked = backend();
  const posting = requestFix(KEY);
  forgetCleared(synced([]));
  await Promise.all([posting, requestFix(KEY)]);
  expect(asked).toEqual([KEY]);
});
