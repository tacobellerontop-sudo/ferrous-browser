//! Tab model.
//!
//! Engine-neutral on purpose: no Servo, no egui, no window. That is the whole
//! point — tab *policy* (what happens when you close the active tab, which tab
//! becomes active, whether an id is ever reused) is the part that is easy to get
//! subtly wrong, and it is the part that must be testable by plain `cargo test`
//! rather than by clicking.
//!
//! `docs/hermes-research.md` idea 1: an engine-neutral core is what separates a
//! browser whose logic CI can verify from one that can only be tried by hand.
//!
//! # Identity
//!
//! A [`TabId`] is a monotonically increasing counter that is **never reused**,
//! even after the tab closes. That matters because engine callbacks
//! (title changed, load finished) arrive asynchronously: if a late callback from
//! a closed tab could be confused with a newly opened one, it would land on the
//! wrong tab. A reused id makes that possible; a never-reused one makes it
//! impossible.

use std::sync::Arc;

/// A page's icon as straight (not premultiplied) RGBA pixels, already
/// converted from whatever format the engine produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Favicon {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// Different for every icon the engine reports, so the UI can tell when to
    /// replace a texture without comparing pixels.
    pub version: u64,
}

/// Stable, never-reused identifier for a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(u64);

impl TabId {
    /// The raw counter. Exposed for logging and for `HashMap` keys.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Everything the UI needs to draw one tab.
///
/// `address_text` lives here rather than in the browser-wide state because the
/// address bar is per tab: switching to a background tab must show the address
/// *it* is on, not whatever the previously active tab had half-typed.
// `PartialEq` only: `zoom` is a float.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: TabId,
    /// Displayed address, which differs from `url` while the user is editing.
    pub address_text: String,
    /// Canonical URL the engine last reported.
    pub url: String,
    pub title: String,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
    pub favicon: Option<Favicon>,
    /// Page zoom, 1.0 = 100%.
    pub zoom: f32,
}

impl Tab {
    /// What to show on the tab itself: the page title if there is one, otherwise
    /// the address, otherwise something obviously empty.
    ///
    /// YouTube's title is often longer than any tab is wide, so truncation is
    /// the UI's job; this only decides *what*.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            if self.address_text.is_empty() {
                "New Tab"
            } else {
                &self.address_text
            }
        } else {
            &self.title
        }
    }
}

/// What happened when a tab was closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    /// A tab remains and is now active.
    Closed(TabId),
    /// The last tab closed; the caller should shut the window down.
    ClosedLast,
}

/// The ordered set of tabs, plus which one is active.
///
/// Order is the order they appear in the strip, so the vector index *is* the
/// strip position.
#[derive(Debug)]
pub struct Tabs {
    tabs: Vec<Tab>,
    active: usize,
    /// Next id to hand out. Starts at 1 so that `0` is never a valid id.
    next_id: u64,
}

impl Default for Tabs {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
        }
    }
}

