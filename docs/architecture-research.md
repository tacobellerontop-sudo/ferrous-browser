# Servo Embedding Architecture — Research

**Method:** every API claim below was read directly from the Servo checkout at
`servo/` (commit `b2ef57f642816e77e67e5a322ddccdbde6784f3c`). Where a claim comes
from a third-party blog, old tutorial, or the Hermes project, it is marked
**UNVERIFIED** or **CONTRADICTED**. Nothing here is from memory.

Paths are relative to `servo/`.

---

## 1. Checkout facts

| Field | Value | Source |
| --- | --- | --- |
| Commit | `b2ef57f642816e77e67e5a322ddccdbde6784f3c` | `git log -1` |
| Commit date | 2026-09-30 | `git log -1` |
| Workspace version | `0.6.0` | `Cargo.toml:19` |
| Rust edition | `2024` | `Cargo.toml:23` |
| MSRV | `1.88.0` | `Cargo.toml:27` |
| Pinned toolchain | `1.97.1` (with `clippy`, `llvm-tools`, `rustc-dev`, `rustfmt`, `rust-src`) | `rust-toolchain.toml` |
| Python (build) | `3.11` | `.python-version` |
| Working tree | clean (0 modified files) | `git status --porcelain` |

Note: our default installed toolchain is `stable` = `rustc 1.98.1`. Servo pins
`1.97.1`, which **is already installed** locally. Anything built inside `servo/`
will use `1.97.1` automatically via `rust-toolchain.toml`.

---

## 2. Relevant crates and ports

| Path | Crate name | Why it matters |
| --- | --- | --- |
| `components/servo/` | `servo` | **The embedder API.** This is the only crate we should depend on. |
| `ports/servoshell/` | `servoshell` | Upstream reference browser. Source of truth for real-world usage. |
| `components/servo/examples/winit_minimal.rs` | — | 168-line minimal embedder. The best starting reference. |
| `components/servo/tests/webview.rs` | — | 1008-line API test suite; shows complete WebView usage. |
| `components/servo/tests/common/mod.rs` | — | Test harness showing `show()`/paint sequencing. |
| `components/shared/paint/rendering_context.rs` | `paint_api` | `RenderingContext` trait + window/offscreen/software contexts. |
| `components/shared/embedder/` | `embedder_traits` | `InputEvent`, `WebViewPoint`, `EventLoopWaker`, `RefreshDriver`. |
| `components/servo/webview_delegate.rs` | `servo` | `WebViewDelegate` trait (31 methods). |
| `components/servo/servo_delegate.rs` | `servo` | `ServoDelegate` trait (engine-wide). |
| `components/config/opts.rs` | `servo-config` | `Opts` (multiprocess, sandbox, …) and `Preferences`. |

`components/servo` is not listed explicitly in `[workspace] members`; it is an
implicit member because `ports/servoshell` path-depends on it.

### Workspace membership (`Cargo.toml:1-9`)

```toml
[workspace]
resolver = "2"
members = [
    "components/media/examples",
    "components/xpath",
    "ffi/capi",
    "ports/servoshell",
    "tests/capi",
    "tests/unit/*",
]
default-members = ["ports/servoshell"]
exclude = [".cargo", "support/crown"]
```

---

## 3. servoshell file map (desktop)

`ports/servoshell/desktop/` — 18 files, ~4,970 lines:

| File | Lines | Role |
| --- | --- | --- |
| `headed_window.rs` | 1346 | window + GL context + event routing (the important one) |
| `gui.rs` | 866 | **egui chrome**, favicons, address bar, status text, blit registration |
| `dialog.rs` | 763 | embedder dialogs (alert/confirm/prompt/file/permission) |
| `event_loop.rs` | 169 | winit `EventLoop` construction |
| `app.rs` | 266 | `ApplicationHandler` implementation |
| `headless_window.rs` | 180 | `SoftwareRenderingContext` path |
| `keyutils.rs` | 601 | key bindings |
| `gamepad.rs` | 310 | gamepad delegate |
| `accelerated_gl_media.rs` | 75 | GL media init |
| `webxr.rs` | 90 | WebXR registry |

Shared: `ports/servoshell/window.rs` (484), `running_app_state.rs` (971),
`prefs.rs` (931), `webdriver.rs` (346).

---

## 4. Current embedding architecture

Servo's embedding API is **handle + delegate + rendering context**:

