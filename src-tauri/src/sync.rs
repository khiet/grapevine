use std::{sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{github, github::PullRequest, keychain, settings, unread};

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
    /// Last-read watermarks; authoritative in memory, mirrored to disk on
    /// every change so unread state survives restarts.
    pub read_state: Mutex<unread::ReadState>,
    /// Wakes the poll loop early, e.g. right after settings change.
    pub wake: Notify,
}

/// Spawns the background loop: sync now, then every [`POLL_INTERVAL`] or
/// whenever [`SyncState::wake`] is notified, whichever comes first.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    *app.state::<SyncState>().read_state.lock().unwrap() = unread::load(&app);
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

pub fn total_unread(prs: &[PullRequest]) -> u64 {
    prs.iter().map(|pr| pr.unread_count).sum()
}

/// Shows the total unread count next to the tray icon, or none at zero so
/// the icon returns to its plain state.
pub fn update_tray_count(app: &AppHandle, total: u64) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_title((total > 0).then(|| total.to_string()));
    }
}

async fn sync_once(app: &AppHandle) {
    let result = fetch(app).await;
    let state = app.state::<SyncState>();
    let new_snapshot = match result {
        Ok(Some(mut prs)) => {
            let mut read_state = state.read_state.lock().unwrap();
            if unread::apply(&mut prs, &mut read_state) {
                if let Err(e) = unread::save(app, &read_state) {
                    eprintln!("cannot persist unread state: {e}");
                }
            }
            Snapshot {
                prs,
                has_synced: true,
            }
        }
        // Unconfigured: clear any list left over from previous settings,
        // but keep the read watermarks for when a token comes back.
        Ok(None) => Snapshot::default(),
        Err(e) => {
            eprintln!("sync failed: {e}");
            return;
        }
    };
    let snapshot = {
        let mut snapshot = state.snapshot.lock().unwrap();
        *snapshot = new_snapshot;
        snapshot.clone()
    };
    update_tray_count(app, total_unread(&snapshot.prs));
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
                unread_count: 3,
                // Internal working data; must not leak into the payload.
                activity: vec!["2026-07-10T12:00:00Z".into()],
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
                    "section": "mine",
                    "unread_count": 3
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
