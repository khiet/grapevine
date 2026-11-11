import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Adapter } from "./index";

/** The real backend: Tauri IPC to the Rust side. */
export const tauriAdapter: Adapter = {
  invoke,
  listen: (event, handler) => listen(event, (e) => handler(e.payload as never)),
  openUrl: (url) => openUrl(url),
  appVersion: getVersion,
};
