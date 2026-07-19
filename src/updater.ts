import { useSyncExternalStore } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

// One store shared by the popover banner and the settings row, so a background
// check and a manual check can never disagree about what the app knows.
export type UpdateState =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "up-to-date" }
  | { phase: "downloading"; version: string }
  | { phase: "ready"; version: string }
  | { phase: "failed" };

const CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;

let state: UpdateState = { phase: "idle" };
const listeners = new Set<() => void>();

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
      return `v${state.version} is ready. Restart to apply it.`;
    case "failed":
      return "Could not check for updates.";
  }
}

/**
 * Check the release feed and, if an update exists, download and stage it.
 * The staged update only takes effect after restartToUpdate(). Failures on a
 * background check reset to idle so nothing is surfaced; a manual check moves
 * to "failed"/"up-to-date" so the settings row always answers the click.
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
    await update.downloadAndInstall();
    setState({ phase: "ready", version: update.version });
  } catch (error) {
    console.error("Update check failed:", error);
    setState(manual ? { phase: "failed" } : { phase: "idle" });
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

export async function restartToUpdate(): Promise<void> {
  try {
    await relaunch();
  } catch (error) {
    console.error("Relaunch failed:", error);
  }
}
