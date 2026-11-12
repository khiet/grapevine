import { useSyncExternalStore } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

// One store shared by the popover banner and the settings row, so a background
// check and a manual check can never disagree about what the app knows.
export type UpdateState =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "up-to-date" }
  | { phase: "downloading"; version: string }
  // needsPassword: applying the update will raise an administrator prompt.
  | { phase: "ready"; version: string; needsPassword: boolean }
  | { phase: "failed"; reason: "check" | "install" };

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

let state: UpdateState = { phase: "idle" };
const listeners = new Set<() => void>();

// Installing a .deb needs administrator rights, so on Linux the install is
// held back until the user asks for it, so the password prompt is always the
// result of their own click and never of a background check.
function installNeedsPassword(): boolean {
  return /\bLinux\b/.test(navigator.userAgent);
}

// Downloaded and verified but not yet installed. Non-null only while the
// "ready" state has needsPassword set; restartToUpdate() consumes it.
let awaitingInstall: Update | null = null;

function setState(next: UpdateState) {
  state = next;
  for (const listener of listeners) listener();
}

export function useUpdateState(): UpdateState {
  return useSyncExternalStore(
    (onChange) => {
      listeners.add(onChange);
      return () => listeners.delete(onChange);
    },
    () => state,
  );
}

/** Imperative read of the same store useUpdateState() binds to. */
export function currentUpdateState(): UpdateState {
  return state;
}

/** Settings-row copy for each update phase; empty when there is nothing to say. */
export function updateStatusLabel(state: UpdateState): string {
  switch (state.phase) {
    case "idle":
      return "";
    case "checking":
      return "Checking…";
    case "up-to-date":
      return "Grapevine is up to date.";
    case "downloading":
      return `Downloading v${state.version}…`;
    case "ready":
      return state.needsPassword
        ? `v${state.version} is ready. Restart to apply it; you will be asked for your password.`
        : `v${state.version} is ready. Restart to apply it.`;
    case "failed":
      return state.reason === "install"
        ? "Could not install the update."
        : "Could not check for updates.";
  }
}

/** Popover banner copy for a ready update. */
export function restartLabel(
  state: Extract<UpdateState, { phase: "ready" }>,
): string {
  return state.needsPassword
    ? `Restart to update to v${state.version} (asks for your password)`
    : `Restart to update to v${state.version}`;
}

/**
 * Check the release feed and, if an update exists, download it. Where the
 * install needs no password it is staged here too; otherwise the install waits
 * for restartToUpdate(). Either way nothing takes effect before then, and
 * this function never raises a prompt. Failures on a background check reset
 * to idle so nothing is surfaced; a manual check moves to "failed" or
 * "up-to-date" so the settings row always answers the click.
 */
export async function checkForUpdates(manual: boolean): Promise<void> {
  // A staged or in-flight download stays authoritative: re-checking cannot
  // improve on it until the app restarts. A concurrent check is a no-op too.
  if (
    state.phase === "checking" ||
    state.phase === "downloading" ||
    state.phase === "ready"
  ) {
    return;
  }
  setState({ phase: "checking" });
  try {
    const update = await check();
    if (update === null) {
      setState(manual ? { phase: "up-to-date" } : { phase: "idle" });
      return;
    }
    setState({ phase: "downloading", version: update.version });
    const needsPassword = installNeedsPassword();
    if (needsPassword) {
      await update.download();
      awaitingInstall = update;
    } else {
      await update.downloadAndInstall();
    }
    setState({ phase: "ready", version: update.version, needsPassword });
  } catch (error) {
    console.error("Update check failed:", error);
    setState(manual ? { phase: "failed", reason: "check" } : { phase: "idle" });
  }
}

let started = false;

/**
 * Check now and every six hours from now on. Guarded so StrictMode's double
 * mount (or any later remount) cannot stack intervals or duplicate checks.
 * Never runs in dev: a dev process has no .app bundle to update, so a check
 * that found something would download it only to fail the install.
 */
export function startUpdateChecks(): void {
  if (started || import.meta.env.DEV) return;
  started = true;
  void checkForUpdates(false);
  setInterval(() => void checkForUpdates(false), CHECK_INTERVAL_MS);
}

let applying = false;

/**
 * Apply the ready update and relaunch into it. A held-back install runs first
 * and is what raises the password prompt; if it is declined or fails, the
 * running version stays in place and the store moves to "failed", from where
 * a new check can download the update again.
 */
export async function restartToUpdate(): Promise<void> {
  // The banner stays clickable while the password prompt is open.
  if (applying) return;
  applying = true;
  try {
    if (awaitingInstall !== null) {
      const update = awaitingInstall;
      awaitingInstall = null;
      try {
        await update.install();
      } catch (error) {
        console.error("Update install failed:", error);
        // The plugin keeps the downloaded package in memory after a failed
        // install; release it, since the next check downloads a fresh one.
        await update.close().catch(() => {});
        setState({ phase: "failed", reason: "install" });
        return;
      }
      // Installed: if the relaunch below fails, a retry only relaunches, so
      // the copy must stop promising a password prompt.
      setState({ phase: "ready", version: update.version, needsPassword: false });
    }
    try {
      await relaunch();
    } catch (error) {
      console.error("Relaunch failed:", error);
    }
  } finally {
    applying = false;
  }
}
