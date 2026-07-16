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
            Ok(Some(prs)) => {
                *snapshot = Snapshot {
                    prs,
                    has_synced: true,
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::Section;
    use serde_json::json;

    /// The `Snapshot` interface in `src/PrList.tsx` consumes this JSON
    /// verbatim: field names and section values are the wire contract, so a
    /// rename or a `rename_all` attribute here silently breaks the popover.
    #[test]
    fn snapshot_serializes_to_the_shape_the_frontend_expects() {
        let snapshot = Snapshot {
            prs: vec![PullRequest {
                number: 7,
                title: "Fix the thing".into(),
                url: "https://github.com/acme/widgets/pull/7".into(),
                repo: "acme/widgets".into(),
                author: "someone".into(),
                created_at: "2026-07-10T12:00:00Z".into(),
                section: Section::Mine,
            }],
            has_synced: true,
        };
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap(),
            json!({
                "prs": [{
                    "number": 7,
                    "title": "Fix the thing",
                    "url": "https://github.com/acme/widgets/pull/7",
                    "repo": "acme/widgets",
                    "author": "someone",
                    "created_at": "2026-07-10T12:00:00Z",
                    "section": "mine"
                }],
                "has_synced": true
            })
        );
    }

    /// `PrList.tsx` filters rows with `pr.section === key`; a mismatched
    /// value drops PRs from the popover without any error.
    #[test]
    fn every_section_variant_serializes_to_its_frontend_key() {
        for (section, expected) in [
            (Section::Mine, "mine"),
            (Section::Participated, "participated"),
            (Section::All, "all"),
        ] {
            assert_eq!(serde_json::to_value(section).unwrap(), json!(expected));
        }
    }
}
