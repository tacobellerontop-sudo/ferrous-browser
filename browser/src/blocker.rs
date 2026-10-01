//! Ad and tracker blocking.
//!
//! Requests a page makes to known advertising and tracking hosts are cancelled
//! before they reach the network, through Servo's `load_web_resource` hook (see
//! `engine.rs`). This is Hermes Browser's "native content blocking", done with a
//! small built-in domain list rather than a full filter-list engine: no new
//! dependency, nothing to download, and a lookup per request that costs a few
//! hash probes.
//!
//! Rules:
//!
//! - **A domain blocks itself and its subdomains.** `doubleclick.net` covers
//!   `stats.g.doubleclick.net`, but not `notdoubleclick.net`.
//! - **The page itself is never blocked.** Only the requests a page makes are;
//!   typing `doubleclick.net` into the address bar still goes there.
//! - **Sites can be allowed.** If blocking breaks a site, the shield in the
//!   address bar turns it off for that site, and the choice is remembered in
//!   `%APPDATA%\Ferrous\allowed-sites.txt`.
//!
//! Engine-neutral: only `url`, so it is tested by plain `cargo test`.

use std::cell::RefCell;
use std::collections::HashSet;

use url::Url;

use crate::storage;

const LIST: &str = include_str!("blocklist.txt");
const ALLOWED_FILE: &str = "allowed-sites.txt";

pub struct Blocker {
    domains: HashSet<&'static str>,
    /// Hosts of pages where blocking is turned off.
    allowed: RefCell<HashSet<String>>,
    /// Whether to read and write the allowlist file. Off in tests.
    persist: bool,
}

impl Default for Blocker {
    /// The built-in list, with no sites allowed and nothing saved: what tests
    /// and the `BrowserState` default use.
    fn default() -> Self {
        Self::new(false)
    }
}

impl Blocker {
    /// The built-in list plus the saved allowlist.
    pub fn load() -> Self {
        Self::new(true)
    }

    fn new(persist: bool) -> Self {
        let domains = parse_list(LIST);
        let allowed = if persist {
            storage::read(ALLOWED_FILE)
                .map(|text| text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect())
                .unwrap_or_default()
        } else {
            HashSet::new()
        };
        Self { domains, allowed: RefCell::new(allowed), persist }
    }

    /// Whether a request should be cancelled.
    ///
    /// `page` is the URL of the page making the request, if known; `main_frame`
    /// is true for the navigation of the page itself.
    pub fn should_block(&self, request: &Url, page: Option<&Url>, main_frame: bool) -> bool {
        if main_frame {
            return false;
        }
        if page.and_then(Url::host_str).is_some_and(|host| self.is_allowed(host)) {
            return false;
        }
        request.host_str().is_some_and(|host| self.is_listed(host))
    }

    /// Whether `host` or any parent domain of it is on the list.
    pub fn is_listed(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        suffixes(&host).any(|suffix| self.domains.contains(suffix))
    }

    /// Whether blocking is turned off on pages from `host`.
    pub fn is_allowed(&self, host: &str) -> bool {
        self.allowed.borrow().contains(&host.to_ascii_lowercase())
    }

    /// Turn blocking off for `host` if it was on, or back on if it was off.
    /// Returns whether the site is now allowed.
    pub fn toggle_allowed(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        let now_allowed = {
            let mut allowed = self.allowed.borrow_mut();
            if allowed.remove(&host) {
                false
            } else {
                allowed.insert(host);
                true
            }
        };
        if self.persist {
            let mut hosts: Vec<_> = self.allowed.borrow().iter().cloned().collect();
            hosts.sort();
            if let Err(err) = storage::write(ALLOWED_FILE, &(hosts.join("\n") + "\n")) {
                log::error!("blocker: could not save allowed sites: {err}");
            }
        }
        now_allowed
    }
}

/// The non-comment, non-blank lines of a list, lowercased by construction
/// (the list is written lowercase).
fn parse_list(list: &'static str) -> HashSet<&'static str> {
    list.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

/// `a.b.example.com`, `b.example.com`, `example.com`, `com`.
fn suffixes(host: &str) -> impl Iterator<Item = &str> {
    std::iter::once(host).chain(host.match_indices('.').map(move |(i, _)| &host[i + 1..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    fn page() -> Url {
        url("https://news.example/article")
    }

    #[test]
    fn listed_domains_and_their_subdomains_are_blocked() {
        let b = Blocker::default();
        assert!(b.should_block(&url("https://doubleclick.net/x"), Some(&page()), false));
        assert!(b.should_block(&url("https://stats.g.doubleclick.net/x"), Some(&page()), false));
        assert!(b.should_block(&url("https://www.google-analytics.com/analytics.js"), Some(&page()), false));
    }

    #[test]
    fn lookalikes_and_ordinary_sites_are_not() {
        let b = Blocker::default();
        assert!(!b.should_block(&url("https://notdoubleclick.net/"), Some(&page()), false));
        assert!(!b.should_block(&url("https://fonts.googleapis.com/css"), Some(&page()), false));
        assert!(!b.should_block(&url("https://en.wikipedia.org/"), Some(&page()), false));
        // A listed *subdomain* does not drag its parent in.
        assert!(!b.should_block(&url("https://linkedin.com/feed"), Some(&page()), false));
        assert!(b.should_block(&url("https://ads.linkedin.com/x"), Some(&page()), false));
    }

    #[test]
    fn the_page_itself_is_never_blocked() {
        let b = Blocker::default();
        assert!(!b.should_block(&url("https://doubleclick.net/"), None, true));
    }

    #[test]
    fn host_matching_ignores_case_and_a_trailing_dot() {
        let b = Blocker::default();
        assert!(b.is_listed("DoubleClick.NET"));
        assert!(b.is_listed("doubleclick.net."));
    }

    #[test]
    fn allowing_a_site_lets_its_requests_through_and_can_be_undone() {
        let b = Blocker::default();
        let tracker = url("https://doubleclick.net/x");
        assert!(b.toggle_allowed("news.example"));
        assert!(!b.should_block(&tracker, Some(&page()), false));
        // Other sites are still protected.
        assert!(b.should_block(&tracker, Some(&url("https://other.example/")), false));
        assert!(!b.toggle_allowed("NEWS.example"), "toggling again turns blocking back on");
        assert!(b.should_block(&tracker, Some(&page()), false));
    }

    #[test]
    fn the_list_parses_without_comments_or_blanks() {
        let domains = parse_list(LIST);
        assert!(domains.len() > 80, "list unexpectedly short: {}", domains.len());
        assert!(domains.iter().all(|d| !d.contains(' ') && !d.starts_with('#') && *d == d.to_ascii_lowercase()));
    }

    #[test]
    fn suffixes_walk_up_the_domain() {
        let all: Vec<_> = suffixes("a.b.example.com").collect();
        assert_eq!(all, ["a.b.example.com", "b.example.com", "example.com", "com"]);
    }
}
