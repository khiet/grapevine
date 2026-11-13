import { useEffect, useState } from "react";
import "./App.css";
import {
  EMPTY_SNAPSHOT,
  markAllRead,
  subscribeSnapshot,
  syncNow,
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

type View = "list" | "settings";

/** The header's buttons: Done inside Settings, otherwise the two icon
 * buttons. `onShow` is handed the view a click asks for. */
export function HeaderActions({
  inSettings,
  offerMarkAllRead,
  onShow,
}: {
  inSettings: boolean;
  offerMarkAllRead: boolean;
  onShow: (view: View) => void;
}) {
  if (inSettings) {
    return (
      <button
        type="button"
        className="settings-link"
        onClick={() => onShow("list")}
      >
        Done
      </button>
    );
  }
  return (
    <>
      {/* Always in the header, so the buttons never shift: the dot says
          there is something to mark, and without it the button has nothing
          to do. */}
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
        {offerMarkAllRead && <span className="header-dot" aria-hidden="true" />}
      </button>
      <button
        type="button"
        className="header-icon pr-tip"
        data-tip="Settings"
        aria-label="Settings"
        onClick={() => onShow("settings")}
      >
        <GlyphSvg strokeWidth={2} size={16}>
          {/* A toothed gear, not a ring of rays: rays read as a sun, which
              the list already uses for Wake. */}
          <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
          <circle cx="12" cy="12" r="3" />
        </GlyphSvg>
      </button>
    </>
  );
}

/** The footer as a status bar: the sync time and Sync now on the right, and a
 * sync error with its Retry on the left, which is otherwise left free. Both
 * buttons ask for the same sync. */
export function SyncStatus({
  lastSyncAt,
  error,
  syncing,
}: {
  lastSyncAt: number | null;
  error: string | null;
  syncing: boolean;
}) {
  // aria-disabled, not disabled: a disabled button drops keyboard focus, and
  // the one that started the sync should still hold it when the sync ends.
  // The backend ignores a press it should not act on, so nothing is guarded
  // here.
  const sync = () => syncNow().catch(() => {});
  const busy = syncing ? true : undefined;
  const label = syncing ? "Syncing" : "Sync now";
  return (
    <footer className="sync-status">
      {error && (
        <>
          {/* title carries the full message; the footer line clips it. */}
          <span className="sync-error" title={error}>
            {error}
          </span>
          <button
            type="button"
            className="sync-retry"
            aria-disabled={busy}
            onClick={sync}
          >
            Retry
          </button>
        </>
      )}
      <span className="sync-right">
        <span className="sync-time" aria-live="polite">
          {syncing ? "Syncing" : formatLastSync(lastSyncAt)}
        </span>
        <button
          type="button"
          className="sync-now pr-tip"
          data-tip={label}
          aria-label={label}
          aria-disabled={busy}
          onClick={sync}
        >
          <GlyphSvg strokeWidth={2} size={14}>
            <path d="M21 12a9 9 0 0 0-15.5-6.2L3 8" />
            <path d="M3 3v5h5" />
            <path d="M3 12a9 9 0 0 0 15.5 6.2L21 16" />
            <path d="M16 16h5v5" />
          </GlyphSvg>
        </button>
      </span>
    </footer>
  );
}

function App() {
  const [view, setView] = useState<View>("list");
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
          <HeaderActions
            inSettings={inSettings}
            offerMarkAllRead={offerMarkAllRead}
            onShow={setView}
          />
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
        <SyncStatus
          lastSyncAt={snapshot.last_sync_at}
          error={snapshot.sync_error}
          syncing={snapshot.syncing}
        />
      )}
    </div>
  );
}

export default App;
