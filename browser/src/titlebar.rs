//! Custom window title bar, replacing the OS one.
//!
//! # Why this needs Win32 interop
//!
//! winit 0.30 provides **no** way to drag or resize a window:
//! `Window::start_drag` and `Window::begin_resize_drag` do not exist, and there
//! is no `maximize`/`unmaximize` at all. Building a frameless window without
//! those leaves the user with a window they cannot move or resize, which is
//! worse than having no custom title bar at all.
//!
//! The standard Windows solution is to hand the drag back to the window manager:
//! `ReleaseCapture` followed by `SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)`.
//! Windows then runs its own modal drag loop, which means correct behaviour comes
//! for free — edge snapping, Aero Shake, double-click to maximise and
//! multi-monitor bounds all keep working, none of which a hand-rolled "track the
//! cursor and call set_outer_position" loop would get right.
//!
//! The same message with `HTLEFT`/`HTTOPRIGHT`/... drives edge resizing.
//!
//! This is the project's only `unsafe` block. It is genuine platform interop,
//! not a workaround for a type error, and it is confined to this module.

use egui::{Rect, Sense, Ui};

use crate::icons::{self, WindowIcon};
use crate::theme;

/// What the user asked the *window* (not the page) to do.
///
/// Dragging is deliberately absent. `WM_NCLBUTTONDOWN` blocks in a modal
/// Windows loop, so it is issued directly from the mouse-down handler — see
/// [`drag_title_bar`] — rather than queued through a frame like these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    Minimize,
    ToggleMaximize,
    Close,
}

/// Which edge or corner a resize grab applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Left,
    Right,
    Top,
    TopLeft,
    TopRight,
    Bottom,
    BottomLeft,
    BottomRight,
}

/// How far from the window edge counts as a resize grab, in egui points.
/// Matches the gap around the page card (`chrome::GAP`), so the whole visible
/// margin is the grip, as it would be on a native frame.
pub const RESIZE_BORDER: f32 = 8.0;

/// Height of the custom title bar. The tab strip lives in this row.
pub const TITLE_BAR_HEIGHT: f32 = 38.0;

/// 46 wide is the native Windows 10/11 caption button width, so the controls sit
/// where muscle memory expects them.
const BUTTON_WIDTH: f32 = 46.0;
pub const CONTROL_WIDTH: f32 = BUTTON_WIDTH * 3.0;

/// The right end of a title `row`, where the three window controls sit.
pub fn controls_rect(row: Rect) -> Rect {
    Rect::from_min_max(egui::pos2(row.right() - CONTROL_WIDTH, row.top()), row.max)
}

/// Whether a press at `point` should start a title-bar drag.
///
/// `row` is where the title row was drawn last frame, or [`Rect::NOTHING`]
/// while the chrome is hidden — the page is under the pointer then, and a press
/// at the top of a page must reach the page. `tabs` is the area the tabs and
/// the new-tab button occupied; presses there belong to the tabs. The window
/// controls at the right end of the row are excluded too, so their presses fall
/// through to egui.
///
/// The drag area is not an egui widget, and hit testing lives here, so that a
/// drag can never accumulate egui interaction state — see `drag_title_bar`.
pub fn is_in_drag_area(point: egui::Pos2, row: Rect, tabs: Rect) -> bool {
    row.contains(point) && !controls_rect(row).contains(point) && !tabs.contains(point)
}

/// Which edge, if any, a point sits on. `None` means the interior.
pub fn resize_edge_at(point: egui::Pos2, window: Rect) -> Option<ResizeEdge> {
    // Without this, a point to the *left* of the window has a negative distance to
    // the right edge, which satisfies `<= RESIZE_BORDER` and would be reported as
    // a right-edge resize.
    if point.x < window.min.x
        || point.x > window.max.x
        || point.y < window.min.y
        || point.y > window.max.y
    {
        return None;
    }

    let on_left = point.x - window.min.x <= RESIZE_BORDER;
    let on_right = window.max.x - point.x <= RESIZE_BORDER;
    let on_top = point.y - window.min.y <= RESIZE_BORDER;
    let on_bottom = window.max.y - point.y <= RESIZE_BORDER;

    match (on_left, on_right, on_top, on_bottom) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (_, true, true, _) => Some(ResizeEdge::TopRight),
        (true, _, _, true) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Left),
        (_, true, _, _) => Some(ResizeEdge::Right),
        (_, _, true, _) => Some(ResizeEdge::Top),
        (_, _, _, true) => Some(ResizeEdge::Bottom),
        _ => None,
    }
}

