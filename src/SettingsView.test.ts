import { expect, test } from "vitest";
import { filterGroups, groupRepos, pollLabel } from "./SettingsView";

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
  // visible or it could never be unchecked.
  const groups = groupRepos(["acme/widgets"], ["rails/rails", "acme/widgets"]);
  expect(groups.map((g) => g.owner)).toEqual(["acme", "rails"]);
  expect(groups[1].repos).toEqual([
    { fullName: "rails/rails", name: "rails", watched: true },
  ]);
});

test("a failed fetch degrades to the watched repos as checked rows", () => {
  const groups = groupRepos([], ["acme/widgets", "khietle/dotfiles"]);
  expect(groups.flatMap((g) => g.repos).map((r) => r.watched)).toEqual([
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

test("groups emptied by the filter disappear", () => {
  const groups = groupRepos(["acme/widgets", "zeta/api"], []);
  expect(filterGroups(groups, "api").map((g) => g.owner)).toEqual(["zeta"]);
});

test("a blank filter returns every group", () => {
  const groups = groupRepos(["acme/widgets", "zeta/api"], []);
  expect(filterGroups(groups, "")).toBe(groups);
  expect(filterGroups(groups, "   ")).toBe(groups);
});
