//! Servo adapter.
//!
//! Everything that knows about Servo types lives here. The UI layer never sees a
//! `WebView`, a `WebViewPoint`, or a delegate; it calls plain methods and gets
//! plain data back.
//!
//! # One web view per tab, all on one rendering context
//!
//! Each tab gets its own Servo `WebView`, and they all share the single offscreen
//! context. That is what lets a tab keep its own page, history and scroll
//! position while only one of them is ever visible.
//!
//! Only the **active** web view is painted, resized and sent input. The others
//! are hidden. Three facts from the current checkout make this work:
//!
//! * `WebView` is `Rc<RefCell<WebViewInner>>` and derives `Clone`
//!   (`components/servo/webview.rs:84`), so handles are cheap to store.
//! * `WebViewInner` implements `Drop`, which sends `CloseWebView` and removes it
//!   from the painter (`webview.rs:144-151`). Dropping the last clone is how a
//!   tab is genuinely torn down — nothing else closes a web view.
//! * A web view starts **visible**: `WebViewRenderer::hidden` defaults to `false`
//!   (`components/paint/webview_renderer.rs:161`). A background tab therefore has
//!   to be hidden explicitly or it will paint over the active one.
//!
//! `set_throttled` does not exist in this version — it was removed — so hiding is
//! the only lever for making a background tab cheap.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use euclid::Scale;
use log::info;
use servo::{
    DeviceIndependentPixel, DevicePixel, EventLoopWaker, Image, InputEvent, LoadStatus, PixelFormat,
    EmbedderControl, EmbedderControlId, RenderingContext, WebResourceLoad, WebResourceResponse,
    Scroll, Servo, ServoBuilder, WebView, WebViewBuilder, WebViewDelegate, WebViewPoint,
};
use servo::protocol_handler::ProtocolRegistry;
use servo::{Opts, UserContentManager, UserScript};
use url::Url;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::browser_state::{BrowserState, TabEvent};
use crate::homepage;
use crate::tab::{Favicon, TabId};

/// Owns the Servo instance and one `WebView` per tab.
pub struct Engine {
    servo: Servo,
    /// One entry per open tab. Dropping a `WebView` tears it down; see the module
    /// docs for why that matters and why nothing else does it.
    webviews: RefCell<HashMap<TabId, WebView>>,
    /// The web view currently receiving input and being painted.
    active: RefCell<Option<TabId>>,
}

impl Engine {
    /// Create the engine. Tabs are added afterwards with [`Engine::open_tab`].
    ///
    /// `rendering_context` is the *offscreen* context, not the window one. That
    /// is what lets the web page occupy only the region below the toolbar:
    /// WebRender draws into an FBO which is then blitted into the window surface
    /// under the egui chrome. Servo's own doc comment recommends this
    /// arrangement (components/shared/paint/rendering_context.rs:391-398), and
    /// servoshell uses it for the same reason
    /// (ports/servoshell/desktop/headed_window.rs:186).
    /// Give Servo a chance to process queued work. Must be called after every
    /// input/navigation call and on every wake-up, or Servo stalls.
    pub fn spin(&self) {
        self.servo.spin_event_loop();
    }

    /// Tear down a tab's web view.
    ///
    /// Dropping the handle is the whole mechanism — `WebViewInner::drop` sends
    /// `CloseWebView` and removes it from the painter.
    pub fn close_tab(&self, tab: TabId) {
        let removed = self.webviews.borrow_mut().remove(&tab);
        if let Some(webview) = removed {
            // Blur before dropping so the engine does not leave a stale focus.
            webview.set_focused(false);
            webview.hide();
        }
        info!("engine: closed tab {}", tab.get());
    }

    /// Make `tab` the visible, focused, input-receiving one.
    ///
    /// The previous tab is hidden and blurred first. Hiding also tells the
    /// constellation to stop working on it, which is the only background-tab
    /// saving available now that `set_throttled` is gone.
    pub fn activate(&self, tab: TabId) {
        if self.active.borrow().is_some_and(|current| current == tab) {
            return;
        }

        if let Some(previous) = *self.active.borrow() {
            if let Some(webview) = self.webviews.borrow().get(&previous) {
                webview.set_focused(false);
                webview.hide();
            }
        }

        *self.active.borrow_mut() = Some(tab);
        if let Some(webview) = self.webviews.borrow().get(&tab) {
            webview.show();
        }
        info!("engine: activated tab {}", tab.get());
    }

