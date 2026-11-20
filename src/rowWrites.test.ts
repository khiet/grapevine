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
import type { WriteId } from "./popoverView";
import {
  clickWrite,
  requestWrite,
  resetWritesPending,
  writesPending,
} from "./rowWrites";

afterEach(() => {
  setAdapter(tauriAdapter);
  resetWritesPending();
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
  threads_open: false,
  ready_to_merge: false,
  turn: "yours",
  turn_since: null,
  can_fix_conflict: true,
  fix_requested: false,
  agent_task: null,
  actions: [],
  merge_method: "merge",
  base_branch: "main",
  head_oid: "abc123",
  unread_count: 0,
  bot_unread_count: 0,
};

const READY_KEY = "acme/widgets#8";

/** One of the viewer's PRs with nothing left but the merge, which can also
 * be left to auto-merge. */
const ready: PullRequest = {
  ...conflicted,
  number: 8,
  url: "https://github.com/acme/widgets/pull/8",
  blocked_reasons: [],
  can_fix_conflict: false,
  ready_to_merge: true,
  actions: ["merge", "enable_auto_merge"],
};

/** Installs a backend holding one fixable PR and one ready PR. `asked`
 * collects the keys the fix was asked for, in order, `sent` the other writes
 * as "key action", and `snapshot` reads back what the backend now holds.
 * `fail` makes the backend turn each write down. */
function backend(fail: string | null = null) {
  const fake = createFakeBackend({
    ...EMPTY_SNAPSHOT,
    prs: [conflicted, ready],
    has_synced: true,
  });
  const asked: string[] = [];
  const sent: string[] = [];
  const adapter: Adapter = {
    ...fake.adapter,
    async invoke<T>(command: string, args?: Record<string, unknown>) {
      if (command === "fix_conflict") asked.push(args?.key as string);
      if (command === "act_on_pr") sent.push(`${args?.key} ${args?.action}`);
      if (fail !== null && (command === "fix_conflict" || command === "act_on_pr")) {
        throw fail;
      }
      return fake.adapter.invoke<T>(command, args);
    },
  };
  setAdapter(adapter);
  const snapshot = () => fake.adapter.invoke<Snapshot>("get_prs");
  const pr = async () => (await snapshot()).prs[0];
  return { asked, sent, pr, snapshot };
}

/** A row's writes as the row holds them: which one is armed, carried from
 * click to click. */
function control(pr: PullRequest) {
  let armed: WriteId | null = null;
  return {
    async click(id: WriteId = "fix"): Promise<string | null> {
      const next = clickWrite(pr, id, armed);
      armed = next.armed;
      return next.request;
    },
  };
}

const requestFix = (pr: PullRequest) => requestWrite(pr, "fix");

test("one click never posts", async () => {
  const { asked, pr } = backend();
  await control(conflicted).click();
  expect(asked).toEqual([]);
  expect((await pr()).fix_requested).toBe(false);
});

test("two clicks post exactly once, and the PR then waits on Copilot", async () => {
  const { asked, pr } = backend();
  const fix = control(conflicted);
  await fix.click();
  expect(await fix.click()).toBeNull();
  expect(asked).toEqual([KEY]);
  expect(await pr()).toMatchObject({ fix_requested: true, turn: "theirs" });
});

test("a request reads as posting only while it is on its way", async () => {
  backend();
  const first = requestFix(conflicted);
  expect(writesPending().has(KEY)).toBe(true);
  await first;
  expect(writesPending().has(KEY)).toBe(false);
});

test("a second request made while the first is on its way posts nothing", async () => {
  const { asked } = backend();
  await Promise.all([requestFix(conflicted), requestFix(conflicted)]);
  expect(asked).toEqual([KEY]);
});

test("a failed post hands back the backend's message and leaves the row free to ask again", async () => {
  const { asked, pr } = backend("GitHub error: Resource not accessible.");
  expect(await requestFix(conflicted)).toBe("GitHub error: Resource not accessible.");
  expect(writesPending().has(KEY)).toBe(false);
  expect((await pr()).fix_requested).toBe(false);
  await requestFix(conflicted);
  expect(asked).toEqual([KEY, KEY]);
});

test("the backend refuses a PR that is not fixable", async () => {
  backend();
  expect(await requestFix(ready)).toBe(
    "Nothing here for Copilot to fix.",
  );
});

test("the backend refuses a second request for a PR that already waits on Copilot", async () => {
  backend();
  await requestFix(conflicted);
  expect(await requestFix(conflicted)).toBe(
    "Copilot has already been asked to fix this.",
  );
});

test("one click on a write never sends it", async () => {
  const { sent } = backend();
  await control(ready).click("merge");
  expect(sent).toEqual([]);
});

test("two clicks on Merge merge once, and the PR moves to Merged", async () => {
  const { sent, snapshot } = backend();
  const row = control(ready);
  await row.click("merge");
  expect(await row.click("merge")).toBeNull();
  expect(sent).toEqual([`${READY_KEY} merge`]);
  const after = await snapshot();
  expect(after.prs.map((pr) => pr.number)).toEqual([7]);
  expect(after.merged.map((pr) => pr.number)).toEqual([8]);
});

test("a click on another write stands the armed one down and arms that one instead", async () => {
  const { sent } = backend();
  const row = control(ready);
  await row.click("merge");
  await row.click("enable_auto_merge");
  expect(sent).toEqual([]);
  await row.click("enable_auto_merge");
  expect(sent).toEqual([`${READY_KEY} enable_auto_merge`]);
});

test("auto-merge armed from the row shows on the PR, which then offers to cancel it", async () => {
  const { snapshot } = backend();
  await requestWrite(ready, "enable_auto_merge");
  expect((await snapshot()).prs[1]).toMatchObject({
    auto_merge: true,
    ready_to_merge: false,
    turn: "theirs",
    actions: ["disable_auto_merge"],
  });
  await requestWrite(ready, "disable_auto_merge");
  expect((await snapshot()).prs[1]).toMatchObject({
    auto_merge: false,
    actions: ["enable_auto_merge"],
  });
});

test("a write reads as pending only while it is on its way, under its own name", async () => {
  backend();
  const merging = requestWrite(ready, "merge");
  expect(writesPending().get(READY_KEY)).toBe("merge");
  await merging;
  expect(writesPending().has(READY_KEY)).toBe(false);
});

test("a row sends one write at a time", async () => {
  const { sent } = backend();
  await Promise.all([
    requestWrite(ready, "merge"),
    requestWrite(ready, "enable_auto_merge"),
  ]);
  expect(sent).toEqual([`${READY_KEY} merge`]);
});

test("a write GitHub turns down hands back its message, leaves the PR alone and can be tried again", async () => {
  const { sent, snapshot } = backend("GitHub error: Head branch was modified.");
  expect(await requestWrite(ready, "merge")).toBe(
    "GitHub error: Head branch was modified.",
  );
  expect((await snapshot()).prs[1].actions).toEqual(["merge", "enable_auto_merge"]);
  await requestWrite(ready, "merge");
  expect(sent).toEqual([`${READY_KEY} merge`, `${READY_KEY} merge`]);
});

test("the backend refuses a write the PR does not offer", async () => {
  const { snapshot } = backend();
  expect(await requestWrite(ready, "update_branch")).toBe(
    "That is no longer on offer for this PR.",
  );
  expect((await snapshot()).prs).toHaveLength(2);
});

// The row was drawn at one head and the backend has since synced a newer
// one: the click confirmed commits the user was never shown.
test("a write confirmed on a row the PR has moved on from is refused, and the PR is left alone", async () => {
  const { snapshot } = backend();
  const drawnEarlier = { ...ready, head_oid: "an-older-head" };
  expect(await requestWrite(drawnEarlier, "merge")).toBe(
    "That PR has new commits. Check it, then try again.",
  );
  expect((await snapshot()).prs).toHaveLength(2);
});
