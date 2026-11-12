import { expect, test } from "vitest";
import {
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
    landedAt: at(2026, 7, 16, 15, 4).toISOString(),
    failed: false,
    completedAt: at(2026, 7, 16, 15, 4).toISOString(),
    url: RUN,
  });
  expect(tip).toBe("Merged 16 Jul 2026, 14:59\nDeployed 16 Jul 2026, 15:04");
});

test("the tooltip of a deploy under way has only the merge time to give", () => {
  const tip = deployTip(at(2026, 1, 2, 9, 5).toISOString(), {
    label: "Deploying",
    landedAt: null,
    failed: false,
    completedAt: null,
    url: RUN,
  });
  expect(tip).toBe("Merged 2 Jan 2026, 09:05\nDeploying");
});
