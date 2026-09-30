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

use std::cell::Cell;
use std::error::Error;
use std::rc::Rc;

use egui::{LayerId, PaintCallback, Rect};
use egui_glow::EguiGlow;
// euclid geometry, aliased so it is not confused with `egui::Rect` — the blit
// callback needs the integer, bottom-left-origin flavour.
use euclid::{Point2D as EuclidPoint, Rect as EuclidRect, Size2D as EuclidSize};
use log::{error, info};
use servo::{
    Code, DevicePoint, EventLoopWaker, InputEvent, Key, KeyState, Location, Modifiers,
    MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey,
    OffscreenRenderingContext, RenderingContext, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
    WindowRenderingContext,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState, NamedKey as WinitNamedKey};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

mod address;
mod browser_state;
mod chrome;
mod engine;

use browser_state::BrowserState;
use chrome::{Action, Chrome};
use engine::Engine;

/// servoshell uses the same value (ports/servoshell/window.rs:25-27).
const SCROLL_LINE_HEIGHT: f64 = 76.0;

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
    engine: Engine,
    modifiers: Cell<ModifiersState>,
    /// Where the web view sits within the window, in egui points, recorded
    /// during layout. Cursor coordinates must be translated through this before
    /// they mean anything to Servo, because window origin != page origin.
    content_rect: Cell<Option<Rect>>,
    /// Last cursor position expressed in web-view coordinates. winit reports
    /// wheel deltas without any position, so the most recent one is reused.
    last_page_point: Cell<WebViewPoint>,
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
                        .with_inner_size(PhysicalSize::new(1100, 820)),
                )
                .expect("failed to create window"),
        );
        let window_handle = window
            .window_handle()
            .expect("window must expose a window handle");

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
        let engine = Engine::new(
            offscreen_ctx.clone(),
            window.clone(),
            state.clone(),
            Box::new(waker.clone()),
            Url::parse(START_URL).expect("START_URL must be valid"),
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
            content_rect: Cell::new(None),
            last_page_point: Cell::new(WebViewPoint::Device(DevicePoint::new(0.0, 0.0))),
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
            WindowEvent::RedrawRequested => {
                draw_frame(app);
                app.engine.spin();
                return;
            }
            _ => {}
        }

        // Browser shortcuts are policy, so they are matched before egui — which
        // would otherwise happily type "l" into the address bar on Ctrl+L.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if handle_shortcut(app, key, app.modifiers.get()) {
                app.engine.spin();
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
/// The fields are destructured first so that `egui` can be borrowed mutably at
/// the same time as `engine` and `chrome` — they are disjoint fields, and
/// destructuring is what tells the borrow checker so.
fn draw_frame(app: &mut AppState) {
    app.offscreen_ctx
        .make_current()
        .expect("make offscreen context current");

    let AppState {
        egui,
        engine,
        chrome,
        offscreen_ctx,
        state,
        window,
        content_rect,
        ..
    } = app;

    // `window` is an `Rc<Window>`; deref coercion hands egui the `&Window` it
    // wants without a clone (winit 0.30's Window is not Clone).
    egui.run(&window, |ui| {
        let output = chrome.draw(ui);
        content_rect.set(Some(output.content_rect));

        // Whatever the toolbar did not claim belongs to the page.
        let content = output.content_rect;
        let ppp = ui.ctx().pixels_per_point();
        let width = (content.width() * ppp).round().max(1.0) as u32;
        let height = (content.height() * ppp).round().max(1.0) as u32;
        engine.resize(width, height);

        // Servo renders into the offscreen framebuffer here.
        engine.paint();

        for action in output.actions {
            apply_action(engine, chrome, state, action);
        }

        // Keep the page from stealing keys while the user is typing in the bar.
        engine.set_focused(!state.address_focused.get());

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

    // Draw the window surface and swap.
    app.window_ctx.prepare_for_rendering();
    app.egui.paint(&app.window);
    app.window_ctx.present();
}

fn apply_action(engine: &Engine, chrome: &mut Chrome, state: &BrowserState, action: Action) {
    match action {
        Action::Navigate(input) => {
            let url = address::resolve(&input);
            state.note_requested_navigation(url.as_str());
            engine.navigate(url);
        }
        Action::Back => engine.go_back(),
        Action::Forward => engine.go_forward(),
        Action::Reload => engine.reload(),
        Action::FocusAddressBar => chrome.request_focus_address_bar(),
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

    false
}

/// Route an input event to the web page, translating out of the toolbar area.
fn forward_to_page(app: &AppState, event: &WindowEvent) {
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let point = to_page(app, *position);
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
            let (x, y, mode) = match *delta {
                MouseScrollDelta::LineDelta(dx, dy) => (
                    f64::from(dx) * SCROLL_LINE_HEIGHT,
                    f64::from(dy) * SCROLL_LINE_HEIGHT,
                    WheelMode::DeltaLine,
                ),
                MouseScrollDelta::PixelDelta(p) => (p.x, p.y, WheelMode::DeltaPixel),
            };
            app.engine
                .notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    WheelDelta {
                        x,
                        y,
                        z: 0.0,
                        mode,
                    },
                    last_page_point(app),
                )));
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

/// Translate a window-space cursor position into web-view device pixels.
///
/// The subtraction of the content rectangle is the whole point: without it,
/// clicking 40px into the toolbar would register as a click at (40, 0) of the
/// page. servoshell does the equivalent at headed_window.rs:300-301.
fn to_page(app: &AppState, position: PhysicalPosition<f64>) -> WebViewPoint {
    let Some(rect) = app.content_rect.get() else {
        return WebViewPoint::Device(DevicePoint::new(0.0, 0.0));
    };

    // egui points are top-left origin; window positions are in physical pixels.
    let ppp = app.egui.egui_ctx.pixels_per_point();
    let (wx, wy) = (position.x / ppp as f64, position.y / ppp as f64);
    let x = (wx - rect.min.x as f64) as f32;
    let y = (wy - rect.min.y as f64) as f32;

    WebViewPoint::Device(DevicePoint::new(x, y))
}

/// Wheel events need a position but winit reports none, so the last known cursor
/// position is reused.
fn last_page_point(app: &AppState) -> WebViewPoint {
    app.last_page_point.get()
}

/// Key translation via `FromStr` on the winit `Debug` representation.
///
/// `keyboard_types::Key` is `enum { Character(String), Named(NamedKey) }` and its
/// `FromStr` routes through `is_key_string`, which rejects multi-character ASCII
/// strings — so "Enter" parses to a *named* key rather than a literal character.
/// Characters are therefore handled explicitly.
///
/// This is a shortcut. servoshell carries a ~600 line exhaustive match table
/// (ports/servoshell/desktop/keyutils.rs); a full keyboard milestone should port
/// that rather than lean on Debug formatting.
fn keyboard_event(event: &winit::event::KeyEvent, mods: ModifiersState) -> servo::KeyboardEvent {
    let key = match &event.logical_key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        other => format!("{other:?}")
            .parse()
            .unwrap_or(Key::Named(NamedKey::Unidentified)),
    };

    let code = format!("{:?}", event.physical_key)
        .parse::<Code>()
        .unwrap_or(Code::Unidentified);

    let mut modifiers = Modifiers::empty();
    modifiers.set(Modifiers::CONTROL, mods.control_key());
    modifiers.set(Modifiers::SHIFT, mods.shift_key());
    modifiers.set(Modifiers::ALT, mods.alt_key());
    modifiers.set(Modifiers::META, mods.super_key());

    servo::KeyboardEvent::new_without_event(
        match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        key,
        code,
        Location::Standard,
        modifiers,
        event.repeat,
        false,
    )
}

fn mouse_button(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Primary,
        WinitMouseButton::Right => MouseButton::Secondary,
        WinitMouseButton::Middle => MouseButton::Auxiliary,
        WinitMouseButton::Back => MouseButton::Back,
        WinitMouseButton::Forward => MouseButton::Forward,
        WinitMouseButton::Other(n) => MouseButton::Other(n as u16),
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