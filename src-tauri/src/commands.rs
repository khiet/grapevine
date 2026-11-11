use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{github, keychain, pr_state, settings, store::Store, sync};

/// Settings changes should show up in the PR list right away, not after the
/// remainder of the current poll interval.
fn request_sync(app: &AppHandle) {
    app.state::<sync::SyncState>().wake.notify_one();
}

#[tauri::command]
pub fn get_prs(state: tauri::State<'_, sync::SyncState>) -> pr_state::Snapshot {
    state.snapshot()
}

#[tauri::command]
pub fn mark_read(app: AppHandle, key: String) -> Result<(), String> {
    sync::transition(&app, |prs| prs.mark_read(&key))
}

#[tauri::command]
pub fn mark_all_read(app: AppHandle) -> Result<(), String> {
    sync::transition(&app, |prs| prs.mark_all_read())
}

#[tauri::command]
pub fn dismiss_merged(app: AppHandle, key: String) -> Result<(), String> {
    sync::transition(&app, |prs| prs.dismiss_merged(&key))
}

#[tauri::command]
pub fn clear_merged(app: AppHandle) -> Result<(), String> {
    sync::transition(&app, |prs| prs.clear_merged())
}

/// `expires_at` is epoch ms and comes from the webview, which knows the local
/// time zone: the next local 08:00.
#[tauri::command]
pub fn snooze(app: AppHandle, key: String, expires_at: u64) -> Result<(), String> {
    sync::transition(&app, |prs| {
        prs.snooze(&key, sync::now_epoch_ms(), expires_at)
    })
}

#[tauri::command]
pub fn wake(app: AppHandle, key: String) -> Result<(), String> {
    sync::transition(&app, |prs| prs.wake(&key))
}

/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Serialize)]
pub struct TokenStatus {
    pub has_token: bool,
    pub login: Option<String>,
}

#[tauri::command]
pub async fn token_status(app: AppHandle) -> Result<TokenStatus, String> {
    let has_token = keychain::load()?.is_some();
    let login = if has_token {
        settings::load(&app.state::<Store>())?.github_login
    } else {
        None
    };
    Ok(TokenStatus { has_token, login })
}

/// Wire type: `src/backend/index.ts` holds the hand-written TypeScript copy.
/// Change both together.
#[derive(Serialize)]
pub struct SavedToken {
    pub login: String,
    /// Advisory only: the token is already saved when this arrives, so a
    /// `public_repo` token (which we recommend for public-only watching)
    /// still works.
    pub scope_warning: Option<String>,
}

#[tauri::command]
pub async fn save_token(app: AppHandle, token: String) -> Result<SavedToken, String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("Paste a token first.".into());
    }
    let validated = github::validate_token(&token).await?;
    keychain::store(&token)?;
    settings::update(&app.state::<Store>(), |settings| {
        settings.github_login = Some(validated.login.clone());
        Ok(())
    })?;
    request_sync(&app);
    Ok(SavedToken {
        login: validated.login,
        scope_warning: validated.scope_warning,
    })
}

#[tauri::command]
pub async fn clear_token(app: AppHandle) -> Result<(), String> {
    keychain::clear()?;
    settings::update(&app.state::<Store>(), |settings| {
        settings.github_login = None;
        Ok(())
    })?;
    request_sync(&app);
    Ok(())
}

#[tauri::command]
pub async fn list_repos(app: AppHandle) -> Result<Vec<String>, String> {
    Ok(settings::load(&app.state::<Store>())?.repos)
}

#[tauri::command]
pub async fn add_repo(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let (owner, repo) = parse_repo_name(name.trim())?;
    let Some(token) = keychain::load()? else {
        return Err("Save a valid GitHub token first.".into());
    };
    let canonical = github::validate_repo(&token, owner, repo).await?;
    let updated = settings::update(&app.state::<Store>(), |settings| {
        // Checked inside the lock: a concurrent duplicate add must lose here,
        // not slip past a check made against a stale read.
        if settings
            .repos
            .iter()
            .any(|r| r.eq_ignore_ascii_case(&canonical))
        {
            return Err(format!("{canonical} is already in the list."));
        }
        settings.repos.push(canonical.clone());
        Ok(())
    })?;
    request_sync(&app);
    Ok(updated.repos)
}

#[tauri::command]
pub async fn remove_repo(app: AppHandle, name: String) -> Result<Vec<String>, String> {
    let updated = settings::update(&app.state::<Store>(), |settings| {
        settings.repos.retain(|r| !r.eq_ignore_ascii_case(&name));
        Ok(())
    })?;
    request_sync(&app);
    Ok(updated.repos)
}

/// Every repo the settings browse list can offer: owned + org-member,
/// archived excluded. Read-only, so no sync wake.
#[tauri::command]
pub async fn list_affiliated_repos() -> Result<Vec<String>, String> {
    let Some(token) = keychain::load()? else {
        return Err("Save a valid GitHub token first.".into());
    };
    Ok(github::fetch_affiliated_repos(&token).await?)
}

#[tauri::command]
pub async fn get_poll_interval(app: AppHandle) -> Result<u64, String> {
    Ok(settings::load(&app.state::<Store>())?.poll_interval_secs)
}

/// Returns the value actually stored, which may have been clamped.
#[tauri::command]
pub async fn set_poll_interval(app: AppHandle, secs: u64) -> Result<u64, String> {
    let secs = settings::clamp_poll_secs(secs);
    settings::update(&app.state::<Store>(), |settings| {
        settings.poll_interval_secs = secs;
        Ok(())
    })?;
    request_sync(&app);
    Ok(secs)
}

#[tauri::command]
pub fn get_launch_at_login() -> Result<bool, String> {
    let status = unsafe { objc2_service_management::SMAppService::mainAppService().status() };
    Ok(login_item_reports_on(status))
}

/// The settings toggle reflects the app's intent, not raw system state:
/// RequiresApproval means registered but pending the user's consent in
/// System Settings, so it still reports as on.
fn login_item_reports_on(status: objc2_service_management::SMAppServiceStatus) -> bool {
    use objc2_service_management::SMAppServiceStatus;
    status == SMAppServiceStatus::Enabled || status == SMAppServiceStatus::RequiresApproval
}

#[tauri::command]
pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    use objc2_service_management::SMAppService;

    let service = unsafe { SMAppService::mainAppService() };
    let result = if enabled {
        unsafe { service.registerAndReturnError() }
    } else {
        unsafe { service.unregisterAndReturnError() }
    };
    result.map_err(|e| {
        format!(
            "Could not update the login item: {}",
            e.localizedDescription()
        )
    })
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
    use super::{login_item_reports_on, parse_repo_name};
    use objc2_service_management::SMAppServiceStatus;

    #[test]
    fn an_enabled_login_item_reports_on() {
        assert!(login_item_reports_on(SMAppServiceStatus::Enabled));
    }

    /// A fresh registration sits in RequiresApproval until the user consents
    /// in System Settings; reporting it as off would make the toggle appear
    /// to bounce back right after being switched on.
    #[test]
    fn a_login_item_awaiting_user_approval_still_reports_on() {
        assert!(login_item_reports_on(SMAppServiceStatus::RequiresApproval));
    }

    #[test]
    fn an_unregistered_login_item_reports_off() {
        assert!(!login_item_reports_on(SMAppServiceStatus::NotRegistered));
        assert!(!login_item_reports_on(SMAppServiceStatus::NotFound));
    }

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