    fn active_webview(&self) -> Option<WebView> {
        let id = (*self.active.borrow())?;
        // Cloning is a refcount bump; the returned handle does not outlive the
        // borrow of the map.
        self.webviews.borrow().get(&id).cloned()
    }

    /// Add a web view to the map. Called by [`WebEngine::open_tab`] after the
    /// view has been hidden.
    fn insert_webview(&mut self, tab: TabId, webview: WebView) {
        self.webviews.borrow_mut().insert(tab, webview);
    }

    pub fn navigate(&self, url: Url) {
        info!("engine: navigate to {url}");
        if let Some(webview) = self.active_webview() {
            webview.load(url);
        }
    }

    pub fn reload(&self) {
        if let Some(webview) = self.active_webview() {
            webview.reload();
        }
    }

    pub fn go_back(&self) {
        // Traversal is asynchronous; completion arrives via the delegate's
        // history event, which is what re-enables the buttons.
        if let Some(webview) = self.active_webview() {
            let _traversal_id = webview.go_back(1);
        }
    }

    pub fn go_forward(&self) {
        if let Some(webview) = self.active_webview() {
            let _traversal_id = webview.go_forward(1);
        }
    }

    pub fn paint(&self) {
        // Only the active web view. A hidden one has no business touching the
        // shared offscreen framebuffer.
        if let Some(webview) = self.active_webview() {
            webview.paint();
        }
    }

    /// Resize the viewport of the active web view.
    ///
    /// Resized through the web view rather than the shared rendering context, so
    /// the engine performs its CSS viewport update — resizing the context
    /// directly would skip that and leave media queries and `vw` units stale.
    pub fn resize(&self, width: u32, height: u32) {
        if let Some(webview) = self.active_webview() {
            webview.resize(PhysicalSize::new(width, height));
        }
    }

    /// Tell Servo whether the page has keyboard focus. While the address bar is
    /// focused this is false, so typing does not also reach the page.
    pub fn set_focused(&self, focused: bool) {
        if let Some(webview) = self.active_webview() {
            webview.set_focused(focused);
        }
    }

    pub fn notify_input_event(&self, event: InputEvent) {
        if let Some(webview) = self.active_webview() {
            webview.notify_input_event(event);
        }
    }

    /// Scroll the page by a device-pixel delta.
    ///
    /// Separate from `InputEvent::Wheel` because Servo has no notion of an
    /// animated wheel: `Scroll` only offers `Delta`/`Start`/`End`
    /// (components/shared/embedder/lib.rs:173), so each call moves the page one
    /// instantaneous step. [`crate::scroll::SmoothScroll`] turns one wheel notch
    /// into a short sequence of these.
    pub fn notify_scroll_event(&self, scroll: Scroll, point: WebViewPoint) {
        if let Some(webview) = self.active_webview() {
            webview.notify_scroll_event(scroll, point);
        }
    }
}

/// The engine plus the collaborators every tab needs to build a web view.
///
/// Split out from [`Engine`] because `open_tab` needs the rendering context, the
/// shared state and the window, while the rest of the engine does not. Keeping
/// them here means `Engine` stays a plain struct that a `RefCell` can own.
/// A page text field that has asked for the system input method, and where it
/// is: device pixels, relative to the page.
#[derive(Debug, Clone, Copy)]
struct PageIme {
    id: EmbedderControlId,
    area: [i32; 4],
}

pub struct WebEngine {
    engine: RefCell<Engine>,
    interventions: Rc<UserContentManager>,
    /// Shared with every tab's delegate, which sets and clears it.
    page_ime: Rc<Cell<Option<PageIme>>>,
    rendering_context: Rc<dyn RenderingContext>,
    state: Rc<BrowserState>,
    window: Rc<Window>,
    scale_factor: Scale<f32, DeviceIndependentPixel, DevicePixel>,
}

