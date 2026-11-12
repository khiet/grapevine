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
  fixesPosting,
  requestFix,
  resetFixesPosting,
} from "./fixConflict";

afterEach(() => {
  setAdapter(tauriAdapter);
  resetFixesPosting();
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
  fix_requested: false,
  unread_count: 0,
  bot_unread_count: 0,
};

/** Installs a backend holding one fixable PR. `asked` collects the keys the
 * fix was asked for, in order, and `pr` reads the PR back as the backend now
 * has it. `fail` makes the backend turn each request down. */
function backend(fail: string | null = null) {
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
  const pr = async () => (await fake.adapter.invoke<Snapshot>("get_prs")).prs[0];
  return { asked, pr };
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
  const { asked, pr } = backend();
  await control(KEY).click();
  expect(asked).toEqual([]);
  expect((await pr()).fix_requested).toBe(false);
});

test("two clicks post exactly once, and the PR then waits on Copilot", async () => {
  const { asked, pr } = backend();
  const fix = control(KEY);
  await fix.click();
  expect(await fix.click()).toBeNull();
  expect(asked).toEqual([KEY]);
  expect(await pr()).toMatchObject({ fix_requested: true, turn: "theirs" });
});

test("a request reads as posting only while it is on its way", async () => {
  backend();
  const first = requestFix(KEY);
  expect(fixesPosting().has(KEY)).toBe(true);
  await first;
  expect(fixesPosting().has(KEY)).toBe(false);
});

test("a second request made while the first is on its way posts nothing", async () => {
  const { asked } = backend();
  await Promise.all([requestFix(KEY), requestFix(KEY)]);
  expect(asked).toEqual([KEY]);
});

test("a failed post hands back the backend's message and leaves the row free to ask again", async () => {
  const { asked, pr } = backend("GitHub error: Resource not accessible.");
  expect(await requestFix(KEY)).toBe("GitHub error: Resource not accessible.");
  expect(fixesPosting().has(KEY)).toBe(false);
  expect((await pr()).fix_requested).toBe(false);
  await requestFix(KEY);
  expect(asked).toEqual([KEY, KEY]);
});

test("the backend refuses a PR that is not fixable", async () => {
  backend();
  expect(await requestFix("acme/widgets#8")).toBe(
    "Nothing here for Copilot to fix.",
  );
});

test("the backend refuses a second request for a PR that already waits on Copilot", async () => {
  backend();
  await requestFix(KEY);
  expect(await requestFix(KEY)).toBe(
    "Copilot has already been asked to fix this.",
  );
});
