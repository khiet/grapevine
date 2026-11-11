import { expect, test } from "vitest";
import {
  EMPTY_SNAPSHOT,
  type MergedPr,
  type PullRequest,
  type Snapshot,
} from "./backend";
import {
  derivePopover,
  type ListView,
  type RowView,
  type SectionKey,
  type SectionView,
} from "./popoverView";

const pr = (overrides: Partial<PullRequest> = {}): PullRequest => ({
  number: 7,
  title: "Fix the thing",
  url: "https://github.com/acme/widgets/pull/7",
  repo: "acme/widgets",
  author: "someone",
  author_is_bot: false,
  avatar_url: "https://avatars.example/someone",
  owner_avatar_url: "https://avatars.example/acme",
  created_at: "2026-07-10T12:00:00Z",
  updated_at: "2026-07-11T09:30:00Z",
  section: "mine",
  blocked_reasons: [],
  is_draft: false,
  review_requested: false,
  team_review_requested: false,
  awaiting_review: false,
  approved: false,
  merge_queue_position: null,
  unread_count: 0,
  bot_unread_count: 0,
  ...overrides,
});

const mergedPr = (overrides: Partial<MergedPr> = {}): MergedPr => ({
  number: 90,
  title: "Ship the release notes",
  url: "https://github.com/acme/widgets/pull/90",
  repo: "acme/widgets",
  author: "someone",
  avatar_url: "https://avatars.example/someone",
  owner_avatar_url: "https://avatars.example/acme",
  merged_at: "2026-07-11T09:30:00Z",
  ...overrides,
});

const synced = (overrides: Partial<Snapshot> = {}): Snapshot => ({
  ...EMPTY_SNAPSHOT,
  has_synced: true,
  ...overrides,
});

const ALL_EXPANDED: Record<SectionKey, boolean> = {
  mine: false,
  participated: false,
  all: false,
  merged: false,
};
const ALL_COLLAPSED: Record<SectionKey, boolean> = {
  mine: true,
  participated: true,
  all: true,
  merged: true,
};

// Narrows to the list; a placeholder where a list was expected fails here.
function list(
  snapshot: Snapshot,
  query = "",
  collapsed = ALL_EXPANDED,
): ListView {
  const { content } = derivePopover(snapshot, query, collapsed);
  if (content.kind !== "list") throw new Error(`expected a list, got ${content.kind}`);
  return content;
}

function section(view: ListView, key: SectionKey): SectionView {
  const found = view.sections.find((s) => s.key === key);
  if (!found) throw new Error(`no ${key} section`);
  return found;
}

// The PR rows of a flat section; fails on a grouped, merged or empty body.
function rows(view: ListView, key: SectionKey): RowView[] {
  const { body } = section(view, key);
  if (body.kind !== "rows") throw new Error(`${key} body is ${body.kind}`);
  return body.rows;
}

// The single row of a snapshot holding just this PR.
const rowFor = (only: PullRequest): RowView =>
  rows(list(synced({ prs: [only] })), "mine")[0];

const numbers = (view: ListView, key: SectionKey) =>
  rows(view, key).map((row) => row.pr.number);

test("marks run approved check, review glyph, merge queue mark, blocked pills", () => {
  const row = rowFor(
    pr({
      approved: true,
      review_requested: true,
      merge_queue_position: 3,
      blocked_reasons: ["conflict", "ci", "review"],
    }),
  );
  expect(row.marks).toEqual([
    { kind: "approved", tip: "Approved" },
    { kind: "review-requested", tip: "Review requested" },
    { kind: "merge-queue", position: 3, tip: "3rd in merge queue" },
    { kind: "blocked", label: "Merge conflict", tip: null },
    { kind: "blocked", label: "+2", tip: "CI failing; Changes requested" },
  ]);
});

