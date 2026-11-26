import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import type { Appearance, SnoozeLimit } from "./backend";
import {
  AppearanceRow,
  filterGroups,
  groupRepos,
  pollLabel,
  SnoozeLimitRows,
} from "./SettingsView";

const appearanceRow = (value: Appearance | null) =>
  renderToStaticMarkup(createElement(AppearanceRow, { value, onChange: () => {} }));

test("the appearance row offers System, Light and Dark, in that order", () => {
  const html = appearanceRow("system");
  expect(html).toContain("<span>Appearance</span>");
  expect([...html.matchAll(/<option value="(\w+)"[^>]*>(\w+)<\/option>/g)].map((m) => [m[1], m[2]])).toEqual([
    ["system", "System"],
    ["light", "Light"],
    ["dark", "Dark"],
  ]);
});

test("the appearance row shows the stored choice", () => {
  expect(appearanceRow("dark")).toMatch(/<option value="dark" selected="">/);
  expect(appearanceRow("system")).toMatch(/<option value="system" selected="">/);
});

// A default the user did not pick would be a lie for as long as it showed.
test("the appearance row is blank and disabled until the stored choice arrives", () => {
  const html = appearanceRow(null);
  expect(html).toMatch(/<select[^>]*disabled=""/);
  expect(html).toMatch(/<option value="" selected="">/);
});

const snoozeRows = (value: SnoozeLimit) =>
  renderToStaticMarkup(createElement(SnoozeLimitRows, { value, onChange: () => {} }));
const selectsOf = (html: string) => html.match(/<select[\s\S]*?<\/select>/g) ?? [];

test("the snooze limit is a switch, on with the limit", () => {
  const on = snoozeRows({ enabled: true, days: [1, 7, 30] });
  expect(on).toContain("<span>Limit how long a snooze lasts</span>");
  expect(on).toMatch(/<input[^>]*type="checkbox"[^>]*checked=""/);
  const off = snoozeRows({ enabled: false, days: [1, 7, 30] });
  expect(off).toMatch(/<input[^>]*type="checkbox"/);
  expect(off).not.toMatch(/<input[^>]*checked=""/);
});

test("with the limit on, three dropdowns each offer the six lengths and show the one set", () => {
  const selects = selectsOf(snoozeRows({ enabled: true, days: [3, 14, 90] }));
  expect(selects).toHaveLength(3);
  for (const select of selects) {
    expect(
      [...select.matchAll(/<option value="(\d+)"[^>]*>([^<]+)<\/option>/g)].map((m) => [
        m[1],
        m[2],
      ]),
    ).toEqual([
      ["1", "1 day"],
      ["3", "3 days"],
      ["7", "1 week"],
      ["14", "2 weeks"],
      ["30", "1 month"],
      ["90", "3 months"],
    ]);
  }
  expect(selects.map((select) => select.match(/value="(\d+)" selected=""/)?.[1])).toEqual([
    "3",
    "14",
    "90",
  ]);
});

// Three unlabelled dropdowns in a row would all read alike.
test("each duration dropdown says which of the three it is", () => {
  const selects = selectsOf(snoozeRows({ enabled: true, days: [1, 7, 30] }));
  expect(selects.map((select) => select.match(/aria-label="([^"]+)"/)?.[1])).toEqual([
    "First duration",
    "Second duration",
    "Third duration",
  ]);
});

test("with the limit off, the dropdowns are not shown", () => {
  expect(selectsOf(snoozeRows({ enabled: false, days: [3, 14, 90] }))).toEqual([]);
});

test("preset intervals label as minutes or an hour", () => {
  expect(pollLabel(60)).toBe("1 minute");
  expect(pollLabel(120)).toBe("2 minutes");
  expect(pollLabel(300)).toBe("5 minutes");
  expect(pollLabel(3600)).toBe("1 hour");
});

test("sub-minute and off-minute values label as seconds", () => {
  // Reachable only via a hand-edited settings.json; the select still has to
  // render something truthful for them.
  expect(pollLabel(30)).toBe("30 seconds");
  expect(pollLabel(90)).toBe("90 seconds");
});

test("repos group by owner with groups and repos alphabetized", () => {
  const groups = groupRepos(
    ["zeta/api", "Acme/widgets", "acme/anvils", "khietle/dotfiles"],
    [],
  );
  expect(groups.map((g) => g.owner)).toEqual(["Acme", "khietle", "zeta"]);
  expect(groups[0].repos.map((r) => r.name)).toEqual(["anvils", "widgets"]);
  expect(groups.flatMap((g) => g.repos).every((r) => !r.watched)).toBe(true);
});

test("watched matching is case-insensitive and keeps the fetched casing", () => {
  const groups = groupRepos(["Acme/Widgets"], ["acme/widgets"]);
  expect(groups).toHaveLength(1);
  expect(groups[0].repos).toEqual([
    { fullName: "Acme/Widgets", name: "Widgets", watched: true },
  ]);
});

test("watched repos absent from the fetch appear checked in sort position", () => {
  // An external OSS repo (outside the owner/org affiliation) must stay
  // visible or it could never be unchecked. The same union is what degrades
  // a failed fetch (empty `available`) to the watched repos as checked rows.
  const groups = groupRepos(["acme/widgets"], ["rails/rails", "acme/widgets"]);
  expect(groups.map((g) => g.owner)).toEqual(["acme", "rails"]);
  expect(groups[1].repos).toEqual([
    { fullName: "rails/rails", name: "rails", watched: true },
  ]);
  const failed = groupRepos([], ["acme/widgets", "khietle/dotfiles"]);
  expect(failed.flatMap((g) => g.repos).map((r) => r.watched)).toEqual([
    true,
    true,
  ]);
});

test("the filter matches owner, name, and across the slash", () => {
  const groups = groupRepos(["acme/widgets", "acme/anvils", "zeta/api"], []);
  expect(filterGroups(groups, "ACME").map((g) => g.owner)).toEqual(["acme"]);
  expect(
    filterGroups(groups, "widg").flatMap((g) => g.repos.map((r) => r.name)),
  ).toEqual(["widgets"]);
  expect(filterGroups(groups, "acme/a").flatMap((g) => g.repos.map((r) => r.name))).toEqual(
    ["anvils"],
  );
});

test("a blank filter returns every group", () => {
  const groups = groupRepos(["acme/widgets", "zeta/api"], []);
  expect(filterGroups(groups, "")).toEqual(groups);
  expect(filterGroups(groups, "   ")).toEqual(groups);
});