impl WebEngine {
    /// Create the engine. Tabs are added afterwards with [`WebEngine::open_tab`].
    ///
    /// `rendering_context` is the *offscreen* context, not the window one. That
    /// is what lets the web page occupy only the region below the toolbar:
    /// WebRender draws into an FBO which is then blitted into the window surface
    /// under the egui chrome. Servo's own doc comment recommends this
    /// arrangement (components/shared/paint/rendering_context.rs:391-398), and
    /// servoshell uses it for the same reason
    /// (ports/servoshell/desktop/headed_window.rs:186).
    pub fn new(
        rendering_context: Rc<dyn RenderingContext>,
        window: Rc<Window>,
        state: Rc<BrowserState>,
        waker: Box<dyn EventLoopWaker>,
    ) -> Self {
        // `ferrous:` pages (the homepage) are served from the binary. Servo
        // merges this registry with its own `data:`/`blob:`/`file:` handlers.
        let mut protocols = ProtocolRegistry::default();
        protocols
            .register(homepage::SCHEME, homepage::Protocol)
            .expect("ferrous: is not a scheme Servo reserves");

        // Site data lives in the profile folder. Without a config directory
        // Servo writes it relative to the working directory: running the
        // browser from the repository left 11 MB of YouTube IndexedDB there.
        let profile = crate::storage::profile_dir();
        if profile.is_none() {
            log::warn!("no profile directory; site data will not persist");
        }
        let opts = Opts {
            temporary_storage: profile.is_none(),
            config_dir: profile,
            ..Opts::default()
        };

        let servo = ServoBuilder::default()
            .opts(opts)
            .preferences(crate::prefs::web_compat())
            .protocol_registry(protocols)
            .event_loop_waker(waker)
            .build();
        servo.setup_logging();

        // Site interventions (see `compat`), shared by every tab. Each script
        // checks its own host, so they cost nothing on other sites.
        let interventions = Rc::new(UserContentManager::new(&servo));
        for intervention in crate::compat::INTERVENTIONS {
            info!("compat: intervention for {:?}: {}", intervention.hosts, intervention.reason);
            interventions.add_script(Rc::new(UserScript::new(crate::compat::script(intervention), None)));
        }

        WebEngine {
            scale_factor: Scale::new(window.scale_factor() as f32),
            engine: RefCell::new(Engine {
                servo,
                webviews: RefCell::new(HashMap::new()),
                active: RefCell::new(None),
            }),
            page_ime: Rc::new(Cell::new(None)),
            interventions,
            rendering_context,
            state,
            window,
        }
    }

    /// Create a web view for `tab` and load `url` into it.
    ///
    /// Lives here rather than on [`Engine`] because building a web view needs the
    /// rendering context, the shared state, the window and the scale factor —
    /// none of which the engine itself holds.
    pub fn open_tab(&self, tab: TabId, url: Url, make_active: bool) {
        let webview = {
            let engine = self.engine.borrow();
            WebViewBuilder::new(&engine.servo, self.rendering_context.clone())
                .user_content_manager(self.interventions.clone())
                .url(url)
                .hidpi_scale_factor(self.scale_factor)
                .delegate(Rc::new(TabDelegate {
                    state: self.state.clone(),
                    redraw: RedrawRequest(self.window.clone()),
                    tab,
                    page_ime: self.page_ime.clone(),
                }))
                .build()
        };

        // Servo starts web views *visible*, so hide it before it can paint over
        // whatever tab is actually on screen.
        webview.hide();
        self.engine.borrow_mut().insert_webview(tab, webview);

        if make_active {
            self.activate(tab);
        }
        info!("engine: opened tab {}", tab.get());
    }

    pub fn close_tab(&self, tab: TabId) {
        self.engine.borrow().close_tab(tab);
    }

    pub fn activate(&self, tab: TabId) {
        self.engine.borrow().activate(tab);
    }

    pub fn spin(&self) {
        self.engine.borrow().spin();
    }

    pub fn navigate(&self, url: Url) {
        self.engine.borrow().navigate(url);
    }

    pub fn reload(&self) {
        self.engine.borrow().reload();
    }

    pub fn go_back(&self) {
        self.engine.borrow().go_back();
    }

    pub fn go_forward(&self) {
        self.engine.borrow().go_forward();
    }

    pub fn paint(&self) {
        self.engine.borrow().paint();
    }

    /// Where the page's focused text field is, `[x, y, width, height]` in device
    /// pixels relative to the page, while it wants the system input method.
    pub fn page_ime_area(&self) -> Option<[i32; 4]> {
        self.page_ime.get().map(|ime| ime.area)
    }

