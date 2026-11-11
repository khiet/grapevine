// The demo's popover: the real App on the fake backend, so the capture shows
// exactly what the frontend renders for the fixture Snapshot.
import React from "react";
import ReactDOM from "react-dom/client";
import App from "../../src/App";
import { setAdapter } from "../../src/backend";
import { createFakeBackend } from "../../src/backend/fake";
import { totalUnread } from "../../src/PrList";
import { demoSnapshot } from "./fixture";

declare global {
  interface Window {
    /** Login to data URI, injected by demo.mjs before this module runs. */
    DEMO_AVATARS: Record<string, string>;
  }
}

// The stage's tray count mirrors the unread total the way the Rust side sets
// the tray title.
const showTrayCount = (snapshot: { prs: Parameters<typeof totalUnread>[0] }) => {
  const count = parent.document.getElementById("tray-count");
  if (count) count.textContent = String(totalUnread(snapshot.prs) || "");
};

const snapshot = demoSnapshot(window.DEMO_AVATARS);
setAdapter(createFakeBackend(snapshot, showTrayCount).adapter);
showTrayCount(snapshot);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
