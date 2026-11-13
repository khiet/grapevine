import { useEffect, useState } from "react";
import "./App.css";
import {
  EMPTY_SNAPSHOT,
  markAllRead,
  subscribeSnapshot,
  type Snapshot,
} from "./backend";
import { useFixesPosting } from "./fixConflict";
import { derivePopover } from "./popoverView";
import PrList, {
  GlyphSvg,
  formatLastSync,
  useCollapsedSections,
} from "./PrList";
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
  const fixesPosting = useFixesPosting();

  // Recomputed each render: what to show is a pure view over the snapshot,
  // the filter, the collapse state, the Copilot fixes being asked for and the
  // time.
  // Nothing ticks: the waiting labels are as fresh as the last render, and
  // every sync causes one.
  const { offerMarkAllRead, content } = derivePopover(
    snapshot,
    filter,
    collapsed,
    Date.now(),
    fixesPosting,
  );

  useEffect(() => {
    startUpdateChecks();
    return subscribeSnapshot(setSnapshot);
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
          {inSettings ? (
            <button
              type="button"
              className="settings-link"
              onClick={() => setView("list")}
            >
              Done
            </button>
          ) : (
            <>
              {/* Always in the header, so the buttons never shift: the dot
                  says there is something to mark, and without it the button
                  has nothing to do. */}
              <button
                type="button"
                className="header-icon pr-tip"
                data-tip="Mark all read"
                aria-label="Mark all read"
                disabled={!offerMarkAllRead}
                onClick={() => markAllRead().catch(() => {})}
              >
                <GlyphSvg strokeWidth={2} size={16}>
                  <path d="m2 13 4 4L15 7" />
                  <path d="m11 16 1.5 1.5L22 7" />
                </GlyphSvg>
                {offerMarkAllRead && (
                  <span className="header-dot" aria-hidden="true" />
                )}
              </button>
              <button
                type="button"
                className="header-icon pr-tip"
                data-tip="Settings"
                aria-label="Settings"
                onClick={() => setView("settings")}
              >
                <GlyphSvg strokeWidth={2} size={16}>
                  <circle cx="12" cy="12" r="3" />
                  <path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" />
                </GlyphSvg>
              </button>
            </>
          )}
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