test("the outgoing review glyph takes the review slot on the viewer's own PR", () => {
  const row = rowFor(
    pr({ approved: true, awaiting_review: true, merge_queue_position: 1 }),
  );
  expect(row.marks).toEqual([
    { kind: "approved", tip: "Approved" },
    { kind: "awaiting-review", tip: "Awaiting review" },
    { kind: "merge-queue", position: 1, tip: "1st in merge queue" },
  ]);
});

test("a team review request takes the review slot with its own mark", () => {
  const row = rowFor(
    pr({ approved: true, team_review_requested: true, merge_queue_position: 1 }),
  );
  expect(row.marks).toEqual([
    { kind: "approved", tip: "Approved" },
    { kind: "team-review-requested", tip: "Review requested from your team" },
    { kind: "merge-queue", position: 1, tip: "1st in merge queue" },
  ]);
});

test("a request naming you shows alone, never beside the team mark", () => {
  const row = rowFor(
    pr({ review_requested: true, team_review_requested: true }),
  );
  expect(row.marks).toEqual([
    { kind: "review-requested", tip: "Review requested" },
  ]);
});

test("a PR with no signals has no marks, so no marker cluster", () => {
  const row = rowFor(pr());
  expect(row.marks).toEqual([]);
  expect(row.draft).toBe(false);
});

test("a draft shows the Draft pill and nothing the backend did not send", () => {
  const row = rowFor(pr({ is_draft: true }));
  expect(row.draft).toBe(true);
  expect(row.marks).toEqual([]);
});

test("a draft still shows a signal the backend sends for it", () => {
  const row = rowFor(pr({ is_draft: true, merge_queue_position: 2 }));
  expect(row.draft).toBe(true);
  expect(row.marks).toEqual([
    { kind: "merge-queue", position: 2, tip: "2nd in merge queue" },
  ]);
});

// The keys are the wire contract with the Rust BlockedReason enum; the labels
// are kept neutral (a PR property, never "you must act").
test("a single blocked reason is one spelled-out pill with no tooltip", () => {
  const label = (reason: PullRequest["blocked_reasons"][number]) =>
    rowFor(pr({ blocked_reasons: [reason] })).marks;
  expect(label("conflict")).toEqual([
    { kind: "blocked", label: "Merge conflict", tip: null },
  ]);
  expect(label("ci")).toEqual([
    { kind: "blocked", label: "CI failing", tip: null },
  ]);
  expect(label("review")).toEqual([
    { kind: "blocked", label: "Changes requested", tip: null },
  ]);
  expect(label("threads")).toEqual([
    { kind: "blocked", label: "Unresolved threads", tip: null },
  ]);
  expect(label("behind")).toEqual([
    { kind: "blocked", label: "Behind base", tip: null },
  ]);
});

test("extra reasons collapse into a +N pill whose tooltip names them", () => {
  // The first reason leads because the backend orders by severity.
  expect(rowFor(pr({ blocked_reasons: ["ci", "behind"] })).marks).toEqual([
    { kind: "blocked", label: "CI failing", tip: null },
    { kind: "blocked", label: "+1", tip: "Behind base" },
  ]);
  expect(rowFor(pr({ blocked_reasons: ["threads", "behind"] })).marks).toEqual([
    { kind: "blocked", label: "Unresolved threads", tip: null },
    { kind: "blocked", label: "+1", tip: "Behind base" },
  ]);
});

// The teens are the trap: 11, 12 and 13 take "th" although they end in 1, 2
// and 3.
test("the merge queue tooltip states the position as an English ordinal", () => {
  const tip = (position: number) =>
    rowFor(pr({ merge_queue_position: position })).marks[0].tip;
  expect(tip(1)).toBe("1st in merge queue");
  expect(tip(2)).toBe("2nd in merge queue");
  expect(tip(3)).toBe("3rd in merge queue");
  expect(tip(4)).toBe("4th in merge queue");
  expect(tip(11)).toBe("11th in merge queue");
  expect(tip(12)).toBe("12th in merge queue");
  expect(tip(13)).toBe("13th in merge queue");
  expect(tip(21)).toBe("21st in merge queue");
  expect(tip(22)).toBe("22nd in merge queue");
});

