# Dependency Decisions

Every dependency is recorded here with the question it must answer, the decision,
and the reason. The policy questions applied to each:

1. Is it actually necessary?
2. Does Servo already provide this?
3. Does the standard library provide enough?
4. Is there a lightweight Rust alternative?
5. Does it significantly increase binary size or memory usage?
6. Is it actively maintained?
7. Does it conflict with Servo's existing architecture?

Status values: **ACCEPTED** (decided), **REJECTED** (decided against),
**DEFERRED** (will revisit at a named milestone).

---

## Current state

No browser code has been written yet. This document records the *intended*
dependency set and the reasoning, plus decisions already taken during research.

Servo itself is tracked in `docs/servo-version.md` and is not repeated here.

---

## ACCEPTED

### `servo` (path: `servo/components/servo`, version `0.6.0`)

The engine. Non-negotiable for this project. See `docs/servo-version.md`.

Required features for our target (Windows + GPU):

- `no-wgl` — **required on Windows.** Matches `ports/servoshell/Cargo.toml:154`
  and `ffi/capi/Cargo.toml:35`. Expands to ANGLE (`mozangle/egl`,
  `mozangle/build_dlls`) plus `surfman/sm-angle-default`.
- `media-gstreamer`, through our default `media` feature (accepted 2026-10-01
  when the user asked for media). Without it Servo uses its dummy backend and
  no `<video>`/`<audio>` plays. Needs GStreamer 1.22.8 (MSVC x86_64), the
  version `mach bootstrap` uses. `scripts/fetch-gstreamer.ps1` unpacks Servo's
  copies of the packages into `deps/gstreamer` with `msiexec /a`, so there is
  no UAC prompt and no system install. `scripts/copy-gstreamer-dlls.ps1` stages
  the DLLs and plugins next to the executable, which will not start without
  them. Build with `--no-default-features` to go without. It does **not**
  provide `MediaSource`, so YouTube still cannot play.
- Keep Servo's defaults (`bundled`, `clipboard`, `js_jit`, `multiprocess`) for
  now. Trimming features is a measured decision, not a guess.

### `winit` `0.30.13`

Window creation and the event loop. Servo already depends on this exact version
and its embedding API is shaped around winit `ApplicationHandler`
(`components/servo/examples/winit_minimal.rs`). Using a different major version
would mean two winit trees and broken raw-window-handle plumbing.

### `egui` `0.34.3`, `egui-winit` `0.34.3`, `egui_glow` `0.34.3` (feature `winit`)

The browser chrome. Version is pinned to Servo's because
`EguiGlow::new(event_loop, Arc<glow::Context>, ...)` must receive the *same*
`glow::Context` Servo created (`ports/servoshell/desktop/gui.rs:228-234`), and
`glow` must stay on `0.17.0`.

`egui-winit` is used with `default-features = false`. Servoshell enables
`["accesskit", "clipboard", "wayland"]`; the `default` set additionally pulls
`links` (the `webbrowser` crate) and `x11`, neither of which a Windows-first
lightweight browser wants. We enable **`clipboard`** only at first, because
copy/paste in the address bar is real user-visible functionality.
`accesskit` is deferred to Milestone 5.

**Why egui and not something else:** this is not a preference, it is what
upstream already ships. Servoshell's entire browser UI is egui rendered through
the same OpenGL context as WebRender. Adopting egui means we reuse a proven
composition path (offscreen FBO blit driven by an egui `PaintCallback`) instead
of designing one. See `docs/architecture-research.md` §7.

### `glow` `0.17.0`

Required to construct `egui_glow`. We do **not** otherwise use it; it exists
because `EguiGlow` requires `Arc<glow::Context>`.

### `euclid` `0.22.14`

The Servo embedding API is expressed in euclid geometry types
(`Point2D<f32, CSSPixel>`, `Scale<f32, DeviceIndependentPixel, DevicePixel>`,
`Size2D`). Depending on it directly is required to call the API; it is a tiny,
zero-cost, header-only-style crate.

### `url` `2.5.8`

Address-bar classification (URL vs. search query) and `Url` values passed to
`WebView::load`. Servo already depends on the same version, so no duplicate
tree.

### `dpi` `0.1.2`

`PhysicalSize<u32>` is the parameter type of `WebView::resize`
(`components/servo/webview.rs:454`) and `WindowRenderingContext::new`. Tiny
crate, already in the graph.

### `log` `0.4.34`

Servo's logging facade. `Servo::setup_logging()` is in the documented startup
sequence (`components/servo/servo.rs:1080`). Near-zero cost.

### `rustls` `0.23.45` (features `aws-lc-rs`, `std`)

**Required.** Servo's minimal example installs a rustls crypto provider before
anything else (`components/servo/examples/winit_minimal.rs:25-28`):

