//! The decisions behind the Linux popover that do not need a display: which
//! display backend to force, and where the popover goes. Compiled for tests on
//! every platform so the macOS CI job covers them too.

use std::ffi::OsStr;

use tauri::{PhysicalPosition, PhysicalSize};

/// The environment variable GTK reads its display backend from.
#[cfg(target_os = "linux")]
pub const BACKEND_VAR: &str = "GDK_BACKEND";

/// The backend to set in [`BACKEND_VAR`], given its current value, or `None`
/// to leave the environment alone. Wayland does not let an app place its own
/// window, so the popover needs X11 (XWayland on the default session) to land
/// in its corner, stay on top and stay out of the dock. A backend the user
/// already chose wins, even though the popover may then be misplaced.
pub fn backend_to_force(chosen: Option<&OsStr>) -> Option<&'static str> {
    match chosen {
        Some(backend) if !backend.is_empty() => None,
        _ => Some("x11"),
    }
}

/// Where the popover's top-left corner goes so it sits in the top-right
/// corner of a monitor's work area, `gap` pixels in from the top and right
/// edges. The work area already excludes the panel, so "below the panel"
/// falls out of using it. All values are physical pixels.
pub fn top_right(
    work_area_position: PhysicalPosition<i32>,
    work_area_size: PhysicalSize<u32>,
    popover_width: u32,
    gap: i32,
) -> PhysicalPosition<i32> {
    let right_edge = work_area_position.x + work_area_size.width as i32;
    PhysicalPosition::new(
        right_edge - popover_width as i32 - gap,
        work_area_position.y + gap,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forces_x11_when_the_user_chose_no_backend() {
        assert_eq!(backend_to_force(None), Some("x11"));
    }

    #[test]
    fn respects_a_backend_the_user_chose() {
        assert_eq!(backend_to_force(Some(OsStr::new("wayland"))), None);
        assert_eq!(backend_to_force(Some(OsStr::new("x11"))), None);
    }

    #[test]
    fn treats_an_empty_backend_as_no_choice() {
        assert_eq!(backend_to_force(Some(OsStr::new(""))), Some("x11"));
    }

    #[test]
    fn places_the_popover_in_the_top_right_of_the_work_area() {
        // A 1920x1080 monitor with a 32px panel on top: the work area starts
        // at y=32. A 440px-wide popover with an 8px gap ends 8px short of the
        // right edge and starts 8px below the panel.
        let position = top_right(
            PhysicalPosition::new(0, 32),
            PhysicalSize::new(1920, 1048),
            440,
            8,
        );
        assert_eq!(position, PhysicalPosition::new(1472, 40));
    }

    #[test]
    fn places_the_popover_on_a_primary_monitor_that_is_not_at_the_origin() {
        // Primary monitor to the right of a 2560px-wide secondary one.
        let position = top_right(
            PhysicalPosition::new(2560, 32),
            PhysicalSize::new(1920, 1048),
            440,
            8,
        );
        assert_eq!(position, PhysicalPosition::new(4032, 40));
    }
}
