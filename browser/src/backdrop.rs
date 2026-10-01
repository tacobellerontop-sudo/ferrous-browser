//! The window's translucent backdrop: Windows 11 Mica behind the chrome.
//!
//! Mica is a blurred, tinted sample of the desktop wallpaper that the window
//! manager (DWM) draws behind a window. It shows wherever the window's own
//! pixels are transparent, so the frame loop clears to alpha 0 and only the
//! page card and the chrome panel are opaque.
//!
//! Three requirements, all verified on screen rather than assumed:
//!
//! 1. The window is created transparent (`with_transparent(true)`), which makes
//!    winit set `WS_EX_NOREDIRECTIONBITMAP`, so DWM composites the GL swap
//!    chain's alpha channel instead of ignoring it.
//! 2. The DWM frame is extended into the client area. Only a **1px bottom**
//!    margin is used: the usual `-1` "sheet of glass" margin makes DWM paint its
//!    own minimize/maximize/close glyphs over ours, because winit keeps
//!    `WS_SYSMENU` on undecorated windows. One pixel is enough for DWM to treat
//!    the whole window as having a backdrop.
//! 3. `DWMWA_SYSTEMBACKDROP_TYPE` selects Mica, and dark mode is requested so
//!    the tint matches the dark chrome.
//!
//! On systems without the backdrop API (Windows 10, older Windows 11 builds)
//! [`enable`] returns false and the caller paints an opaque background instead,
//! so the window never ends up see-through with nothing behind it.

use std::ffi::c_void;

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    /// `MARGINS`, in device pixels.
    #[repr(C)]
    struct Margins {
        left: i32,
        right: i32,
        top: i32,
        bottom: i32,
    }

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_SYSTEMBACKDROP_TYPE: u32 = 38;
    /// `DWMSBT_MAINWINDOW`, i.e. Mica.
    const DWMSBT_MAINWINDOW: i32 = 2;

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmExtendFrameIntoClientArea(hwnd: *mut c_void, margins: *const Margins) -> i32;
        fn DwmSetWindowAttribute(
            hwnd: *mut c_void,
            attribute: u32,
            value: *const c_void,
            size: u32,
        ) -> i32;
    }

    fn set_attribute(hwnd: *mut c_void, attribute: u32, value: i32) -> bool {
        // SAFETY: `value` is a live i32 for the duration of the call and its
        // size is passed alongside it; the caller guarantees `hwnd`.
        let result = unsafe {
            DwmSetWindowAttribute(hwnd, attribute, (&raw const value).cast(), size_of::<i32>() as u32)
        };
        result >= 0
    }

    pub fn enable(hwnd: *mut c_void) -> bool {
        let margins = Margins { left: 0, right: 0, top: 0, bottom: 1 };
        // SAFETY: `margins` outlives the call; the caller guarantees `hwnd`.
        if unsafe { DwmExtendFrameIntoClientArea(hwnd, &margins) } < 0 {
            return false;
        }
        // Dark mode is cosmetic; failing it is not a reason to give up on Mica.
        set_attribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, 1);
        set_attribute(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_MAINWINDOW)
    }
}

/// Turn on the Mica backdrop for `hwnd`. Returns false if the system does not
/// support it, in which case the window must paint its own opaque background.
///
/// `hwnd` must be a live window handle.
pub fn enable(hwnd: *mut c_void) -> bool {
    #[cfg(windows)]
    {
        platform::enable(hwnd)
    }
    #[cfg(not(windows))]
    {
        let _ = hwnd;
        false
    }
}
