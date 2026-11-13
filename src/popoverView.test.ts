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
  yourTurnCount,
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
  failing_checks: [],
  failing_checks_truncated: false,
  checks_running: false,
  auto_merge: false,
  ready_to_merge: false,
  turn: null,
  turn_since: null,
  can_fix_conflict: false,
  fix_requested: false,
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
  deploy: null,
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

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
const NOW = Date.parse("2026-07-12T12:00:00Z");

// Narrows to the list; a placeholder where a list was expected fails here.
function list(
  snapshot: Snapshot,
  query = "",
  collapsed = ALL_EXPANDED,
  now = NOW,
): ListView {
  const { content } = derivePopover(snapshot, query, collapsed, now);
  if (content.kind !== "list") throw new Error(`expected a list, got ${content.kind}`);
  return content;
}

function section(view: ListView, key: SectionView["key"]): SectionView {
  const found = view.sections.find((s) => s.key === key);
  if (!found) throw new Error(`no ${key} section`);
  return found;
}

// The PR rows of a flat section; fails on a grouped, merged or empty body.
function rows(view: ListView, key: SectionView["key"]): RowView[] {
  const { body } = section(view, key);
  if (body.kind !== "rows") throw new Error(`${key} body is ${body.kind}`);
  return body.rows;
}

// The single row of a snapshot holding just this PR, from Todo when it
// waits on the viewer and from Mine otherwise.
const rowFor = (only: PullRequest): RowView =>
  rows(
    list(synced({ prs: [only] })),
    only.turn === "yours" ? "yours" : "mine",
  )[0];

const numbers = (view: ListView, key: SectionView["key"]) =>
  rows(view, key).map((row) => row.pr.number);

const state = (overrides: Partial<PullRequest>) => rowFor(pr(overrides)).state;

test("a row with one state shows it in words, with nothing folded", () => {
  expect(state({ approved: true })).toEqual({
    reason: "Approved",
    tone: "ok",
    more: null,
    all: ["Approved"],
  });
});

test("a row with no state shows no reason", () => {
  const row = rowFor(pr());
  expect(row.state).toBeNull();
  expect(row.draft).toBe(false);
});

test("a row that is approved, running checks and blocked at once leads with the blocker", () => {
  expect(
    state({ approved: true, checks_running: true, blocked_reasons: ["threads"] }),
  ).toEqual({
    reason: "Unresolved threads",
    tone: "warn",
    more: "+2",
    all: ["Unresolved threads", "Checks running", "Approved"],
  });
});

test("states run blockers, queue, Copilot, checks, auto-merge, approved, awaiting review", () => {
  expect(
    state({
      blocked_reasons: ["conflict", "behind"],
      merge_queue_position: 2,
      fix_requested: true,
      checks_running: true,
      auto_merge: true,
      approved: true,
      awaiting_review: true,
    }),
  ).toEqual({
    reason: "Merge conflict",
    tone: "warn",
    more: "+7",
    all: [
      "Merge conflict",
      "Behind base",
      "In merge queue, 2nd",
      "Copilot fixing",
      "Checks running",
      "Auto-merge armed",
      "Approved",
      "Awaiting review",
    ],
  });
});

test("a review requested from your team comes last", () => {
  expect(state({ approved: true, team_review_requested: true })?.all).toEqual([
    "Approved",
    "Team review requested",
  ]);
});

test("a request naming you is listed alone, never beside the team request", () => {
  expect(
    state({ review_requested: true, team_review_requested: true })?.all,
  ).toEqual(["Review requested"]);
});

test("each state takes its tone: amber blocked, green ready and approved, neutral otherwise", () => {
  const tone = (overrides: Partial<PullRequest>) => state(overrides)?.tone;
  expect(tone({ blocked_reasons: ["ci"] })).toBe("warn");
  expect(tone({ ready_to_merge: true })).toBe("ok");
  expect(tone({ approved: true })).toBe("ok");
  expect(tone({ merge_queue_position: 1 })).toBe("neutral");
  expect(tone({ fix_requested: true })).toBe("neutral");
  expect(tone({ checks_running: true })).toBe("neutral");
  expect(tone({ auto_merge: true })).toBe("neutral");
  expect(tone({ awaiting_review: true })).toBe("neutral");
  expect(tone({ team_review_requested: true })).toBe("neutral");
});

