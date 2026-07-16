use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{github, keychain, settings, sync, unread};

/// Settings changes should show up in the PR list right away, not after the
/// remainder of the current poll interval.
fn request_sync(app: &AppHandle) {
    app.state::<sync::SyncState>().wake.notify_one();
}

#[tauri::command]
pub fn get_prs(state: tauri::State<'_, sync::SyncState>) -> sync::Snapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
pub fn mark_read(app: AppHandle, key: String) -> Result<(), String> {
    mark(&app, Some(&key))
}

#[tauri::command]
pub fn mark_all_read(app: AppHandle) -> Result<(), String> {
    mark(&app, None)
}

/// Moves the read watermark of the matching PR (or all PRs, for `None`) to
/// its newest known activity, then pushes the cleared badges to the popover
/// and the tray. Marking read is anchored to activity the app has actually
/// seen: anything that lands on GitHub after this sync stays unread.
fn mark(app: &AppHandle, only: Option<&str>) -> Result<(), String> {
    let state = app.state::<sync::SyncState>();
    let snapshot = {
        let mut read_state = state.read_state.lock().unwrap();
        let mut snapshot = state.snapshot.lock().unwrap();
        for pr in &mut snapshot.prs {
            let key = unread::key(pr);
            if only.is_none_or(|k| k == key) {
                pr.unread_count = 0;
                read_state.insert(key, unread::read_watermark(pr));
            }
        }
        unread::save(app, &read_state)?;
        snapshot.clone()
    };
    sync::update_tray_count(app, sync::total_unread(&snapshot.prs));
    app.emit("prs-updated", snapshot).map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct TokenStatus {
    pub has_token: bool,
    pub login: Option<String>,
}

#[tauri::command]
pub async fn token_status(app: AppHandle) -> Result<TokenStatus, String> {
    let has_token = keychain::load()?.is_some();
    let login = if has_token {
        settings::load(&app)?.github_login
    } else {
        None
    };
    Ok(TokenStatus { has_token, login })
}

#[tauri::command]
pub async fn save_token(app: AppHandle, token: String) -> Result<String, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Paste a token first.".into());
    }
    let login = github::validate_token(&token).await?;
    keychain::store(&token)?;
    let mut current = settings::load(&app)?;
    current.github_login = Some(login.clone());
    settings::save(&app, &current)?;
    request_sync(&app);
    Ok(login)
}

#[tauri::command]
pub async fn clear_token(app: AppHandle) -> Result<(), String> {
    keychain::clear()?;
    let mut current = settings::load(&app)?;
    current.github_login = None;
    settings::save(&app, &current)?;
    request_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_repos(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(settings::load(&app)?.repos)
}

#[tauri::command]
pub async fn add_repo(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let (owner, repo) = parse_repo_name(name.trim())?;
    let Some(token) = keychain::load()? else {
        return Err("Save a valid GitHub token first.".into());
    };
    let canonical = github::validate_repo(&token, owner, repo).await?;
    let mut current = settings::load(&app)?;
    if current
        .repos
        .iter()
        .any(|r| r.eq_ignore_ascii_case(&canonical))
    {
        return Err(format!("{canonical} is already in the list."));
    }
    current.repos.push(canonical);
    settings::save(&app, &current)?;
    request_sync(&app);
    Ok(current.repos)
}

#[tauri::command]
pub async fn remove_repo(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let mut current = settings::load(&app)?;
    current.repos.retain(|r| !r.eq_ignore_ascii_case(&name));
    settings::save(&app, &current)?;
    request_sync(&app);
    Ok(current.repos)
}

fn parse_repo_name(input: &str) -> Result<(&str, &str), String> {
    let invalid = || "Use the owner/repo format, e.g. rails/rails.".to_string();
    let (owner, repo) = input.split_once('/').ok_or_else(invalid)?;
    if owner.is_empty()
        || repo.is_empty()
        || repo.contains('/')
        || input.contains(char::is_whitespace)
    {
        return Err(invalid());
    }
    Ok((owner, repo))
}

#[cfg(test)]
mod tests {
    use super::parse_repo_name;

    #[test]
    fn splits_owner_and_repo() {
        assert_eq!(
            parse_repo_name("tauri-apps/tauri"),
            Ok(("tauri-apps", "tauri"))
        );
    }

    #[test]
    fn rejects_input_without_a_separator() {
        assert!(parse_repo_name("rails").is_err());
    }

    #[test]
    fn rejects_empty_owner_or_repo() {
        assert!(parse_repo_name("/rails").is_err());
        assert!(parse_repo_name("rails/").is_err());
    }

    #[test]
    fn rejects_a_url_rather_than_treating_the_path_as_a_repo() {
        assert!(parse_repo_name("https://github.com/rails/rails").is_err());
    }

    #[test]
    fn rejects_embedded_whitespace() {
        assert!(parse_repo_name("rails / rails").is_err());
    }
}
