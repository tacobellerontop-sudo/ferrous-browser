//! Ferrous Browser — a lightweight browser shell around the Servo engine.
//!
//! Milestone 2: a working toolbar (back / forward / reload) plus an
//! address-and-search bar, composed over a live Servo web view.
//!
//! ## How a frame is put together
//!
//! The browser chrome and the web page share ONE OpenGL context. Servo renders
//! into an offscreen framebuffer; egui then blits that framebuffer into the
//! window and draws the toolbar on top. This mirrors servoshell exactly
//! (ports/servoshell/desktop/gui.rs:660-708) and is why no second graphics
//! abstraction is needed.
//!
//! 1. `offscreen.make_current()`
//! 2. egui lays out the toolbar; whatever is left is the content rectangle
//! 3. Servo paints the page into the offscreen framebuffer
//! 4. a `PaintCallback` on the background layer blits it into the window
//! 5. egui paints, so the toolbar composites over the page
//! 6. `window_ctx.present()` swaps

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::ffi::c_void;
use std::rc::Rc;
use std::time::Instant;

use egui::{LayerId, PaintCallback, Panel, Rect};
use egui_glow::EguiGlow;
// euclid geometry, aliased so it is not confused with `egui::Rect` — the blit
// callback needs the integer, bottom-left-origin flavour.
use euclid::{Point2D as EuclidPoint, Rect as EuclidRect, Size2D as EuclidSize};
use log::{error, info};
use servo::{
    DevicePoint, DeviceVector2D, EventLoopWaker, InputEvent, MouseButtonAction, MouseButtonEvent,
    MouseMoveEvent, OffscreenRenderingContext, RenderingContext, Scroll, WebViewPoint,
    WebViewVector, WindowRenderingContext,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey as WinitNamedKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

mod address;
mod browser_state;
mod chrome;
mod engine;
mod icons;
mod input;
mod prefs;
mod scroll;
mod tab;
mod titlebar;

use browser_state::BrowserState;
use chrome::{Action, Chrome};
use engine::WebEngine;
use input::{SCROLL_LINE_HEIGHT, keyboard_event, mouse_button, page_point, window_point};
use scroll::SmoothScroll;
use tab::{CloseOutcome, TabId};

const START_URL: &str = "https://servo.org";

fn main() -> Result<(), Box<dyn Error>> {
    // Required before any TLS traffic; Servo installs no default provider itself.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(&event_loop);
    Ok(event_loop.run_app(&mut app)?)
}

struct AppState {
    /// Shared rather than owned because the Servo delegate also needs it to
    /// request repaints, and winit 0.30's `Window` is not `Clone`.
    window: Rc<Window>,
    /// Context for the window surface. Servo reaches it only via the blit.
    window_ctx: Rc<WindowRenderingContext>,
    /// Context the web view actually draws into.
    offscreen_ctx: Rc<OffscreenRenderingContext>,
    egui: EguiGlow,
    chrome: Chrome,
    state: Rc<BrowserState>,
    /// Owns the Servo runtime and one web view per tab.
    engine: WebEngine,
    modifiers: Cell<ModifiersState>,
    /// Last title pushed to the OS window. Guarding on this avoids calling
    /// `SetWindowText` on every single frame, which is a syscall per frame.
    last_window_title: RefCell<String>,
    /// Where the web view sits within the window, in egui points, recorded
    /// during layout. Cursor coordinates must be translated through this before
    /// they mean anything to Servo, because window origin != page origin.
    content_rect: Cell<Option<Rect>>,
    /// Last cursor position expressed in web-view coordinates. winit reports
    /// wheel deltas without any position, so the most recent one is reused.
    last_page_point: Cell<WebViewPoint>,
    /// Native handle, needed for the frameless-window interop in [`titlebar`].
    hwnd: Cell<*mut c_void>,
    /// Smooth-scroll animation state. Drives redraws only while it is active.
    scroll: SmoothScroll,
    /// Start of the current frame, used to give [`SmoothScroll::step`] a real
    /// delta time. Frame-count-based animation would run 2.4x fast at 144Hz.
    frame_started: Instant,
    /// Mirrors the window's maximised state so the title bar can pick its glyph.
    /// winit can report `is_maximized` but cannot change it.
    maximized: Cell<bool>,
    /// True while a modal Windows drag or resize loop is on the stack.
    ///
    /// Those loops pump messages, so a second press can arrive while the first
    /// is still running. Starting a nested loop is what produced the runaway
    /// CPU spin, so the guard is a hard stop rather than a nicety.
    modal_gesture: Cell<bool>,
    /// Cursor position in egui points within the window, used only for the
    /// frameless resize-border hit test.
    cursor: Cell<Option<egui::Pos2>>,
    /// Last (url, active tab) observed by the frame loop, or `None` before the
    /// first frame, so a navigation or tab switch can be detected from one
    /// comparison instead of threading a flag from every navigation site
    /// (toolbar, link click, redirect, history).
    seen_url: RefCell<Option<(String, TabId)>>,
    /// Set when the last tab is closed from a shortcut. `handle_shortcut` has no
    /// access to the `ActiveEventLoop` that owns exiting, so the request is
    /// carried up to `window_event`.
    close_requested: Cell<bool>,
}

enum App {
    Initial(Waker),
    Running(Box<AppState>),
}

impl App {
    fn new(event_loop: &EventLoop<WakerEvent>) -> Self {
        Self::Initial(Waker::new(event_loop))
    }
}

impl ApplicationHandler<WakerEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initial(waker) = self else {
            return;
        };

        let display_handle = event_loop
            .display_handle()
            .expect("event loop must expose a display handle");
        let window = Rc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Ferrous")
                        .with_inner_size(PhysicalSize::new(1100, 820))
                        // Frameless: `titlebar` draws the bar and supplies the
                        // drag/resize/control behaviour winit is missing.
                        .with_decorations(false),
                )
                .expect("failed to create window"),
        );
        let window_handle = window
            .window_handle()
            .expect("window must expose a window handle");

        // Needed by the frameless-window interop; winit only hands this out
        // through the raw window handle trait.
        let hwnd = {
            use winit::raw_window_handle::{RawWindowHandle, Win32WindowHandle};
            let RawWindowHandle::Win32(Win32WindowHandle { hwnd, .. }) = window_handle.as_raw() else {
                unreachable!("this browser is Windows-only; no HWND available");
            };
            hwnd.get() as *mut c_void
        };

        let window_ctx = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("failed to create rendering context"),
        );
        let offscreen_ctx = Rc::new(window_ctx.offscreen_context(window.inner_size()));

        // egui renders with the SAME glow context Servo uses, so there is no
        // interop and no share-list juggling. See docs/architecture-research.md.
        offscreen_ctx
            .make_current()
            .expect("failed to make GL context current");
        let egui = EguiGlow::new(event_loop, offscreen_ctx.glow_gl_api(), None, None, false);

        let state = Rc::new(BrowserState::default());
        let engine = WebEngine::new(
            offscreen_ctx.clone(),
            window.clone(),
            state.clone(),
            Box::new(waker.clone()),
        );

        // The first tab. Its id comes from the tab model, which is the single
        // source of truth for identity; the engine is told about it afterwards.
        let first = state.tabs_mut().create(START_URL);
        engine.open_tab(
            first,
            Url::parse(START_URL).expect("START_URL must be valid"),
            true,
        );

        info!("ferrous: started");
        *self = Self::Running(Box::new(AppState {
            window,
            window_ctx,
            offscreen_ctx,
            egui,
            chrome: Chrome::new(state.clone()),
            state,
            engine,
            modifiers: Cell::new(ModifiersState::default()),
            last_window_title: RefCell::new(String::new()),
            content_rect: Cell::new(None),
            last_page_point: Cell::new(WebViewPoint::Device(DevicePoint::new(0.0, 0.0))),
            hwnd: Cell::new(hwnd),
            scroll: SmoothScroll::default(),
            frame_started: Instant::now(),
            maximized: Cell::new(false),
            modal_gesture: Cell::new(false),
            cursor: Cell::new(None),
            seen_url: RefCell::new(None),
            close_requested: Cell::new(false),
        }));
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: WakerEvent) {
        if let Self::Running(app) = self {
            app.engine.spin();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(app) = self else {
            return;
        };

        // Servo needs a turn before and after each event; skipping this is how
        // embedders end up with a page that has silently stopped responding.
        app.engine.spin();

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::ModifiersChanged(changed) => {
                app.modifiers.set(changed.state());
                return;
            }
            WindowEvent::CursorMoved { position, .. } => {
                app.cursor.set(Some(window_point(position, app.egui.egui_ctx.pixels_per_point())));
            }
            // The user can maximise or restore from outside our buttons (Win+Up,
            // dragging the window to a screen edge, double-clicking the bar), so
            // the tracked state is refreshed rather than assumed.
            WindowEvent::Resized(_) => {
                let maximized_now = app.window.is_maximized();
                if app.maximized.get() != maximized_now {
                    app.maximized.set(maximized_now);
                    app.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let close_requested = draw_frame(app);
                app.engine.spin();
                if close_requested {
                    event_loop.exit();
                }
                return;
            }
            _ => {}
        }

        // Frameless-window gestures: resize border, then title-bar drag. Both are
        // handled here, before egui and before the page, because there is no
        // widget out there — and both block in a modal Windows loop, so they must
        // not be deferred into a frame.
        //
        // Returning early also swallows the mouse-up event, which Windows
        // consumes as part of its own modal loop.
        if let WindowEvent::MouseInput { state, button, .. } = &event {
            if *state == ElementState::Pressed && *button == WinitMouseButton::Left {
                let started = begin_resize(app) || begin_title_bar_drag(app);
                if started {
                    app.engine.spin();
                    return;
                }
            }
        }

        // Browser shortcuts are policy, so they are matched before egui — which
        // would otherwise happily type "l" into the address bar on Ctrl+L.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if handle_shortcut(app, key, app.modifiers.get()) {
                app.engine.spin();
                // Closing the last tab is a shutdown request, but only
                // `window_event` holds the `ActiveEventLoop` that can exit.
                if app.close_requested.replace(false) {
                    event_loop.exit();
                    return;
                }
                return;
            }
        }

        let response = app.egui.on_window_event(&app.window, &event);

        if !response.consumed {
            forward_to_page(app, &event);
        }

        if matches!(event, WindowEvent::Resized(_)) || response.repaint {
            app.window.request_redraw();
        }

        app.engine.spin();
    }
}

