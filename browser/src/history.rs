//! Browsing history, and the address-bar suggestions drawn from it.
//!
//! Every page a tab commits to is recorded with its title, how often it has
//! been visited and when it was last visited. Typing in the address bar ranks
//! matching entries by *frecency* — frequency, decayed by recency — so a site
//! visited daily outranks one visited often a year ago.
//!
//! Stored as tab-separated lines in `%APPDATA%\Ferrous\history.tsv`, capped at
//! [`MAX_ENTRIES`]; the least recently visited go first. Only `http` and `https`
//! pages are recorded: the homepage and `about:` pages are not places.
//!
//! Engine-neutral and time-injected, so ranking is unit tested.

use std::collections::HashMap;

use crate::storage;

pub const MAX_ENTRIES: usize = 5000;
const FILE: &str = "history.tsv";
/// Suggestions shown under the address bar.
pub const MAX_SUGGESTIONS: usize = 6;

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub url: String,
    pub title: String,
    pub visits: u32,
    /// Seconds since the Unix epoch.
    pub last_visit: u64,
}

#[derive(Debug, Default)]
pub struct History {
    entries: HashMap<String, Entry>,
    /// Set by any change, cleared when saved, so an unchanged history is
    /// never rewritten.
    dirty: bool,
}

/// Seconds since the Unix epoch, now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl History {
    /// The saved history, or an empty one.
    pub fn load() -> Self {
        storage::read(FILE).map_or_else(Self::default, |text| Self::from_text(&text))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Record a visit to `url` at time `at`.
    pub fn visit(&mut self, url: &str, at: u64) {
        if !is_recordable(url) {
            return;
        }
        let entry = self.entries.entry(url.to_owned()).or_insert_with(|| Entry {
            url: url.to_owned(),
            title: String::new(),
            visits: 0,
            last_visit: at,
        });
        entry.visits = entry.visits.saturating_add(1);
        entry.last_visit = at;
        self.dirty = true;
        self.trim();
    }

    /// Attach a page title to an already-visited `url`.
    pub fn set_title(&mut self, url: &str, title: &str) {
        if let Some(entry) = self.entries.get_mut(url)
            && entry.title != title
        {
            entry.title = clean(title);
            self.dirty = true;
        }
    }

    /// The best matches for what the user has typed, best first.
    pub fn suggest(&self, query: &str, at: u64) -> Vec<Entry> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let typed_lower = query.trim().to_lowercase();
        let typed = normalise(&typed_lower);
        let mut scored: Vec<(f64, &Entry)> = self
            .entries
            .values()
            .filter_map(|entry| {
                let url_lower = entry.url.to_lowercase();
                let url = normalise(&url_lower);
                let title = entry.title.to_lowercase();
                if !words.iter().all(|w| url.contains(w.as_str()) || title.contains(w.as_str())) {
                    return None;
                }
                // Typing the start of an address is a much stronger signal
                // than a word that happens to appear in a title.
                let prefix_bonus = if url.starts_with(&typed) { 4.0 } else { 1.0 };
                Some((frecency(entry, at) * prefix_bonus, entry))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.url.len().cmp(&b.1.url.len())));
        scored.into_iter().take(MAX_SUGGESTIONS).map(|(_, e)| e.clone()).collect()
    }

    /// Save if anything changed since the last save.
    pub fn save_if_dirty(&mut self) {
        if !self.dirty {
            return;
        }
        match storage::write(FILE, &self.to_text()) {
            Ok(()) => self.dirty = false,
            Err(err) => log::error!("history: could not save: {err}"),
        }
    }

    fn trim(&mut self) {
        if self.entries.len() <= MAX_ENTRIES {
            return;
        }
        let mut by_age: Vec<(u64, String)> =
            self.entries.values().map(|e| (e.last_visit, e.url.clone())).collect();
        by_age.sort();
        for (_, url) in by_age.into_iter().take(self.entries.len() - MAX_ENTRIES) {
            self.entries.remove(&url);
        }
    }

    pub fn to_text(&self) -> String {
        let mut entries: Vec<&Entry> = self.entries.values().collect();
        entries.sort_by(|a, b| b.last_visit.cmp(&a.last_visit));
        entries
            .iter()
            .map(|e| format!("{}\t{}\t{}\t{}\n", e.url, e.visits, e.last_visit, e.title))
            .collect()
    }

    /// Parse [`to_text`](Self::to_text) output, skipping malformed lines
    /// rather than discarding the whole history over one bad line.
    pub fn from_text(text: &str) -> Self {
        let entries = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, '\t');
                let url = fields.next()?.to_owned();
                let visits = fields.next()?.parse().ok()?;
                let last_visit = fields.next()?.parse().ok()?;
                let title = fields.next().unwrap_or_default().to_owned();
                is_recordable(&url).then(|| (url.clone(), Entry { url, title, visits, last_visit }))
            })
            .collect();
        let mut history = Self { entries, dirty: false };
        history.trim();
        history
    }
}

