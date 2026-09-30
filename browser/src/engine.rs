//! Servo adapter.
//!
//! Everything that knows about Servo types lives here. The UI layer never sees a
//! `WebView`, a `WebViewPoint`, or a delegate; it calls plain methods and gets
//! plain data back. That separation is what makes the browser logic testable
//! without starting an engine.

use std::cell::Cell;
use std::rc::Rc;

use euclid::Scale;
use log::info;
use servo::{
    EventLoopWaker, InputEvent, LoadStatus, RenderingContext, Servo, ServoBuilder, WebView,
    WebViewBuilder, WebViewDelegate,
};
use url::Url;
use winit::window::Window;

use crate::browser_state::BrowserState;

/// Owns the Servo instance and the single `WebView` this milestone supports.
pub struct Engine {
    servo: Servo,
    webview: WebView,
}

impl Engine {
    /// Create the engine and its first webview.
    ///
    /// `rendering_context` is the *offscreen* context, not the window one. That
    /// is what lets the web page occupy only the region below the toolbar: WebRender
    /// draws into an FBO which is then blitted into the window surface under the
    /// egui chrome. Servo's own doc comment recommends this arrangement
    /// (components/shared/paint/rendering_context.rs:391-398), and servoshell
    /// uses it for the same reason (ports/servoshell/desktop/headed_window.rs:186).
    pub fn new(
        rendering_context: Rc<dyn RenderingContext>,
        window: Rc<Window>,
        state: Rc<BrowserState>,
        waker: Box<dyn EventLoopWaker>,
        initial_url: Url,
    ) -> Self {
        let servo = ServoBuilder::default()
            .event_loop_waker(waker)
            .build();
        servo.setup_logging();

        let webview = WebViewBuilder::new(&servo, rendering_context)
            .url(initial_url)
            .hidpi_scale_factor(Scale::new(window.scale_factor() as f32))
            .delegate(Rc::new(EngineDelegate {
                state,
                redraw: RedrawRequest(window),
            }))
            .build();

        info!("engine: webview created");
        Self { servo, webview }
    }

    /// Give Servo a chance to process queued work. Must be called after every
    /// input/navigation call and on every wake-up, or Servo stalls.
    pub fn spin(&self) {
        self.servo.spin_event_loop();
    }

    pub fn navigate(&self, url: Url) {
        info!("engine: navigate to {url}");
        self.webview.load(url);
    }

    pub fn reload(&self) {
        self.webview.reload();
    }

    pub fn go_back(&self) {
        // Traversal is asynchronous; completion arrives via the delegate's
        // notify_history_changed, which is what re-enables the buttons.
        let _traversal_id = self.webview.go_back(1);
    }

    pub fn go_forward(&self) {
        let _traversal_id = self.webview.go_forward(1);
    }

    pub fn can_go_back(&self) -> bool {
        self.webview.can_go_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.webview.can_go_forward()
    }

    pub fn paint(&self) {
        self.webview.paint();
    }

    /// Resize the viewport. Clamped to a 1x1 minimum by Servo itself.
    pub fn resize(&self, width: u32, height: u32) {
        self.webview.resize(winit::dpi::PhysicalSize::new(width, height));
    }

    /// Tell Servo whether the page has keyboard focus. While the address bar is
    /// focused this is false, so typing does not also reach the page.
    pub fn set_focused(&self, focused: bool) {
        self.webview.set_focused(focused);
    }

    pub fn notify_input_event(&self, event: InputEvent) {
        self.webview.notify_input_event(event);
    }

    pub fn page_title(&self) -> String {
        self.webview.page_title().unwrap_or_default()
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

/// Translates Servo's per-webview callbacks into [`BrowserState`] updates.
///
/// Every method Servo exposes here has a default no-op body
/// (components/servo/webview_delegate.rs:918-1092), so only the callbacks this
/// shell actually needs are implemented.
struct EngineDelegate {
    state: Rc<BrowserState>,
    redraw: RedrawRequest,
}

impl WebViewDelegate for EngineDelegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        // Painting only when the engine says there is something new keeps idle
        // CPU near zero. URL and title callbacks do NOT prove anything rendered.
        self.redraw.request();
    }

    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        self.state.set_url(url.to_string());
        self.redraw.request();
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        *self.state.title.borrow_mut() = title.unwrap_or_default();
        self.redraw.request();
    }

    fn notify_history_changed(&self, _webview: WebView, entries: Vec<Url>, current: usize) {
        // `current` indexes the active entry. Everything before it is
        // back-history, everything after it is forward-history.
        let can_go_back = current > 0;
        let can_go_forward = current + 1 < entries.len();
        self.state.can_go_back.set(can_go_back);
        self.state.can_go_forward.set(can_go_forward);
        self.redraw.request();
    }

    fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
        self.state.loading.set(status != LoadStatus::Complete);
        self.redraw.request();
    }
}

/// Tracks the last pointer position so wheel events have somewhere to happen.
///
/// winit reports wheel deltas without a position, but Servo needs one.
#[derive(Default)]
pub struct Pointer {
    pub x: Cell<f32>,
    pub y: Cell<f32>,
}

impl Pointer {
    pub fn set(&self, x: f32, y: f32) {
        self.x.set(x);
        self.y.set(y);
    }
}