```text
Application
   |
   +-- ServoBuilder -> Servo            (engine instance, one per process)
   |        |
   |        +-- ServoDelegate            (engine-wide callbacks)
   |
   +-- WindowRenderingContext            (GL context for the OS window, via surfman)
   |        |
   |        +-- OffscreenRenderingContext (FBO for web content only)
   |                 |
   |                 +-- WebViewBuilder -> WebView   (one per tab)
   |                          |
   |                          +-- WebViewDelegate     (per-webview callbacks)
```

Ownership is `Rc`-based and **single-threaded**: `WebView` is
`Rc<RefCell<WebViewInner>>` (`components/servo/webview.rs:84`). These types are
`!Send` and `!Sync`. Everything must happen on the thread that owns them.

---

## 5. How servoshell creates its window

`HeadedWindow::new` — `ports/servoshell/desktop/headed_window.rs:108`.

1. Build `WindowAttributes` (`headed_window.rs:116-127`): title, decorations,
   logical inner size, min inner size, and `.with_visible(false)`.
   The comment at `:125-126` explains why: AccessKit setup must happen before
   first show.
2. `event_loop.create_window(window_attr)` (`headed_window.rs:134-136`).
3. `WindowRenderingContext::new(display_handle, window_handle, inner_size)`
   (`headed_window.rs:170`) — creates the surfman connection, adapter, GL
   context and surface, and binds + makes it current
   (`components/shared/paint/rendering_context.rs:406-460`).
4. `window_rendering_context.make_current()` (`headed_window.rs:183`).
5. `window_rendering_context.offscreen_context(inner_size)`
   (`headed_window.rs:186`) — the FBO that web content will render into.
6. `Gui::new(...)` then `winit_window.set_visible(true)` (`desktop/gui.rs:242`).

Winit version: **0.30.13** (`Cargo.toml:288`). The event loop is
`EventLoop::with_user_event().build()` (`desktop/event_loop.rs:52-58`), and
servoshell implements `winit::application::ApplicationHandler<AppEvent>`
(`desktop/app.rs:186`).

### Can a webview occupy only part of the window?

Yes, but **not** via a viewport/window-region call. There is no `glViewport`
usage and no child view in servoshell. Sub-region composition is done with an
**offscreen FBO blit** driven by an egui `PaintCallback`. See §7.

---

## 6. How a webview is created and configured

Single creation site: `ServoShellWindow::create_toplevel_webview`
(`ports/servoshell/window.rs:106-143`).

```rust
let mut webview_builder = WebViewBuilder::new(state.servo(), self.platform_window.rendering_context())
    .url(url);                                  // or request.builder(...)
webview_builder = webview_builder
    .hidpi_scale_factor(self.platform_window.hidpi_scale_factor())
    .user_content_manager(state.user_content_manager.clone())
    .delegate(state.clone());                   // RunningAppState implements WebViewDelegate
let webview = webview_builder.build();
```

Note `platform_window.rendering_context()` returns the **offscreen** context
(`headed_window.rs:1009-1011`), not the window context.

The delegate is the app-wide `RunningAppState`, shared by every webview in every
window. The owning window is recovered by `Rc::ptr_eq` on the rendering context
(`running_app_state.rs:510-524`).

Our browser should instead create **one delegate per webview**, carrying its
`TabId` — this avoids the `Rc::ptr_eq` reverse lookup entirely (see
`docs/hermes-research.md` §2).

---

## 7. The rendering path (the key finding)

### Servo already does exactly what the brief asks for

Servoshell **already uses egui**, and shares Servo's GL context with it:

| Crate | Version | Source |
| --- | --- | --- |
| `egui` | 0.34.3 | `Cargo.toml:82` |
| `egui-winit` | 0.34.3 (`default-features = false`; servoshell adds `accesskit`, `clipboard`, `wayland`) | `Cargo.toml:84`, `ports/servoshell/Cargo.toml:139` |
| `egui_glow` | 0.34.3 (feature `winit`) | `Cargo.toml:85`, `ports/servoshell/Cargo.toml:140` |
| `glow` | 0.17.0 | `Cargo.toml:106` |
| `winit` | 0.30.13 | `Cargo.toml:288` |
| `surfman` | 0.14.0 (feature `sm-x11`) | `Cargo.toml:247`, `ports/servoshell/Cargo.toml:147` |