test("a draft shows the Draft pill and nothing the backend did not send", () => {
  const row = rowFor(pr({ is_draft: true }));
  expect(row.draft).toBe(true);
  expect(row.state).toBeNull();
});

test("a draft still shows a state the backend sends for it", () => {
  const row = rowFor(pr({ is_draft: true, merge_queue_position: 2 }));
  expect(row.draft).toBe(true);
  expect(row.state?.reason).toBe("In merge queue, 2nd");
});

// The keys are the wire contract with the Rust BlockedReason enum; the labels
// name a PR property, never "you must act".
test("each blocked reason is worded as a property of the PR", () => {
  const reason = (blocked: PullRequest["blocked_reasons"][number]) =>
    state({ blocked_reasons: [blocked] })?.reason;
  expect(reason("conflict")).toBe("Merge conflict");
  expect(reason("ci")).toBe("CI failing");
  expect(reason("review")).toBe("Changes requested");
  expect(reason("threads")).toBe("Unresolved threads");
  expect(reason("behind")).toBe("Behind base");
});

test("several blockers keep the backend's severity order", () => {
  expect(state({ blocked_reasons: ["ci", "review", "behind"] })).toEqual({
    reason: "CI failing",
    tone: "warn",
    more: "+2",
    all: ["CI failing", "Changes requested", "Behind base"],
  });
});

test("the CI reason names the failing check", () => {
  expect(
    state({ blocked_reasons: ["ci"], failing_checks: ["e2e-safari"] })?.reason,
  ).toBe("CI failing: e2e-safari");
});

test("with several checks failing the CI reason names the first and says there are more", () => {
  expect(
    state({ blocked_reasons: ["ci"], failing_checks: ["e2e-safari", "lint"] }),
  ).toEqual({
    reason: "CI failing: e2e-safari and more",
    tone: "warn",
    // Further failing checks are not further states.
    more: null,
    all: ["CI failing: e2e-safari and more"],
  });
});

// A truncated page cannot say how many failed, or that the one it saw is the
// only one.
test("a truncated list of failing checks says there are more even with one name", () => {
  expect(
    state({
      blocked_reasons: ["ci"],
      failing_checks: ["e2e-safari"],
      failing_checks_truncated: true,
    })?.reason,
  ).toBe("CI failing: e2e-safari and more");
});

test("a folded CI blocker names the failing check too", () => {
  expect(
    state({ blocked_reasons: ["conflict", "ci"], failing_checks: ["e2e-safari"] })
      ?.all,
  ).toEqual(["Merge conflict", "CI failing: e2e-safari"]);
});

// Ready is the backend's call and does not imply an approval: a repo with no
// required reviews is ready without one.
test("a ready PR leads with Ready to merge, ahead of its approval", () => {
  expect(state({ approved: true, ready_to_merge: true })?.all).toEqual([
    "Ready to merge",
    "Approved",
  ]);
  expect(state({ ready_to_merge: true })?.all).toEqual(["Ready to merge"]);
});

// The teens are the trap: 11, 12 and 13 take "th" although they end in 1, 2
// and 3.
test("the merge queue reason states the position as an English ordinal", () => {
  const reason = (position: number) =>
    state({ merge_queue_position: position })?.reason;
  expect(reason(1)).toBe("In merge queue, 1st");
  expect(reason(2)).toBe("In merge queue, 2nd");
  expect(reason(3)).toBe("In merge queue, 3rd");
  expect(reason(4)).toBe("In merge queue, 4th");
  expect(reason(11)).toBe("In merge queue, 11th");
  expect(reason(12)).toBe("In merge queue, 12th");
  expect(reason(13)).toBe("In merge queue, 13th");
  expect(reason(21)).toBe("In merge queue, 21st");
  expect(reason(22)).toBe("In merge queue, 22nd");
});

