//! What the popover does when something asks for it, decided without a
//! window so every platform's tests cover it.

/// Why the popover is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// A click on the tray icon or its "Open Grapevine" item.
    TrayClick,
    /// Grapevine was launched while already running. Only Linux raises this;
    /// macOS does not start a second copy in the first place.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    SecondLaunch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Hide,
    Nothing,
}

/// A second launch always ends with the popover on screen, whatever its state:
/// starting the app is never a way to dismiss it. A tray click toggles, except
/// that a click landing just after the popover hid itself on losing focus
/// (`recently_auto_hidden`) does nothing; see `LastAutoHide` for why.
pub fn decide(request: Request, visible: bool, recently_auto_hidden: bool) -> Action {
    match request {
        Request::SecondLaunch => Action::Show,
        Request::TrayClick if visible => Action::Hide,
        Request::TrayClick if recently_auto_hidden => Action::Nothing,
        Request::TrayClick => Action::Show,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tray_click_shows_a_hidden_popover() {
        assert_eq!(decide(Request::TrayClick, false, false), Action::Show);
    }

    #[test]
    fn a_tray_click_hides_an_open_popover() {
        assert_eq!(decide(Request::TrayClick, true, false), Action::Hide);
    }

    #[test]
    fn a_tray_click_that_just_dismissed_the_popover_does_not_reopen_it() {
        assert_eq!(decide(Request::TrayClick, false, true), Action::Nothing);
    }

    #[test]
    fn a_second_launch_shows_a_hidden_popover() {
        assert_eq!(decide(Request::SecondLaunch, false, false), Action::Show);
    }

    #[test]
    fn a_second_launch_keeps_an_open_popover_open() {
        assert_eq!(decide(Request::SecondLaunch, true, false), Action::Show);
    }

    #[test]
    fn a_second_launch_shows_the_popover_even_if_it_just_lost_focus() {
        // Launching from the app grid or a terminal takes focus from an open
        // popover first; the launch must still end with it on screen.
        assert_eq!(decide(Request::SecondLaunch, false, true), Action::Show);
    }
}
