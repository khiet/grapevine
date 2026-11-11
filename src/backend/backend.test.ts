import { afterEach, expect, test } from "vitest";
import {
  dismissMerged,
  listRepos,
  markRead,
  prKey,
  setAdapter,
  subscribeSnapshot,
  type PullRequest,
  type Snapshot,
} from "./index";
import { createFakeBackend } from "./fake";
import { tauriAdapter } from "./tauri";

afterEach(() => setAdapter(tauriAdapter));

function pr(repo: string, number: number, unread_count: number): PullRequest {
  return {
    number,
    title: `PR ${number}`,
    url: `https://github.com/${repo}/pull/${number}`,
    repo,
    author: "octocat",
    author_is_bot: false,
    avatar_url: "",
    owner_avatar_url: "",
    created_at: "2026-09-20T09:00:00Z",
    updated_at: "2026-09-30T09:00:00Z",
    section: "participated",
    blocked_reasons: [],
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
    unread_count,
    bot_unread_count: 0,
  };
}

function snapshot(prs: PullRequest[]): Snapshot {
  return {
    prs,
    merged: [],
    has_synced: true,
    last_sync_at: 1_790_000_000_000,
    sync_error: null,
  };
}

// Lets the subscription's get_prs and listen promises settle.
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("a subscriber receives the initial Snapshot, then each update", async () => {
  const first = snapshot([pr("acme/api", 1, 2)]);
  const second = snapshot([pr("acme/api", 1, 2), pr("acme/web", 7, 0)]);
  const fake = createFakeBackend(first);
  setAdapter(fake.adapter);
  const seen: Snapshot[] = [];

  subscribeSnapshot((next) => seen.push(next));
  await settle();
  expect(seen).toEqual([first]);

  fake.publish(second);
  expect(seen).toEqual([first, second]);
});

test("an update that beats the initial Snapshot is not overwritten by it", async () => {
  const first = snapshot([pr("acme/api", 1, 2)]);
  const second = snapshot([]);
  const fake = createFakeBackend(first);
  // Holds get_prs open until an update has been delivered.
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  setAdapter({
    ...fake.adapter,
    invoke: async (command, args) => {
      const stale = await fake.adapter.invoke(command, args);
      await held;
      return stale as never;
    },
  });
  const seen: Snapshot[] = [];

  subscribeSnapshot((next) => seen.push(next));
  await settle();
  fake.publish(second);
  release();
  await settle();

  expect(seen).toEqual([second]);
});

test("unsubscribing stops delivery", async () => {
  const fake = createFakeBackend(snapshot([]));
  setAdapter(fake.adapter);
  const seen: Snapshot[] = [];

  const unsubscribe = subscribeSnapshot((next) => seen.push(next));
  await settle();
  unsubscribe();
  fake.publish(snapshot([pr("acme/api", 1, 1)]));

  expect(seen).toHaveLength(1);
});

test("the PR key is owner/repo#number", () => {
  expect(prKey({ repo: "acme/api", number: 42 })).toBe("acme/api#42");
});

test("marking a PR read clears its badge and bot note and pushes a Snapshot", async () => {
  const withBotNote = (repo: string, unread: number): PullRequest => ({
    ...pr(repo, 1, unread),
    bot_unread_count: 4,
  });
  const fake = createFakeBackend(
    snapshot([withBotNote("acme/api", 2), withBotNote("acme/web", 3)]),
  );
  setAdapter(fake.adapter);
  const seen: Snapshot[] = [];
  subscribeSnapshot((next) => seen.push(next));
  await settle();

  await markRead("acme/web#1");

  expect(
    seen[seen.length - 1].prs.map((p) => [p.unread_count, p.bot_unread_count]),
  ).toEqual([
    [2, 4],
    [0, 0],
  ]);
});

test("dismissing a merged PR removes only the row with that key", async () => {
  const merged = (repo: string, number: number) => ({
    number,
    title: `PR ${number}`,
    url: `https://github.com/${repo}/pull/${number}`,
    repo,
    author: "octocat",
    avatar_url: "",
    owner_avatar_url: "",
    merged_at: "2026-09-30T09:00:00Z",
  });
  const fake = createFakeBackend({
    ...snapshot([]),
    merged: [merged("acme/api", 5), merged("acme/web", 5)],
  });
  setAdapter(fake.adapter);
  const seen: Snapshot[] = [];
  subscribeSnapshot((next) => seen.push(next));
  await settle();

  await dismissMerged("acme/api#5");

  expect(seen[seen.length - 1].merged.map((pr) => pr.repo)).toEqual(["acme/web"]);
});

test("a command the fake does not implement rejects", async () => {
  setAdapter(createFakeBackend(snapshot([])).adapter);

  await expect(listRepos()).rejects.toThrow("unhandled command list_repos");
});