test("with no filter Todo, Mine, Participated and All are present even when empty", () => {
  const view = list(synced({ prs: [pr({ section: "participated" })] }));
  expect(view.sections.map((s) => [s.key, s.label, s.count])).toEqual([
    ["yours", "Todo", 0],
    ["mine", "Mine", 0],
    ["participated", "Participated", 1],
    ["all", "All", 0],
  ]);
  expect(section(view, "yours").body).toEqual({ kind: "empty" });
  expect(section(view, "mine").body).toEqual({ kind: "empty" });
  expect(section(view, "all").body).toEqual({ kind: "empty" });
});

test("Merged follows the PR sections while it holds rows", () => {
  const merged = [mergedPr({ number: 90 }), mergedPr({ number: 91 })];
  const view = list(synced({ merged }));
  expect(view.sections.map((s) => s.key)).toEqual([
    "yours",
    "mine",
    "participated",
    "all",
    "merged",
  ]);
  const mergedSection = section(view, "merged");
  expect(mergedSection.label).toBe("Merged");
  expect(mergedSection.count).toBe(2);
  expect(mergedSection.body).toEqual({
    kind: "merged",
    rows: merged.map((pr) => ({ pr, deploy: null, actions: expect.any(Array) })),
  });
});

const RUN_URL = "https://github.com/acme/widgets/actions/runs/1";

// The deploy status of the single Merged row of a snapshot holding just
// this PR.
const deployFor = (only: MergedPr) => {
  const { body } = section(list(synced({ merged: [only] })), "merged");
  if (body.kind !== "merged") throw new Error(`merged body is ${body.kind}`);
  return body.rows[0].deploy;
};

test("a merged row with a deploy under way says Deploying", () => {
  const deploy = deployFor(
    mergedPr({
      deploy: { state: "deploying", completed_at: null, url: RUN_URL },
    }),
  );
  expect(deploy).toEqual({
    label: "Deploying",
    brief: "Deploying",
    showsTime: false,
    failed: false,
    completedAt: null,
    url: RUN_URL,
  });
});

const DEPLOY_DONE = "2026-07-11T09:42:00Z";

test("a deployed row says Deployed and when the deploy landed", () => {
  const deploy = deployFor(
    mergedPr({
      deploy: { state: "deployed", completed_at: DEPLOY_DONE, url: RUN_URL },
    }),
  );
  expect(deploy).toEqual({
    label: "Deployed",
    brief: "Deployed",
    showsTime: true,
    failed: false,
    completedAt: DEPLOY_DONE,
    url: RUN_URL,
  });
});

test("a failed deploy is the one status in the failure color", () => {
  const deploy = deployFor(
    mergedPr({
      deploy: { state: "failed", completed_at: DEPLOY_DONE, url: RUN_URL },
    }),
  );
  expect(deploy).toEqual({
    label: "Deploy failed",
    brief: "Failed",
    showsTime: false,
    failed: true,
    completedAt: DEPLOY_DONE,
    url: RUN_URL,
  });
});

test("a cancelled deploy is named without the failure color", () => {
  const deploy = deployFor(
    mergedPr({
      deploy: { state: "cancelled", completed_at: DEPLOY_DONE, url: RUN_URL },
    }),
  );
  expect(deploy).toEqual({
    label: "Deploy cancelled",
    brief: "Cancelled",
    showsTime: false,
    failed: false,
    completedAt: DEPLOY_DONE,
    url: RUN_URL,
  });
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
    rows: [{ pr: FILTER_FIXTURE.merged[0], deploy: null, actions: expect.any(Array) }],
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
  expect(derivePopover(FILTER_FIXTURE, "gadgets flaky", ALL_EXPANDED, NOW).content)
    .toEqual({ kind: "no-matches", query: "gadgets flaky" });
});

