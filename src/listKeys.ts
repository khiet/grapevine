// The list's keyboard model, as pure rules: where the selection goes, what a
// key means in each place focus can be, and the hints the footer shows. The
// hook in PrList.tsx applies them to the DOM.
//
// Two zones take keys. In the filter every printable key belongs to the
// query, so only the arrows, Enter and Tab are commands there. In the list
// focus sits on a row's own controls, and letters are free to be shortcuts.

import { prKey } from "./backend";
import type { ListView, RowAction, RowActionId } from "./popoverView";

/** Where keyboard focus is; null when it is on neither (the header buttons,
 * the footer, Settings). */
export type Zone = "filter" | "list" | null;

/** The slice of a keyboard event the rules read. */
export interface KeyPress {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  /** True for the presses a held key keeps sending. */
  repeat: boolean;
}

/** The row `step` away from `selected` in `keys`, the rows in the order they
 * are shown. It stops at the ends rather than wrapping, so holding a key
 * down lands on the first or last row. With nothing selected, Down starts at
 * the first row and Up at the last. */
export function moveSelection(
  keys: string[],
  selected: string | null,
  step: 1 | -1,
): string | null {
  if (keys.length === 0) return null;
  const index = selected === null ? -1 : keys.indexOf(selected);
  if (index === -1) return step === 1 ? keys[0] : keys[keys.length - 1];
  return keys[Math.max(0, Math.min(keys.length - 1, index + step))];
}

/** The selection once the rows change from `before` to `after`: unchanged
 * while its row is still shown, otherwise the nearest row that followed it
 * and is still shown, or failing that the nearest that preceded it. */
export function selectionAfter(
  before: string[],
  after: string[],
  selected: string | null,
): string | null {
  if (selected === null || after.includes(selected)) return selected;
  const index = before.indexOf(selected);
  if (index === -1) return null;
  const shown = new Set(after);
  const following = before.slice(index + 1).find((key) => shown.has(key));
  if (following !== undefined) return following;
  const preceding = before.slice(0, index).reverse();
  return preceding.find((key) => shown.has(key)) ?? null;
}

const modified = (press: KeyPress) =>
  press.metaKey || press.ctrlKey || press.altKey;

/** What a key pressed in the filter asks of the list; null leaves the key to
 * the field. Never a printable key: those are the query. */
export function filterCommand(
  press: KeyPress,
): "next" | "previous" | "open" | "list" | null {
  if (modified(press)) return null;
  switch (press.key) {
    case "ArrowDown":
      return "next";
    case "ArrowUp":
      return "previous";
    case "Enter":
      return "open";
    case "Tab":
      return press.shiftKey ? null : "list";
    default:
      return null;
  }
}

/** What a key pressed anywhere in the list asks for. */
export function listCommand(
  press: KeyPress,
): "next" | "previous" | "filter" | null {
  if (modified(press)) return null;
  switch (press.key) {
    case "ArrowDown":
      return "next";
    case "ArrowUp":
      return "previous";
    case "Escape":
      return "filter";
    default:
      return null;
  }
}

const offered = (actions: RowAction[], ...ids: RowActionId[]) =>
  actions.find((action) => ids.includes(action.id)) ?? null;

/** The action a key pressed on a row runs, out of the ones that row offers:
 * S snoozes or wakes, Delete dismisses. One press, one row: focus moves to
 * the next row when the action removes this one, so a held key would
 * otherwise run down the list. */
export function rowCommand(
  press: KeyPress,
  actions: RowAction[],
): RowActionId | null {
  if (modified(press) || press.repeat) return null;
  switch (press.key) {
    case "s":
    case "S":
      return offered(actions, "snooze", "wake")?.id ?? null;
    // A Mac's Delete key reports Backspace.
    case "Delete":
    case "Backspace":
      return offered(actions, "dismiss")?.id ?? null;
    default:
      return null;
  }
}

export interface KeyHint {
  keys: string;
  label: string;
}

/** The keys that apply right now, for the footer. `actions` are the selected
 * row's; null when no row is selected. */
export function keyHints(zone: Zone, actions: RowAction[] | null): KeyHint[] {
  if (zone === "filter") {
    const hints = [{ keys: "↑↓", label: "Select" }];
    if (actions) hints.push({ keys: "Enter", label: "Open" });
    return hints;
  }
  if (zone === "list") {
    const hints: KeyHint[] = [];
    const snooze = actions && offered(actions, "snooze", "wake");
    if (snooze) hints.push({ keys: "S", label: snooze.label });
    const dismiss = actions && offered(actions, "dismiss");
    if (dismiss) hints.push({ keys: "Del", label: dismiss.label });
    hints.push({ keys: "Esc", label: "Filter" });
    return hints;
  }
  return [];
}

export interface FoundRow {
  /** The row in words, for a screen reader told of a selection that moved
   * while focus stayed in the filter. */
  label: string;
  actions: RowAction[];
}

/** The row filed under `key`, wherever its section keeps it. */
export function findRow(view: ListView, key: string): FoundRow | null {
  for (const { body } of view.sections) {
    if (body.kind === "empty") continue;
    if (body.kind === "merged") {
      const row = body.rows.find((row) => prKey(row.pr) === key);
      if (!row) continue;
      const { title, number, repo } = row.pr;
      return {
        label: `${title}, #${number} ${repo}, Merged`,
        actions: row.actions,
      };
    }
    const rows =
      body.kind === "groups"
        ? body.groups.flatMap((group) => group.rows)
        : [...body.rows, ...(body.snoozed?.rows ?? [])];
    const row = rows.find((row) => prKey(row.pr) === key);
    if (!row) continue;
    const { title, number, repo } = row.pr;
    const words = [
      title,
      `#${number} ${repo}`,
      ...(row.draft ? ["Draft"] : []),
      ...(row.state?.all ?? []),
    ];
    return { label: words.join(", "), actions: row.actions };
  }
  return null;
}