/// One full frame: chrome layout, Servo paint, blit, present.
///
/// Returns true when the user closed the window from the custom title bar.
/// winit 0.30 has no `Window::close`, so the request is carried back up to
/// `window_event`, which owns the `ActiveEventLoop` needed to exit.
///
/// The fields are destructured first so that `egui` can be borrowed mutably at
/// the same time as `engine` and `chrome` — they are disjoint fields, and
/// destructuring is what tells the borrow checker so.
fn draw_frame(app: &mut AppState) -> bool {
    app.offscreen_ctx
        .make_current()
        .expect("make offscreen context current");

    // Frame delta for the smooth-scroll animation. Sampled before destructuring
    // so the timestamp can be both read and reset in one place.
    let now = Instant::now();
    let dt = now.duration_since(app.frame_started);
    app.frame_started = now;

    // Apply engine callbacks before anything reads the tab model. This is the
    // only place the queue is drained, which is what makes it safe: no
    // `RefCell` borrow of the tab set is live at this point, so a callback that
    // arrives mid-frame cannot panic.
    app.state.drain_events();

    // Title-bar commands are collected during the frame but executed *after*
    // `present()`. `SendMessageW(WM_NCLBUTTONDOWN, ...)` opens a modal Windows
    // drag loop that blocks until the gesture ends; running it inside the egui
    // closure would re-enter the event loop with egui's borrow stack half
    // unwound.
    let mut window_commands: Vec<titlebar::WindowCommand> = Vec::new();
    // Set when a chrome action closes the last tab. Declared out here because the
    // egui closure below is what sets it and the code after `present()` reads it.
    let mut close_window = false;

    // `Tabs::active()` indexes directly and would panic on an empty set. Closing
    // the last tab is supposed to shut the window down before another frame runs,
    // but a frame can already be in flight when that happens — so bail out rather
    // than index into nothing. Cheap, and it turns a would-be crash into a no-op.
    if app.state.tabs().is_empty() {
        return true;
    }

    let AppState {
        egui,
        engine,
        chrome,
        offscreen_ctx,
        state: browser,
        window,
        content_rect,
        last_window_title,
        last_page_point,
        scroll,
        maximized,
        seen_url,
        ..
    } = app;

    // `window` is an `Rc<Window>`; deref coercion hands egui the `&Window` it
    // wants without a clone (winit 0.30's Window is not Clone).
    egui.run(&window, |ui| {
        // Title bar first, so the toolbar panel lays out beneath it and the
        // content rectangle reported by the chrome ends up below both.
        //
        // Zero inner margin is load-bearing: `titlebar::is_in_drag_area` hit
        // tests against `drag_rect`, which assumes the painted strip starts at
        // the window origin. With egui's default 8pt margin the two would
        // disagree by 8pt and presses would land slightly off.
        let frame = egui::Frame::new()
            .fill(ui.visuals().panel_fill)
            .inner_margin(egui::Margin::ZERO)
            .outer_margin(egui::Margin::ZERO);
        Panel::top("titlebar").frame(frame).show_inside(ui, |ui| {
            let page_title = browser.active_tab().title.clone();
            let label = if page_title.is_empty() {
                "Ferrous".to_owned()
            } else {
                page_title
            };
            window_commands.extend(titlebar::draw(ui, &label, maximized.get()));
        });

        // One borrow of the tab set for the whole chrome draw. `BrowserState`
        // holds it behind a `RefCell`, which permits only one borrow at a time,
        // so per-widget borrows would collide — and a borrow held past this block
        // would panic the moment anything below read the model again.
        //
        // `len` is checked here because this is the one place that would
        // otherwise index into an empty tab set.
        let output = {
            let mut tabs = browser.tabs_mut();
            assert!(tabs.len() >= 1, "tab set emptied mid-frame");
            chrome.draw(ui, &mut tabs)
        };
        content_rect.set(Some(output.content_rect));

        // Whatever the toolbar did not claim belongs to the page.
        let content = output.content_rect;
        let ppp = ui.ctx().pixels_per_point();
        let width = (content.width() * ppp).round().max(1.0) as u32;
        let height = (content.height() * ppp).round().max(1.0) as u32;
        engine.resize(width, height);

        // Servo renders into the offscreen framebuffer here.
        engine.paint();

        // Drop an in-flight glide when the page or the active tab changes.
        // Scrolling the momentum of the page the user just left onto the page
        // they just opened reads as a glitch, and a new document starts at the
        // top anyway.
        {
            let (current, active) = {
                let tabs = browser.tabs();
                (tabs.active().url.clone(), tabs.active().id)
            };
            let seen = (current, active);
            if seen_url.borrow().as_ref() != Some(&seen) {
                scroll.cancel();
                *seen_url.borrow_mut() = Some(seen);
            }
        }

        // Deliver one slice of any in-flight wheel glide.
        if let Some(delta) = scroll.step(dt) {
            engine.notify_scroll_event(
                Scroll::Delta(WebViewVector::Device(DeviceVector2D::new(delta.x, delta.y))),
                last_page_point.get(),
            );
        }

        for action in output.actions {
            if apply_action(engine, chrome, browser, action) {
                close_window = true;
            }
        }

        // Closing the last tab empties the tab set, and every accessor on it
        // indexes directly, so nothing may read the model after that point.
        // `apply_action` is deliberately the last thing that touches it.
        //
        // `address_focused` is a plain `Cell` and stays safe; only `active_tab`
        // needs guarding.
        engine.set_focused(!browser.address_focused.get());

        if !close_window {
            // Reflect the page title in the OS window title bar, but only when it
            // actually changed: `set_title` is a syscall and this runs every frame.
            let page_title = browser.active_tab().title.clone();
            if !page_title.is_empty() {
                let mut last = last_window_title.borrow_mut();
                if *last != page_title {
                    window.set_title(&format!("{page_title} - Ferrous"));
                    *last = page_title.clone();
                }
            }
        }
        // Blit the page into the window, underneath everything egui draws. The
        // background layer is painted first, so the toolbar composites on top.
        if let Some(render_to_parent) = offscreen_ctx.render_to_parent_callback() {
            ui.ctx().layer_painter(LayerId::background()).add(PaintCallback {
                rect: content,
                callback: std::sync::Arc::new(egui_glow::CallbackFn::new(
                    move |info, painter| {
                        // The destination rect is in *device pixels*, as a
                        // bottom-left-origin euclid rect — not egui points.
                        // Matches servoshell gui.rs:669-677.
                        let clip = info.viewport_in_pixels();
                        render_to_parent(
                            painter.gl(),
                            EuclidRect::new(
                                EuclidPoint::new(clip.left_px, clip.from_bottom_px),
                                EuclidSize::new(clip.width_px, clip.height_px),
                            ),
                        );
                    },
                )),
            });
        }
    });

    // Keep the loop awake only while a glide is actually in flight. Without this
    // the easing tail would stall; without the `is_animating` guard the browser
    // would spin at full frame rate while idle, which is exactly the cost this
    // project is trying to avoid.
    if app.scroll.is_animating() {
        app.window.request_redraw();
    }

    // Draw the window surface and swap.
    app.window_ctx.prepare_for_rendering();
    app.egui.paint(&app.window);
    app.window_ctx.present();

    for command in window_commands {
        if apply_window_command(app, command) {
            return true;
        }
    }
    close_window
}

