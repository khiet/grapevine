import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { EMPTY_SNAPSHOT, type MergedPr, type PullRequest } from "./backend";
import { derivePopover, type ListView } from "./popoverView";
import PrList, {
  DEFAULT_COLLAPSED,
  deployTip,
  formatLastSync,
  formatUpdated,
  parseCollapsed,
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

test("nothing stored yields the defaults: Mine and All collapsed", () => {
  expect(parseCollapsed(null)).toEqual({
    mine: true,
    participated: false,
    all: true,
    merged: false,
  });
});

test("stored toggles override their defaults, others keep theirs", () => {
  expect(parseCollapsed('{"mine":false,"merged":true}')).toEqual({
    mine: false,
    participated: false,
    all: true,
    merged: true,
  });
});

test("corrupt or wrong-shaped blobs fall back to the defaults", () => {
  expect(parseCollapsed("not json")).toEqual(DEFAULT_COLLAPSED);
  expect(parseCollapsed('"a string"')).toEqual(DEFAULT_COLLAPSED);
  expect(parseCollapsed("null")).toEqual(DEFAULT_COLLAPSED);
  // Unknown keys and non-boolean values are dropped, valid ones kept.
  expect(parseCollapsed('{"mine":"yes","bogus":true,"all":false}')).toEqual({
    ...DEFAULT_COLLAPSED,
    all: false,
  });
});

test("parseCollapsed never returns the shared defaults object", () => {
  const state = parseCollapsed(null);
  state.mine = false;
  expect(DEFAULT_COLLAPSED.mine).toBe(true);
});

const RUN = "https://github.com/acme/widgets/actions/runs/1";

test("the deploy tooltip gives the exact local merge and deploy times", () => {
  const tip = deployTip(at(2026, 7, 16, 14, 59).toISOString(), {
    label: "Deployed",
    showsTime: true,
    failed: false,
    completedAt: at(2026, 7, 16, 15, 4).toISOString(),
    url: RUN,
  });
  expect(tip).toBe("Merged 16 Jul 2026, 14:59\nDeployed 16 Jul 2026, 15:04");
});

test("the tooltip of a deploy under way has only the merge time to give", () => {
  const tip = deployTip(at(2026, 1, 2, 9, 5).toISOString(), {
    label: "Deploying",
    showsTime: false,
    failed: false,
    completedAt: null,
    url: RUN,
  });
  expect(tip).toBe("Merged 2 Jan 2026, 09:05\nDeploying");
});

// The rendered markup is the seam for what assistive tech is handed: the
// author's handle has to be in each row's accessible name, which a button
// computes from its content, and in the row's title.
const openPr = {
  number: 7,
  title: "Fix the thing",
  url: "https://github.com/acme/widgets/pull/7",
  repo: "acme/widgets",
  author: "octocat",
  author_is_bot: false,
  avatar_url: "",
  owner_avatar_url: "",
  created_at: "2026-07-10T12:00:00Z",
  updated_at: "2026-07-11T09:30:00Z",
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
  turn: null,
  turn_since: null,
  can_fix_conflict: false,
  fix_requested: false,
  unread_count: 0,
  bot_unread_count: 0,
} satisfies PullRequest;

const mergedPr = {
  number: 90,
  title: "Ship the release notes",
  url: "https://github.com/acme/widgets/pull/90",
  repo: "acme/widgets",
  author: "hubot",
  avatar_url: "",
  owner_avatar_url: "",
  merged_at: "2026-07-11T09:30:00Z",
  deploy: null,
} satisfies MergedPr;

function renderRows(): string[] {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs: [openPr], merged: [mergedPr] },
    "",
    { ...DEFAULT_COLLAPSED },
    NOW.getTime(),
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  return html.match(/<li[\s\S]*?<\/li>/g) ?? [];
}

test("each row carries its author's handle in its title and accessible name", () => {
  const [open, merged] = renderRows().filter((li) => li.includes("pr-row"));
  for (const [li, handle] of [
    [open, "@octocat"],
    [merged, "@hubot"],
  ]) {
    expect(li).toMatch(new RegExp(`^<li[^>]* title="${handle}"`));
    // Inside the row button, so the button's name includes it.
    const button = li.match(/<button[^>]*class="pr-row[\s\S]*?<\/button>/)?.[0];
    expect(button).toContain(`aria-label="${handle}"`);
  }
});

test("Todo has a heading and no collapse toggle; the other sections keep theirs", () => {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs: [{ ...openPr, turn: "yours" }] },
    "",
    { ...DEFAULT_COLLAPSED },
    NOW.getTime(),
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  const [yours, mine] = html.match(/<section[\s\S]*?<\/section>/g) ?? [];
  expect(yours).toMatch(/<h2[^>]*>.*Todo/);
  expect(yours).not.toContain("aria-expanded");
  expect(mine).toContain("aria-expanded");
});