So the brief's GUI choice (egui, GPU-backed, native Rust, keeps the browser UI
separate from the web renderer) is **already the upstream-sanctioned design**.
We should follow servoshell's integration rather than invent one.

### How egui is bound to the GL context

`desktop/gui.rs:225-234`:

```rust
rendering_context.make_current()...;
let mut context = EguiGlow::new(
    event_loop,
    rendering_context.glow_gl_api(),   // Arc<glow::Context> — the SAME one Servo uses
    None,                              // shader_version: auto-detect
    None,                              // pixels_per_point: follow winit scale factor
    false,                             // dithering
);
```

`OffscreenRenderingContext::glow_gl_api()` simply forwards to the parent
(`rendering_context.rs:870-872`), which returns the surfman-created
`glow::Context` (`rendering_context.rs:558-560`). **There is one GL context, one
`glow::Context`, no interop and no share-list juggling.** `EguiGlow::new` takes
`Arc<glow::Context>` (`egui_glow-0.34.3/src/winit.rs:24-30`).

### One frame, in order

`Gui::update` (`gui.rs:399-696`):

1. `rendering_context.make_current()` (`gui.rs:405-407`).
2. `context.run(window, |ctx| ...)` — egui layout pass.
3. Toolbar `Panel::top("toolbar")` (`gui.rs:429`), then tab strip
   `Panel::top("tabs")` (`gui.rs:562`); record `toolbar_height`
   (`gui.rs:608`).
4. `let available_rect = ctx.available_rect_before_wrap();` (`gui.rs:620`).
5. `webview.resize(PhysicalSize::new(size.width as u32, size.height as u32))` if
   the size changed (`gui.rs:653-659`).
6. `window.repaint_webviews();` (`gui.rs:664`) →
   `rendering_context.make_current(); webview.paint(); rendering_context.present();`
   (`window.rs:146-157`). Servo renders WebRender output into the **offscreen
   FBO**. `present()` is a **no-op** for the offscreen context
   (`rendering_context.rs:860`).
7. Register the blit as an egui `PaintCallback` on `LayerId::background()`
   (`gui.rs:666-678`).

`Gui::paint` (`gui.rs:698-708`):

```rust
self.rendering_context.make_current()...;
self.rendering_context.parent_context().prepare_for_rendering();  // bind window FBO
self.context.paint(window);                                        // egui draws
self.rendering_context.parent_context().present();                // swap buffers
```

Inside `EguiGlow::paint`, egui tessellates and paints layers in order;
`LayerId::background()` is painted first, so the blit runs **before** the
toolbar/tab strip, i.e. **the browser chrome composites on top of the web
content**.

### The blit

`OffscreenRenderingContext::render_to_parent_callback()`
(`rendering_context.rs:751-767`) returns
`Option<Box<dyn Fn(&glow::Context, Rect<i32>) + Send + Sync>>` (type alias at
`:730`) which calls `blit_framebuffer` → `gl.blit_framebuffer(..., COLOR_BUFFER_BIT, NEAREST)`
(`rendering_context.rs:769-807`). The `&glow::Context` handed to the closure is
`egui_glow::Painter::gl()` — again the same context.

Servo's own doc comment recommends this exact composition strategy
(`rendering_context.rs:391-398`): *"If you would like to paint to only a portion
of the window, consider using `OffscreenRenderingContext`"*.

**Conclusion: we do not need a new graphics abstraction.** WebRender + surfman +
glow + egui_glow already compose the browser UI over web content with a single
GL context and one FBO blit per frame.

### The 0.6.0 frame trigger

`notify_new_frame_ready` → `window.set_needs_repaint()` → winit
`request_redraw()` → `RedrawRequested` → `Gui::update` + `Gui::paint`. This is the
"paint on demand" mechanism; we should reuse it rather than spinning a timer.

---

## 8. How input reaches the webview

Single router: `HeadedWindow::handle_winit_window_event`
(`headed_window.rs:508-755`).

There are **two** hit-tests:

1. **egui-side** — `should_forward_mouse_event_to_egui` (`headed_window.rs:538-550`)
   returns true if a dialog is open, else asks
   `Gui::is_in_egui_toolbar_rect(point)` (`gui.rs:297-303`), which is literally
   `position.y < self.toolbar_height.get()`.
2. **webview-side** — the point must be inside `webview.size()`
   (`headed_window.rs:260-264`).

