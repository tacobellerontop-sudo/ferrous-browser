//! Milestone 1 spike — deliberately minimal, deliberately throwaway.
//!
//! PURPOSE: prove, on this machine, that Servo can be built, embedded, and made
//! to render actual pixels inside a window. See docs/next-step.md.
//!
//! Every API below was read out of the Servo checkout. Do not "improve" this
//! file — it exists to de-risk the build, then get replaced.
//!
//! Verified references:
//!   components/servo/examples/winit_minimal.rs   (in-tree, canonical)
//!   ports/servoshell/desktop/keyutils.rs         (key translation)
//!   components/shared/embedder/input_events.rs  (event constructors)

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::rc::Rc;

use euclid::Scale;
use log::{error, info};
use servo::{
    Code, DevicePoint, EventLoopWaker, InputEvent, KeyboardEvent, Key, KeyState, Location,
    Modifiers, MouseButton, MouseButtonAction, MouseButtonEvent, MouseMoveEvent, NamedKey,
    RenderingContext, Servo, ServoBuilder, WebView, WebViewBuilder, WebViewDelegate, WebViewPoint,
    WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WinitKey, ModifiersState};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::Window;

const START_URL: &str = "https://servo.org";
/// servoshell uses the same value (ports/servoshell/window.rs:25-27).
const SCROLL_LINE_HEIGHT: f64 = 76.0;

fn main() -> Result<(), Box<dyn Error>> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install rustls crypto provider");

    let event_loop = EventLoop::with_user_event().build()?;
    let mut app = App::new(&event_loop);
    Ok(event_loop.run_app(&mut app)?)
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

struct AppState {
    window: Window,
    servo: Servo,
    rendering_context: Rc<WindowRenderingContext>,
    /// Boxed because a WebView has no useful Default; one webview for now.
    webview: RefCell<Option<WebView>>,
    /// Wheel events need a position; winit reports deltas without one.
    last_pointer: Cell<PhysicalPosition<f64>>,
    /// winit reports modifier state separately from key events, so it has to be
    /// tracked or Shift/Ctrl would never reach the page.
    ///
    /// `Cell` because `AppState` lives in an `Rc` shared with the delegate, which
    /// Servo requires to be `Rc<dyn WebViewDelegate>`; plain fields would be
    /// immutable through that handle.
    modifiers: Cell<ModifiersState>,
}

/// The single delegate. Servo calls this when a frame is ready; the only thing
/// we do is ask winit for a redraw. Painting happens in `window_event`.
impl WebViewDelegate for AppState {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.window.request_redraw();
    }
}

enum App {
    Initial(Waker),
    Running(Rc<AppState>),
}

impl App {
    fn new(event_loop: &EventLoop<WakerEvent>) -> Self {
        Self::Initial(Waker::new(event_loop))
    }
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

impl ApplicationHandler<WakerEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let Self::Initial(waker) = self else {
            return;
        };

        let display_handle = event_loop
            .display_handle()
            .expect("event loop must expose a display handle");
        let window = event_loop
            .create_window(Window::default_attributes().with_title("Ferrous spike"))
            .expect("failed to create window");
        let window_handle = window
            .window_handle()
            .expect("window must expose a window handle");

