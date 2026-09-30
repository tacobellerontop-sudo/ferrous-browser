//! Address-bar input classification.
//!
//! Deliberately free of Servo and egui types so it can be unit tested with a
//! plain `cargo test`. Everything here is a pure function of its input.
//!
//! The heuristic is the one browsers have converged on: if the text parses as a
//! URL with a scheme we support, use it; otherwise try reading it as a host; if
//! that fails too, the user meant a search.

use url::Url;

/// Schemes we are willing to hand to the engine directly. Anything else typed by
/// a user is far more likely a typo or a search term than a real intent
/// (`javascript:`, `data:`, `file:` are all reachable another way, or not at
/// all, and silently navigating to them from an address bar is how browsers end
/// up executing attacker-supplied script).
const SUPPORTED_SCHEMES: [&str; 3] = ["http", "https", "about"];

const SEARCH_ENDPOINT: &str = "https://duckduckgo.com/";

/// What the user typed, once classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A URL to navigate to directly.
    Url(Url),
    /// Free text to be turned into a search.
    Search(String),
}

impl Address {
    /// Resolve to a URL. Infallible given the variants, so it returns `Url`
    /// rather than a `Result`; the only fallible part (URL parsing) already
    /// happened in [`classify`].
    pub fn into_url(self) -> Url {
        match self {
            Self::Url(url) => url,
            Self::Search(query) => search_url(&query),
        }
    }
}

/// Classify raw address-bar text without committing to a destination.
pub fn classify(input: &str) -> Address {
    let input = input.trim();
    if input.is_empty() {
        return Address::Search(String::new());
    }

    if let Ok(url) = Url::parse(input)
        && SUPPORTED_SCHEMES.contains(&url.scheme())
    {
        return Address::Url(url);
    }

    if let Some(url) = parse_as_host(input) {
        return Address::Url(url);
    }

    Address::Search(input.to_owned())
}

/// Convenience wrapper: classify and resolve in one step.
pub fn resolve(input: &str) -> Url {
    classify(input).into_url()
}

/// Try reading `input` as a bare hostname, e.g. `example.com` or `localhost:8080`.
///
/// Requires the result to actually look like a host: contains a dot, is exactly
/// `localhost`, or parses as an IP literal. Without that check, any single word
/// would become `https://word`, and searching would be impossible.
fn parse_as_host(input: &str) -> Option<Url> {
    // Anything already carrying a scheme or path-ish punctuation is not a bare
    // host, and guessing here produces nonsense like `https://https://x`.
    if input.contains("://") || input.contains('/') || input.contains(' ') {
        return None;
    }

    let candidate = Url::parse(&format!("https://{input}")).ok()?;

    // Credentials in a bare host are a phishing pattern, not something a user
    // types into an address bar to browse.
    if !candidate.username().is_empty() || candidate.password().is_some() {
        return None;
    }

    let host = candidate.host_str()?;
    let looks_like_host = host.eq_ignore_ascii_case("localhost")
        || host.contains('.')
        || host.parse::<std::net::IpAddr>().is_ok();

    looks_like_host.then_some(candidate)
}

fn search_url(query: &str) -> Url {
    let mut url = Url::parse(SEARCH_ENDPOINT).expect("SEARCH_ENDPOINT must be a valid URL");
    url.query_pairs_mut().append_pair("q", query);
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url_of(input: &str) -> String {
        resolve(input).to_string()
    }

    #[test]
    fn absolute_urls_pass_through() {
        assert_eq!(url_of("https://example.com/path"), "https://example.com/path");
        assert_eq!(url_of("http://example.com"), "http://example.com/");
        assert_eq!(url_of("about:blank"), "about:blank");
    }

    #[test]
    fn unsupported_schemes_become_searches() {
        // Navigating to `javascript:` from an address bar is a security footgun.
        assert!(matches!(classify("javascript:alert(1)"), Address::Search(_)));
        assert!(matches!(classify("data:text/html,x"), Address::Search(_)));
    }

    #[test]
    fn bare_hosts_are_upgraded_to_https() {
        assert_eq!(url_of("example.com"), "https://example.com/");
        assert_eq!(url_of("localhost"), "https://localhost/");
        assert_eq!(url_of("localhost:8080"), "https://localhost:8080/");
        assert_eq!(url_of("127.0.0.1:3000"), "https://127.0.0.1:3000/");
    }

    #[test]
    fn bare_words_become_searches() {
        assert_eq!(classify("hello world"), Address::Search("hello world".into()));
        assert_eq!(classify("rust lang"), Address::Search("rust lang".into()));
    }

    #[test]
    fn search_queries_reach_the_search_endpoint() {
        let url = url_of("rust embedding");
        assert!(url.starts_with(SEARCH_ENDPOINT), "got {url}");
        assert!(url.contains("q=rust+embedding") || url.contains("q=rust%20embedding"));
    }

    #[test]
    fn empty_input_yields_an_empty_search() {
        assert_eq!(classify(""), Address::Search(String::new()));
        assert_eq!(classify("   "), Address::Search(String::new()));
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(classify("  example.com  "), Address::Url(Url::parse("https://example.com/").unwrap()));
    }

    #[test]
    fn embedded_credentials_are_rejected_as_hosts() {
        // `https://user:pass@evil.test` typed bare should not silently navigate.
        assert!(matches!(classify("user:pass@evil.test"), Address::Search(_)));
    }

    #[test]
    fn strings_with_slashes_are_not_treated_as_bare_hosts() {
        assert!(matches!(classify("example.com/path"), Address::Search(_)));
    }
}