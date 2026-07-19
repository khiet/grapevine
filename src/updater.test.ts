import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
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

// The updater plugins are the branch's first mocked seam: checkForUpdates is
// pure decision logic wrapped around them, so the tests drive check() and read
// the resulting store phase. Fresh module state per test starts each at idle.
const { check } = vi.hoisted(() => ({ check: vi.fn() }));
vi.mock("@tauri-apps/plugin-updater", () => ({ check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: vi.fn() }));

describe("checkForUpdates", () => {
  beforeEach(() => {
    vi.resetModules();
    check.mockReset();
    // The failure paths log to console.error by design; keep the suite quiet.
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("a background check that finds nothing stays silent", async () => {
    check.mockResolvedValue(null);
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);

    expect(currentUpdateState()).toEqual({ phase: "idle" });
  });

  test("a manual check that finds nothing reports up to date", async () => {
    check.mockResolvedValue(null);
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(true);

    expect(currentUpdateState()).toEqual({ phase: "up-to-date" });
  });

  test("a background check that errors stays silent", async () => {
    check.mockRejectedValue(new Error("offline"));
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);

    expect(currentUpdateState()).toEqual({ phase: "idle" });
  });

  test("a manual check that errors surfaces the failure", async () => {
    check.mockRejectedValue(new Error("offline"));
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(true);

    expect(currentUpdateState()).toEqual({ phase: "failed" });
  });

  test("a found update downloads and stages itself for restart", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: "0.6.0", downloadAndInstall });
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);

    expect(downloadAndInstall).toHaveBeenCalledOnce();
    expect(currentUpdateState()).toEqual({ phase: "ready", version: "0.6.0" });
  });

  test("a staged update is not re-checked or replaced", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: "0.6.0", downloadAndInstall });
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);
    await checkForUpdates(true);

    // The guard short-circuits once an update is ready: check runs only once,
    // so the staged version survives instead of being downloaded again.
    expect(check).toHaveBeenCalledOnce();
    expect(currentUpdateState()).toEqual({ phase: "ready", version: "0.6.0" });
  });
});