test("with no filter Mine, Participated and All are present even when empty", () => {
  const view = list(synced({ prs: [pr({ section: "participated" })] }));
  expect(view.sections.map((s) => [s.key, s.label, s.count])).toEqual([
    ["mine", "Mine", 0],
    ["participated", "Participated", 1],
    ["all", "All", 0],
  ]);
  expect(section(view, "mine").body).toEqual({ kind: "empty" });
  expect(section(view, "all").body).toEqual({ kind: "empty" });
});

test("Merged follows the PR sections while it holds rows", () => {
  const merged = [mergedPr({ number: 90 }), mergedPr({ number: 91 })];
  const view = list(synced({ merged }));
  expect(view.sections.map((s) => s.key)).toEqual([
    "mine",
    "participated",
    "all",
    "merged",
  ]);
  const mergedSection = section(view, "merged");
  expect(mergedSection.label).toBe("Merged");
  expect(mergedSection.count).toBe(2);
  expect(mergedSection.body).toEqual({ kind: "merged", rows: merged });
});

test("each PR lands in the section the backend assigned it", () => {
  const view = list(
    synced({
      prs: [
        pr({ number: 1, section: "mine" }),
        pr({ number: 2, section: "participated" }),
        pr({ number: 3, section: "mine" }),
      ],
    }),
  );
  expect(numbers(view, "mine")).toEqual([1, 3]);
  expect(numbers(view, "participated")).toEqual([2]);
});

test("sections follow the stored collapse state and can be toggled", () => {
  const view = list(synced(), "", { ...ALL_EXPANDED, mine: true });
  expect(section(view, "mine").expanded).toBe(false);
  expect(section(view, "participated").expanded).toBe(true);
  expect(view.canToggle).toBe(true);
});

const FILTER_FIXTURE = synced({
  prs: [
    pr({ number: 42, section: "mine", title: "Fix the flaky sync test", author: "octocat" }),
    pr({ number: 43, section: "mine", title: "Bump deps", repo: "acme/gadgets" }),
    pr({ number: 44, section: "participated", title: "Rename the tray icon" }),
  ],
  merged: [
    mergedPr({ number: 90, title: "Flaky retry removed" }),
    mergedPr({ number: 91, title: "Docs pass" }),
  ],
});

test("a filter force-expands collapsed sections and keeps only matching rows", () => {
  const view = list(FILTER_FIXTURE, "flaky", ALL_COLLAPSED);
  // Participated has no match, so it drops out rather than showing empty.
  expect(view.sections.map((s) => [s.key, s.count, s.expanded])).toEqual([
    ["mine", 1, true],
    ["merged", 1, true],
  ]);
  expect(numbers(view, "mine")).toEqual([42]);
  expect(section(view, "merged").body).toEqual({
    kind: "merged",
    rows: [FILTER_FIXTURE.merged[0]],
  });
  expect(view.canToggle).toBe(false);
});

test("a term matches title, repo, author, or #number, case-insensitively", () => {
  const match = (query: string) => numbers(list(FILTER_FIXTURE, query), "mine");
  expect(match("FLAKY")).toEqual([42]);
  expect(match("fix")).toEqual([42]);
  expect(match("gadgets")).toEqual([43]);
  expect(match("octocat")).toEqual([42]);
  expect(match("#43")).toEqual([43]);
  expect(match("43")).toEqual([43]);
});

test("multiple terms are AND'd across the searchable fields", () => {
  // "widgets" hits the repo, "flaky" hits the title: both must match.
  expect(numbers(list(FILTER_FIXTURE, "widgets flaky"), "mine")).toEqual([42]);
  expect(derivePopover(FILTER_FIXTURE, "gadgets flaky", ALL_EXPANDED).content)
    .toEqual({ kind: "no-matches", query: "gadgets flaky" });
});