impl Tabs {
    /// Add a tab and make it active. Returns its id.
    pub fn create(&mut self, address: &str) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            address_text: address.to_owned(),
            url: address.to_owned(),
            title: String::new(),
            can_go_back: false,
            can_go_forward: false,
            loading: false,
            favicon: None,
            zoom: 1.0,
        });
        self.active = self.tabs.len() - 1;
        id
    }

    pub fn len(&self) -> usize {
        self.tabs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    /// The active tab. Callers must not call this when `is_empty()`, which the
    /// browser guarantees by exiting when the last tab closes.
    pub fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn get_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    /// Make `id` active. Returns false when no such tab exists, which happens
    /// for a callback naming a tab that has already been closed.
    pub fn select(&mut self, id: TabId) -> bool {
        match self.tabs.iter().position(|tab| tab.id == id) {
            Some(index) => {
                self.active = index;
                true
            }
            None => false,
        }
    }

    /// Move the selection by `delta` positions, wrapping at both ends.
    ///
    /// Returns the newly active id, or `None` if there are no tabs.
    pub fn select_offset(&mut self, delta: isize) -> Option<TabId> {
        if self.tabs.is_empty() {
            return None;
        }
        let count = self.tabs.len() as isize;
        // Two roundings are needed: the modulo can go negative, and a negative
        // `%` in Rust keeps the sign of the dividend.
        let next = (self.active as isize + delta).rem_euclid(count);
        self.active = next as usize;
        Some(self.tabs[self.active].id)
    }

    /// Close `id`.
    ///
    /// Which tab becomes active is the subtle part:
    /// * closing a background tab leaves the selection alone;
    /// * closing the active tab selects the tab that slid into its place, i.e.
    ///   the one to its right, or the one to its left if it was rightmost.
    ///
    /// That "right, else left" rule is what browsers do, and picking "always
    /// left" instead is a visible behavioural difference users notice.
    pub fn close(&mut self, id: TabId) -> CloseOutcome {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            // Already gone. Report the current state rather than panicking: a
            // close button can be double-clicked and a callback can race it.
            return if self.tabs.is_empty() {
                CloseOutcome::ClosedLast
            } else {
                CloseOutcome::Closed(self.tabs[self.active].id)
            };
        };

        self.tabs.remove(index);

        if self.tabs.is_empty() {
            self.active = 0;
            return CloseOutcome::ClosedLast;
        }

        if index < self.active {
            // Everything after the hole shifted left, including the selection.
            self.active -= 1;
        } else if index == self.active {
            // `self.active` already points at the tab that moved into this
            // slot. If the closed tab was last, that index is now one past the
            // end, so step back onto the new rightmost.
            if self.active >= self.tabs.len() {
                self.active = self.tabs.len() - 1;
            }
        }

        CloseOutcome::Closed(self.tabs[self.active].id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs_with(n: usize) -> Tabs {
        let mut tabs = Tabs::default();
        for i in 0..n {
            tabs.create(&format!("https://example{i}.com"));
        }
        tabs
    }

    fn ids(tabs: &Tabs) -> Vec<u64> {
        tabs.iter().map(|tab| tab.id.get()).collect()
    }

    /// `Tabs` has no `get` accessor — nothing outside tests needs one — so tests
    /// find a tab by id through the iterator.
    fn tab<'a>(tabs: &'a Tabs, id: u64) -> &'a Tab {
        tabs.iter().find(|tab| tab.id.get() == id).expect("tab exists")
    }

    #[test]
    fn first_tab_becomes_active() {
        let mut tabs = Tabs::default();
        assert!(tabs.is_empty());
        let id = tabs.create("https://a.test");
        assert_eq!(tabs.active().id, id);
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn creating_a_tab_makes_it_active_and_keeps_the_others() {
        let mut tabs = tabs_with(3);
        let newest = tabs.create("https://new.test");
        assert_eq!(tabs.active().id, newest);
        assert_eq!(tabs.len(), 4);
        assert_eq!(ids(&tabs), vec![1, 2, 3, 4]);
    }

    #[test]
    fn ids_are_never_reused() {
        let mut tabs = tabs_with(2);
        let first = tabs.active().id;
        tabs.close(first);
        // A closed tab's id must never come back, or a late engine callback
        // naming it would land on whichever tab now owns the number.
        let reused = tabs.create("https://later.test");
        assert_ne!(reused, first);
        assert!(tabs.iter().all(|t| t.id != first));
    }

    #[test]
    fn closing_the_last_tab_reports_it() {
        let mut tabs = tabs_with(1);
        let id = tabs.active().id;
        assert_eq!(tabs.close(id), CloseOutcome::ClosedLast);
        assert!(tabs.is_empty());
    }

    #[test]
    fn closing_a_background_tab_leaves_the_selection_alone() {
        let mut tabs = tabs_with(3);
        let background = tab(&tabs, 1).id;
        let active = tabs.active().id;
        let outcome = tabs.close(background);
        assert_eq!(outcome, CloseOutcome::Closed(active));
        assert_eq!(tabs.active().id, active);
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn closing_the_active_tab_selects_the_one_to_its_right() {
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(1)));
        // Tabs are 1,2,3 and tab 1 is active. Closing it should select 2, which
        // slid into index 0.
        let outcome = tabs.close(TabId(1));
        assert_eq!(outcome, CloseOutcome::Closed(TabId(2)));
    }

    #[test]
    fn closing_the_rightmost_tab_falls_back_to_the_left() {
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(3)));
        let outcome = tabs.close(TabId(3));
        assert_eq!(outcome, CloseOutcome::Closed(TabId(2)));
    }

    #[test]
    fn closing_the_active_tab_keeps_the_selection_on_a_real_tab() {
        // Regression: closing the last tab while it is active leaves the index
        // one past the end, which would panic on the next `active()`.
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(3)));
        tabs.close(TabId(3));
        assert_eq!(tabs.active().id, TabId(2), "selection must land on a real tab");
        assert_eq!(tabs.active().id, TabId(2));
    }

    #[test]
    fn closing_a_tab_before_the_active_one_keeps_the_same_tab_active() {
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(3)));
        let outcome = tabs.close(TabId(1));
        // Tab 3 is still the active one; it just moved down an index.
        assert_eq!(outcome, CloseOutcome::Closed(TabId(3)));
        assert_eq!(tabs.active().id, TabId(3));
        assert_eq!(tabs.iter().position(|t| t.id == TabId(3)), Some(1));
    }

    #[test]
    fn closing_an_unknown_tab_is_harmless() {
        let mut tabs = tabs_with(2);
        let active = tabs.active().id;
        assert_eq!(tabs.close(TabId(999)), CloseOutcome::Closed(active));
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn closing_an_unknown_tab_when_empty_reports_closed_last() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.close(TabId(1)), CloseOutcome::ClosedLast);
    }

    #[test]
    fn select_offset_wraps_forwards() {
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(3)));
        assert_eq!(tabs.select_offset(1), Some(TabId(1)));
    }

    #[test]
    fn select_offset_wraps_backwards() {
        let mut tabs = tabs_with(3);
        assert!(tabs.select(TabId(1)));
        assert_eq!(tabs.select_offset(-1), Some(TabId(3)));
    }

    #[test]
    fn select_offset_handles_a_large_jump() {
        // Guards the negative-modulo bug: in Rust `-7 % 4` is `-3`, not `1`, so
        // a plain `%` would produce a negative index. `rem_euclid` is what makes
        // the wrap correct. Four tabs starting at index 0 (TabId 1); -7 lands on
        // index 1, which is TabId 2.
        let mut tabs = tabs_with(4);
        assert!(tabs.select(TabId(1)));
        assert_eq!(tabs.select_offset(-7), Some(TabId(2)));
    }

    #[test]
    fn select_offset_on_an_empty_set_is_none() {
        let mut tabs = Tabs::default();
        assert_eq!(tabs.select_offset(1), None);
        assert_eq!(tabs.select_offset(-1), None);
    }

    #[test]
    fn select_rejects_unknown_ids_without_changing_anything() {
        let mut tabs = tabs_with(2);
        let active = tabs.active().id;
        assert!(!tabs.select(TabId(999)));
        assert_eq!(tabs.active().id, active);
    }

    #[test]
    fn get_mut_reaches_only_the_named_tab() {
        let mut tabs = tabs_with(2);
        tabs.get_mut(TabId(1)).unwrap().title = "first".into();
        tabs.get_mut(TabId(2)).unwrap().title = "second".into();
        assert_eq!(tab(&tabs, 1).title, "first");
        assert_eq!(tab(&tabs, 2).title, "second");
        assert!(tabs.iter().all(|t| t.id.get() != 3));
    }

    #[test]
    fn label_prefers_title_then_address_then_placeholder() {
        let mut tabs = Tabs::default();
        let id = tabs.create("");
        assert_eq!(tab(&tabs, id.get()).label(), "New Tab");
        tabs.get_mut(id).unwrap().address_text = "https://a.test".into();
        assert_eq!(tab(&tabs, id.get()).label(), "https://a.test");
        tabs.get_mut(id).unwrap().title = "A Page".into();
        assert_eq!(tab(&tabs, id.get()).label(), "A Page");
    }

    #[test]
    fn a_long_title_is_not_truncated_by_the_model() {
        // The model must not silently shorten data the UI might want in full
        // (tooltip, history). Truncation is a rendering concern.
        let mut tabs = Tabs::default();
        let id = tabs.create("");
        let long = "x".repeat(500);
        tabs.get_mut(id).unwrap().title = long.clone();
        assert_eq!(tab(&tabs, id.get()).label(), long);
    }
}