/// Carry out a window (not page) command. None of these block, so unlike the
/// drag they can safely be deferred to the end of the frame.
fn apply_window_command(app: &AppState, command: titlebar::WindowCommand) -> bool {
    let hwnd = app.hwnd.get();
    match command {
        titlebar::WindowCommand::Minimize => app.window.set_minimized(true),
        titlebar::WindowCommand::ToggleMaximize => {
            let next = !app.maximized.get();
            titlebar::set_maximized(hwnd, next);
            app.maximized.set(next);
        }
        titlebar::WindowCommand::Close => return true,
    }
    false
}

/// Run `gesture`, a modal Windows drag-or-resize loop, exactly once at a time.
///
/// `WM_NCLBUTTONDOWN` does not return until the gesture ends, and pumps messages
/// while it waits. Guarding re-entry here is what keeps one drag from becoming an
/// unbounded nest of them.
fn run_modal_gesture<T>(app: &AppState, gesture: impl FnOnce() -> T) -> Option<T> {
    if app.modal_gesture.replace(true) {
        return None;
    }
    let result = gesture();
    app.modal_gesture.set(false);
    Some(result)
}

/// If the cursor is on the frameless resize border, start a native resize and
/// report that the event was consumed.
fn begin_resize(app: &AppState) -> bool {
    // A maximised window has no edges to grab; the border would just confuse.
    if app.maximized.get() {
        return false;
    }
    let Some(pos) = app.cursor.get() else {
        return false;
    };
    let Some(edge) = titlebar::resize_edge_at(pos, app.egui.egui_ctx.content_rect()) else {
        return false;
    };
    let hwnd = app.hwnd.get();
    run_modal_gesture(app, || {
        // SAFETY: `hwnd` came from winit and the window is still alive.
        unsafe { titlebar::resize(hwnd, edge) };
    });
    true
}

