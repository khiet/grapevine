import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import PrList, {
  formatLastSync,
  matchesFilter,
  Snapshot,
  totalUnread,
} from "./PrList";
import SettingsView from "./SettingsView";
import { restartToUpdate, startUpdateChecks, useUpdateState } from "./updater";

function App() {
  const [view, setView] = useState<"list" | "settings">("list");
  const [filter, setFilter] = useState("");
  const [snapshot, setSnapshot] = useState<Snapshot>({
    prs: [],
    merged: [],
    has_synced: false,
    last_sync_at: null,
    sync_error: null,
  });
  const inSettings = view === "settings";
  const updateState = useUpdateState();

  // The filter is a view over the current snapshot, recomputed each render.
  const visiblePrs = snapshot.prs.filter((pr) => matchesFilter(pr, filter));
  const visibleMerged = snapshot.merged.filter((pr) => matchesFilter(pr, filter));
  const hasAny = snapshot.prs.length > 0 || snapshot.merged.length > 0;
  const hasVisible = visiblePrs.length > 0 || visibleMerged.length > 0;

  useEffect(() => {
    startUpdateChecks();
    invoke<Snapshot>("get_prs").then(setSnapshot).catch(() => {});
    const unlisten = listen<Snapshot>("prs-updated", (event) =>
      setSnapshot(event.payload),
    );
    return () => {
      unlisten.then((stop) => stop());
    };
  }, []);

  return (
    <div className="popover">
      <header className="popover-header">
        {inSettings ? (
          <span className="popover-title">Settings</span>
        ) : (
          <div className="header-search">
            <svg
              className="header-search-icon"
              viewBox="0 0 16 16"
              aria-hidden="true"
            >
              <path
                fill="currentColor"
                d="M6.5 1a5.5 5.5 0 0 1 4.383 8.82l3.148 3.15a.75.75 0 0 1-1.06 1.06l-3.15-3.147A5.5 5.5 0 1 1 6.5 1Zm0 1.5a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z"
              />
            </svg>
            <input
              type="search"
              className="header-search-input"
              placeholder="Filter"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape") setFilter("");
              }}
              autoFocus
            />
          </div>
        )}
        <span className="header-actions">
          {!inSettings && totalUnread(snapshot.prs) > 0 && (
            <button
              type="button"
              className="settings-link"
              onClick={() => invoke("mark_all_read").catch(() => {})}
            >
              Mark all read
            </button>
          )}
          <button
            type="button"
            className="settings-link"
            onClick={() => setView(inSettings ? "list" : "settings")}
          >
            {inSettings ? "Done" : "Settings"}
          </button>
        </span>
      </header>
      {/* Above the body in both views: an update staged by a background check
          has no other way to get noticed in a popover that is usually hidden. */}
      {updateState.phase === "ready" && (
        <button
          type="button"
          className="update-banner"
          onClick={() => restartToUpdate()}
        >
          Restart to update to v{updateState.version}
        </button>
      )}
      {inSettings ? (
        <SettingsView />
      ) : hasVisible ? (
        <PrList
          prs={visiblePrs}
          merged={visibleMerged}
          filtering={filter.trim() !== ""}
        />
      ) : hasAny ? (
        /* Rows exist but the filter hid them all. */
        <main className="popover-body">
          <p className="placeholder-heading">No matches</p>
          <p className="placeholder-detail">
            No pull requests match "{filter}".
          </p>
        </main>
      ) : (
        <main className="popover-body">
          <p className="placeholder-heading">
            {snapshot.has_synced ? "No open pull requests" : "No pull requests yet"}
          </p>
          <p className="placeholder-detail">
            {snapshot.has_synced
              ? "The watched repositories have no open PRs."
              : "Configure a token and repositories to start watching."}
          </p>
        </main>
      )}
      {!inSettings && (snapshot.last_sync_at !== null || snapshot.sync_error) && (
        <footer className="sync-status">
          {snapshot.last_sync_at !== null && (
            <span className="sync-time">
              {formatLastSync(snapshot.last_sync_at)}
            </span>
          )}
          {snapshot.sync_error && (
            /* title carries the full message; the footer line clips it. */
            <span className="sync-error" title={snapshot.sync_error}>
              {snapshot.sync_error}
            </span>
          )}
        </footer>
      )}
    </div>
  );
}

export default App;
