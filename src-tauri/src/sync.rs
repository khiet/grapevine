use std::{sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{github, github::PullRequest, keychain, settings};

const POLL_INTERVAL: Duration = Duration::from_secs(180);

/// Latest sync result. A failed sync leaves the previous snapshot in place
/// so the popover keeps showing the last known list instead of blanking.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Snapshot {
    pub prs: Vec<PullRequest>,
    /// False until a sync has completed while configured; the UI uses this
    /// to tell "not set up yet" apart from "no open PRs".
    pub has_synced: bool,
}

#[derive(Default)]
pub struct SyncState {
    pub snapshot: Mutex<Snapshot>,
    /// Wakes the poll loop early, e.g. right after settings change.
    pub wake: Notify,
}

/// Spawns the background loop: sync now, then every [`POLL_INTERVAL`] or
/// whenever [`SyncState::wake`] is notified, whichever comes first.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            sync_once(&app).await;
            let state = app.state::<SyncState>();
            tokio::select! {
                _ = tokio::time::sleep(POLL_INTERVAL) => {}
                _ = state.wake.notified() => {}
            }
        }
    });
}

async fn sync_once(app: &AppHandle) {
    let result = fetch(app).await;
    let state = app.state::<SyncState>();
    let snapshot = {
        let mut snapshot = state.snapshot.lock().unwrap();
        match result {
            Ok(Some(prs)) => *snapshot = Snapshot { prs, has_synced: true },
            // Unconfigured: clear any list left over from previous settings.
            Ok(None) => *snapshot = Snapshot::default(),
            Err(e) => {
                eprintln!("sync failed: {e}");
                return;
            }
        }
        snapshot.clone()
    };
    let _ = app.emit("prs-updated", snapshot);
}

/// `Ok(None)` means the app is not configured (no token or no repos).
async fn fetch(app: &AppHandle) -> Result<Option<Vec<PullRequest>>, String> {
    let Some(token) = keychain::load()? else {
        return Ok(None);
    };
    let repos = settings::load(app)?.repos;
    if repos.is_empty() {
        return Ok(None);
    }
    github::fetch_open_prs(&token, &repos).await.map(Some)
}
