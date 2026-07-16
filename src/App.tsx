import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import PrList, { Snapshot, totalUnread } from "./PrList";
import SettingsView from "./SettingsView";

function App() {
  const [view, setView] = useState<"list" | "settings">("list");
  const [snapshot, setSnapshot] = useState<Snapshot>({
    prs: [],
    merged: [],
    has_synced: false,
  });
  const inSettings = view === "settings";

  useEffect(() => {
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
        <span className="popover-title">
          {inSettings ? "Settings" : "Grapevine"}
        </span>
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
      {inSettings ? (
        <SettingsView />
      ) : snapshot.prs.length > 0 || snapshot.merged.length > 0 ? (
        <PrList prs={snapshot.prs} merged={snapshot.merged} />
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
    </div>
  );
}

export default App;
