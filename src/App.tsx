import { useState } from "react";
import "./App.css";
import SettingsView from "./SettingsView";

function App() {
  const [view, setView] = useState<"list" | "settings">("list");
  const inSettings = view === "settings";

  return (
    <div className="popover">
      <header className="popover-header">
        <span className="popover-title">
          {inSettings ? "Settings" : "Grapevine"}
        </span>
        <button
          type="button"
          className="settings-link"
          onClick={() => setView(inSettings ? "list" : "settings")}
        >
          {inSettings ? "Done" : "Settings"}
        </button>
      </header>
      {inSettings ? (
        <SettingsView />
      ) : (
        <main className="popover-body">
          <p className="placeholder-heading">No pull requests yet</p>
          <p className="placeholder-detail">
            Configure a token and repositories to start watching.
          </p>
        </main>
      )}
    </div>
  );
}

export default App;
