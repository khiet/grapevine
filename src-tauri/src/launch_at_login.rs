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

/// An XDG autostart entry in the user's config directory: the setting is on
/// exactly when the entry exists.
#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};
    use std::{env, fs, io};

    pub fn is_enabled() -> Result<bool, String> {
        Autostart::for_this_user()?.is_enabled()
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let executable = env::current_exe()
            .map_err(|e| format!("Could not find the Grapevine executable: {e}"))?;
        Autostart::for_this_user()?.set_enabled(enabled, &executable)
    }

    /// The autostart entry file. Its existence is the setting: there is no
    /// second record of it to fall out of step with what the session does.
    struct Autostart {
        entry: PathBuf,
    }

    impl Autostart {
        fn for_this_user() -> Result<Self, String> {
            let config = config_dir(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))?;
            Ok(Self::in_config_dir(&config))
        }

        fn in_config_dir(config: &Path) -> Self {
            Self {
                entry: config.join("autostart").join("grapevine.desktop"),
            }
        }

        fn is_enabled(&self) -> Result<bool, String> {
            self.entry
                .try_exists()
                .map_err(|e| format!("Could not read the autostart entry: {e}"))
        }

        /// `executable` is what the session will start; it is only used when
        /// turning the setting on.
        fn set_enabled(&self, enabled: bool, executable: &Path) -> Result<(), String> {
            let result = if enabled {
                let contents = autostart_entry(executable)?;
                self.write(&contents)
            } else {
                match fs::remove_file(&self.entry) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
                    _ => Ok(()),
                }
            };
            result.map_err(|e| format!("Could not update the autostart entry: {e}"))
        }

        fn write(&self, contents: &str) -> io::Result<()> {
            if let Some(directory) = self.entry.parent() {
                fs::create_dir_all(directory)?;
            }
            fs::write(&self.entry, contents)
        }
    }

    /// The XDG config directory, from the values of `XDG_CONFIG_HOME` and
    /// `HOME`.
    fn config_dir(
        xdg_config_home: Option<OsString>,
        home: Option<OsString>,
    ) -> Result<PathBuf, String> {
        let chosen = xdg_config_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        match chosen {
            Some(path) => Ok(path),
            None => home
                .filter(|home| !home.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
                .ok_or_else(|| {
                    "Could not find your config directory: HOME is not set.".to_string()
                }),
        }
    }

    /// The XDG autostart entry that starts `executable` with the session.
    /// Pure, so the contents are tested without touching the filesystem.
    fn autostart_entry(executable: &Path) -> Result<String, String> {
        // Desktop entries are UTF-8; a path that is not cannot be written
        // faithfully, and a lossy one would point at the wrong file.
        let executable = executable.to_str().ok_or_else(|| {
            format!(
                "Could not write the autostart entry: {} is not valid UTF-8",
                executable.display()
            )
        })?;
        Ok(format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Grapevine\n\
             Exec={}\n\
             Terminal=false\n",
            exec_value(executable)
        ))
    }

    /// `executable` as the single argument of an `Exec` key. The Desktop
    /// Entry spec applies three layers, innermost first: quoting for the
    /// characters the launcher treats specially, `%%` for a literal percent
    /// sign, then the escapes every string value takes.
    fn exec_value(executable: &str) -> String {
        const RESERVED: &str = " \t\n\"'\\><~|&;$*?#()`";
        const ESCAPED_IN_QUOTES: &str = "\"`$\\";

        let argument = if executable.contains(|c| RESERVED.contains(c)) {
            let mut quoted = String::from('"');
            for c in executable.chars() {
                if ESCAPED_IN_QUOTES.contains(c) {
                    quoted.push('\\');
                }
                quoted.push(c);
            }
            quoted.push('"');
            quoted
        } else {
            executable.to_string()
        };

        argument
            .replace('%', "%%")
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\t', "\\t")
            .replace('\r', "\\r")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_autostart_entry_starts_the_installed_executable() {
            assert_eq!(
                autostart_entry(Path::new("/usr/bin/grapevine")).unwrap(),
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Grapevine\n\
                 Exec=/usr/bin/grapevine\n\
                 Terminal=false\n"
            );
        }

        fn exec_line(executable: &str) -> String {
            let entry = autostart_entry(Path::new(executable)).unwrap();
            let line = entry.lines().find(|line| line.starts_with("Exec="));
            line.unwrap().to_string()
        }

        /// Unquoted, the session would split the path at the space and try
        /// to run `/home/khiet/My`.
        #[test]
        fn an_executable_path_with_a_space_is_quoted() {
            assert_eq!(
                exec_line("/home/khiet/My Apps/grapevine"),
                r#"Exec="/home/khiet/My Apps/grapevine""#
            );
        }

        /// The worked example from the Desktop Entry spec: a literal dollar
        /// sign in a quoted argument is written `\\$`.
        #[test]
        fn a_dollar_sign_in_the_executable_path_is_escaped() {
            assert_eq!(
                exec_line("/opt/a$b/grapevine"),
                r#"Exec="/opt/a\\$b/grapevine""#
            );
        }

        /// A bare `%` starts a field code such as `%f`.
        #[test]
        fn a_percent_sign_in_the_executable_path_is_doubled() {
            assert_eq!(
                exec_line("/opt/100%/grapevine"),
                "Exec=/opt/100%%/grapevine"
            );
        }

        const EXECUTABLE: &str = "/usr/bin/grapevine";

        fn turn_on(autostart: &Autostart) {
            autostart.set_enabled(true, Path::new(EXECUTABLE)).unwrap();
        }

        #[test]
        fn launch_at_login_reads_as_off_until_it_is_turned_on() {
            let config = tempfile::tempdir().unwrap();
            let autostart = Autostart::in_config_dir(config.path());

            assert_eq!(autostart.is_enabled(), Ok(false));
        }

        /// The session only looks in `autostart/` under the config
        /// directory, which does not exist on a fresh account.
        #[test]
        fn turning_launch_at_login_on_writes_the_entry_where_the_session_looks() {
            let config = tempfile::tempdir().unwrap();
            let autostart = Autostart::in_config_dir(config.path());

            turn_on(&autostart);

            let written =
                std::fs::read_to_string(config.path().join("autostart/grapevine.desktop")).unwrap();
            assert!(written.contains("Exec=/usr/bin/grapevine\n"), "{written}");
            assert_eq!(autostart.is_enabled(), Ok(true));
        }

        #[test]
        fn turning_launch_at_login_off_removes_the_entry() {
            let config = tempfile::tempdir().unwrap();
            let autostart = Autostart::in_config_dir(config.path());
            turn_on(&autostart);

            autostart.set_enabled(false, Path::new(EXECUTABLE)).unwrap();

            assert!(!config.path().join("autostart/grapevine.desktop").exists());
            assert_eq!(autostart.is_enabled(), Ok(false));
        }

        /// Off is the state it is already in, so asking for it is not an
        /// error.
        #[test]
        fn turning_launch_at_login_off_when_it_is_already_off_succeeds() {
            let config = tempfile::tempdir().unwrap();
            let autostart = Autostart::in_config_dir(config.path());

            assert_eq!(autostart.set_enabled(false, Path::new(EXECUTABLE)), Ok(()));
        }

        /// Turning it on again after the app moved must not leave the
        /// session starting the old path.
        #[test]
        fn turning_launch_at_login_on_again_points_the_entry_at_the_current_executable() {
            let config = tempfile::tempdir().unwrap();
            let autostart = Autostart::in_config_dir(config.path());
            autostart
                .set_enabled(true, Path::new("/opt/old/grapevine"))
                .unwrap();

            turn_on(&autostart);

            let written =
                std::fs::read_to_string(config.path().join("autostart/grapevine.desktop")).unwrap();
            assert!(written.contains("Exec=/usr/bin/grapevine\n"), "{written}");
        }

        #[test]
        fn the_config_directory_is_the_one_the_user_set() {
            assert_eq!(
                config_dir(Some("/home/khiet/cfg".into()), Some("/home/khiet".into())),
                Ok(PathBuf::from("/home/khiet/cfg"))
            );
        }

        #[test]
        fn the_config_directory_defaults_to_dot_config_under_home() {
            assert_eq!(
                config_dir(None, Some("/home/khiet".into())),
                Ok(PathBuf::from("/home/khiet/.config"))
            );
        }

        /// The XDG spec calls a relative or empty value invalid and says to
        /// ignore it.
        #[test]
        fn a_relative_or_empty_config_directory_setting_is_ignored() {
            for invalid in ["cfg", ""] {
                assert_eq!(
                    config_dir(Some(invalid.into()), Some("/home/khiet".into())),
                    Ok(PathBuf::from("/home/khiet/.config"))
                );
            }
        }

        #[test]
        fn no_config_directory_and_no_home_is_an_error() {
            let error = config_dir(None, None).unwrap_err();
            assert!(error.contains("HOME"), "{error}");
        }
    }
}
