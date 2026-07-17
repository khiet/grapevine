use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// Bounds for the poll interval. The floor keeps a mistyped value from
/// hammering GitHub; the ceiling keeps the app from looking dead.
pub const MIN_POLL_SECS: u64 = 30;
pub const MAX_POLL_SECS: u64 = 3600;
pub const DEFAULT_POLL_SECS: u64 = 180;

fn default_poll_secs() -> u64 {
    DEFAULT_POLL_SECS
}

/// App state persisted to `settings.json` in the app data directory. The
/// GitHub token is deliberately absent: it lives only in the macOS Keychain.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub repos: Vec<String>,
    pub github_login: Option<String>,
    /// Seconds between syncs. Persisted values outside the bounds are
    /// clamped on read, so a hand-edited file cannot break the loop.
    #[serde(default = "default_poll_secs")]
    pub poll_interval_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            repos: Vec::new(),
            github_login: None,
            poll_interval_secs: DEFAULT_POLL_SECS,
        }
    }
}

pub fn clamp_poll_secs(secs: u64) -> u64 {
    secs.clamp(MIN_POLL_SECS, MAX_POLL_SECS)
}

fn settings_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("cannot resolve app data dir: {e}"))?;
    Ok(dir.join("settings.json"))
}

pub fn load(app: &tauri::AppHandle) -> Result<Settings, String> {
    let path = settings_path(app)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("cannot read settings: {e}"))?;
    let mut settings: Settings =
        serde_json::from_str(&raw).map_err(|e| format!("settings file is corrupt: {e}"))?;
    settings.poll_interval_secs = clamp_poll_secs(settings.poll_interval_secs);
    Ok(settings)
}

pub fn save(app: &tauri::AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("cannot create app data dir: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("cannot write settings: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings files written before the poll interval existed have no such
    /// field; they must load with the default rather than fail or read zero.
    #[test]
    fn a_settings_file_without_a_poll_interval_gets_the_default() {
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.poll_interval_secs, DEFAULT_POLL_SECS);
    }

    #[test]
    fn poll_intervals_clamp_to_the_allowed_range() {
        assert_eq!(clamp_poll_secs(0), MIN_POLL_SECS);
        assert_eq!(clamp_poll_secs(29), MIN_POLL_SECS);
        assert_eq!(clamp_poll_secs(30), 30);
        assert_eq!(clamp_poll_secs(300), 300);
        assert_eq!(clamp_poll_secs(3600), 3600);
        assert_eq!(clamp_poll_secs(1_000_000), MAX_POLL_SECS);
    }
}