    /// Forward an IME event to the page.
    ///
    /// `Ime::Disabled` arrives both when the user dismisses the IME and when
    /// the browser turns it off itself (focus moved on). Only the first should
    /// reach the page, as a dismissal that blurs the field; servoshell makes
    /// the same distinction by whether an input method is still expected.
    pub fn notify_ime(&self, ime: &winit::event::Ime) {
        let event = match crate::input::composition_event(ime) {
            Some(event) => event,
            None if self.page_ime.take().is_some() => servo::ImeEvent::Dismissed,
            None => return,
        };
        self.notify_input_event(InputEvent::Ime(event));
    }

    /// Whether `tab` already has a web view. Tabs restored from the last session
    /// do not, until they are first shown.
    pub fn has_tab(&self, tab: TabId) -> bool {
        self.engine.borrow().webviews.borrow().contains_key(&tab)
    }

    /// Set the active tab's page zoom; 1.0 is 100%.
    pub fn set_page_zoom(&self, zoom: f32) {
        if let Some(webview) = self.engine.borrow().active_webview() {
            webview.set_page_zoom(zoom);
        }
    }

    pub fn resize(&self, width: u32, height: u32) {
        self.engine.borrow().resize(width, height);
    }

    pub fn set_focused(&self, focused: bool) {
        self.engine.borrow().set_focused(focused);
    }

    pub fn notify_input_event(&self, event: InputEvent) {
        self.engine.borrow().notify_input_event(event);
    }

    pub fn notify_scroll_event(&self, scroll: Scroll, point: WebViewPoint) {
        self.engine.borrow().notify_scroll_event(scroll, point);
    }
}

/// Requests a repaint of the OS window. Held by the delegate so that a new frame
/// arriving from a Servo thread can wake the event loop.
///
/// `Rc<Window>` rather than `Window`: winit 0.30 removed `impl Clone for Window`
/// (it existed in 0.29), so the window is shared rather than copied.
struct RedrawRequest(Rc<Window>);

impl RedrawRequest {
    fn request(&self) {
        self.0.request_redraw();
    }
}

/// Translates Servo's callbacks for **one** web view into queued events.
///
/// The `tab` field is the whole point. Servo delivers these asynchronously on its
/// own threads; without knowing which tab produced a callback, a title that
/// arrives after the user switched tabs would be applied to the wrong one.
///
/// Every method Servo exposes here has a default no-op body
/// (components/servo/webview_delegate.rs:918-1092), so only the callbacks this
/// shell needs are implemented.
struct TabDelegate {
    state: Rc<BrowserState>,
    redraw: RedrawRequest,
    tab: TabId,
    page_ime: Rc<Cell<Option<PageIme>>>,
}

impl WebViewDelegate for TabDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        // Painting only when the engine says there is something new keeps idle
        // CPU near zero. URL and title callbacks do NOT prove anything rendered.
        self.state.mark_needs_paint();
        self.redraw.request();
    }

    /// Every request a page makes passes through here first. Ones the blocker
    /// matches are cancelled, which the page sees as a network error — the same
    /// as an extension-based blocker. Anything not intercepted continues
    /// untouched once `load` is dropped.
    fn load_web_resource(&self, webview: WebView, load: WebResourceLoad) {
        let request = load.request();
        let page = webview.url();
        if self.state.blocker.should_block(&request.url, page.as_ref(), request.is_for_main_frame) {
            log::debug!("blocked {}", request.url);
            let url = request.url.clone();
            load.intercept(WebResourceResponse::new(url)).cancel();
            self.state.push_event(TabEvent::RequestBlocked { tab: self.tab });
            self.redraw.request();
        }
    }

    /// A page text field took focus and wants the system input method. Other
    /// controls (select pickers, dialogs) are left to their defaults for now.
    fn show_embedder_control(&self, _webview: WebView, control: EmbedderControl) {
        if let EmbedderControl::InputMethod(input) = control {
            let rect = input.position();
            self.page_ime.set(Some(PageIme {
                id: input.id(),
                area: [rect.min.x, rect.min.y, rect.width(), rect.height()],
            }));
            self.redraw.request();
        }
    }

    fn hide_embedder_control(&self, _webview: WebView, id: EmbedderControlId) {
        if self.page_ime.get().is_some_and(|ime| ime.id == id) {
            self.page_ime.set(None);
            self.redraw.request();
        }
    }

    fn notify_favicon_changed(&self, webview: WebView) {
        let favicon = webview.favicon().and_then(|image| favicon_from(&image));
        self.state.push_event(TabEvent::FaviconChanged { tab: self.tab, favicon });
        self.redraw.request();
    }

    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        self.state.push_event(TabEvent::UrlChanged {
            tab: self.tab,
            url: url.to_string(),
        });
        self.redraw.request();
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        self.state.push_event(TabEvent::TitleChanged {
            tab: self.tab,
            title: title.unwrap_or_default(),
        });
        self.redraw.request();
    }

    fn notify_history_changed(&self, _webview: WebView, entries: Vec<Url>, current: usize) {
        // `current` indexes the active entry. Everything before it is
        // back-history, everything after it is forward-history.
        self.state.push_event(TabEvent::HistoryChanged {
            tab: self.tab,
            can_go_back: current > 0,
            can_go_forward: current + 1 < entries.len(),
        });
        self.redraw.request();
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        self.state.push_event(TabEvent::LoadStatus {
            tab: self.tab,
            loading: status != LoadStatus::Complete,
        });
        self.redraw.request();
    }
}

