//! Remembering where the window was.
//!
//! The window's last *normal* (not maximised, not minimised) position and size,
//! plus whether it was maximised, are written to a small text file on exit and
//! read back on start-up.
//!
//! The normal bounds are tracked separately from the maximised flag, the way
//! Windows' own `GetWindowPlacement` does it. Saving the maximised window's
//! bounds instead would make "restore" after a restart do nothing, because the
//! remembered size would already be the full screen.
//!
//! The format is `key=value` lines rather than JSON, so this needs no new
//! dependency and stays readable if someone opens the file. Everything here is
//! pure except [`load`] and [`save`], so the parsing and the "is it still on a
//! screen?" check are unit tested.

use crate::storage;

/// Smallest window the browser will open at or be resized to, in physical
/// pixels at 100% scale. Below this the tab strip, toolbar and window controls
/// no longer fit side by side.
pub const MIN_WIDTH: u32 = 500;
pub const MIN_HEIGHT: u32 = 300;

/// How much of the title bar must land on a monitor for saved bounds to be
/// trusted, in physical pixels. Enough to grab and drag the window back.
const VISIBLE_GRIP_WIDTH: i32 = 120;
const VISIBLE_GRIP_HEIGHT: i32 = 30;

/// Window position and size in physical pixels, plus the maximised flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowState {
    /// Outer top-left corner. Equal to the inner one for a frameless window.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// A monitor's rectangle in physical desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowState {
    pub fn to_text(self) -> String {
        format!(
            "x={}\ny={}\nwidth={}\nheight={}\nmaximized={}\n",
            self.x, self.y, self.width, self.height, self.maximized
        )
    }

    /// Parse [`to_text`](Self::to_text) output. Returns `None` if any field is
    /// missing or malformed: a half-read file is not worth guessing at, and the
    /// default window is a perfectly good fallback.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut x = None;
        let mut y = None;
        let mut width = None;
        let mut height = None;
        let mut maximized = None;
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "x" => x = value.parse().ok(),
                "y" => y = value.parse().ok(),
                "width" => width = value.parse().ok(),
                "height" => height = value.parse().ok(),
                "maximized" => maximized = value.parse().ok(),
                _ => {}
            }
        }
        Some(Self {
            x: x?,
            y: y?,
            width: width?,
            height: height?,
            maximized: maximized?,
        })
    }

    /// Make saved bounds safe to open at on the current monitors.
    ///
    /// The size is clamped to the minimum. If the top of the window would not
    /// be reachable on any monitor — a disconnected second screen is the usual
    /// cause — returns `None` so the caller falls back to the default
    /// placement, rather than opening a window the user cannot see or drag.
    pub fn fit_to(self, monitors: &[Monitor]) -> Option<Self> {
        let state = Self {
            width: self.width.max(MIN_WIDTH),
            height: self.height.max(MIN_HEIGHT),
            ..self
        };
        let grip_width = VISIBLE_GRIP_WIDTH.min(state.width as i32);
        let reachable = monitors.iter().any(|monitor| {
            let overlap_x = (state.x + state.width as i32).min(monitor.x + monitor.width as i32)
                - state.x.max(monitor.x);
            let top_on_screen =
                state.y >= monitor.y && state.y + VISIBLE_GRIP_HEIGHT <= monitor.y + monitor.height as i32;
            overlap_x >= grip_width && top_on_screen
        });
        reachable.then_some(state)
    }
}

/// Whether bounds reported by the window are worth remembering as the normal
/// placement. Windows parks a minimised window at (-32000, -32000) with a tiny
/// size, and that must never be saved as where the window "was".
pub fn is_normal_placement(x: i32, y: i32, width: u32, height: u32) -> bool {
    x > -32000 && y > -32000 && width >= 100 && height >= 100
}

const FILE: &str = "window.txt";

/// Read the saved state, if there is any. Missing or unreadable files are
/// normal (first run), so they are not errors.
pub fn load() -> Option<WindowState> {
    WindowState::from_text(&storage::read(FILE)?)
}

/// Write the state. See `storage::write` for why this cannot truncate it.
pub fn save(state: WindowState) -> std::io::Result<()> {
    storage::write(FILE, &state.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: WindowState = WindowState {
        x: 120,
        y: 80,
        width: 1280,
        height: 900,
        maximized: false,
    };

    const PRIMARY: Monitor = Monitor {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };

    #[test]
    fn round_trips_through_text() {
        assert_eq!(WindowState::from_text(&STATE.to_text()), Some(STATE));
        let maximized = WindowState { maximized: true, x: -1500, ..STATE };
        assert_eq!(WindowState::from_text(&maximized.to_text()), Some(maximized));
    }

    #[test]
    fn incomplete_or_corrupt_text_is_rejected() {
        assert_eq!(WindowState::from_text(""), None);
        assert_eq!(WindowState::from_text("x=1\ny=2\nwidth=3\nheight=4\n"), None);
        assert_eq!(WindowState::from_text(&STATE.to_text().replace("900", "tall")), None);
        assert_eq!(WindowState::from_text(&STATE.to_text().replace("1280", "-5")), None);
    }

    #[test]
    fn unknown_keys_and_whitespace_are_tolerated() {
        let text = format!("# comment\nversion=2\n{}", STATE.to_text().replace("x=", " x = "));
        assert_eq!(WindowState::from_text(&text), Some(STATE));
    }

    #[test]
    fn bounds_on_a_monitor_are_kept() {
        assert_eq!(STATE.fit_to(&[PRIMARY]), Some(STATE));
    }

    #[test]
    fn bounds_on_a_disconnected_monitor_are_dropped() {
        // Saved while on a second screen to the right, which is now unplugged.
        let elsewhere = WindowState { x: 2200, ..STATE };
        assert_eq!(elsewhere.fit_to(&[PRIMARY]), None);
        let secondary = Monitor { x: 1920, ..PRIMARY };
        assert_eq!(elsewhere.fit_to(&[PRIMARY, secondary]), Some(elsewhere));
    }

    #[test]
    fn a_title_bar_above_the_screen_is_unreachable() {
        assert_eq!(WindowState { y: -200, ..STATE }.fit_to(&[PRIMARY]), None);
        assert_eq!(WindowState { y: 1070, ..STATE }.fit_to(&[PRIMARY]), None);
    }

    #[test]
    fn a_window_mostly_off_the_side_is_kept_if_it_can_be_grabbed() {
        let mostly_off = WindowState { x: 1920 - 200, ..STATE };
        assert_eq!(mostly_off.fit_to(&[PRIMARY]), Some(mostly_off));
        let sliver = WindowState { x: 1920 - 40, ..STATE };
        assert_eq!(sliver.fit_to(&[PRIMARY]), None);
    }

    #[test]
    fn tiny_sizes_are_raised_to_the_minimum() {
        let tiny = WindowState { width: 10, height: 10, ..STATE };
        let fitted = tiny.fit_to(&[PRIMARY]).unwrap();
        assert_eq!((fitted.width, fitted.height), (MIN_WIDTH, MIN_HEIGHT));
    }

    #[test]
    fn minimised_placement_is_not_remembered() {
        assert!(is_normal_placement(100, 100, 1100, 820));
        assert!(!is_normal_placement(-32000, -32000, 160, 28));
        assert!(!is_normal_placement(100, 100, 0, 0));
        // Negative but legitimate: a monitor to the left of the primary one.
        assert!(is_normal_placement(-1800, 40, 1100, 820));
    }
}