Coordinate translation subtracts the chrome height
(`headed_window.rs:300-301`):

```rust
let mut point = winit_position_to_euclid_point(position).to_f32();
point.y -= (self.toolbar_height() * self.hidpi_scale_factor()).0;
```

Dispatch functions:

| Input | Function | Servo call |
| --- | --- | --- |
| Keyboard | `handle_keyboard_input` (`headed_window.rs:224`) | `notify_input_event(InputEvent::Keyboard(..))` (`:246`) |
| Mouse button | `handle_mouse_button_event` (`:253`) | `InputEvent::MouseButton` (`:291-295`) |
| Mouse move | `handle_mouse_move_event` (`:299`) | `InputEvent::MouseMove` (`:326`), `InputEvent::MouseLeftViewport` (`:309-311`) |
| Wheel | inline (`:654-678`) | `InputEvent::Wheel` (`:674`) |
| Touch | inline (`:680`) | `InputEvent::Touch` |
| IME | inline (`:704-720`) | `InputEvent::Ime` |

Keyboard focus arbitration: keyboard events go to the webview only when
`!gui.has_keyboard_focus()` (`headed_window.rs:598`); egui responses are
honoured via a `consumed` flag (`headed_window.rs:602-630`, fan-out at `:632`).
`Gui::has_keyboard_focus` inspects `egui_ctx.memory(|m| m.focused().is_some())`
(`gui.rs:269-273`).

---

## 9. Event loop and threading

- winit `ApplicationHandler`; `resumed` creates the window, `window_event`
  handles events, `user_event` drains Servo wakes.
- `Servo::spin_event_loop()` (`components/servo/servo.rs:1076`) must be called
  after any input/navigation call and on every wake-up.
- `embedder_traits::EventLoopWaker` requires exactly two methods
  (`components/servo/examples/winit_minimal.rs:157-166`):
  `fn clone_box(&self) -> Box<dyn EventLoopWaker>` and `fn wake(&self)`.
  Both `servo` and the example re-export the trait as `servo::EventLoopWaker`.
- Servo runs internal work on its own threads; the embedder thread must spin.

---

## 10. How a URL gets loaded (end-to-end)

