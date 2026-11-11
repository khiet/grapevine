// The demo's popover: the real App on the fake backend, so the capture shows
// exactly what the frontend renders for the fixture Snapshot.
import React from "react";
import ReactDOM from "react-dom/client";
import App from "../../src/App";
import { setAdapter, type Snapshot } from "../../src/backend";
import { createFakeBackend } from "../../src/backend/fake";
import { totalUnread } from "../../src/PrList";
import { demoSnapshot } from "./fixture";

// The stage's tray count mirrors the unread total the way the Rust side sets
// the tray title.
const showTrayCount = (snapshot: Snapshot) => {
  const count = parent.document.getElementById("tray-count");
  if (count) count.textContent = String(totalUnread(snapshot.prs) || "");
};

// Login to data URI, injected by demo.mjs before this module runs.
const avatars = (window as unknown as { DEMO_AVATARS: Record<string, string> })
  .DEMO_AVATARS;
const snapshot = demoSnapshot(avatars);
setAdapter(createFakeBackend(snapshot, showTrayCount).adapter);
showTrayCount(snapshot);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
