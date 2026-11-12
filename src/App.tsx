import { useEffect, useState } from "react";
import "./App.css";
import {
  EMPTY_SNAPSHOT,
  markAllRead,
  subscribeSnapshot,
  type Snapshot,
} from "./backend";
import { forgetCleared, useFixRequests } from "./fixConflict";
import { derivePopover } from "./popoverView";
import PrList, { formatLastSync, useCollapsedSections } from "./PrList";
import SettingsView from "./SettingsView";
import {
  restartLabel,
  restartToUpdate,
  startUpdateChecks,
  useUpdateState,
} from "./updater";

function App() {
  const [view, setView] = useState<"list" | "settings">("list");
  const [filter, setFilter] = useState("");
  const [snapshot, setSnapshot] = useState<Snapshot>(EMPTY_SNAPSHOT);
  const inSettings = view === "settings";
  const updateState = useUpdateState();

  const [collapsed, toggleSection] = useCollapsedSections();
  const fixRequests = useFixRequests();

  // Recomputed each render: what to show is a pure view over the snapshot,
  // the filter, the collapse state, the Copilot fixes asked for and the time.
  // Nothing ticks: the waiting labels are as fresh as the last render, and
  // every sync causes one.
  const { offerMarkAllRead, content } = derivePopover(
    snapshot,
    filter,
    collapsed,
    Date.now(),
    fixRequests,
  );

  useEffect(() => {
    startUpdateChecks();
    return subscribeSnapshot((next) => {
      forgetCleared(next);
      setSnapshot(next);
    });
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
          {!inSettings && offerMarkAllRead && (
            <button
              type="button"
              className="settings-link"
              onClick={() => markAllRead().catch(() => {})}
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
          {restartLabel(updateState)}
        </button>
      )}
      {inSettings ? (
        <SettingsView />
      ) : content.kind === "setup" ? (
        <main className="popover-body">
          <p className="placeholder-heading">No pull requests yet</p>
          <p className="placeholder-detail">
            Configure a token and repositories to start watching.
          </p>
        </main>
      ) : content.kind === "no-matches" ? (
        <main className="popover-body">
          <p className="placeholder-heading">No matches</p>
          <p className="placeholder-detail">
            No pull requests match "{content.query}".
          </p>
        </main>
      ) : (
        <PrList view={content} onToggle={toggleSection} />
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