// A whitespace-only query matches everything, so it must not force-expand or
// drop the empty sections either.
test("a whitespace-only query is no filter at all", () => {
  for (const query of ["", "   ", "\t \n"]) {
    const view = list(FILTER_FIXTURE, query, ALL_COLLAPSED);
    expect(view.sections.map((s) => [s.key, s.count, s.expanded])).toEqual([
      ["mine", 2, false],
      ["participated", 1, false],
      ["all", 0, false],
      ["merged", 2, false],
    ]);
    expect(view.canToggle).toBe(true);
  }
});

const UNREAD_FIXTURE = synced({
  prs: [
    pr({ number: 1, unread_count: 2 }),
    pr({ number: 2, unread_count: 0 }),
    pr({ number: 3, unread_count: 5 }),
  ],
});

test("a collapsed section reports its unread total", () => {
  const view = list(UNREAD_FIXTURE, "", { ...ALL_EXPANDED, mine: true });
  expect(section(view, "mine").collapsedUnread).toBe(7);
});

test("an expanded section reports no unread: its rows carry the badges", () => {
  const view = list(UNREAD_FIXTURE, "", ALL_EXPANDED);
  expect(section(view, "mine").collapsedUnread).toBe(0);
});

test("Mark all read is offered while any PR is unread, filtered out or not", () => {
  expect(derivePopover(UNREAD_FIXTURE, "", ALL_EXPANDED).offerMarkAllRead).toBe(true);
  expect(derivePopover(UNREAD_FIXTURE, "zzz", ALL_EXPANDED).offerMarkAllRead).toBe(true);
  expect(
    derivePopover(synced({ prs: [pr()] }), "", ALL_EXPANDED).offerMarkAllRead,
  ).toBe(false);
  expect(derivePopover(EMPTY_SNAPSHOT, "", ALL_EXPANDED).offerMarkAllRead).toBe(false);
});

test("Clear all is offered for Merged only while it is expanded", () => {
  const snapshot = synced({ prs: [pr()], merged: [mergedPr()] });
  const expanded = list(snapshot, "", ALL_EXPANDED);
  expect(section(expanded, "merged").offerClearAll).toBe(true);
  expect(section(expanded, "mine").offerClearAll).toBe(false);

  const collapsed = list(snapshot, "", { ...ALL_EXPANDED, merged: true });
  expect(section(collapsed, "merged").offerClearAll).toBe(false);

  // Force-expanded by the filter, so its rows are visible again.
  const filtered = list(snapshot, "ship", { ...ALL_EXPANDED, merged: true });
  expect(section(filtered, "merged").offerClearAll).toBe(true);
});

test("before the first sync with nothing to show, the setup prompt", () => {
  expect(derivePopover(EMPTY_SNAPSHOT, "", ALL_EXPANDED).content).toEqual({
    kind: "setup",
  });
  // Setup outranks the filter: there is nothing to have matched.
  expect(derivePopover(EMPTY_SNAPSHOT, "fix", ALL_EXPANDED).content).toEqual({
    kind: "setup",
  });
});

test("a synced but empty snapshot shows the empty section scaffold", () => {
  const view = list(synced());
  expect(view.sections.map((s) => [s.key, s.body.kind])).toEqual([
    ["mine", "empty"],
    ["participated", "empty"],
    ["all", "empty"],
  ]);
});

test("rows restored before the first sync show as a list, not the setup prompt", () => {
  const snapshot: Snapshot = { ...EMPTY_SNAPSHOT, merged: [mergedPr()] };
  expect(derivePopover(snapshot, "", ALL_EXPANDED).content.kind).toBe("list");
});

test("a filter that hides every row shows no matches, quoting the query", () => {
  expect(derivePopover(FILTER_FIXTURE, "zzz", ALL_EXPANDED).content).toEqual({
    kind: "no-matches",
    query: "zzz",
  });
});

const inAll = (repo: string, number: number, updated_at: string) =>
  pr({ section: "all", repo, number, updated_at });