/// If the press landed on the title-bar drag strip, start a native drag and
/// report that the event was consumed.
///
/// Hit tested geometrically rather than through an egui widget on purpose. An
/// egui drag widget queues a command on every frame it is dragged, and because
/// the native drag pumps messages that re-enters the frame loop — each pass
/// queueing another. Windows holds the mouse for the duration, so the release
/// event never arrives and the nesting never unwinds.
fn begin_title_bar_drag(app: &AppState) -> bool {
    let Some(pos) = app.cursor.get() else {
        return false;
    };
    let width = app.egui.egui_ctx.content_rect().width();
    if !titlebar::is_in_drag_area(pos, width) {
        return false;
    }
    let hwnd = app.hwnd.get();
    run_modal_gesture(app, || {
        // SAFETY: `hwnd` came from winit and the window is still alive.
        unsafe { titlebar::drag_title_bar(hwnd) };
    });
    true
}

/// Carry out a chrome action. Returns true when the caller should shut the
/// window down, which happens only when the last tab is closed.
fn apply_action(
    engine: &WebEngine,
    _chrome: &mut Chrome,
    state: &BrowserState,
    action: Action,
) -> bool {
    match action {
        Action::Navigate(input) => {
            let url = address::resolve(&input);
            state.note_requested_navigation(url.as_str());
            engine.navigate(url);
        }
        Action::Back => engine.go_back(),
        Action::Forward => engine.go_forward(),
        Action::Reload => engine.reload(),
        Action::NewTab => open_new_tab(engine, state),
        Action::SelectTab(id) => {
            // `select` returns false for a tab that closed between the click and
            // here, which is not an error.
            if state.tabs_mut().select(id) {
                engine.activate(id);
            }
        }
        Action::CloseTab(id) => return close_tab(engine, state, id),
    }
    false
}

