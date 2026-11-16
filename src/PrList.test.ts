import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { EMPTY_SNAPSHOT, type MergedPr, type PullRequest } from "./backend";
import { derivePopover, type ListView, type WritesPending } from "./popoverView";
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
    tone: "ok",
    showsTime: true,
    completedAt: at(2026, 7, 16, 15, 4).toISOString(),
    url: RUN,
  });
  expect(tip).toBe("Merged 16 Jul 2026, 14:59\nDeployed 16 Jul 2026, 15:04");
});

test("the tooltip of a deploy under way has only the merge time to give", () => {
  const tip = deployTip(at(2026, 1, 2, 9, 5).toISOString(), {
    label: "Deploying",
    tone: "neutral",
    showsTime: false,
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
  threads_open: false,
  ready_to_merge: false,
  turn: null,
  turn_since: null,
  can_fix_conflict: false,
  fix_requested: false,
  actions: [],
  merge_method: "merge",
  base_branch: "main",
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

test("each row carries its author's handle in its accessible name", () => {
  const [open, merged] = renderRows().filter((li) => li.includes("pr-row"));
  for (const [li, handle] of [
    [open, "@octocat"],
    [merged, "@hubot"],
  ]) {
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
      ' aria-label="Unresolved threads, Checks running, Approved">' +
      '<span class="pr-state-reason">Unresolved threads</span>' +
      '<span class="pr-state-more">+2</span></span>',
  );
});

// The action bar opens beside the state, never over it, or the tooltip
// could not be reached with a pointer. The bar is outside the row button, so
// what pushes it aside is a hidden copy of the state's words after it.
test("an open row's action bar is followed by a hidden copy of the state", () => {
  const [li] = renderList([
    { ...openPr, approved: true, checks_running: true },
  ]);
  expect(li).toContain(
    '</span><span class="pr-state pr-state-space" aria-hidden="true">' +
      '<span class="pr-state-reason">Checks running</span>' +
      '<span class="pr-state-more">+1</span></span></span></li>',
  );
  expect(li).toMatch(/<span class="pr-row-end"><span class="pr-actions"/);
});

test("a draft's action bar is followed by a hidden copy of Draft, then of its state", () => {
  const [li] = renderList([
    { ...openPr, is_draft: true, merge_queue_position: 2 },
  ]);
  expect(li).toContain(
    '</span><span class="pr-state pr-state-space pr-draft" aria-hidden="true">Draft</span>' +
      '<span class="pr-state pr-state-space" aria-hidden="true">' +
      '<span class="pr-state-reason">Queued 2nd</span></span></span></li>',
  );
});

test("a row with no state has nothing after its action bar", () => {
  const [li] = renderList([openPr]);
  expect(li).not.toContain("pr-state");
});

// Draft reads like any other neutral state: a filled shape there would pass
// for the action bar's buttons, which open on the same corner.
test("a draft says Draft in plain words, ahead of any state it has", () => {
  expect(renderOpenRow({ is_draft: true })).toContain(
    '<span class="pr-state pr-draft">Draft</span></span></span></button>',
  );
  expect(renderOpenRow({ is_draft: true, merge_queue_position: 2 })).toContain(
    '<span class="pr-state pr-draft">Draft</span>' +
      '<span class="pr-state is-neutral"><span class="pr-state-reason">Queued 2nd</span></span>',
  );
});

// The row has no room for a check's name, so a lone CI failure gets the
// tooltip and the label a folded row gets, and they name the check.
test("a row whose only state is CI failing says so, and names the check in its tooltip and label", () => {
  const row = renderOpenRow({
    blocked_reasons: ["ci"],
    failing_checks: ["e2e-safari", "lint"],
  });
  expect(row).toContain(
    '<span class="pr-state is-warn pr-tip" role="img"' +
      ' data-tip="CI failing: e2e-safari and more"' +
      ' aria-label="CI failing: e2e-safari and more">' +
      '<span class="pr-state-reason">CI failing</span></span>',
  );
});

test("an open thread nothing requires resolving reads in the neutral tone", () => {
  const row = renderOpenRow({ threads_open: true });
  expect(row).toContain(
    '<span class="pr-state is-neutral">' +
      '<span class="pr-state-reason">Unresolved threads</span></span>',
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
  writesPending: WritesPending = new Map(),
): string[] {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs, merged },
    "",
    { mine: false, participated: false, all: false, merged: false },
    NOW.getTime(),
    writesPending,
  );
  const html = renderToStaticMarkup(
    createElement(PrList, { view: content as ListView, onToggle: () => {} }),
  );
  return (html.match(/<li[\s\S]*?<\/li>/g) ?? []).filter((li) => li.includes("pr-row"));
}

test("each kind of row words its actions in a bar", () => {
  const [plain, fixable, merged] = renderList(
    [
      openPr,
      { ...openPr, number: 8, can_fix_conflict: true, blocked_reasons: ["conflict"] },
    ],
    [mergedPr],
  );
  expect(barOf(plain)).toEqual(["Snooze"]);
  expect(barOf(fixable)).toEqual(["Snooze", "Fix with Copilot"]);
  expect(barOf(merged)).toEqual(["Dismiss"]);
});

// The deploy status of a merged row: the control that opens the run, and
// the row button it must stay out of.
function renderDeploy(deploy: MergedPr["deploy"]) {
  const [li] = renderList([], [{ ...mergedPr, deploy }]);
  const row = li.match(/<button[^>]*class="pr-row[\s\S]*?<\/button>/)?.[0] ?? "";
  const status = li.match(/<button[^>]*class="[^"]*pr-deploy"[\s\S]*?<\/button>/)?.[0];
  return { row, status };
}

// The status reads in the state slot the way an open row's state does:
// words in a tone and nothing else in the button. The time is the row's own
// timestamp format.
test("each deploy state reads in words in its tone", () => {
  const done = at(2026, 7, 11, 12, 0).toISOString();
  const shown = (["deploying", "deployed", "failed", "cancelled"] as const).map(
    (state) => {
      const completed_at = state === "deploying" ? null : done;
      const { status } = renderDeploy({ state, completed_at, url: RUN });
      return status?.match(/class="([^"]*)"[^>]*>([^<]*)<\/button>$/)?.slice(1);
    },
  );
  expect(shown).toEqual([
    ["pr-state is-neutral pr-deploy", "Deploying"],
    ["pr-state is-ok pr-deploy", "Deployed 11 Jul"],
    ["pr-state is-warn pr-deploy", "Deploy failed"],
    ["pr-state is-neutral pr-deploy", "Deploy cancelled"],
  ]);
});

// The row abbreviates; the title is where the exact times live.
test("the deploy status tips the exact merge and deploy times", () => {
  const { status } = renderDeploy({
    state: "deployed",
    completed_at: at(2026, 7, 11, 12, 0).toISOString(),
    url: RUN,
  });
  expect(status).toMatch(/title="Merged [^"]*\nDeployed 11 Jul 2026, 12:00"/);
});

// A button inside the row button is invalid HTML, so the control sits
// beside it and the row keeps a hidden copy of the words to hold their place
// at the end of the metadata line.
test("the deploy control sits outside the row button, over a hidden copy in the state slot", () => {
  const { row, status } = renderDeploy({
    state: "failed",
    completed_at: null,
    url: RUN,
  });
  expect(status).toBeDefined();
  expect(row.match(/<button/g)).toHaveLength(1);
  expect(row).toMatch(
    /<span class="pr-origin">.*<span class="pr-state is-warn pr-state-space" aria-hidden="true">Deploy failed<\/span><\/span>/,
  );
  expect(row).toMatch(
    /<span class="pr-title">[^<]*<\/span><span class="pr-updated">[^<]+<\/span><\/span>/,
  );
});

test("a merged row with no deploy run shows nothing in the state slot", () => {
  const { row, status } = renderDeploy(null);
  expect(status).toBeUndefined();
  expect(row).not.toContain("pr-state");
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
    new Map([["acme/widgets#7", "fix"]]),
  );
  expect(li).toContain('<span class="pr-action-status" role="status">Asking</span>');
  expect(barOf(li)).toEqual(["Snooze"]);
});

test("the bar words the merge, the queue and the branch update", () => {
  const [ready, queued, behind] = renderList([
    { ...openPr, actions: ["merge"] },
    { ...openPr, number: 8, actions: ["queue"] },
    { ...openPr, number: 9, actions: ["update_branch"] },
  ]);
  expect(barOf(ready)).toEqual(["Snooze", "Merge"]);
  expect(barOf(queued)).toEqual(["Snooze", "Queue"]);
  expect(barOf(behind)).toEqual(["Snooze", "Update branch"]);
});

// Arming and cancelling auto-merge are the menu's alone.
test("the bar leaves auto-merge to the menu", () => {
  const [behind, armed] = renderList([
    { ...openPr, actions: ["update_branch", "enable_auto_merge"] },
    { ...openPr, number: 8, actions: ["disable_auto_merge"] },
  ]);
  expect(barOf(behind)).toEqual(["Snooze", "Update branch"]);
  expect(barOf(armed)).toEqual(["Snooze"]);
});

test("a row whose merge is on its way says Merging in place of its writes", () => {
  const [li] = renderList(
    [{ ...openPr, actions: ["merge", "enable_auto_merge"] }],
    [],
    new Map([["acme/widgets#7", "merge"]]),
  );
  expect(li).toContain('<span class="pr-action-status" role="status">Merging</span>');
  expect(barOf(li)).toEqual(["Snooze"]);
});

test("the selected row, and only that row, is marked as the selection", () => {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs: [openPr], merged: [mergedPr] },
    "",
    { ...DEFAULT_COLLAPSED },
    NOW.getTime(),
  );
  const html = renderToStaticMarkup(
    createElement(PrList, {
      view: content as ListView,
      onToggle: () => {},
      keys: {
        ref: { current: null },
        selected: "acme/widgets#90",
        select: () => {},
        acted: () => {},
        onKeyDown: () => {},
      },
    }),
  );
  const selected = (html.match(/<li[^>]*>/g) ?? [])
    .filter((li) => li.includes("is-selected"))
    .map((li) => li.match(/data-row-key="([^"]*)"/)?.[1]);
  expect(selected).toEqual(["acme/widgets#90"]);
});

test("with no selection no row is marked", () => {
  expect(renderRows().join("")).not.toContain("is-selected");
});