1. UI produces a `Url` (`url` crate type; re-exported through Servo's types).
2. `WebViewBuilder::new(&servo, rendering_context).url(url)` at construction, or
   `WebView::load(url)` afterwards (`components/servo/webview.rs:518`).
3. `load` sends `EmbedderToConstellationMessage::LoadURL` to the constellation
   (`webview.rs:518-529`).
4. Default URL when none supplied is `about:blank` (`webview.rs:225-228`).
5. The constellation parses, fetches, and pipes into script/layout/paint.
6. WebRender produces display lists; `notify_new_frame_ready` fires on the
   delegate (`webview_delegate.rs:949`).
7. Our code paints and presents (§7).
8. State comes back through delegate callbacks: `notify_url_changed` (`:927`),
   `notify_page_title_changed` (`:930`), `notify_load_status_changed` (`:941`),
   `notify_history_changed` (`:953`), `notify_favicon_changed` (`:947`).

---

## 11. Verified API surface (Servo 0.6.0)

Signatures below were read from source; `file:line` is authoritative.

### `Servo` / `ServoBuilder` — `components/servo/servo.rs`

```rust
ServoBuilder::default()                       // :1441
    .opts(Opts) -> Self                        // :1457
    .preferences(Preferences) -> Self          // :1462
    .event_loop_waker(Box<dyn EventLoopWaker>) -> Self  // :1467
    .protocol_registry(ProtocolRegistry) -> Self         // :1472
    .build() -> Servo                          // :1453

impl Servo {
    fn spin_event_loop(&self)                  // :1076
    fn setup_logging(&self)                    // :1080
    fn set_delegate(&self, Rc<dyn ServoDelegate>)  // :1061
    fn delegate(&self) -> Rc<dyn ServoDelegate>    // :1057
    fn set_preference(&self, &str, PrefValue)  // :1105
    fn network_manager(&self) -> Ref<'_, NetworkManager>  // :1111
    fn site_data_manager(&self) -> &SiteDataManager       // :1115
    fn create_memory_report(&self, GenericCallback<MemoryReportResult>)  // :1093
}

pub fn run_content_process(token: String)      // :1324 (feature "multiprocess")
```

### `WebViewBuilder` — `components/servo/webview.rs:1064-1156`

```rust
WebViewBuilder::new(servo: &Servo, rendering_context: Rc<dyn RenderingContext>) -> Self  // :1082
    .delegate(Rc<dyn WebViewDelegate>) -> Self        // :1109
    .url(Url) -> Self                                 // :1115
    .hidpi_scale_factor(Scale<f32, DeviceIndependentPixel, DevicePixel>) -> Self  // :1121
    .user_content_manager(Rc<UserContentManager>) -> Self  // :1132
    .clipboard_delegate(Rc<dyn ClipboardDelegate>) -> Self  // :1139
    .gamepad_delegate(Rc<dyn GamepadDelegate>) -> Self       // :1147 (feature "gamepad")
    .build() -> WebView                                // :1153
```

### `WebView` — `components/servo/webview.rs`

| Method | Line | Notes |
| --- | --- | --- |
| `load(Url)` | 518 | pushes a history entry |
| `reload()` | 541 | |
| `can_go_back()` / `go_back(usize) -> TraversalId` | 552 / 561 | |
| `can_go_forward()` / `go_forward(usize) -> TraversalId` | 578 / 588 | |
| `resize(PhysicalSize<u32>)` | 454 | clamps to 1×1 min; resizes the shared rendering context |
| `size() -> DeviceSize` | 444 | |
| `set_hidpi_scale_factor(Scale<..>)` | 474 | |
| `show()` / `hide()` | 489 / 497 | compositor-level visibility |
| `set_focused(bool)` | 418 | **all webviews start focused (`webview.rs:184`)** |
| `focused()` | 392 | |
| `notify_input_event(InputEvent) -> InputEventId` | 615 | |
| `notify_scroll_event(Scroll, WebViewPoint)` | 603 | |
| `paint()` | 720 | |
| `url() / page_title() / status_text() / favicon()` | 334 / 364 / 347 / 382 | |
| `load_status() -> LoadStatus` | 320 | |
| `cursor()` / `animating()` | 401 / 431 | |
| `id() -> WebViewId` | 307 | |
| `evaluate_javascript` / `take_screenshot` | 731 / 760 | |
| `set_page_zoom` / `page_zoom` | 657 / 662 | |
| `notify_theme_change(Theme)` | 505 | |
| `clear_session_history()` | 1026 | |

### APIs that DO NOT exist in 0.6.0 (contrary to common/older sources)

| Missing | Reality |
| --- | --- |
| `Servo::new_webview(url)` | Replaced by `WebViewBuilder`. `grep` finds no such symbol. |
| `WebView::focus()` / `WebView::blur()` | Replaced by `WebView::set_focused(bool)`. |
| `WebView::set_throttled(bool)` | **Removed.** `grep -i throttl` over `components/servo/*.rs` returns nothing. Hermes calls this on 0.4.0 — it is stale. |
| `EmbedderDelegate` / `WindowMethods` / `WindowEvent` / `EmbedderMsg` | Legacy pre-0.5 API. Present only in old third-party tutorials. |
| `servo::WindowMethods` | No longer exported. |

**Background-tab throttling must be done differently.** Options: `hide()` +
Servo preferences, or relying on `notify_animating_changed` (`:938`) to decide
whether to keep spinning. This is unresolved and needs its own investigation.

### `RenderingContext` — `components/shared/paint/rendering_context.rs:34-86`

```rust
pub trait RenderingContext {
    fn prepare_for_rendering(&self) {}                                    // :37  (defaulted!)
    fn read_to_image(&self, DeviceIntRect) -> Option<RgbaImage>;          // :45  (required)
    fn size(&self) -> PhysicalSize<u32>;                                  // :47
    fn resize(&self, PhysicalSize<u32>);                                  // :54
    fn present(&self);                                                    // :57
    fn make_current(&self) -> Result<(), Error>;                          // :61
    fn gleam_gl_api(&self) -> Rc<dyn gleam::gl::Gl>;                      // :63
    fn glow_gl_api(&self) -> Arc<glow::Context>;                          // :65
    fn create_texture(&self, Surface) -> Option<(SurfaceTexture, u32, Size2D<i32>)>;  // :68
    fn destroy_texture(&self, SurfaceTexture) -> Option<Surface> {}       // :75
    fn connection(&self) -> Option<Connection> {}                         // :79
    fn refresh_driver(&self) -> Option<Rc<dyn RefreshDriver>> {}          // :84
}
```

Implementations:
- `SoftwareRenderingContext` (`:281`) — headless/CI.
- `WindowRenderingContext` (`:399`), `::new(DisplayHandle, WindowHandle, PhysicalSize<u32>)` (`:406`),
  `::offscreen_context(self: &Rc<Self>, PhysicalSize<u32>) -> OffscreenRenderingContext` (`:462`),
  `::surfman_details()` (`:515`), `::set_window()` (`:485`).
- `OffscreenRenderingContext` (`:724`), `::parent_context()` (`:747`),
  `::render_to_parent_callback()` (`:751`).

Note `prepare_for_rendering` has a **default empty body**. A custom context that
omits it binds no framebuffer and silently renders nowhere. The test suite has a
`ConnectionlessRenderingContext` wrapper purely to forward this method
(`components/servo/tests/webview.rs:85-89`) — a good warning sign.

### `WebViewDelegate` — `components/servo/webview_delegate.rs:918-1092`

31 methods, **all with default no-op bodies**, so we implement only what we need.
Most relevant: `notify_new_frame_ready` (`:949`), `notify_url_changed` (`:927`),
`notify_page_title_changed` (`:930`), `notify_load_status_changed` (`:941`),
`notify_history_changed` (`:953`), `notify_favicon_changed` (`:947`),
`notify_closed` (`:959`), `notify_input_event_handled` (`:964`),
`notify_cursor_changed` (`:944`), `notify_animating_changed` (`:938`),
`notify_crashed` (`:972`), `request_create_new` (`:1028`), `screen_geometry` (`:922`),
`load_web_resource` (`:1067`), `show_console_message` (`:1074`).

`ServoDelegate` — `components/servo/servo_delegate.rs:21-47`:
`notify_devtools_server_started`, `request_devtools_connection`, `notify_error`,
`show_console_message`, `load_web_resource`.

### Input types — `components/shared/embedder/input_events.rs` / `lib.rs`

```rust
pub enum InputEvent {                       // input_events.rs:53
    EditingAction(ClipboardAction),
    #[cfg(feature = "gamepad")] Gamepad(GamepadEvent),
    Ime(ImeEvent), Keyboard(KeyboardEvent), MouseButton(MouseButtonEvent),
    MouseLeftViewport(MouseLeftViewportEvent), MouseMove(MouseMoveEvent),
    Touch(TouchEvent), Wheel(WheelEvent),
}

MouseMoveEvent::new(WebViewPoint)                                   // :110
MouseButtonEvent::new(MouseButtonAction, MouseButton, WebViewPoint)  // :104
WheelEvent::new(WheelDelta, WebViewPoint)                            // :166
KeyboardEvent::new(keyboard_types::KeyboardEvent)                   // :?  (preferred)
KeyboardEvent::new_without_event(KeyState, Key, Code, Location, Modifiers, bool, bool)

pub enum WebViewPoint { Device(DevicePoint), Page(Point2D<f32, CSSPixel>) }  // embedder/lib.rs:61

bitflags! { pub struct InputEventResult: u8 { DefaultPrevented, Consumed, DispatchFailed } }
```

`keyboard_types` types (`Key`, `Code`, `Modifiers`, `Location`, `KeyState`,
`CompositionEvent`, `CompositionState`, `NamedKey`) are re-exported from `servo`
(`components/servo/lib.rs:66-68`).

---

## 12. Things that look unstable / experimental

1. **The embedding API is still explicitly a work in progress.** The upstream
   tracking issue is servo/servo#34522 ("embedding API"). `Servo::new_webview`
   was removed in favour of `WebViewBuilder` within a single release cycle, and
   `set_throttled` disappeared between 0.4.0 and 0.6.0. **Expect breakage.**
2. **No public background-tab throttling API.** See §11.
3. **`prepare_for_rendering` is silently defaulted.** A custom `RenderingContext`
   can compile, run, and render nothing.
4. **Multiprocess + embedding.** `Opts::default().multiprocess = false`
   (`components/config/opts.rs:249`). Hermes documents that on 0.4.0 the
   cross-process Paint path never reached the embedder's rendering context,
   leaving blank pages. **Unverified for 0.6.0 — must be tested.** Security
   guidance: do not enable multiprocess until we have proven pages render.
5. **`render_to_parent_callback` returns `Option`** — `None` is possible if the
   offscreen framebuffer id is zero. Must be handled.
6. **`WebView::resize` resizes the shared rendering context**, affecting all
   webviews using it (`webview.rs:449-451`). One rendering context per window
   means tabs cannot have independent sizes — they must all be window-sized and
   visibility-managed instead.
7. **All webviews start focused** (`webview.rs:184`); the doc comment at
   `webview.rs:414-417` makes the embedder responsible for clearing it.
8. `servo-allocator/use-jemalloc` is a **no-op on Windows** — the jemalloc
   dependencies are gated to non-Windows targets
   (`components/allocator/Cargo.toml:22-30`). Windows uses the system allocator.
9. Servo's in-repo docs are **stubs** pointing at `book.servo.org`
   (`CONTRIBUTING.md:1-5`, `docs/HACKING_QUICKSTART.md:1-5`). Only
   `README.md:46-57` documents Windows builds locally.

---

## 13. Windows platform notes (verified)

Servo supports Windows and CI builds it (`.github/workflows/windows.yml`).

| Fact | Source |
| --- | --- |
| Documented build: `.\mach bootstrap` then `.\mach build` | `README.md:46-57` |
| Required VS components: Windows SDK ≥ 10.0.19041.0, MSVC v143 x64/x86, C++ ATL for v143 | `README.md:51-54` |
| `mach bootstrap` installs via winget: CMake, LLVM, Ninja, WiX | `python/servo/platform/windows/winget.json` |
| GStreamer 1.22.8 MSIs installed with UAC elevation | `python/servo/platform/windows.py:27-28, 192-212` |
| `mach.ps1` = `uv run --frozen python mach <args>` (+ glob expansion, ARM64 fix) | `mach.ps1:1-32` |
| Compiler is `clang-cl.exe`, not `clang` | `python/servo/command_base.py:386-391` |
| ANGLE is built from source; `libEGL.dll`/`libGLESv2.dll` copied post-build | `python/servo/build_commands.py:334-345` |

**Critical for us:** on Windows both `ports/servoshell/Cargo.toml:154` and
`ffi/capi/Cargo.toml:35` enable **`servo/no-wgl`**, defined at
`components/servo/Cargo.toml:66` as
`["dep:mozangle", "mozangle/egl", "mozangle/build_dlls", "surfman/sm-angle-default", "paint_api/no-wgl"]`.
Without it, Windows uses native WGL. We should match servoshell.

Other Windows caveats: ASAN/TSAN unsupported
(`python/servo/build_commands.py:41-54`); no WPT CI on Windows; no MinGW
support (MSVC only).

### Local machine readiness (checked)

| Requirement | Status |
| --- | --- |
| VS 2022 Build Tools 14.44.35207 | present |
| Windows SDK 10.0.26100.0 | present (≥ 10.0.19041.0 required) |
| **C++ ATL for v143** | **MISSING** (`atlmfc` absent) |
| clang / clang-cl | present (`C:\Program Files\LLVM`) |
| libclang.dll | present |
| CMake | present |
| Python 3.11 | present (`python.exe` on PATH) |
| **uv** | **MISSING** (build falls back to `python`, warns) |
| **ninja** | **MISSING** |
| **moztools 4.0** | **MISSING** (`.\mach fetch` downloads it) |
| rustc 1.97.1 toolchain | installed |
| **rustc-dev + llvm-tools for 1.97.1** | **MISSING** (needed by `crown`) |

---

## 14. Build-system implication (important)

Servo's root `Cargo.toml` carries Windows-specific build tuning that only
applies **inside the Servo workspace**:

```toml
# Cargo.toml:393-399
[profile.dev.package.servo-script]
# On windows builds the default 256 codegen units cause the script rlib file
# to exceed 4 GB, which causes build errors due to rust-lang/rust#151184.
codegen-units = 16
```

Profiles are **not inherited across workspaces**. If we create our own workspace
with a path dependency on `servo/components/servo`, this workaround disappears
and the Windows build may fail with a >4 GB rlib. Our workspace must replicate it
(at minimum), and ideally we should keep Servo's `[patch.crates-io]` section
(empty/commented, `Cargo.toml:444+`) in mind for future forks.

This is the single biggest build-integration risk found so far and must be
validated before writing any browser code.