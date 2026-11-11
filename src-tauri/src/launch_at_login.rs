//! Whether Grapevine starts with the user's session. Each platform provides
//! the same two operations so the commands, and the settings toggle behind
//! them, do not vary by platform.

pub use platform::{is_enabled, set_enabled};

#[cfg(target_os = "macos")]
mod platform {
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn is_enabled() -> Result<bool, String> {
        let status = unsafe { SMAppService::mainAppService().status() };
        Ok(login_item_reports_on(status))
    }

    /// The settings toggle reflects the app's intent, not raw system state:
    /// RequiresApproval means registered but pending the user's consent in
    /// System Settings, so it still reports as on.
    fn login_item_reports_on(status: SMAppServiceStatus) -> bool {
        status == SMAppServiceStatus::Enabled || status == SMAppServiceStatus::RequiresApproval
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_enabled_login_item_reports_on() {
            assert!(login_item_reports_on(SMAppServiceStatus::Enabled));
        }

        /// A fresh registration sits in RequiresApproval until the user
        /// consents in System Settings; reporting it as off would make the
        /// toggle appear to bounce back right after being switched on.
        #[test]
        fn a_login_item_awaiting_user_approval_still_reports_on() {
            assert!(login_item_reports_on(SMAppServiceStatus::RequiresApproval));
        }

        #[test]
        fn an_unregistered_login_item_reports_off() {
            assert!(!login_item_reports_on(SMAppServiceStatus::NotRegistered));
            assert!(!login_item_reports_on(SMAppServiceStatus::NotFound));
        }
    }
}

/// Not implemented off macOS yet: nothing is ever registered, so the setting
/// reads as off and cannot be turned on.
#[cfg(not(target_os = "macos"))]
mod platform {
    pub fn is_enabled() -> Result<bool, String> {
        Ok(false)
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        if enabled {
            return Err("Launch at login is not supported on Linux yet.".into());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn launch_at_login_reads_as_off() {
            assert_eq!(is_enabled(), Ok(false));
        }

        #[test]
        fn turning_launch_at_login_on_says_it_is_not_supported_yet() {
            let error = set_enabled(true).unwrap_err();
            assert!(error.contains("not supported"), "{error}");
        }

        /// The settings toggle rolls back by turning the setting off again;
        /// that must not raise a second error over the first.
        #[test]
        fn turning_launch_at_login_off_succeeds() {
            assert_eq!(set_enabled(false), Ok(()));
        }
    }
}