```rust
rustls::crypto::aws_lc_rs::default_provider().install_default()
```

Without a default provider installed, TLS to HTTPS sites fails. This is an
engine requirement, not an optional extra. Note the instruction to *not* weaken
TLS verification — we keep default verification.

---

## REJECTED

### `surfman` — direct dependency

**Servo already provides this.** We never create a GL context ourselves; we call
`WindowRenderingContext::new(...)` and receive `glow_gl_api()` and
`connection()` from it. Depending on `surfman` directly would be redundant.
(Hermes *does* depend on it directly — see `docs/hermes-research.md` "what not
to copy".)

### `wgpu` — as a browser renderer

**Servo does not use wgpu.** It uses WebRender (`webrender 0.70.0`) with an
OpenGL backend via surfman. WebGPU support in Servo (`servo/webgpu` feature) is
for *web content*, not for embedding the engine. Adding wgpu would mean writing a
second GPU abstraction that Servo cannot use. Explicitly rejected.

### `egui-wgpu`

Rejected because our window surface is an OpenGL context created by surfman, and
we must share it with WebRender. `egui_wlow`/`egui_glow` is the correct bridge.
A wgpu-based egui would need its own surface and a texture round-trip from
Servo's output — exactly the "unnecessary GPU copies" the brief forbids.

### Electron, Tauri, wry, web-based UI

Rejected by the brief. Also mutually exclusive with "no embedded web-based UI".

### `slint` / `slint-build`

Rejected: the brief specifies egui, and egui is what upstream uses. (Hermes uses
Slint — reference only.)

### `adblock`

Rejected for now. Not in any milestone. Adds a substantial dependency plus six
bundled filter lists (~MB) and a build script that compiles them. Revisit only on
an explicit request.

### `serde` / `serde_json` — for browser data

`serde` is already in the graph via Servo, so depending on it is *free*. We keep
it on the "free" list rather than the rejected list. Milestone 4 (history,
bookmarks, settings) will need a persistence format; the decision on *which*
format is deferred to that milestone.

### `anyhow` / `thiserror`

Not yet added. Milestone 1 needs at most a handful of error cases; explicit
`Display`/`Error` impls are clearer and dependency-free. Revisit if error
plumbing actually gets noisy.

### `tokio` / any async runtime in browser code

Rejected for browser code. Servo owns its own async runtime
(`tokio` appears in `components/servo/Cargo.toml` with only the `sync` feature).
Our event loop is synchronous — winit callbacks enqueue, the frame drains. Adding
an async runtime to the app would duplicate a runtime we do not own.

---

## DEFERRED

| Dependency / decision | Needed for | Trigger to decide |
| --- | --- | --- |
| `accesskit` + `egui-winit/accesskit` | browser-chrome accessibility | Milestone 5 (perf/measure) or an explicit a11y requirement |
| `image` | favicon decoding | servoshell decodes favicons itself (`desktop/gui.rs:842-866`). Reuse the same approach rather than adding a dependency — `image` is already in the graph, so the cost is nil either way. Decide with Milestone 3 (tabs need favicons). |
| persistence format (JSON / SQLite / `rusqlite`) | history, bookmarks, session restore | Milestone 4. Note the brief flags database access cost as a perf concern. |
| `servo/webxr` | WebXR | servoshell enables it by default; we do not need it initially. Adds a real dependency. |
| `servo/background-hang_monitor` | diagnosing hangs | dev-only; consider a feature flag |
| `egui-file-dialog` | file pickers | Milestone 4 (downloads), if native file dialogs are needed |

---

## Transitive-dependency notes

- `mozangle` `0.7.1` arrives via `servo/no-wgl` and **builds ANGLE from source**,
  producing `libEGL.dll` and `libGLESv2.dll`. This is the single largest
  Windows-specific build cost and is unavoidable on this platform.
- `gaol` (sandbox) is **not** built on Windows — the dependency is gated to
  non-Windows targets (`components/servo/Cargo.toml`). The
  `cfg(not(... windows ...))` guard on the `gaol` dependency is explicit.
- `tikv-jemalloc-sys` is likewise **not** built on Windows
  (`components/allocator/Cargo.toml:22-30`); Windows uses the system allocator.
  This means our "lightweight" goal should not be measured against jemalloc
  behaviour on this platform.
- `crown` is not in the lockfile. It is only needed if we enable `servo/crown`;
  Servo's default binding generation path does not require it. (Servo's
  `rust-toolchain.toml` still requests `rustc-dev`/`llvm-tools` "for support/crown".)

## Rules for adding a dependency

Before adding anything, answer all seven policy questions above and append a row
to the ACCEPTED or REJECTED table with the reason. "It was in someone else's
Cargo.toml" is not a reason. If the answer is "Servo already provides it", do not
add it.