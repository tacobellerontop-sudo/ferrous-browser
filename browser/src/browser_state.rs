//! Browser state that outlives a single frame.
//!
//! The egui UI reads this every frame; the engine delegate writes to it as
//! pages load. Keeping it in one place is what lets the UI stay a pure function
//! of state and the delegate stay a pure function of engine events.
//!
//! Deliberately free of Servo and egui types: this module is the seam that lets
//! browser logic be tested without either.

use std::cell::{Cell, RefCell};

/// State shared between the UI layer and the Servo delegate.
#[derive(Default)]
pub struct BrowserState {
    /// Current page URL as a display string. Empty means "no page yet".
    pub url: RefCell<String>,
    pub title: RefCell<String>,

    pub can_go_back: Cell<bool>,
    pub can_go_forward: Cell<bool>,
    pub loading: Cell<bool>,

    /// Address bar contents. User-editable, so it is *not* always equal to
    /// `url`: it holds half-typed text while the user is mid-edit.
    pub address_text: RefCell<String>,

    /// Whether the address bar currently holds keyboard focus. While true,
    /// keystrokes must go to the UI rather than to the web page, or typing a
    /// search would both edit the box and scroll the page behind it.
    pub address_focused: Cell<bool>,

    /// Set when the engine reports a URL we did not ask for (a redirect, or a
    /// clicked link). The UI uses it to decide when to overwrite whatever the
    /// user was typing, which should not happen while they are mid-edit.
    pub url_changed_externally: Cell<bool>,
}

impl BrowserState {
    /// Record a URL reported by the engine.
    ///
    /// The address bar is only overwritten when the user is not editing it.
    /// Yanking the text out from under someone mid-keystroke is the kind of
    /// small hostility that makes a browser feel broken.
    pub fn set_url(&self, url: String) {
        *self.url.borrow_mut() = url.clone();
        if !self.address_focused.get() {
            *self.address_text.borrow_mut() = display_url(&url);
            self.url_changed_externally.set(true);
        }
    }

    /// Record a navigation the browser itself initiated, which should always
    /// win over stale address-bar text.
    pub fn note_requested_navigation(&self, url: &str) {
        *self.url.borrow_mut() = url.to_owned();
        *self.address_text.borrow_mut() = url.to_owned();
        self.address_focused.set(false);
    }
}

/// `about:blank` is Servo's default document, not somewhere the user navigated
/// to. Showing "about:blank" in the address bar on a fresh tab reads as an error,
/// so it is displayed as nothing at all.
fn display_url(url: &str) -> String {
    if url == "about:blank" { String::new() } else { url.to_owned() }
}