/// Convert Servo's favicon to the engine-neutral model.
///
/// Only the first frame of an animated icon is used, which is what browsers
/// show in a tab anyway. Each conversion gets a fresh version number so the UI
/// knows to replace its texture.
fn favicon_from(image: &Image) -> Option<Favicon> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_VERSION: AtomicU64 = AtomicU64::new(1);

    let rgba = to_rgba(image.format, image.width, image.height, image.data())?;
    Some(Favicon {
        width: image.width,
        height: image.height,
        rgba: rgba.into(),
        version: NEXT_VERSION.fetch_add(1, Ordering::Relaxed),
    })
}

/// Expand any of Servo's pixel formats to straight RGBA8. Returns `None` if
/// `data` is too short for the stated size, rather than reading past it.
fn to_rgba(format: PixelFormat, width: u32, height: u32, data: &[u8]) -> Option<Vec<u8>> {
    let pixels = (width as usize).checked_mul(height as usize)?;
    if pixels == 0 {
        return None;
    }
    let channels = match format {
        PixelFormat::K8 => 1,
        PixelFormat::KA8 => 2,
        PixelFormat::RGB8 => 3,
        PixelFormat::RGBA8 | PixelFormat::BGRA8 => 4,
    };
    let data = data.get(..pixels.checked_mul(channels)?)?;
    let mut rgba = Vec::with_capacity(pixels * 4);
    for px in data.chunks_exact(channels) {
        let [r, g, b, a] = match format {
            PixelFormat::K8 => [px[0], px[0], px[0], 255],
            PixelFormat::KA8 => [px[0], px[0], px[0], px[1]],
            PixelFormat::RGB8 => [px[0], px[1], px[2], 255],
            PixelFormat::RGBA8 => [px[0], px[1], px[2], px[3]],
            PixelFormat::BGRA8 => [px[2], px[1], px[0], px[3]],
        };
        rgba.extend_from_slice(&[r, g, b, a]);
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pixel_format_becomes_rgba() {
        assert_eq!(to_rgba(PixelFormat::K8, 1, 1, &[7]), Some(vec![7, 7, 7, 255]));
        assert_eq!(to_rgba(PixelFormat::KA8, 1, 1, &[7, 9]), Some(vec![7, 7, 7, 9]));
        assert_eq!(to_rgba(PixelFormat::RGB8, 1, 1, &[1, 2, 3]), Some(vec![1, 2, 3, 255]));
        assert_eq!(to_rgba(PixelFormat::RGBA8, 1, 1, &[1, 2, 3, 4]), Some(vec![1, 2, 3, 4]));
        assert_eq!(to_rgba(PixelFormat::BGRA8, 1, 1, &[3, 2, 1, 4]), Some(vec![1, 2, 3, 4]));
    }

    #[test]
    fn short_or_empty_data_is_rejected_not_overread() {
        assert_eq!(to_rgba(PixelFormat::RGBA8, 2, 2, &[0; 15]), None);
        assert_eq!(to_rgba(PixelFormat::RGBA8, 0, 0, &[]), None);
    }

    #[test]
    fn trailing_frames_are_ignored() {
        // An animated icon carries more frames after the first.
        let two_frames = [1, 2, 3, 4, 9, 9, 9, 9];
        assert_eq!(to_rgba(PixelFormat::RGBA8, 1, 1, &two_frames), Some(vec![1, 2, 3, 4]));
    }
}