        let rendering_context = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("failed to create rendering context"),
        );
        rendering_context
            .make_current()
            .expect("failed to make GL context current");

        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(waker.clone()))
            .build();
        servo.setup_logging();

        let state = Rc::new(AppState {
            window,
            servo,
            rendering_context,
            webview: RefCell::new(None),
            last_pointer: Cell::new(PhysicalPosition::new(0.0, 0.0)),
            modifiers: Cell::new(ModifiersState::default()),
        });

        let url = Url::parse(START_URL).expect("START_URL must be a valid URL");
        let webview = WebViewBuilder::new(&state.servo, state.rendering_context.clone())
            .url(url)
            .hidpi_scale_factor(Scale::new(state.window.scale_factor() as f32))
            .delegate(state.clone())
            .build();
        *state.webview.borrow_mut() = Some(webview);

        info!("spike: webview created, loading {START_URL}");
        *self = Self::Running(state);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: WakerEvent) {
        if let Self::Running(state) = self {
            state.servo.spin_event_loop();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        let Self::Running(state) = self else {
            return;
        };

        // Servo must be given a chance to run before and after every input event.
        state.servo.spin_event_loop();

        let mut redraw = false;

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(webview) = state.webview.borrow().as_ref() {
                    webview.resize(size);
                }
            }
            WindowEvent::RedrawRequested => redraw = true,
            WindowEvent::ModifiersChanged(changed) => {
                state.modifiers.set(changed.state());
            }
            WindowEvent::CursorMoved { position, .. } => {
                // Deliver immediately. Hover effects (and the cursor changing to
                // a hand over a link) depend on Servo receiving these even when
                // no new frame is pending, so tying this to `redraw` would
                // break the cursor entirely.
                state.last_pointer.set(position);
                let point = page_point(position);
                if let Some(webview) = state.webview.borrow().as_ref() {
                    let _ =
                        webview.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(point)));
                }
            }
            WindowEvent::MouseInput { state: button_state, button, .. } => {
                if let Some(webview) = state.webview.borrow().as_ref() {
                    let _ = webview.notify_input_event(InputEvent::MouseButton(
                        MouseButtonEvent::new(
                            if button_state == ElementState::Pressed {
                                MouseButtonAction::Down
                            } else {
                                MouseButtonAction::Up
                            },
                            mouse_button(button),
                            page_point(state.last_pointer.get()),
                        ),
                    ));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if let Some(webview) = state.webview.borrow().as_ref() {
                    // winit 0.30 yields f32 line deltas, but Servo's WheelDelta carries f64.
                    let (x, y, mode) = match delta {
                        MouseScrollDelta::LineDelta(dx, dy) => (
                            f64::from(dx) * SCROLL_LINE_HEIGHT,
                            f64::from(dy) * SCROLL_LINE_HEIGHT,
                            WheelMode::DeltaLine,
                        ),
                        MouseScrollDelta::PixelDelta(p) => (p.x, p.y, WheelMode::DeltaPixel),
                    };
                    let _ = webview.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                        WheelDelta {
                            x,
                            y,
                            z: 0.0,
                            mode,
                        },
                        page_point(state.last_pointer.get()),
                    )));
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let modifiers = state.modifiers.get();
                if let Some(webview) = state.webview.borrow().as_ref() {
                    let _ =
                        webview.notify_input_event(InputEvent::Keyboard(keyboard_event(&event, modifiers)));
                }
            }
            _ => {}
        }

        if redraw {
            state.rendering_context.make_current().expect("make current");
            if let Some(webview) = state.webview.borrow().as_ref() {
                webview.paint();
            }
            state.rendering_context.present();
        }

        state.servo.spin_event_loop();
    }
}

// ---------------------------------------------------------------------------
// Input translation
// ---------------------------------------------------------------------------

/// Converts a winit cursor position into a point Servo accepts.
///
/// Device pixels relative to the WebView origin, matching servoshell
/// (`desktop/geometry.rs:13` and `headed_window.rs:326`, which passes
/// `point.into()`). Using `WebViewPoint::Page` here would be wrong: page
/// coordinates are CSS pixels after page-zoom and pinch-zoom scaling, and are
/// only equal to window coordinates in the degenerate case of no chrome and
/// zoom 1.0. When a toolbar is added this must additionally subtract the chrome
/// height, exactly as `headed_window.rs:300-301` does.
///
/// The variant is named explicitly rather than using `.into()`: Servo provides
/// two `From<Point2D<f32, _>>` impls for `WebViewPoint` (CSSPixel and
/// DevicePixel), so inference alone is ambiguous.
fn page_point(position: PhysicalPosition<f64>) -> WebViewPoint {
    WebViewPoint::Device(DevicePoint::new(position.x as f32, position.y as f32))
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

fn modifiers_from_winit(mods: ModifiersState) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    modifiers.set(Modifiers::CONTROL, mods.control_key());
    modifiers.set(Modifiers::SHIFT, mods.shift_key());
    modifiers.set(Modifiers::ALT, mods.alt_key());
    modifiers.set(Modifiers::META, mods.super_key());
    modifiers
}

/// Key translation via `FromStr` on the winit `Debug` representation.
///
/// `keyboard_types::Key` is `enum { Character(String), Named(NamedKey) }` and its
/// `FromStr` routes through `is_key_string` (keyboard-types-0.8.3/src/key.rs),
/// which rejects any multi-character ASCII string — so `"Enter"` correctly parses
/// to `Key::Named(NamedKey::Enter)` while `"a"` parses to `Key::Character("a")`.
/// Characters are therefore handled explicitly and only named keys go through
/// `parse`.
///
/// This is a spike shortcut, not the final answer. servoshell carries a ~600
/// line exhaustive match table (ports/servoshell/desktop/keyutils.rs) mapping
/// every `winit::NamedKey` and `winit::KeyCode` variant. Replaced later; noted
/// here so nobody mistakes it for a finished implementation.
fn keyboard_event(event: &winit::event::KeyEvent, mods: ModifiersState) -> KeyboardEvent {
    let key = match &event.logical_key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        other => format!("{other:?}")
            .parse()
            .unwrap_or(Key::Named(NamedKey::Unidentified)),
    };

    let code = format!("{:?}", event.physical_key)
        .parse::<Code>()
        .unwrap_or(Code::Unidentified);

    KeyboardEvent::new_without_event(
        match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        key,
        code,
        Location::Standard,
        modifiers_from_winit(mods),
        event.repeat,
        false,
    )
}

// ---------------------------------------------------------------------------
// Event loop waker
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct WakerEvent;

/// Servo runs on its own threads and needs a way to interrupt a blocked winit
/// loop. This is the entire contract: `clone_box` + `wake`
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