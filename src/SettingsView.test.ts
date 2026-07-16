import { expect, test } from "vitest";
import { pollLabel } from "./SettingsView";

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
