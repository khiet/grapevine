import { expect, test } from "vitest";
import { EMPTY_SNAPSHOT, type MergedPr, type PullRequest } from "./backend";
import {
  filterCommand,
  findRow,
  keyHints,
  listCommand,
  moveSelection,
  rowCommand,
  selectionAfter,
} from "./listKeys";
import { derivePopover, type ListView, type RowAction } from "./popoverView";

const KEYS = ["a", "b", "c"];

test("Down and Up step the selection through the rows", () => {
  expect(moveSelection(KEYS, "a", 1)).toBe("b");
  expect(moveSelection(KEYS, "b", 1)).toBe("c");
  expect(moveSelection(KEYS, "c", -1)).toBe("b");
});

test("the selection stops at either end of the list", () => {
  expect(moveSelection(KEYS, "c", 1)).toBe("c");
  expect(moveSelection(KEYS, "a", -1)).toBe("a");
});

test("with nothing selected, Down starts at the first row and Up at the last", () => {
  expect(moveSelection(KEYS, null, 1)).toBe("a");
  expect(moveSelection(KEYS, null, -1)).toBe("c");
});

test("a list with no rows has nothing to select", () => {
  expect(moveSelection([], null, 1)).toBeNull();
  expect(moveSelection([], "a", -1)).toBeNull();
});

test("a selection whose row is still listed stays put", () => {
  expect(selectionAfter(KEYS, ["c", "b", "a"], "b")).toBe("b");
  expect(selectionAfter(KEYS, KEYS, null)).toBeNull();
});

test("when the selected row leaves, the selection moves to the next row", () => {
  expect(selectionAfter(KEYS, ["a", "c"], "b")).toBe("c");
  expect(selectionAfter(KEYS, ["b", "c"], "a")).toBe("b");
});

test("when the last row leaves, the selection moves to the previous one", () => {
  expect(selectionAfter(KEYS, ["a", "b"], "c")).toBe("b");
});

test("rows that left with the selected one are passed over", () => {
  expect(selectionAfter(["a", "b", "c", "d"], ["a", "d"], "b")).toBe("d");
  expect(selectionAfter(["a", "b", "c", "d"], ["a", "x"], "b")).toBe("a");
});

test("when every row leaves, nothing is selected", () => {
  expect(selectionAfter(KEYS, [], "b")).toBeNull();
  expect(selectionAfter(KEYS, ["x"], "b")).toBeNull();
});

const key = (name: string, modifiers: Partial<Record<"metaKey" | "ctrlKey" | "altKey" | "shiftKey" | "repeat", boolean>> = {}) => ({
  key: name,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  repeat: false,
  ...modifiers,
});

test("from the filter, the arrows move the selection, Enter opens and Tab enters the list", () => {
  expect(filterCommand(key("ArrowDown"))).toBe("next");
  expect(filterCommand(key("ArrowUp"))).toBe("previous");
  expect(filterCommand(key("Enter"))).toBe("open");
  expect(filterCommand(key("Tab"))).toBe("list");
});

// Every printable key belongs to the query while the filter has focus.
test("letters typed in the filter are not shortcuts", () => {
  for (const letter of ["s", "S", "d", "x", " ", "Backspace", "Delete"]) {
    expect(filterCommand(key(letter))).toBeNull();
  }
});

test("Shift+Tab leaves the filter the ordinary way", () => {
  expect(filterCommand(key("Tab", { shiftKey: true }))).toBeNull();
});

const SNOOZE: RowAction = { id: "snooze", label: "Snooze", hint: null };
const WAKE: RowAction = { id: "wake", label: "Wake", hint: null };
const OPEN: RowAction = { id: "open", label: "Open", hint: null };
const DISMISS: RowAction = { id: "dismiss", label: "Dismiss", hint: null };

test("S snoozes a row that can be snoozed and wakes a snoozed one", () => {
  expect(rowCommand(key("s"), [SNOOZE, OPEN])).toBe("snooze");
  expect(rowCommand(key("S", { shiftKey: true }), [SNOOZE, OPEN])).toBe("snooze");
  expect(rowCommand(key("s"), [WAKE, OPEN])).toBe("wake");
});

test("S does nothing on a row that offers neither", () => {
  expect(rowCommand(key("s"), [OPEN])).toBeNull();
  expect(rowCommand(key("s"), [DISMISS, OPEN])).toBeNull();
});