/// The pointer shape for a resize edge, so the border advertises itself.
pub fn resize_cursor(edge: ResizeEdge) -> egui::CursorIcon {
    use egui::CursorIcon::*;
    match edge {
        ResizeEdge::Left | ResizeEdge::Right => ResizeHorizontal,
        ResizeEdge::Top | ResizeEdge::Bottom => ResizeVertical,
        ResizeEdge::TopLeft | ResizeEdge::BottomRight => ResizeNwSe,
        ResizeEdge::TopRight | ResizeEdge::BottomLeft => ResizeNeSw,
    }
}

/// Draw the minimize/maximize/close buttons into `rect`, which must be the
/// rightmost [`CONTROL_WIDTH`] of the title bar, and return the window commands
/// the user triggered.
///
/// The rest of the row is the tab strip and the drag area, both of which the
/// caller draws. The drag area itself is never an egui widget: it is hit tested
/// geometrically in [`is_in_drag_area`]. Giving egui a draggable widget there
/// meant `dragged()` stayed true for as long as the pointer was down, and it
/// queued a native drag every frame; `WM_NCLBUTTONDOWN` pumps messages, so each
/// one re-entered the frame loop and queued another. One title-bar drag was
/// measured burning 609ms of CPU over the following 4 seconds, with the UI
/// unresponsive. See the note on `drag_title_bar`.
pub fn controls(ui: &mut Ui, rect: Rect, maximized: bool) -> Vec<WindowCommand> {
    let mut commands = Vec::new();

    // Left-to-right, matching the Windows convention. Laid out by hand, not by
    // `ui.horizontal`, so inter-item spacing cannot push the last button off the
    // right edge of the window.
    let buttons = [
        (WindowIcon::Minimize, "Minimize", WindowCommand::Minimize),
        (
            if maximized { WindowIcon::Restore } else { WindowIcon::Maximize },
            if maximized { "Restore" } else { "Maximize" },
            WindowCommand::ToggleMaximize,
        ),
        (WindowIcon::Close, "Close", WindowCommand::Close),
    ];
    for (i, (icon, tip, command)) in buttons.into_iter().enumerate() {
        let button = Rect::from_min_size(
            egui::pos2(rect.left() + i as f32 * BUTTON_WIDTH, rect.top()),
            egui::vec2(BUTTON_WIDTH, rect.height()),
        );
        let response = ui.interact(button, ui.id().with(("window_control", i)), Sense::click());

        if response.clicked() {
            commands.push(command);
        }

        // A rounded wash on hover rather than Windows' full-height block,
        // which would square off the floating panel's rounded corner. Close
        // still turns red with a white glyph, as on every Windows window.
        let (fill, color) = match (icon, response.hovered(), response.is_pointer_button_down_on()) {
            (WindowIcon::Close, true, _) => (Some(theme::CLOSE_HOVER), egui::Color32::WHITE),
            (_, true, true) => (Some(theme::PRESS), theme::TEXT),
            (_, true, false) => (Some(theme::HOVER), theme::TEXT),
            _ => (None, theme::TEXT_WEAK),
        };
        if let Some(fill) = fill {
            ui.painter().rect_filled(button.shrink2(egui::vec2(5.0, 6.0)), 7.0, fill);
        }
        icons::window(ui.painter(), button.center(), icon, color);

        // Last, because `on_hover_text` consumes the `Response`.
        response.on_hover_text(tip);
    }

    commands
}

