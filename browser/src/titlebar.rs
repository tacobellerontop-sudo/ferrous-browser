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

/// What the user asked the *window* (not the page) to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowCommand {
    /// Begin a native title-bar drag.
    DragTitleBar,
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
/// Close to the native Windows frame thickness, so it feels familiar.
pub const RESIZE_BORDER: f32 = 5.0;

/// Height of the custom title bar.
pub const TITLE_BAR_HEIGHT: f32 = 30.0;

const BUTTON_WIDTH: f32 = 34.0;
const CONTROL_WIDTH: f32 = BUTTON_WIDTH * 3.0;

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

/// Draw the title bar and return the window commands the user triggered.
pub fn draw(ui: &mut Ui, title: &str, maximized: bool) -> Vec<WindowCommand> {
    let mut commands = Vec::new();

    // Window controls are contiguous on every other window on the platform, and
    // they have to be: `CONTROL_WIDTH` reserves exactly `3 * BUTTON_WIDTH` for
    // them, so the default inter-item spacing would push the last one off the
    // right edge. This runs inside the title-bar panel's own `Ui`, so the change
    // does not leak into the toolbar below.
    ui.spacing_mut().item_spacing.x = 0.0;

    ui.horizontal(|ui| {
        // The drag strip occupies everything except the three control buttons.
        let drag_width = (ui.available_width() - CONTROL_WIDTH).max(0.0);
        // egui 0.34 returns (rect, response) in that order.
        let (drag_rect, drag) = ui.allocate_exact_size(
            egui::vec2(drag_width, TITLE_BAR_HEIGHT),
            Sense::click_and_drag(),
        );

        let painter = ui.painter().clone();
        painter.rect_filled(
            drag_rect,
            0.0,
            ui.visuals().widgets.noninteractive.bg_fill,
        );
        painter.text(
            drag_rect.center(),
            egui::Align2::CENTER_CENTER,
            title,
            egui::FontId::proportional(12.0),
            ui.visuals().text_color(),
        );

        if drag.clicked() || drag.dragged() {
            commands.push(WindowCommand::DragTitleBar);
        }

        // Left-to-right, matching the Windows convention. `ui.horizontal` lays
        // out in order, so the array order is the visual order.
        ui.horizontal(|ui| {
            for (icon, tip, command) in [
                (WindowIcon::Minimize, "Minimize", WindowCommand::Minimize),
                (
                    if maximized { WindowIcon::Restore } else { WindowIcon::Maximize },
                    if maximized { "Restore" } else { "Maximize" },
                    WindowCommand::ToggleMaximize,
                ),
                (WindowIcon::Close, "Close", WindowCommand::Close),
            ] {
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(BUTTON_WIDTH, TITLE_BAR_HEIGHT),
                    Sense::click(),
                );

                if response.clicked() {
                    commands.push(command);
                }
                if response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }

                let color = match (icon, response.hovered()) {
                    (WindowIcon::Close, true) => icons::CLOSE_HOVER,
                    (_, true) => ui.visuals().strong_text_color(),
                    _ => ui.visuals().weak_text_color(),
                };
                icons::window(ui.painter(), rect.center(), icon, color);

                // Last, because `on_hover_text` consumes the `Response`.
                response.on_hover_text(tip);
            }
        });
    });

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
    /// # Safety
    /// `hwnd` must be a live window handle for the duration of the call.
    unsafe fn send_non_client(hwnd: *mut c_void, hit_test: u32) {
        unsafe {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, hit_test as usize, cursor_lparam());
        }
    }

    pub fn drag_title_bar(hwnd: *mut c_void) {
        // SAFETY: caller passes a live HWND obtained from winit.
        unsafe { send_non_client(hwnd, HTCAPTION) };
    }

    pub fn resize(hwnd: *mut c_void, edge: ResizeEdge) {
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
        // SAFETY: caller passes a live HWND obtained from winit.
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
pub fn drag_title_bar(_hwnd: *mut std::ffi::c_void) {}

#[cfg(not(windows))]
pub fn resize(_hwnd: *mut std::ffi::c_void, _edge: ResizeEdge) {}

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
}