function groups(prs: PullRequest[]) {
  const { body } = section(list(synced({ prs })), "all");
  if (body.kind !== "groups") throw new Error(`All body is ${body.kind}`);
  return body.groups.map((group) => ({
    repo: group.repo,
    numbers: group.rows.map((row) => row.pr.number),
  }));
}

test("All collapses into one group per repo, rows in incoming order", () => {
  expect(
    groups([
      inAll("acme/widgets", 1, "2026-07-10T00:00:00Z"),
      inAll("acme/gadgets", 2, "2026-07-11T00:00:00Z"),
      inAll("acme/widgets", 3, "2026-07-15T00:00:00Z"),
      inAll("acme/widgets", 4, "2026-07-12T00:00:00Z"),
    ]),
  ).toEqual([
    { repo: "acme/widgets", numbers: [1, 3, 4] },
    { repo: "acme/gadgets", numbers: [2] },
  ]);
});

test("the repo whose newest PR moved most recently leads", () => {
  // acme/widgets holds the single freshest PR; acme/gadgets has more PRs and
  // wins on average recency, so this pins newest-wins over count-or-average.
  expect(
    groups([
      inAll("acme/gadgets", 1, "2026-07-14T00:00:00Z"),
      inAll("acme/gadgets", 2, "2026-07-13T00:00:00Z"),
      inAll("acme/widgets", 3, "2026-07-01T00:00:00Z"),
      inAll("acme/widgets", 4, "2026-07-15T00:00:00Z"),
    ]).map((group) => group.repo),
  ).toEqual(["acme/widgets", "acme/gadgets"]);
});

test("an unparseable timestamp sinks its group instead of throwing", () => {
  expect(
    groups([
      inAll("acme/broken", 1, "not-a-date"),
      inAll("acme/widgets", 2, "2026-07-01T00:00:00Z"),
    ]).map((group) => group.repo),
  ).toEqual(["acme/widgets", "acme/broken"]);
});

test("rows in a repo group carry their marks like any other row", () => {
  const { body } = section(
    list(synced({ prs: [pr({ section: "all", approved: true })] })),
    "all",
  );
  if (body.kind !== "groups") throw new Error(`All body is ${body.kind}`);
  expect(body.groups[0].rows[0].marks).toEqual([
    { kind: "approved", tip: "Approved" },
  ]);
});

test("a row with new bot activity carries a note counting it", () => {
  expect(rowFor(pr({ bot_unread_count: 8 })).botNote).toBe("8 new from bots");
  expect(rowFor(pr({ bot_unread_count: 1 })).botNote).toBe("1 new from bots");
});

test("a row with no new bot activity carries no note", () => {
  expect(rowFor(pr({ unread_count: 3 })).botNote).toBeNull();
});

test("a bot-authored PR is tagged, whatever section it sits in", () => {
  expect(rowFor(pr({ author_is_bot: true })).botAuthor).toBe(true);
  expect(rowFor(pr()).botAuthor).toBe(false);
  const { body } = section(
    list(synced({ prs: [pr({ section: "all", author_is_bot: true })] })),
    "all",
  );
  if (body.kind !== "groups") throw new Error(`All body is ${body.kind}`);
  expect(body.groups[0].rows[0].botAuthor).toBe(true);
});

test("new bot activity never reaches a collapsed section's badge", () => {
  const view = list(
    synced({ prs: [pr({ unread_count: 3, bot_unread_count: 8 })] }),
    "",
    ALL_COLLAPSED,
  );
  expect(section(view, "mine").collapsedUnread).toBe(3);
});

// Mark all read is the one way to clear every note at once, so notes alone
// are enough to offer it.
test("Mark all read is offered when the only thing new is bot activity", () => {
  const snapshot = synced({ prs: [pr({ bot_unread_count: 8 })] });
  expect(derivePopover(snapshot, "", ALL_EXPANDED).offerMarkAllRead).toBe(true);
});
