import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { restartLabel, updateStatusLabel } from "./updater";

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
  expect(
    updateStatusLabel({ phase: "ready", version: "0.6.0", needsPassword: false }),
  ).toBe("v0.6.0 is ready. Restart to apply it.");
  expect(updateStatusLabel({ phase: "failed", reason: "check" })).toBe(
    "Could not check for updates.",
  );
  expect(updateStatusLabel({ phase: "failed", reason: "install" })).toBe(
    "Could not install the update.",
  );
});

test("the ready copy warns about the password only where applying asks for one", () => {
  const linux = { phase: "ready", version: "0.6.0", needsPassword: true } as const;
  const macos = { phase: "ready", version: "0.6.0", needsPassword: false } as const;

  expect(updateStatusLabel(linux)).toBe(
    "v0.6.0 is ready. Restart to apply it; you will be asked for your password.",
  );
  expect(restartLabel(linux)).toBe(
    "Restart to update to v0.6.0 (asks for your password)",
  );
  expect(restartLabel(macos)).toBe("Restart to update to v0.6.0");
});

// The updater plugins are the branch's first mocked seam: checkForUpdates is
// pure decision logic wrapped around them, so the tests drive check() and read
// the resulting store phase. Fresh module state per test starts each at idle.
const { check, relaunch } = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
}));
vi.mock("@tauri-apps/plugin-updater", () => ({ check }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch }));

const MACOS_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)";
const LINUX_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko)";

describe("checkForUpdates", () => {
  beforeEach(() => {
    vi.resetModules();
    check.mockReset();
    relaunch.mockReset();
    vi.stubGlobal("navigator", { userAgent: MACOS_UA });
    // The failure paths log to console.error by design; keep the suite quiet.
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
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

    expect(currentUpdateState()).toEqual({
      phase: "failed",
      reason: "check",
    });
  });

  test("a found update downloads and stages itself for restart", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: "0.6.0", downloadAndInstall });
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);

    expect(downloadAndInstall).toHaveBeenCalledOnce();
    expect(currentUpdateState()).toEqual({
      phase: "ready",
      version: "0.6.0",
      needsPassword: false,
    });
  });

  test("restarting a staged update relaunches into it", async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined);
    check.mockResolvedValue({ version: "0.6.0", downloadAndInstall });
    const { checkForUpdates, restartToUpdate } = await import("./updater");
    await checkForUpdates(false);

    await restartToUpdate();

    expect(relaunch).toHaveBeenCalledOnce();
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
    expect(currentUpdateState()).toEqual({
      phase: "ready",
      version: "0.6.0",
      needsPassword: false,
    });
  });
});

describe("on Linux, where installing asks for the password", () => {
  beforeEach(() => {
    vi.resetModules();
    check.mockReset();
    relaunch.mockReset();
    vi.stubGlobal("navigator", { userAgent: LINUX_UA });
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  function foundUpdate(install: () => Promise<void>) {
    const update = {
      version: "0.6.0",
      download: vi.fn().mockResolvedValue(undefined),
      install: vi.fn(install),
      close: vi.fn().mockResolvedValue(undefined),
      downloadAndInstall: vi.fn().mockResolvedValue(undefined),
    };
    check.mockResolvedValue(update);
    return update;
  }

  test("a background check downloads the update without installing it", async () => {
    const update = foundUpdate(async () => {});
    const { checkForUpdates, currentUpdateState } = await import("./updater");

    await checkForUpdates(false);

    expect(update.download).toHaveBeenCalledOnce();
    expect(update.install).not.toHaveBeenCalled();
    expect(update.downloadAndInstall).not.toHaveBeenCalled();
    expect(currentUpdateState()).toEqual({
      phase: "ready",
      version: "0.6.0",
      needsPassword: true,
    });
  });

  test("restarting installs the downloaded update, then relaunches", async () => {
    const steps: string[] = [];
    const update = foundUpdate(async () => {
      steps.push("install");
    });
    relaunch.mockImplementation(async () => {
      steps.push("relaunch");
    });
    const { checkForUpdates, restartToUpdate } = await import("./updater");
    await checkForUpdates(false);

    await restartToUpdate();

    expect(update.install).toHaveBeenCalledOnce();
    expect(steps).toEqual(["install", "relaunch"]);
  });

  test("a declined install keeps the app running and reports failure", async () => {
    foundUpdate(async () => {
      throw new Error("authentication failed");
    });
    const { checkForUpdates, currentUpdateState, restartToUpdate } =
      await import("./updater");
    await checkForUpdates(false);

    await restartToUpdate();

    expect(relaunch).not.toHaveBeenCalled();
    expect(currentUpdateState()).toEqual({ phase: "failed", reason: "install" });
  });

  test("a relaunch that fails after the install stops promising a password prompt", async () => {
    foundUpdate(async () => {});
    relaunch.mockRejectedValue(new Error("spawn failed"));
    const { checkForUpdates, currentUpdateState, restartToUpdate } =
      await import("./updater");
    await checkForUpdates(false);

    await restartToUpdate();

    expect(currentUpdateState()).toEqual({
      phase: "ready",
      version: "0.6.0",
      needsPassword: false,
    });
  });

  test("a second click while the password prompt is open does not install twice", async () => {
    let finishInstall = () => {};
    const update = foundUpdate(
      () => new Promise<void>((resolve) => (finishInstall = resolve)),
    );
    const { checkForUpdates, restartToUpdate } = await import("./updater");
    await checkForUpdates(false);

    const first = restartToUpdate();
    await restartToUpdate();
    finishInstall();
    await first;

    expect(update.install).toHaveBeenCalledOnce();
    expect(relaunch).toHaveBeenCalledOnce();
  });

  test("after a declined install a new check downloads the update again", async () => {
    const update = foundUpdate(async () => {
      throw new Error("authentication failed");
    });
    const { checkForUpdates, currentUpdateState, restartToUpdate } =
      await import("./updater");
    await checkForUpdates(false);
    await restartToUpdate();

    await checkForUpdates(true);

    expect(update.download).toHaveBeenCalledTimes(2);
    expect(currentUpdateState()).toEqual({
      phase: "ready",
      version: "0.6.0",
      needsPassword: true,
    });
  });
});
