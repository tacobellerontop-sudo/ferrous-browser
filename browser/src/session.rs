//! Reopening the tabs that were open last time.
//!
//! The session is the list of open tabs (address, title, zoom) and which one
//! was active. It is written to `%APPDATA%\Ferrous\session.txt` on exit and,
//! debounced, whenever it changes, so a crash does not lose it.
//!
//! Restoring is **lazy**, following Hermes Browser's design: only the active
//! tab gets an engine web view at start-up. The others exist in the tab strip
//! with their saved titles and are loaded the first time they are selected, so
//! reopening twenty tabs does not load twenty pages.

use crate::storage;
use crate::tab::Tabs;

const FILE: &str = "session.txt";

#[derive(Debug, Clone, PartialEq)]
pub struct SavedTab {
    pub url: String,
    pub title: String,
    pub zoom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub tabs: Vec<SavedTab>,
    pub active: usize,
}

impl Session {
    /// Capture the open tabs.
    pub fn of(tabs: &Tabs) -> Self {
        let active_id = tabs.active().id;
        Self {
            tabs: tabs
                .iter()
                .map(|t| SavedTab { url: t.url.clone(), title: t.title.clone(), zoom: t.zoom })
                .collect(),
            active: tabs.iter().position(|t| t.id == active_id).unwrap_or(0),
        }
    }

    /// The saved session, if there is a usable one.
    pub fn load() -> Option<Self> {
        Self::from_text(&storage::read(FILE)?)
    }

    pub fn save(&self) {
        if let Err(err) = storage::write(FILE, &self.to_text()) {
            log::error!("session: could not save: {err}");
        }
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("active\t{}\n", self.active);
        for tab in &self.tabs {
            let title = tab.title.replace(['\t', '\n', '\r'], " ");
            text.push_str(&format!("tab\t{}\t{}\t{}\n", tab.zoom, tab.url, title));
        }
        text
    }

    /// Parse [`to_text`](Self::to_text) output. Unusable tab lines are skipped;
    /// a session with no tabs left is `None`, and the browser opens the
    /// homepage instead.
    pub fn from_text(text: &str) -> Option<Self> {
        let mut active = 0;
        let mut tabs = Vec::new();
        for line in text.lines() {
            let mut fields = line.splitn(4, '\t');
            match fields.next() {
                Some("active") => active = fields.next()?.parse().ok()?,
                Some("tab") => {
                    let zoom: f32 = fields.next().and_then(|z| z.parse().ok()).unwrap_or(1.0);
                    let Some(url) = fields.next().filter(|u| url::Url::parse(u).is_ok()) else {
                        continue;
                    };
                    let title = fields.next().unwrap_or_default().to_owned();
                    tabs.push(SavedTab {
                        url: url.to_owned(),
                        title,
                        zoom: if zoom.is_finite() { zoom.clamp(0.25, 5.0) } else { 1.0 },
                    });
                }
                _ => {}
            }
        }
        if tabs.is_empty() {
            return None;
        }
        Some(Self { active: active.min(tabs.len() - 1), tabs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            tabs: vec![
                SavedTab { url: "https://a.test/".into(), title: "A".into(), zoom: 1.0 },
                SavedTab { url: "ferrous:newtab".into(), title: "New Tab".into(), zoom: 1.25 },
            ],
            active: 1,
        }
    }

    #[test]
    fn round_trips_through_text() {
        assert_eq!(Session::from_text(&session().to_text()), Some(session()));
    }

    #[test]
    fn captures_open_tabs_and_the_active_one() {
        let mut tabs = Tabs::default();
        tabs.create("https://a.test/");
        let second = tabs.create("https://b.test/");
        tabs.select(second);
        let saved = Session::of(&tabs);
        assert_eq!(saved.tabs.len(), 2);
        assert_eq!(saved.active, 1);
        assert_eq!(saved.tabs[1].url, "https://b.test/");
    }

    #[test]
    fn bad_lines_are_skipped_and_an_empty_session_is_none() {
        let text = "active\t0\ntab\t1\tnot a url\tX\ntab\t1\thttps://ok.test/\tOK\n";
        let s = Session::from_text(text).unwrap();
        assert_eq!(s.tabs.len(), 1);
        assert_eq!(Session::from_text("active\t0\n"), None);
        assert_eq!(Session::from_text(""), None);
    }

    #[test]
    fn an_out_of_range_active_index_is_clamped() {
        let text = "active\t9\ntab\t1\thttps://a.test/\tA\n";
        assert_eq!(Session::from_text(text).unwrap().active, 0);
    }

    #[test]
    fn titles_cannot_break_the_format() {
        let mut s = session();
        s.tabs[0].title = "evil\ttitle\nwith lines".into();
        let back = Session::from_text(&s.to_text()).unwrap();
        assert_eq!(back.tabs.len(), 2);
        assert_eq!(back.tabs[0].title, "evil title with lines");
    }

    #[test]
    fn absurd_zoom_values_are_clamped() {
        let s = Session::from_text("active\t0\ntab\t99\thttps://a.test/\tA\n").unwrap();
        assert_eq!(s.tabs[0].zoom, 5.0);
    }
}
