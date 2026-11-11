//! What the popover does when something asks for it, decided without a
//! window so every platform's tests cover it.

/// Why the popover is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// A click on the tray icon or its "Open Grapevine" item.
    Toggle,
    /// Grapevine was launched while already running. Only Linux raises this;
    /// macOS brings the running copy forward by itself.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    SecondLaunch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Hide,
    Nothing,
}

/// `recently_auto_hidden` is whether the popover hid itself on losing focus a
/// moment ago. A tray click on an open popover blurs it first and only then
/// arrives, so without that flag the click would re-show what it just
/// dismissed.
pub fn decide(request: Request, visible: bool, recently_auto_hidden: bool) -> Action {
    match request {
        Request::SecondLaunch => Action::Show,
        Request::Toggle if visible => Action::Hide,
        Request::Toggle if recently_auto_hidden => Action::Nothing,
        Request::Toggle => Action::Show,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tray_click_shows_a_hidden_popover() {
        assert_eq!(decide(Request::Toggle, false, false), Action::Show);
    }

    #[test]
    fn a_tray_click_hides_an_open_popover() {
        assert_eq!(decide(Request::Toggle, true, false), Action::Hide);
    }

    #[test]
    fn a_tray_click_that_just_dismissed_the_popover_does_not_reopen_it() {
        assert_eq!(decide(Request::Toggle, false, true), Action::Nothing);
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
