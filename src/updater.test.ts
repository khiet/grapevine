import { expect, test } from "vitest";
import { updateStatusLabel } from "./updater";

test("quiet phases say nothing", () => {
  expect(updateStatusLabel({ phase: "idle" })).toBe("");
});

test("in-flight phases report progress", () => {
  expect(updateStatusLabel({ phase: "checking" })).toBe("Checking…");
  expect(updateStatusLabel({ phase: "downloading", version: "0.6.0" })).toBe(
    "Downloading v0.6.0…",
  );
});

test("terminal phases answer the manual check", () => {
  expect(updateStatusLabel({ phase: "up-to-date" })).toBe(
    "Grapevine is up to date.",
  );
  expect(updateStatusLabel({ phase: "ready", version: "0.6.0" })).toBe(
    "v0.6.0 is ready. Restart to apply it.",
  );
  expect(updateStatusLabel({ phase: "failed" })).toBe(
    "Could not check for updates.",
  );
});