// A Mac's Delete key reports Backspace; the forward-delete key reports Delete.
test("Delete dismisses a merged row and nothing else", () => {
  expect(rowCommand(key("Delete"), [DISMISS, OPEN])).toBe("dismiss");
  expect(rowCommand(key("Backspace"), [DISMISS, OPEN])).toBe("dismiss");
  expect(rowCommand(key("Delete"), [SNOOZE, OPEN])).toBeNull();
});

test("a key held with Command, Control or Option is not a row shortcut", () => {
  expect(rowCommand(key("s", { metaKey: true }), [SNOOZE, OPEN])).toBeNull();
  expect(rowCommand(key("s", { ctrlKey: true }), [SNOOZE, OPEN])).toBeNull();
  expect(rowCommand(key("Backspace", { altKey: true }), [DISMISS, OPEN])).toBeNull();
});

// The row after a removed one takes focus, so a held key would act on it too.
test("a held key acts on one row only", () => {
  expect(rowCommand(key("s", { repeat: true }), [SNOOZE, OPEN])).toBeNull();
  expect(rowCommand(key("Delete", { repeat: true }), [DISMISS, OPEN])).toBeNull();
});

test("in the list, the arrows move between rows and Escape returns to the filter", () => {
  expect(listCommand(key("ArrowDown"))).toBe("next");
  expect(listCommand(key("ArrowUp"))).toBe("previous");
  expect(listCommand(key("Escape"))).toBe("filter");
  expect(listCommand(key("s"))).toBeNull();
});

const hintText = (hints: { keys: string; label: string }[]) =>
  hints.map((hint) => `${hint.keys} ${hint.label}`);

test("the filter's hints offer Open only once a row is selected", () => {
  expect(hintText(keyHints("filter", null))).toEqual(["↑↓ Select"]);
  expect(hintText(keyHints("filter", [SNOOZE, OPEN]))).toEqual([
    "↑↓ Select",
    "Enter Open",
  ]);
});

test("the list's hints name what the keys do to the selected row", () => {
  expect(hintText(keyHints("list", [SNOOZE, OPEN]))).toEqual(["S Snooze", "Esc Filter"]);
  expect(hintText(keyHints("list", [WAKE, OPEN]))).toEqual(["S Wake", "Esc Filter"]);
  expect(hintText(keyHints("list", [DISMISS, OPEN]))).toEqual(["Del Dismiss", "Esc Filter"]);
  expect(hintText(keyHints("list", [OPEN]))).toEqual(["Esc Filter"]);
  expect(hintText(keyHints("list", null))).toEqual(["Esc Filter"]);
});

test("with focus elsewhere there are no hints", () => {
  expect(keyHints(null, [SNOOZE, OPEN])).toEqual([]);
});

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

function listView(prs: PullRequest[], snoozed: string[] = []): ListView {
  const { content } = derivePopover(
    { ...EMPTY_SNAPSHOT, has_synced: true, prs, merged: [mergedPr], snoozed },
    "",
    { mine: false, participated: false, all: false, merged: false },
    Date.parse("2026-07-16T15:30:00Z"),
  );
  return content as ListView;
}

test("a row is found by its key, with the words a screen reader is given", () => {
  const view = listView([
    { ...openPr, approved: true, checks_running: true, is_draft: true },
  ]);
  expect(findRow(view, "acme/widgets#7")).toEqual({
    label: "Fix the thing, #7 acme/widgets, Draft, Checks running, Approved",
    actions: [
      { id: "snooze", label: "Snooze", hint: "Snooze until it changes" },
      { id: "open", label: "Open", hint: null },
    ],
  });
});

test("rows are found wherever a section keeps them", () => {
  const view = listView(
    [
      { ...openPr, number: 1, turn: "yours" },
      { ...openPr, number: 2, section: "all" },
      { ...openPr, number: 3 },
    ],
    ["acme/widgets#3"],
  );
  expect(findRow(view, "acme/widgets#1")?.label).toBe("Fix the thing, #1 acme/widgets");
  expect(findRow(view, "acme/widgets#2")?.actions.map((a) => a.id)).toEqual(["open"]);
  expect(findRow(view, "acme/widgets#3")?.actions.map((a) => a.id)).toEqual(["wake", "open"]);
  expect(findRow(view, "acme/widgets#90")).toEqual({
    label: "Ship the release notes, #90 acme/widgets, Merged",
    actions: [
      { id: "dismiss", label: "Dismiss", hint: null },
      { id: "open", label: "Open", hint: null },
    ],
  });
});

test("a key that names no row finds nothing", () => {
  expect(findRow(listView([openPr]), "acme/widgets#404")).toBeNull();
});