// ---------------------------------------------------------------------------
// Platform interop
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod platform {
    use super::ResizeEdge;
    use std::ffi::c_void;

    // Verified against windows-sys 0.45.0 rather than written from memory.
    const WM_NCLBUTTONDOWN: u32 = 161;
    const HTCAPTION: u32 = 2;
    const HTLEFT: u32 = 10;
    const HTRIGHT: u32 = 11;
    const HTTOP: u32 = 12;
    const HTTOPLEFT: u32 = 13;
    const HTTOPRIGHT: u32 = 14;
    const HTBOTTOM: u32 = 15;
    const HTBOTTOMLEFT: u32 = 16;
    const HTBOTTOMRIGHT: u32 = 17;

    // `ShowWindow` command codes, needed because winit 0.30 exposes
    // `is_maximized` and `set_minimized` but no `maximize`/`unmaximize`.
    const SW_MAXIMIZE: i32 = 3;
    const SW_RESTORE: i32 = 9;

    // `unsafe extern` because the crate is on edition 2024, which requires the
    // safety obligation to be spelled out at the block. Every call site below
    // carries its own SAFETY note.
    unsafe extern "system" {
        fn ReleaseCapture() -> i32;
        fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn ShowWindow(hwnd: *mut c_void, cmd: i32) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    /// Pack a screen point into an `LPARAM`, the way Win32 message parameters
    /// carry coordinates: 16 bits of X then 16 bits of Y.
    ///
    /// This must be the *real* cursor position. Passing `0` looks harmless, and
    /// a vertical resize even works by accident, but Windows validates the point
    /// against the window bounds — with `0, 0` a left, right or caption drag is
    /// silently discarded while a bottom-edge drag still goes through. Found by
    /// testing each edge, not by reading the docs.
    fn cursor_lparam() -> isize {
        let mut point = Point { x: 0, y: 0 };
        // SAFETY: `point` is a live, correctly aligned, initialised `POINT`.
        let ok = unsafe { GetCursorPos(&raw mut point) };
        if ok == 0 {
            return 0;
        }
        // `as u16` truncates rather than saturating, which is exactly the
        // wraparound Win32 expects, including for negative monitor coordinates.
        ((point.y as u32 as u16) as isize) << 16 | (point.x as u32 as u16) as isize
    }

    /// Hand the interaction back to the window manager, which then runs its own
    /// modal loop for dragging or resizing.
    ///
    /// **Blocks until the gesture ends**, pumping messages while it does. Call
    /// it from the mouse-down handler and nowhere else — never from inside a
    /// frame, because the pumped messages re-enter the frame loop and anything
    /// the frame queued gets duplicated.
    ///
    /// # Safety
    /// `hwnd` must be a live window handle for the duration of the call.
    unsafe fn send_non_client(hwnd: *mut c_void, hit_test: u32) {
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, hit_test as usize, cursor_lparam());
        }
    }

    /// Begin a native title-bar drag. See the module docs for why the drag is
    /// routed through Windows at all.
    ///
    /// # Safety
    /// `hwnd` must be a live window handle for the duration of the call.
    pub unsafe fn drag_title_bar(hwnd: *mut c_void) {
        // SAFETY: the caller guarantees a live HWND from winit.
        unsafe { send_non_client(hwnd, HTCAPTION) };
    }

    /// Begin a native edge resize.
    ///
    /// # Safety
    /// `hwnd` must be a live window handle for the duration of the call.
    pub unsafe fn resize(hwnd: *mut c_void, edge: ResizeEdge) {
        let hit = match edge {
            ResizeEdge::Left => HTLEFT,
            ResizeEdge::Right => HTRIGHT,
            ResizeEdge::Top => HTTOP,
            ResizeEdge::TopLeft => HTTOPLEFT,
            ResizeEdge::TopRight => HTTOPRIGHT,
            ResizeEdge::Bottom => HTBOTTOM,
            ResizeEdge::BottomLeft => HTBOTTOMLEFT,
            ResizeEdge::BottomRight => HTBOTTOMRIGHT,
        };
        // SAFETY: the caller guarantees a live HWND from winit.
        unsafe { send_non_client(hwnd, hit) };
    }

    /// Maximise or restore the window.
    pub fn set_maximized(hwnd: *mut c_void, maximized: bool) {
        // SAFETY: caller passes a live HWND obtained from winit. The return value
        // is the previous visibility state, which we have no use for.
        unsafe {
            ShowWindow(hwnd, if maximized { SW_MAXIMIZE } else { SW_RESTORE });
        }
    }
}

#[cfg(windows)]
pub use platform::{drag_title_bar, resize, set_maximized};

#[cfg(not(windows))]
pub unsafe fn drag_title_bar(_hwnd: *mut std::ffi::c_void) {}

#[cfg(not(windows))]
pub unsafe fn resize(_hwnd: *mut std::ffi::c_void, _edge: ResizeEdge) {}