test("an empty section shows a dash named for assistive tech, in Todo as elsewhere", () => {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true },
    "",
    { mine: false, participated: false, all: false, merged: false },
    NOW.getTime(),
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  const sections = html.match(/<section[\s\S]*?<\/section>/g) ?? [];
  expect(sections).toHaveLength(4);
  for (const section of sections) {
    expect(section).toMatch(
      /<li class="pr-empty"><span role="img" aria-label="No pull requests"><svg/,
    );
  }
});

function renderOpenRow(overrides: Partial<PullRequest>): string {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs: [{ ...openPr, ...overrides }] },
    "",
    { ...DEFAULT_COLLAPSED },
    NOW.getTime(),
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  const row = html.match(/<button[^>]*class="pr-row[\s\S]*?<\/button>/)?.[0];
  if (!row) throw new Error("no row rendered");
  return row;
}

test("a row with one state shows it as plain words in its tone", () => {
  const row = renderOpenRow({ approved: true });
  expect(row).toContain(
    '<span class="pr-state is-ok"><span class="pr-state-reason">Approved</span></span>',
  );
});

// The row button computes its name from its content, so the label on the
// state is what puts every state in the row's accessible name.
test("a row with several states shows one reason and +N, and names them all in its tooltip and label", () => {
  const row = renderOpenRow({
    approved: true,
    checks_running: true,
    blocked_reasons: ["threads"],
  });
  expect(row).toContain(
    '<span class="pr-state is-warn pr-tip" role="img"' +
      ' data-tip="Unresolved threads\nChecks running\nApproved"' +
      ' aria-label="Unresolved threads, Checks running, Approved" title="">' +
      '<span class="pr-state-reason">Unresolved threads</span>' +
      '<span class="pr-state-more">+2</span></span>',
  );
});

// The bar is the row's visible route to its actions, so its buttons are read
// off the rendered row: each is a real button with its label as text.
function barOf(li: string): string[] {
  if (!li.includes('<span class="pr-actions"')) throw new Error("no action bar rendered");
  return [...li.matchAll(/<button[^>]*class="pr-action[^>]*>([^<]+)<\/button>/g)].map(
    (m) => m[1],
  );
}

function renderList(
  prs: PullRequest[],
  merged: MergedPr[] = [],
  fixesPosting: ReadonlySet<string> = new Set(),
): string[] {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs, merged },
    "",
    { mine: false, participated: false, all: false, merged: false },
    NOW.getTime(),
    fixesPosting,
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  return (html.match(/<li[\s\S]*?<\/li>/g) ?? []).filter((li) => li.includes("pr-row"));
}

test("each kind of row words its actions in a bar", () => {
  const [plain, fixable, inAll, merged] = renderList(
    [
      openPr,
      { ...openPr, number: 8, can_fix_conflict: true, blocked_reasons: ["conflict"] },
      { ...openPr, number: 9, section: "all" },
    ],
    [mergedPr],
  );
  expect(barOf(plain)).toEqual(["Snooze", "Open"]);
  expect(barOf(fixable)).toEqual(["Snooze", "Fix with Copilot", "Open"]);
  expect(barOf(inAll)).toEqual(["Open"]);
  expect(barOf(merged)).toEqual(["Dismiss", "Open"]);
});

// The timestamp is part of the row button, never a control of its own, so a
// click on it opens the PR whatever else the row offers.
test("the timestamp sits inside the row button on every kind of row", () => {
  const rows = renderList(
    [
      openPr,
      { ...openPr, number: 8, can_fix_conflict: true, blocked_reasons: ["conflict"] },
      { ...openPr, number: 9, turn: "yours" },
    ],
    [mergedPr],
  );
  expect(rows).toHaveLength(4);
  for (const li of rows) {
    const button = li.match(/<button[^>]*class="pr-row[\s\S]*?<\/button>/)?.[0];
    expect(button).toMatch(/<span class="pr-updated">[^<]+<\/span>/);
    expect(li.match(/pr-updated/g)).toHaveLength(1);
  }
});

// While the request is on its way there is nothing to ask for, so the bar
// says so where the fix was.
test("a row whose Copilot fix is being posted says Asking in place of the fix", () => {
  const [li] = renderList(
    [{ ...openPr, can_fix_conflict: true, blocked_reasons: ["conflict"] }],
    [],
    new Set(["acme/widgets#7"]),
  );
  expect(li).toContain('<span class="pr-action-status" role="status">Asking</span>');
  expect(barOf(li)).toEqual(["Snooze", "Open"]);
});