// A whitespace-only query matches everything, so it must not force-expand or
// drop the empty sections either. Todo is open whatever is stored: it
// has no collapse state.
test("a whitespace-only query is no filter at all", () => {
  for (const query of ["", "   ", "\t \n"]) {
    const view = list(FILTER_FIXTURE, query, ALL_COLLAPSED);
    expect(view.sections.map((s) => [s.key, s.count, s.expanded])).toEqual([
      ["yours", 0, true],
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
  expect(derivePopover(UNREAD_FIXTURE, "", ALL_EXPANDED, NOW).offerMarkAllRead).toBe(true);
  expect(derivePopover(UNREAD_FIXTURE, "zzz", ALL_EXPANDED, NOW).offerMarkAllRead).toBe(true);
  expect(
    derivePopover(synced({ prs: [pr()] }), "", ALL_EXPANDED, NOW).offerMarkAllRead,
  ).toBe(false);
  expect(derivePopover(EMPTY_SNAPSHOT, "", ALL_EXPANDED, NOW).offerMarkAllRead).toBe(false);
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
  expect(derivePopover(EMPTY_SNAPSHOT, "", ALL_EXPANDED, NOW).content).toEqual({
    kind: "setup",
  });
  // Setup outranks the filter: there is nothing to have matched.
  expect(derivePopover(EMPTY_SNAPSHOT, "fix", ALL_EXPANDED, NOW).content).toEqual({
    kind: "setup",
  });
});

test("a synced but empty snapshot shows the empty section scaffold", () => {
  const view = list(synced());
  expect(view.sections.map((s) => [s.key, s.body.kind])).toEqual([
    ["yours", "empty"],
    ["mine", "empty"],
    ["participated", "empty"],
    ["all", "empty"],
  ]);
});

test("rows restored before the first sync show as a list, not the setup prompt", () => {
  const snapshot: Snapshot = { ...EMPTY_SNAPSHOT, merged: [mergedPr()] };
  expect(derivePopover(snapshot, "", ALL_EXPANDED, NOW).content.kind).toBe("list");
});

test("a filter that hides every row shows no matches, quoting the query", () => {
  expect(derivePopover(FILTER_FIXTURE, "zzz", ALL_EXPANDED, NOW).content).toEqual({
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

test("rows in a repo group carry their state like any other row", () => {
  const { body } = section(
    list(synced({ prs: [pr({ section: "all", approved: true })] })),
    "all",
  );
  if (body.kind !== "groups") throw new Error(`All body is ${body.kind}`);
  expect(body.groups[0].rows[0].state?.reason).toBe("Approved");
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
  expect(derivePopover(snapshot, "", ALL_EXPANDED, NOW).offerMarkAllRead).toBe(true);
});

test("team-requested rows lead Mine and Participated, then the rest in the order sent", () => {
  const participated = { section: "participated" } as const;
  const view = list(
    synced({
      prs: [
        pr({ number: 1 }),
        pr({ number: 2, team_review_requested: true }),
        pr({ number: 3, turn: "theirs", awaiting_review: true }),
        pr({ number: 4, team_review_requested: true }),
        pr({ ...participated, number: 5, turn: "theirs" }),
        pr({ ...participated, number: 6, team_review_requested: true }),
      ],
    }),
  );
  expect(numbers(view, "mine")).toEqual([2, 4, 1, 3]);
  expect(numbers(view, "participated")).toEqual([6, 5]);
});

// The reason on a your-turn row is the one the backend decided the turn on:
// a review asked of you, else the primary blocker, else ready to merge.
test("a your-turn row asked for review leads with the request, in the accent", () => {
  expect(
    state({
      turn: "yours",
      review_requested: true,
      approved: true,
      blocked_reasons: ["ci"],
    }),
  ).toEqual({
    reason: "Review requested",
    tone: "accent",
    more: "+2",
    all: ["Review requested", "CI failing", "Approved"],
  });
});

test("a blocked your-turn row leads with its primary blocker", () => {
  expect(
    state({
      turn: "yours",
      approved: true,
      merge_queue_position: 1,
      blocked_reasons: ["conflict", "threads"],
    }),
  ).toEqual({
    reason: "Merge conflict",
    tone: "warn",
    more: "+3",
    all: ["Merge conflict", "Unresolved threads", "In merge queue, 1st", "Approved"],
  });
});

test("a ready your-turn row leads with Ready to merge", () => {
  expect(
    state({ turn: "yours", approved: true, auto_merge: true, ready_to_merge: true }),
  ).toEqual({
    reason: "Ready to merge",
    tone: "ok",
    more: "+2",
    all: ["Ready to merge", "Auto-merge armed", "Approved"],
  });
});

// The accent says the row waits on the viewer; a request seen on a row that
// does not wait on them is plain news.
test("a review request on a row that is not your turn stays neutral", () => {
  expect(state({ turn: null, review_requested: true })?.tone).toBe("neutral");
});

test("a your-turn row says how long it has waited: hours under a day, days from one day up", () => {
  const waiting = (since: number) =>
    rowFor(pr({ turn: "yours", turn_since: since })).waiting;
  expect(waiting(NOW - 3 * HOUR)).toBe("waiting 3h");
  expect(waiting(NOW - (DAY - 1))).toBe("waiting 23h");
  expect(waiting(NOW - DAY)).toBe("waiting 1d");
  expect(waiting(NOW - (2 * DAY + 5 * HOUR))).toBe("waiting 2d");
});

test("a turn that began within the hour reads as under an hour, not zero", () => {
  const row = rowFor(pr({ turn: "yours", turn_since: NOW - 60_000 }));
  expect(row.waiting).toBe("waiting <1h");
});

test("a your-turn row with no known start still carries the cue", () => {
  expect(rowFor(pr({ turn: "yours", turn_since: null })).waiting).toBe(
    "waiting",
  );
});

test("a row that is not the viewer's turn has no waiting label", () => {
  expect(rowFor(pr({ turn: "theirs" })).waiting).toBeNull();
  expect(rowFor(pr({ turn: null })).waiting).toBeNull();
  // The turn decides, not the start.
  expect(
    rowFor(pr({ turn: "theirs", turn_since: NOW - 3 * HOUR })).waiting,
  ).toBeNull();
});

const SNOOZE_FIXTURE = synced({
  prs: [
    pr({ number: 1, title: "Quiet one" }),
    pr({ number: 2, title: "Parked review", turn: "yours", unread_count: 2 }),
    pr({ number: 3, title: "Waiting", turn: "yours" }),
    pr({ number: 4, title: "Parked elsewhere", section: "participated" }),
    pr({ number: 5, section: "all" }),
  ],
  snoozed: ["acme/widgets#2", "acme/widgets#4"],
});

// The folded group at the foot of a flat section; fails when there is none.
function snoozedGroup(view: ListView, key: SectionKey) {
  const { body } = section(view, key);
  if (body.kind !== "rows" || !body.snoozed) throw new Error(`no snoozed group in ${key}`);
  return body.snoozed;
}

test("snoozed rows leave the ordered rows for a folded group at the section's foot", () => {
  const view = list(SNOOZE_FIXTURE);
  expect(numbers(view, "mine")).toEqual([1]);
  // The header still counts the snoozed row: it has not left the section.
  expect(section(view, "mine").count).toBe(2);
  const group = snoozedGroup(view, "mine");
  expect(group.rows.map((row) => row.pr.number)).toEqual([2]);
  expect(group.label).toBe("1 snoozed");
  expect(group.forcedOpen).toBe(false);
});

test("a section with nothing snoozed has no folded group", () => {
  const { body } = section(list(synced({ prs: [pr()] })), "mine");
  expect(body).toMatchObject({ kind: "rows", snoozed: null });
});

test("a section whose every row is snoozed keeps its header count and shows only the folded group", () => {
  const view = list(SNOOZE_FIXTURE);
  expect(section(view, "participated").count).toBe(1);
  expect(numbers(view, "participated")).toEqual([]);
  expect(snoozedGroup(view, "participated").label).toBe("1 snoozed");
});

test("a filter shows matching snoozed rows by forcing their group open", () => {
  const view = list(SNOOZE_FIXTURE, "parked");
  expect(numbers(view, "mine")).toEqual([]);
  const group = snoozedGroup(view, "mine");
  expect(group.rows.map((row) => row.pr.number)).toEqual([2]);
  expect(group.forcedOpen).toBe(true);
});

test("a filter that matches no snoozed row leaves no folded group", () => {
  const { body } = section(list(SNOOZE_FIXTURE, "quiet"), "mine");
  expect(body).toMatchObject({ kind: "rows", snoozed: null });
});

const labels = (row: { actions: { label: string }[] }) =>
  row.actions.map((action) => action.label);

// Opening the PR is the click on the row, never a listed action.
test("a row offers Snooze; a snoozed row Wake; a row in All nothing", () => {
  const view = list(SNOOZE_FIXTURE);
  expect(rows(view, "mine").map(labels)).toEqual([["Snooze"]]);
  expect(labels(snoozedGroup(view, "mine").rows[0])).toEqual(["Wake"]);
  const { body } = section(view, "all");
  if (body.kind !== "groups") throw new Error("All is not grouped");
  expect(labels(body.groups[0].rows[0])).toEqual([]);
});

test("a merged row offers Dismiss", () => {
  const view = list(synced({ merged: [mergedPr()] }));
  const { body } = section(view, "merged");
  if (body.kind !== "merged") throw new Error("Merged is not merged rows");
  expect(body.rows.map(labels)).toEqual([["Dismiss"]]);
});

const fixable = (overrides: Partial<PullRequest> = {}) =>
  pr({ blocked_reasons: ["conflict"], can_fix_conflict: true, ...overrides });

test("a row the backend calls fixable offers the Copilot fix, and no other row does", () => {
  const view = list(
    synced({ prs: [fixable({ number: 1 }), pr({ number: 2, blocked_reasons: ["conflict"] })] }),
  );
  expect(rows(view, "mine").map((row) => row.fix)).toEqual(["offer", null]);
  expect(rows(view, "mine").map(labels)).toEqual([
    ["Snooze", "Fix with Copilot"],
    ["Snooze"],
  ]);
});

test("a snoozed row offers no Copilot fix", () => {
  const view = list(
    synced({ prs: [fixable({ number: 1 })], snoozed: ["acme/widgets#1"] }),
  );
  expect(snoozedGroup(view, "mine").rows[0].fix).toBeNull();
});

test("a row whose fix is being posted shows that in place of the offer", () => {
  const snapshot = synced({ prs: [fixable({ number: 1 }), fixable({ number: 2 })] });
  const posting = new Set(["acme/widgets#1"]);
  const { content } = derivePopover(snapshot, "", ALL_EXPANDED, NOW, posting);
  expect(rows(content as ListView, "mine").map((row) => row.fix)).toEqual([
    "posting",
    "offer",
  ]);
  // A request on its way is not something to ask for again.
  expect(rows(content as ListView, "mine").map(labels)).toEqual([
    ["Snooze"],
    ["Snooze", "Fix with Copilot"],
  ]);
});

// As the backend sends a PR it has handed to Copilot: still conflicting,
// still fixable by GitHub's state, and no longer the viewer's turn.
const fixing = (overrides: Partial<PullRequest> = {}) =>
  fixable({ fix_requested: true, turn: "theirs", ...overrides });

test("a row with a requested fix says Copilot is fixing after its conflict, and has no fix control", () => {
  const [row] = rows(list(synced({ prs: [fixing()] })), "mine");
  expect(row.state?.all).toEqual(["Merge conflict", "Copilot fixing"]);
  expect(row.fix).toBeNull();
});

test("a row with a requested fix leaves Todo and the your-turn count", () => {
  const snapshot = synced({
    prs: [
      fixing({ number: 1 }),
      fixable({ number: 2, turn: "yours" }),
      pr({ number: 3 }),
    ],
  });
  expect(numbers(list(snapshot), "yours")).toEqual([2]);
  expect(numbers(list(snapshot), "mine")).toEqual([1, 3]);
  expect(yourTurnCount(snapshot)).toBe(1);
});

test("a requested fix beside a review asked of you leaves the row yours, leading with the request", () => {
  const [row] = rows(
    list(synced({ prs: [fixing({ review_requested: true, turn: "yours" })] })),
    "yours",
  );
  expect(row.state?.all).toEqual([
    "Review requested",
    "Merge conflict",
    "Copilot fixing",
  ]);
});

test("a snoozed row's unread stays off its collapsed section's badge", () => {
  const view = list(SNOOZE_FIXTURE, "", ALL_COLLAPSED);
  expect(section(view, "mine").collapsedUnread).toBe(0);
});

test("the your-turn count leaves snoozed rows out", () => {
  expect(yourTurnCount(SNOOZE_FIXTURE)).toBe(1);
});

const YOUR_TURN_FIXTURE = synced({
  prs: [
    pr({ number: 1, title: "Flaky spec", turn: "yours" }),
    pr({ number: 2, title: "Quiet one" }),
    pr({ number: 3, title: "Review me", section: "participated", turn: "yours" }),
    pr({ number: 4, title: "Org-wide ask", section: "all", turn: "yours" }),
    pr({ number: 5, title: "Passing by", section: "all" }),
  ],
});

test("Todo leads the list and holds every row waiting on the viewer, in the order sent", () => {
  const view = list(YOUR_TURN_FIXTURE);
  expect(view.sections.map((s) => [s.key, s.label, s.count])).toEqual([
    ["yours", "Todo", 3],
    ["mine", "Mine", 1],
    ["participated", "Participated", 0],
    ["all", "All", 1],
  ]);
  expect(numbers(view, "yours")).toEqual([1, 3, 4]);
});

test("a row in Todo appears in no other section, and the other counts leave it out", () => {
  const view = list(YOUR_TURN_FIXTURE);
  expect(numbers(view, "mine")).toEqual([2]);
  expect(section(view, "participated").body).toEqual({ kind: "empty" });
  const { body } = section(view, "all");
  if (body.kind !== "groups") throw new Error("All is not grouped");
  expect(body.groups.flatMap((g) => g.rows.map((row) => row.pr.number))).toEqual([5]);
});

test("with no filter the Todo count is the tray number", () => {
  for (const [snapshot, tray] of [
    [YOUR_TURN_FIXTURE, 3],
    [SNOOZE_FIXTURE, 1],
    [synced(), 0],
  ] as const) {
    expect(section(list(snapshot), "yours").count).toBe(tray);
    expect(yourTurnCount(snapshot)).toBe(tray);
  }
});

test("a snoozed your-turn row stays in its home section's fold until it is woken", () => {
  const snoozed = list(SNOOZE_FIXTURE);
  expect(numbers(snoozed, "yours")).toEqual([3]);
  expect(snoozedGroup(snoozed, "mine").rows.map((row) => row.pr.number)).toEqual([2]);

  const woken = { ...SNOOZE_FIXTURE, snoozed: ["acme/widgets#4"] };
  expect(numbers(list(woken), "yours")).toEqual([2, 3]);
  expect(section(list(woken), "mine").body).toMatchObject({ snoozed: null });
  expect(yourTurnCount(woken)).toBe(2);
});

test("a row in Todo keeps the snooze action of its home section", () => {
  expect(rows(list(YOUR_TURN_FIXTURE), "yours").map(labels)).toEqual([
    ["Snooze"],
    ["Snooze"],
    [],
  ]);
});

test("a filter narrows Todo to its matches and counts only those", () => {
  const view = list(YOUR_TURN_FIXTURE, "review");
  expect(view.sections.map((s) => [s.key, s.count])).toEqual([["yours", 1]]);
  expect(numbers(view, "yours")).toEqual([3]);
});

test("a filter that matches nothing in Todo hides it", () => {
  const view = list(YOUR_TURN_FIXTURE, "quiet");
  expect(view.sections.map((s) => [s.key, s.count])).toEqual([["mine", 1]]);
});

// The row shows its own badge in Todo, so its home section's collapsed
// header must not count the same unread again.
test("a collapsed section's unread badge leaves out rows moved to Todo", () => {
  const snapshot = synced({
    prs: [
      pr({ number: 1, unread_count: 2 }),
      pr({ number: 2, unread_count: 5, turn: "yours" }),
    ],
  });
  const view = list(snapshot, "", ALL_COLLAPSED);
  expect(section(view, "mine").collapsedUnread).toBe(2);
  expect(section(view, "yours").collapsedUnread).toBe(0);
});