/// Open a blank-ish tab and make it active.
///
/// The new tab starts on the start page rather than `about:blank`, because
/// `about:blank` is Servo's empty document and reads as a failed load.
fn open_new_tab(engine: &WebEngine, state: &BrowserState) {
    let id = state.tabs_mut().create(START_URL);
    engine.open_tab(
        id,
        Url::parse(START_URL).expect("START_URL must be valid"),
        true,
    );
}

/// Close a tab and activate whichever one took its place.
///
/// Returns true when that was the last tab, so the caller can shut down.
fn close_tab(engine: &WebEngine, state: &BrowserState, id: TabId) -> bool {
    // The engine goes first: dropping the web view is what actually frees it, and
    // doing that before the model forgets the tab means any callback still in
    // flight has a tab to land on rather than being dropped.
    engine.close_tab(id);
    match state.tabs_mut().close(id) {
        CloseOutcome::ClosedLast => true,
        CloseOutcome::Closed(active) => {
            engine.activate(active);
            false
        }
    }
}

/// Browser-level keyboard shortcuts. Returns true when handled, meaning the
/// event must not reach the page.
fn handle_shortcut(
    app: &mut AppState,
    event: &winit::event::KeyEvent,
    mods: ModifiersState,
) -> bool {
    let pressed = event.state == ElementState::Pressed;
    let primary = mods.control_key() || mods.super_key();
    let is_char = |c: char| match &event.logical_key {
        WinitKey::Character(k) if k.eq_ignore_ascii_case(&c.to_string()) => true,
        _ => false,
    };

    // Ctrl/Cmd+L — focus address bar.
    if primary && is_char('l') {
        if pressed {
            app.chrome.request_focus_address_bar();
            app.window.request_redraw();
        }
        return true;
    }

    // Ctrl/Cmd+R or F5 — reload.
    if (primary && is_char('r')) || matches!(event.logical_key, WinitKey::Named(WinitNamedKey::F5))
    {
        if pressed {
            app.engine.reload();
        }
        return true;
    }

    // Alt+Left / Alt+Right — history. Deliberately not Ctrl+Left/Right, which
    // web content uses for word-wise caret movement.
    let nav = match event.logical_key {
        WinitKey::Named(WinitNamedKey::ArrowLeft) if mods.alt_key() => Some(Action::Back),
        WinitKey::Named(WinitNamedKey::ArrowRight) if mods.alt_key() => Some(Action::Forward),
        _ => None,
    };
    if let Some(action) = nav {
        if pressed {
            match action {
                Action::Back => app.engine.go_back(),
                Action::Forward => app.engine.go_forward(),
                _ => {}
            }
        }
        return true;
    }

    // Tab shortcuts. All of these must be matched here, above the web view, or
    // the page would receive them and do something entirely unrelated.
    if primary {
        if is_char('t') {
            if pressed {
                open_new_tab(&app.engine, &app.state);
                app.window.request_redraw();
            }
            return true;
        }
        if is_char('w') {
            if pressed {
                let id = app.state.tabs().active().id;
                if close_tab(&app.engine, &app.state, id) {
                    app.close_requested.set(true);
                }
                app.window.request_redraw();
            }
            return true;
        }
        if matches!(event.logical_key, WinitKey::Named(WinitNamedKey::Tab)) {
            if pressed {
                // Ctrl+Shift+Tab goes backwards, matching every other browser.
                let delta = if mods.shift_key() { -1 } else { 1 };
                if let Some(id) = app.state.tabs_mut().select_offset(delta) {
                    app.engine.activate(id);
                }
                app.window.request_redraw();
            }
            return true;
        }
    }

    false
}

