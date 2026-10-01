//! The built-in homepage, served from `ferrous:newtab`.
//!
//! The page is compiled into the binary with `include_str!` and handed to Servo
//! through a custom protocol handler, the same mechanism servoshell uses for
//! `servo:newtab` (ports/servoshell/desktop/protocols/servo.rs). Compared with
//! the alternatives:
//!
//! - a `file:` URL would need an HTML file shipped next to the executable, and
//!   the address bar would show a path into the install directory;
//! - a `data:` URL would put the whole document in the address bar and the tab
//!   tooltip.
//!
//! A custom scheme gives a short, stable URL that [`is_homepage`] can recognise,
//! so the address bar can show nothing on the homepage, as it does on
//! `about:blank`.

use http::HeaderValue;
use http::header::CONTENT_TYPE;
use servo::protocol_handler::{
    DoneChannel, FetchContext, NetworkError, ProtocolHandler, Request, ResourceFetchTiming,
    Response, ResponseBody,
};
use std::future::{Future, ready};
use std::pin::Pin;

/// Scheme for pages built into the browser. `about:` would be the obvious
/// choice, but Servo refuses to let an embedder register it.
pub const SCHEME: &str = "ferrous";

/// Where new tabs and the first window open.
pub const URL: &str = "ferrous:newtab";

const PAGE: &str = include_str!("homepage.html");

/// Whether `url` is the homepage. Used to keep its internal URL out of the
/// address bar.
pub fn is_homepage(url: &str) -> bool {
    url == URL
}

/// Serves `ferrous:` URLs. Only `newtab` exists; anything else is a network
/// error, which Servo shows as its ordinary load-failure page.
#[derive(Default)]
pub struct Protocol;

impl ProtocolHandler for Protocol {
    fn load<'a>(
        &'a self,
        request: &'a mut Request,
        _done_chan: &mut DoneChannel,
        _context: &FetchContext,
    ) -> Pin<Box<dyn Future<Output = Response> + Send + 'a>> {
        let url = request.current_url();
        if url.path() != "newtab" {
            return Box::pin(ready(Response::network_error(NetworkError::ResourceLoadError(
                format!("no built-in page at {url}"),
            ))));
        }

        let mut response = Response::new(url, ResourceFetchTiming::new(request.timing_type()));
        response
            .headers
            .insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
        *response.body.lock() = ResponseBody::Done(PAGE.as_bytes().to_vec());
        Box::pin(ready(response))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn homepage_url_is_recognised() {
        assert!(is_homepage(URL));
        assert!(!is_homepage("ferrous:newtab2"));
        assert!(!is_homepage("https://servo.org/"));
    }

    #[test]
    fn homepage_url_uses_the_registered_scheme() {
        let url = url::Url::parse(URL).expect("homepage URL must parse");
        assert_eq!(url.scheme(), SCHEME);
        assert_eq!(url.path(), "newtab");
    }

    #[test]
    fn page_is_a_complete_document() {
        assert!(PAGE.starts_with("<!doctype html>"));
        assert!(PAGE.contains("<title>New Tab</title>"));
    }
}
