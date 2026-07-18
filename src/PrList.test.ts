import { expect, test } from "vitest";
import {
  blockedTitle,
  formatLastSync,
  formatUpdated,
  groupByRepo,
  matchesFilter,
  totalUnread,
  PullRequest,
} from "./PrList";

// formatUpdated works in local calendar days, so fixtures are built in local
// time; the expectations then hold in any timezone.
const at = (y: number, month: number, d: number, h = 0, m = 0) =>
  new Date(y, month - 1, d, h, m);
const NOW = at(2026, 7, 16, 15, 30);

test("updates from today render as a 24-hour clock time", () => {
  expect(formatUpdated(at(2026, 7, 16, 14, 59).toISOString(), NOW)).toBe("14:59");
  expect(formatUpdated(at(2026, 7, 16, 9, 5).toISOString(), NOW)).toBe("09:05");
});

test("timestamps slightly in the future still render as a time", () => {
  expect(formatUpdated(at(2026, 7, 17, 0, 10).toISOString(), NOW)).toBe("00:10");
});

test("yesterday is a calendar-day split, not a 24-hour window", () => {
  expect(formatUpdated(at(2026, 7, 15, 23, 59).toISOString(), NOW)).toBe("Yesterday");
  expect(formatUpdated(at(2026, 7, 15, 0, 0).toISOString(), NOW)).toBe("Yesterday");
});

test("older dates render as day and short month", () => {
  expect(formatUpdated(at(2026, 7, 14, 12, 0).toISOString(), NOW)).toBe("14 Jul");
  expect(formatUpdated(at(2026, 1, 22, 8, 0).toISOString(), NOW)).toBe("22 Jan");
  expect(formatUpdated(at(2025, 12, 31, 8, 0).toISOString(), NOW)).toBe("31 Dec");
});

test("the footer label reuses the row-timestamp format", () => {
  expect(formatLastSync(at(2026, 7, 16, 14, 59).getTime(), NOW)).toBe(
    "Synced 14:59",
  );
  expect(formatLastSync(at(2026, 7, 15, 23, 59).getTime(), NOW)).toBe(
    "Synced Yesterday",
  );
  expect(formatLastSync(at(2026, 7, 1, 8, 0).getTime(), NOW)).toBe(
    "Synced 1 Jul",
  );
});

test("no sync yet means no footer label", () => {
  expect(formatLastSync(null, NOW)).toBeNull();
});

const prWithUnread = (unread_count: number): PullRequest => ({
  number: 7,
  title: "Fix the thing",
  url: "https://github.com/acme/widgets/pull/7",
  repo: "acme/widgets",
  author: "someone",
  avatar_url: "https://avatars.example/someone",
  owner_avatar_url: "https://avatars.example/acme",
  created_at: "2026-07-10T12:00:00Z",
  updated_at: "2026-07-11T09:30:00Z",
  section: "all",
  blocked_reasons: [],
  is_draft: false,
  unread_count,
});

test("the tray-facing total sums unread across PRs", () => {
  expect(totalUnread([])).toBe(0);
  expect(totalUnread([prWithUnread(2), prWithUnread(0), prWithUnread(5)])).toBe(7);
});

const pr = (repo: string, number: number, updated_at: string): PullRequest => ({
  ...prWithUnread(0),
  number,
  repo,
  updated_at,
  url: `https://github.com/${repo}/pull/${number}`,
});

test("PRs collapse into one group per repo", () => {
  const groups = groupByRepo([
    pr("acme/widgets", 1, "2026-07-10T00:00:00Z"),
    pr("acme/gadgets", 2, "2026-07-11T00:00:00Z"),
    pr("acme/widgets", 3, "2026-07-09T00:00:00Z"),
  ]);
  expect(groups.map((g) => g.repo)).toEqual(["acme/gadgets", "acme/widgets"]);
  expect(groups[1].prs.map((p) => p.number)).toEqual([1, 3]);
});

test("the repo whose newest PR moved most recently leads", () => {
  // acme/widgets holds the single freshest PR; acme/gadgets has more PRs and
  // wins on average recency, so this pins newest-wins over count-or-average.
  const groups = groupByRepo([
    pr("acme/gadgets", 1, "2026-07-14T00:00:00Z"),
    pr("acme/gadgets", 2, "2026-07-13T00:00:00Z"),
    pr("acme/widgets", 3, "2026-07-01T00:00:00Z"),
    pr("acme/widgets", 4, "2026-07-15T00:00:00Z"),
  ]);
  expect(groups.map((g) => g.repo)).toEqual(["acme/widgets", "acme/gadgets"]);
});

test("rows keep their incoming order within a group", () => {
  const groups = groupByRepo([
    pr("acme/widgets", 1, "2026-07-09T00:00:00Z"),
    pr("acme/widgets", 2, "2026-07-15T00:00:00Z"),
    pr("acme/widgets", 3, "2026-07-12T00:00:00Z"),
  ]);
  expect(groups[0].prs.map((p) => p.number)).toEqual([1, 2, 3]);
});

test("an unparseable timestamp sinks its group instead of throwing", () => {
  const groups = groupByRepo([
    pr("acme/broken", 1, "not-a-date"),
    pr("acme/widgets", 2, "2026-07-01T00:00:00Z"),
  ]);
  expect(groups.map((g) => g.repo)).toEqual(["acme/widgets", "acme/broken"]);
});

test("an empty list produces no groups", () => {
  expect(groupByRepo([])).toEqual([]);
});

const filterable = {
  title: "Fix the flaky sync test",
  repo: "acme/widgets",
  author: "octocat",
  number: 42,
};

test("an empty or whitespace query matches every row", () => {
  expect(matchesFilter(filterable, "")).toBe(true);
  expect(matchesFilter(filterable, "   ")).toBe(true);
});

test("a term matches title, repo, author, or #number, case-insensitively", () => {
  expect(matchesFilter(filterable, "FLAKY")).toBe(true);
  expect(matchesFilter(filterable, "widgets")).toBe(true);
  expect(matchesFilter(filterable, "octocat")).toBe(true);
  expect(matchesFilter(filterable, "#42")).toBe(true);
  expect(matchesFilter(filterable, "42")).toBe(true);
  expect(matchesFilter(filterable, "gadgets")).toBe(false);
  // Folding runs on both sides: a lowercase query matches the capital "F" in
  // the title, which pins haystack-side folding, not just the query's.
  expect(matchesFilter(filterable, "fix")).toBe(true);
});

test("multiple terms are AND'd across the searchable fields", () => {
  // "acme" hits the repo, "flaky" hits the title: both must match.
  expect(matchesFilter(filterable, "acme flaky")).toBe(true);
  expect(matchesFilter(filterable, "acme missing")).toBe(false);
});

// The keys are the wire contract with the Rust BlockedReason enum; the labels
// are the tooltip copy, kept neutral (a PR property, never "you must act").
test("the dot's tooltip names every reason in the backend's order", () => {
  expect(blockedTitle(["conflict", "ci", "review"])).toBe(
    "Merge conflict; CI failing; Changes requested",
  );
  expect(blockedTitle(["ci"])).toBe("CI failing");
});
