use std::{collections::HashSet, sync::Mutex, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

use crate::{github, github::PullRequest, keychain, merged, settings, unread};

const POLL_INTERVAL: Duration = Duration::from_secs(180);

/// Latest sync result. A failed sync leaves the previous snapshot in place
/// so the popover keeps showing the last known list instead of blanking.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Snapshot {
    pub prs: Vec<PullRequest>,
    /// Merged-but-not-dismissed PRs, rendered as the Merged section.
    pub merged: Vec<merged::MergedPr>,
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
    /// Merged-but-not-dismissed PRs; authoritative in memory, mirrored to
    /// disk on every change so the Merged section survives restarts.
    pub merged: Mutex<merged::MergedState>,
    /// Wakes the poll loop early, e.g. right after settings change.
    pub wake: Notify,
}

/// Spawns the background loop: sync now, then every [`POLL_INTERVAL`] or
/// whenever [`SyncState::wake`] is notified, whichever comes first.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    *app.state::<SyncState>().read_state.lock().unwrap() = unread::load(&app);
    {
        // Seed the initial snapshot too, so the Merged section is visible
        // before the first sync completes.
        let loaded = merged::load(&app);
        let state = app.state::<SyncState>();
        state.snapshot.lock().unwrap().merged = loaded.clone();
        *state.merged.lock().unwrap() = loaded;
    }
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

/// The tray title for `total`, empty at zero to hide the badge.
///
/// Empty rather than `None`: tray-icon's macOS `set_title` skips the button
/// entirely on `None` (no else branch), so passing it leaves the previous
/// count sitting on the icon and a cleared badge never disappears.
fn tray_title(total: u64) -> String {
    if total > 0 {
        total.to_string()
    } else {
        String::new()
    }
}

/// Shows the total unread count next to the tray icon, or nothing at zero so
/// the icon returns to its plain state.
pub fn update_tray_count(app: &AppHandle, total: u64) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_title(Some(tray_title(total)));
    }
}

async fn sync_once(app: &AppHandle) {
    let result = fetch(app).await;
    let state = app.state::<SyncState>();
    let new_snapshot = match result {
        Ok(Some(FetchResult {
            mut prs,
            newly_merged,
        })) => {
            let merged = {
                let mut merged_state = state.merged.lock().unwrap();
                if merged::apply(&prs, newly_merged, &mut merged_state) {
                    if let Err(e) = merged::save(app, &merged_state) {
                        eprintln!("cannot persist merged state: {e}");
                    }
                }
                merged_state.clone()
            };
            let mut read_state = state.read_state.lock().unwrap();
            if unread::apply(&mut prs, &mut read_state) {
                if let Err(e) = unread::save(app, &read_state) {
                    eprintln!("cannot persist unread state: {e}");
                }
            }
            Snapshot {
                prs,
                merged,
                has_synced: true,
            }
        }
        // Unconfigured: clear any list left over from previous settings,
        // but keep the read watermarks and merged history for when a token
        // comes back.
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

/// One sync's fetch: the open PRs plus any tracked PRs that turned out to
/// have merged since the previous sync.
struct FetchResult {
    prs: Vec<PullRequest>,
    newly_merged: Vec<merged::MergedPr>,
}

/// `Ok(None)` means the app is not configured (no token or no repos).
async fn fetch(app: &AppHandle) -> Result<Option<FetchResult>, String> {
    let Some(token) = keychain::load()? else {
        return Ok(None);
    };
    let repos = settings::load(app)?.repos;
    if repos.is_empty() {
        return Ok(None);
    }
    let prs = github::fetch_open_prs(&token, &repos).await?;
    let newly_merged = detect_merged(app, &token, &repos, &prs).await?;
    Ok(Some(FetchResult { prs, newly_merged }))
}

/// Finds tracked PRs that left the open set since the previous sync and asks
/// GitHub which of them merged; PRs closed without merging are simply absent
/// from the answer. A failed status query fails the whole sync, keeping the
/// old snapshot so the departure is re-detected next round — a network blip
/// never silently drops a merge. Merges that happen while the app is quit
/// are missed entirely: the previous open set lives only in memory.
async fn detect_merged(
    app: &AppHandle,
    token: &str,
    repos: &[String],
    live_prs: &[PullRequest],
) -> Result<Vec<merged::MergedPr>, String> {
    let live: HashSet<String> = live_prs.iter().map(unread::key).collect();
    let departed: Vec<PullRequest> = {
        let state = app.state::<SyncState>();
        let snapshot = state.snapshot.lock().unwrap();
        snapshot
            .prs
            .iter()
            .filter(|pr| !live.contains(&unread::key(pr)))
            // A repo removed from the watchlist takes its PRs with it; those
            // departed because of settings, not because they merged.
            .filter(|pr| repos.iter().any(|r| r.eq_ignore_ascii_case(&pr.repo)))
            .cloned()
            .collect()
    };
    if departed.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = departed.iter().map(unread::key).collect();
    let merged_at = github::fetch_merged_status(token, &keys).await?;
    Ok(departed
        .iter()
        .filter_map(|pr| {
            merged_at
                .get(&unread::key(pr))
                .map(|t| merged::from_pr(pr, t.clone()))
        })
        .collect())
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
                avatar_url: "https://avatars.example/someone".into(),
                created_at: "2026-07-10T12:00:00Z".into(),
                updated_at: "2026-07-11T09:30:00Z".into(),
                section: Section::Mine,
                unread_count: 3,
                // Internal working data; must not leak into the payload.
                activity: vec!["2026-07-10T12:00:00Z".into()],
            }],
            merged: vec![merged::MergedPr {
                number: 3,
                title: "Ship the other thing".into(),
                url: "https://github.com/acme/widgets/pull/3".into(),
                repo: "acme/widgets".into(),
                author: "someone".into(),
                avatar_url: "https://avatars.example/someone".into(),
                merged_at: "2026-07-11T10:00:00Z".into(),
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
                    "avatar_url": "https://avatars.example/someone",
                    "created_at": "2026-07-10T12:00:00Z",
                    "updated_at": "2026-07-11T09:30:00Z",
                    "section": "mine",
                    "unread_count": 3
                }],
                "merged": [{
                    "number": 3,
                    "title": "Ship the other thing",
                    "url": "https://github.com/acme/widgets/pull/3",
                    "repo": "acme/widgets",
                    "author": "someone",
                    "avatar_url": "https://avatars.example/someone",
                    "merged_at": "2026-07-11T10:00:00Z"
                }],
                "has_synced": true
            })
        );
    }

    /// Zero must produce an empty title, never `None`: tray-icon's macOS
    /// `set_title` ignores `None`, which would strand the last count on the
    /// icon after the badges clear.
    #[test]
    fn a_zero_total_clears_the_tray_title_instead_of_leaving_it() {
        assert_eq!(tray_title(0), "");
        assert_eq!(tray_title(1), "1");
        assert_eq!(tray_title(42), "42");
    }

    #[test]
    fn the_tray_total_sums_unread_across_prs() {
        let pr = |unread_count: u64| PullRequest {
            number: 7,
            title: "Fix the thing".into(),
            url: "https://github.com/acme/widgets/pull/7".into(),
            repo: "acme/widgets".into(),
            author: "someone".into(),
            avatar_url: String::new(),
            created_at: "2026-07-10T12:00:00Z".into(),
            updated_at: "2026-07-11T09:30:00Z".into(),
            section: Section::All,
            unread_count,
            activity: vec![],
        };
        assert_eq!(total_unread(&[]), 0);
        assert_eq!(total_unread(&[pr(2), pr(0), pr(5)]), 7);
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