#[cfg(not(windows))]
pub fn set_maximized(_hwnd: *mut std::ffi::c_void, _maximized: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0))
    }

    #[test]
    fn interior_is_not_a_resize_edge() {
        assert_eq!(resize_edge_at(egui::pos2(400.0, 300.0), window()), None);
    }

    #[test]
    fn corners_pick_the_diagonal_edge() {
        let w = window();
        assert_eq!(resize_edge_at(egui::pos2(0.0, 0.0), w), Some(ResizeEdge::TopLeft));
        assert_eq!(resize_edge_at(egui::pos2(800.0, 0.0), w), Some(ResizeEdge::TopRight));
        assert_eq!(resize_edge_at(egui::pos2(0.0, 600.0), w), Some(ResizeEdge::BottomLeft));
        assert_eq!(resize_edge_at(egui::pos2(800.0, 600.0), w), Some(ResizeEdge::BottomRight));
    }

    #[test]
    fn single_edges_are_detected() {
        let w = window();
        assert_eq!(resize_edge_at(egui::pos2(0.0, 300.0), w), Some(ResizeEdge::Left));
        assert_eq!(resize_edge_at(egui::pos2(800.0, 300.0), w), Some(ResizeEdge::Right));
        assert_eq!(resize_edge_at(egui::pos2(400.0, 0.0), w), Some(ResizeEdge::Top));
        assert_eq!(resize_edge_at(egui::pos2(400.0, 600.0), w), Some(ResizeEdge::Bottom));
    }

    #[test]
    fn just_outside_the_border_is_interior() {
        let w = window();
        assert_eq!(resize_edge_at(egui::pos2(RESIZE_BORDER + 0.5, 300.0), w), None);
    }

    #[test]
    fn points_outside_the_window_are_not_edges() {
        // Regression: a point left of the window has a *negative* distance to the
        // right edge, so a naive `<= RESIZE_BORDER` test called it a right-edge
        // resize. winit reports cursor positions relative to the client area, and
        // those can fall outside it while a drag is being tracked.
        let w = window();
        assert_eq!(resize_edge_at(egui::pos2(-99.0, 300.0), w), None);
        assert_eq!(resize_edge_at(egui::pos2(300.0, -99.0), w), None);
        assert_eq!(resize_edge_at(egui::pos2(900.0, 700.0), w), None);
    }

    /// A title row at the window origin, 800 wide.
    fn row() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, TITLE_BAR_HEIGHT))
    }

    /// No tab strip at all, for tests that are about the row's outer bounds.
    const NO_TABS: Rect = Rect::NOTHING;

    #[test]
    fn drag_area_spans_the_row_but_not_the_controls() {
        assert!(is_in_drag_area(egui::pos2(10.0, 5.0), row(), NO_TABS));
        assert!(is_in_drag_area(egui::pos2(400.0, 15.0), row(), NO_TABS));

        // The rightmost `CONTROL_WIDTH` belongs to the three buttons, which egui
        // handles. If this ever returned true the buttons would be dead.
        assert!(!is_in_drag_area(egui::pos2(800.0 - CONTROL_WIDTH + 0.5, 15.0), row(), NO_TABS));
        assert!(!is_in_drag_area(egui::pos2(799.0, 15.0), row(), NO_TABS));
    }

    #[test]
    fn drag_area_stops_at_the_bottom_of_the_row() {
        assert!(is_in_drag_area(egui::pos2(400.0, TITLE_BAR_HEIGHT - 0.5), row(), NO_TABS));
        // The toolbar starts here, and must stay clickable.
        assert!(!is_in_drag_area(egui::pos2(400.0, TITLE_BAR_HEIGHT + 1.0), row(), NO_TABS));
    }

    #[test]
    fn tabs_are_not_a_drag_area_but_the_space_around_them_is() {
        let tabs = Rect::from_min_max(egui::pos2(8.0, 6.0), egui::pos2(300.0, TITLE_BAR_HEIGHT));
        assert!(!is_in_drag_area(egui::pos2(100.0, 20.0), row(), tabs));
        assert!(is_in_drag_area(egui::pos2(100.0, 3.0), row(), tabs), "above the tabs");
        assert!(is_in_drag_area(egui::pos2(400.0, 20.0), row(), tabs), "right of the tabs");
    }

    #[test]
    fn a_floating_row_is_hit_tested_where_it_is() {
        // The chrome panel floats inside the window margin, not at the origin.
        let floating = row().translate(egui::vec2(8.0, 8.0));
        assert!(!is_in_drag_area(egui::pos2(4.0, 4.0), floating, NO_TABS), "in the margin");
        assert!(is_in_drag_area(egui::pos2(400.0, 20.0), floating, NO_TABS));
        assert!(!is_in_drag_area(egui::pos2(808.0 - 10.0, 20.0), floating, NO_TABS), "controls");
    }

    #[test]
    fn a_hidden_chrome_never_drags() {
        // While the chrome is hidden the page is under the pointer, and a press
        // at the top of the page must not start moving the window.
        assert!(!is_in_drag_area(egui::pos2(400.0, 10.0), Rect::NOTHING, NO_TABS));
    }
}