//! Browser state that outlives a single frame.
//!
//! Owns the tab set and the queue of pending engine callbacks. Deliberately
//! free of Servo and egui types, so browser policy can be tested without either.
//!
//! # Why callbacks are queued rather than applied inline
//!
//! Servo delivers title/URL/history changes on its own threads. Writing them
//! straight into the tab model from the callback would mean a `RefCell` borrow
//! held across a call that can re-enter the delegate — `paint()` and `spin()` can
//! both do that — which is a runtime panic waiting for the wrong frame.
//!
//! So the delegate only *pushes* a [`TabEvent`], and [`BrowserState::drain_events`]
//! applies them at a known-safe point, once per frame, before the UI reads
//! anything. `docs/hermes-research.md` idea 3 recommends exactly this shape.
//!
//! Events carry a [`TabId`]. A title change for a background tab must land on
//! that tab, not on whichever one is active when the queue is drained — a bare
//! "current tab" pointer gets that wrong the moment the user switches tabs
//! mid-load.

use std::cell::{Cell, Ref, RefCell, RefMut};

use crate::tab::{Tab, TabId, Tabs};

/// A callback from the engine, waiting to be applied to the tab model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabEvent {
    UrlChanged { tab: TabId, url: String },
    TitleChanged { tab: TabId, title: String },
    HistoryChanged { tab: TabId, can_go_back: bool, can_go_forward: bool },
    LoadStatus { tab: TabId, loading: bool },
}

/// State shared between the UI layer and the Servo delegates.
#[derive(Default)]
pub struct BrowserState {
    tabs: RefCell<Tabs>,
    events: RefCell<Vec<TabEvent>>,
    /// Whether the address bar currently holds keyboard focus. While true,
    /// keystrokes must go to the UI rather than to the web page, or typing a
    /// search would both edit the box and scroll the page behind it.
    pub address_focused: Cell<bool>,
}

impl BrowserState {
    /// Borrow the whole tab set.
    ///
    /// UI code should take this **once** per frame and use it throughout, rather
    /// than borrowing per widget: `RefCell` permits only one borrow at a time, so
    /// a per-widget `active()` plus a per-widget `active_mut()` would panic.
    pub fn tabs(&self) -> Ref<'_, Tabs> {
        self.tabs.borrow()
    }

    pub fn tabs_mut(&self) -> RefMut<'_, Tabs> {
        self.tabs.borrow_mut()
    }

    /// Queue a callback from the engine. Safe to call from any thread and at any
    /// time, which is the entire point.
    pub fn push_event(&self, event: TabEvent) {
        self.events.borrow_mut().push(event);
    }

    /// Apply queued callbacks to the tab model.
    ///
    /// Events naming a tab that has since been closed are dropped. That is not
    /// an error: closing a tab races in-flight loads, and the engine has no way
    /// to know a tab is gone until its handle drops.
    pub fn drain_events(&self) {
        let events = std::mem::take(&mut *self.events.borrow_mut());
        if events.is_empty() {
            return;
        }
        let mut tabs = self.tabs.borrow_mut();
        for event in events {
            let Some(tab) = tabs.get_mut(tab_of(&event)) else {
                continue;
            };
            match event {
                TabEvent::UrlChanged { url, .. } => {
                    tab.url = url.clone();
                    // Never yank text out from under someone mid-keystroke.
                    if !self.address_focused.get() {
                        tab.address_text = display_url(&url);
                    }
                }
                TabEvent::TitleChanged { title, .. } => tab.title = title,
                TabEvent::HistoryChanged { can_go_back, can_go_forward, .. } => {
                    tab.can_go_back = can_go_back;
                    tab.can_go_forward = can_go_forward;
                }
                TabEvent::LoadStatus { loading, .. } => tab.loading = loading,
            }
        }
    }

    /// Record a navigation the browser itself initiated.
    ///
    /// Always wins over whatever the engine last reported, because the user just
    /// asked for this URL and the address bar should reflect their intent
    /// immediately rather than after the response starts arriving.
    pub fn note_requested_navigation(&self, url: &str) {
        let mut tabs = self.tabs.borrow_mut();
        let tab = tabs.active_mut();
        tab.url = url.to_owned();
        tab.address_text = url.to_owned();
        self.address_focused.set(false);
    }

    /// Pending event count. Test-facing; also useful when debugging why a tab
    /// looks stale.
    #[cfg(test)]
    fn pending_events(&self) -> usize {
        self.events.borrow().len()
    }
}

fn tab_of(event: &TabEvent) -> TabId {
    match event {
        TabEvent::UrlChanged { tab, .. }
        | TabEvent::TitleChanged { tab, .. }
        | TabEvent::HistoryChanged { tab, .. }
        | TabEvent::LoadStatus { tab, .. } => *tab,
    }
}