/// Visits, decayed by a week-scale half-life: a page visited today counts
/// fully, one last visited a week ago about half.
fn frecency(entry: &Entry, at: u64) -> f64 {
    let age_days = at.saturating_sub(entry.last_visit) as f64 / 86_400.0;
    f64::from(entry.visits) / (1.0 + age_days / 7.0)
}

fn is_recordable(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && !url.contains(['\t', '\n'])
}

/// Drop the scheme and a leading `www.`, so typing `git` matches
/// `https://github.com` as a prefix.
fn normalise(url: &str) -> &str {
    let url = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    url.strip_prefix("www.").unwrap_or(url)
}

/// Titles can contain anything; tabs and newlines would break the file format.
fn clean(title: &str) -> String {
    title.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;
    const NOW: u64 = 1_800_000_000;

    fn with(visits: &[(&str, &str, u32, u64)]) -> History {
        let mut h = History::default();
        for &(url, title, count, at) in visits {
            for _ in 0..count {
                h.visit(url, at);
            }
            h.set_title(url, title);
        }
        h
    }

    fn urls(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.url.as_str()).collect()
    }

    #[test]
    fn visits_accumulate_and_titles_attach() {
        let h = with(&[("https://a.test/", "A site", 3, NOW)]);
        let e = &h.suggest("a.test", NOW)[0];
        assert_eq!((e.visits, e.title.as_str()), (3, "A site"));
    }

    #[test]
    fn only_web_pages_are_recorded() {
        let h = with(&[("ferrous:newtab", "", 1, NOW), ("about:blank", "", 1, NOW)]);
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn typing_the_start_of_a_host_matches_without_scheme_or_www() {
        let h = with(&[("https://www.github.com/", "GitHub", 1, NOW)]);
        assert_eq!(urls(&h.suggest("git", NOW)), ["https://www.github.com/"]);
    }

    #[test]
    fn every_word_must_match_url_or_title() {
        let h = with(&[
            ("https://docs.rs/egui", "egui - Rust", 1, NOW),
            ("https://example.com/rust", "Rust book", 1, NOW),
        ]);
        assert_eq!(urls(&h.suggest("rust egui", NOW)), ["https://docs.rs/egui"]);
        assert!(h.suggest("python", NOW).is_empty());
    }

    #[test]
    fn frequent_recent_pages_rank_first() {
        let h = with(&[
            ("https://news.test/old", "News archive", 20, NOW - 120 * DAY),
            ("https://news.test/today", "News today", 5, NOW),
        ]);
        assert_eq!(urls(&h.suggest("news", NOW))[0], "https://news.test/today");
    }

    #[test]
    fn an_address_prefix_beats_a_title_mention() {
        let h = with(&[
            ("https://blog.test/servo-post", "All about servo", 3, NOW),
            ("https://servo.org/", "Servo", 1, NOW),
        ]);
        assert_eq!(urls(&h.suggest("servo", NOW))[0], "https://servo.org/");
    }

    #[test]
    fn suggestions_are_capped() {
        let pages: Vec<String> = (0..20).map(|i| format!("https://site{i}.test/")).collect();
        let visits: Vec<_> = pages.iter().map(|p| (p.as_str(), "", 1, NOW)).collect();
        assert_eq!(with(&visits).suggest("site", NOW).len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn history_round_trips_and_survives_bad_lines() {
        let h = with(&[("https://a.test/", "Title with\ttab", 2, NOW)]);
        let text = h.to_text();
        let back = History::from_text(&(text.clone() + "garbage line\n"));
        assert_eq!(back.to_text(), text);
        assert!(!text.lines().next().unwrap().contains("with\ttab"), "tab in title escaped");
    }

    #[test]
    fn the_oldest_entries_are_dropped_past_the_cap() {
        let mut h = History::default();
        for i in 0..(MAX_ENTRIES + 10) {
            h.visit(&format!("https://p{i}.test/"), NOW + i as u64);
        }
        assert_eq!(h.len(), MAX_ENTRIES);
        assert!(h.suggest("p0.test", NOW + 10_000).is_empty(), "the oldest went first");
    }

    #[test]
    fn saving_is_skipped_when_nothing_changed() {
        let mut h = History::from_text("https://a.test/\t1\t1\tA\n");
        assert!(!h.dirty);
        h.set_title("https://a.test/", "A");
        assert!(!h.dirty, "same title is not a change");
    }
}