/// Wheel events need a position but winit reports none, so the last known cursor
/// position is reused.
fn last_page_point(app: &AppState) -> WebViewPoint {
    app.last_page_point.get()
}

/// Route an input event to the web page, translating out of the toolbar area.
fn forward_to_page(app: &mut AppState, event: &WindowEvent) {
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let point = page_point(*position, app.egui.egui_ctx.pixels_per_point(), app.content_rect.get());
            app.last_page_point.set(point);
            app.engine
                .notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point)));
        }
        WindowEvent::MouseInput { state, button, .. } => {
            app.engine
                .notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(
                    if *state == ElementState::Pressed {
                        MouseButtonAction::Down
                    } else {
                        MouseButtonAction::Up
                    },
                    mouse_button(*button),
                    last_page_point(app),
                )));
        }
        WindowEvent::MouseWheel { delta, .. } => {
            // winit gives f32 line deltas; Servo's WheelDelta is f64.
            let (x, y) = match *delta {
                MouseScrollDelta::LineDelta(dx, dy) => (
                    f64::from(dx) * SCROLL_LINE_HEIGHT,
                    f64::from(dy) * SCROLL_LINE_HEIGHT,
                ),
                MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
            };

            // Queued for animation rather than sent as `InputEvent::Wheel`. A
            // wheel event is one instantaneous jump in Servo, which is what
            // makes native scrolling feel like a series of hops.
            //
            // `scroll_delta_from_wheel` inverts the sign; see the note there.
            // Passing winit's value straight through scrolls the page backwards.
            let delta = scroll::scroll_delta_from_wheel(x, y);
            if app.scroll.push(delta) {
                app.window.request_redraw();
            } else {
                // Too small to be worth a 200ms glide, so pass it straight
                // through and the scroll lands on this very frame.
                app.engine.notify_scroll_event(
                    Scroll::Delta(WebViewVector::Device(DeviceVector2D::new(delta.x, delta.y))),
                    last_page_point(app),
                );
            }
        }
        WindowEvent::KeyboardInput { event, .. } => {
            // Never steal keys while the address bar has them.
            if app.state.address_focused.get() {
                return;
            }
            let mods = app.modifiers.get();
            app.engine
                .notify_input_event(InputEvent::Keyboard(keyboard_event(event, mods)));
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Event loop waker
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct WakerEvent;

/// Servo runs on its own threads and needs to interrupt a blocked winit loop.
/// This is the entire contract: `clone_box` + `wake`
/// (components/shared/embedder/lib.rs:239).
#[derive(Clone)]
struct Waker(winit::event_loop::EventLoopProxy<WakerEvent>);

impl Waker {
    fn new(event_loop: &EventLoop<WakerEvent>) -> Self {
        Self(event_loop.create_proxy())
    }
}

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        if let Err(err) = self.0.send_event(WakerEvent) {
            error!("failed to wake the event loop: {err}");
        }
    }
}