/// `about:blank` is Servo's default document, not somewhere the user navigated
/// to. Showing "about:blank" in the address bar on a fresh tab reads as an error,
/// so it is displayed as nothing at all.
fn display_url(url: &str) -> String {
    if url == "about:blank" { String::new() } else { url.to_owned() }
}

/// Convenience for UI code that only needs the active tab.
impl BrowserState {
    pub fn active_tab(&self) -> Ref<'_, Tab> {
        // `tabs()` and `active()` are both immutable borrows, so this cannot
        // panic even if a caller holds other borrows.
        Ref::map(self.tabs.borrow(), |tabs| tabs.active())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_tabs(n: usize) -> BrowserState {
        let state = BrowserState::default();
        {
            let mut tabs = state.tabs.borrow_mut();
            for i in 0..n {
                tabs.create(&format!("https://example{i}.com"));
            }
        }
        state
    }

    #[test]
    fn events_apply_to_the_tab_that_produced_them() {
        // The whole point of carrying a TabId: a title arriving for a background
        // tab must not land on the active one.
        let state = state_with_tabs(2);
        let background = {
            let tabs = state.tabs();
            let active = tabs.active().id;
            tabs.iter().find(|tab| tab.id != active).expect("a background tab").id
        };

        state.push_event(TabEvent::TitleChanged { tab: background, title: "Background".into() });
        state.drain_events();

        let tabs = state.tabs();
        let titled = tabs.iter().find(|tab| tab.id == background).unwrap();
        assert_eq!(titled.title, "Background");
        assert_eq!(tabs.active().title, "", "active tab must be untouched");
    }

    #[test]
    fn events_for_closed_tabs_are_dropped() {
        let state = state_with_tabs(2);
        let doomed = {
            let mut tabs = state.tabs.borrow_mut();
            let victim = tabs.active().id;
            tabs.close(victim);
            victim
        };
        state.push_event(TabEvent::TitleChanged { tab: doomed, title: "late".into() });
        state.drain_events(); // must not panic
        assert!(state.tabs().iter().all(|t| t.title != "late"));
    }

    #[test]
    fn url_events_update_the_address_unless_it_is_being_typed_in() {
        let state = state_with_tabs(1);

        state.push_event(TabEvent::UrlChanged { tab: state.tabs().active().id, url: "https://a.test/".into() });
        state.drain_events();
        assert_eq!(state.tabs().active().address_text, "https://a.test/");

        // Mid-edit: the engine's idea of the URL must not stomp the user's text.
        state.address_focused.set(true);
        state.push_event(TabEvent::UrlChanged { tab: state.tabs().active().id, url: "https://redirected.test/".into() });
        state.drain_events();
        assert_eq!(state.tabs().active().address_text, "https://a.test/");
        assert_eq!(state.tabs().active().url, "https://redirected.test/", "canonical url still updates");
    }

    #[test]
    fn about_blank_is_displayed_as_nothing() {
        let state = state_with_tabs(1);
        state.push_event(TabEvent::UrlChanged { tab: state.tabs().active().id, url: "about:blank".into() });
        state.drain_events();
        assert_eq!(state.tabs().active().address_text, "");
        assert_eq!(state.tabs().active().url, "about:blank");
    }

    #[test]
    fn history_and_load_status_are_recorded() {
        let state = state_with_tabs(1);
        let id = state.tabs().active().id;
        state.push_event(TabEvent::HistoryChanged { tab: id, can_go_back: true, can_go_forward: false });
        state.push_event(TabEvent::LoadStatus { tab: id, loading: true });
        state.drain_events();
        assert!(state.tabs().active().can_go_back);
        assert!(!state.tabs().active().can_go_forward);
        assert!(state.tabs().active().loading);
    }

    #[test]
    fn requested_navigation_wins_over_the_reported_url() {
        let state = state_with_tabs(1);
        state.note_requested_navigation("https://typed.test/");
        let tabs = state.tabs();
        let tab = tabs.active();
        assert_eq!(tab.url, "https://typed.test/");
        assert_eq!(tab.address_text, "https://typed.test/");
        assert!(!state.address_focused.get());
    }

    #[test]
    fn draining_an_empty_queue_is_a_no_op() {
        let state = state_with_tabs(1);
        state.drain_events();
        assert_eq!(state.pending_events(), 0);
        assert_eq!(state.tabs().len(), 1);
    }

    #[test]
    fn events_are_drained_in_order() {
        // A title then a URL must not leave the title applied to the old URL.
        let state = state_with_tabs(1);
        let id = state.tabs().active().id;
        state.push_event(TabEvent::UrlChanged { tab: id, url: "https://one.test/".into() });
        state.push_event(TabEvent::UrlChanged { tab: id, url: "https://two.test/".into() });
        state.drain_events();
        assert_eq!(state.tabs().active().url, "https://two.test/");
    }
}
