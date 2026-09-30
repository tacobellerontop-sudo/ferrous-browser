# Hermes Browser — Architectural Research

**Purpose:** mine Hermes Browser for architectural *ideas*, not code or API usage.

> Hermes is a **reference, not an API authority**. Every Servo API mentioned in this
> document was written against `servo = 0.4.0`. Our Servo checkout is newer. Current
> Servo source wins over Hermes in all cases. See `docs/servo-version.md`.

## Reference snapshot

| Field | Value |
| --- | --- |
| Repository | `https://github.com/KaykCaputo/hermes-browser` |
| Commit studied | `9b57523394c771f269b593eed07bd845a1ac9145` |
| Commit date | 2026-08-05 |
| Servo crate version used | `servo = "=0.4.0"` (features `vello`, Linux-only `media-gstreamer`) |
| Rust edition | 2024 |

Note the version skew already: Hermes pins Servo `0.4.0`; our checkout ships a newer
release. Hermes is therefore roughly one engine generation behind.

## Workspace layout

```text
apps/hermes-desktop        app composition, winit events, OpenGL surface
crates/hermes-ui           Slint markup + generated UI types
crates/talaria-core        engine-neutral browser model   (depends only on `url`)
crates/talaria-servo       Servo adapter                 (depends on `servo`)
third_party/gaol           vendored fork
third_party/surfman        vendored fork
third_party/servo-media-gstreamer   vendored fork
```

## Architectural ideas worth adopting

### 1. An engine-neutral core is the single most valuable idea

`talaria-core` depends on **only `url`**. It contains tabs, navigation policy, state
transitions, address classification, and an engine-neutral `WebView` trait.

The payoff is concrete: the browser's behaviour is testable with plain `cargo test` —
no Servo, no GPU, no window, no event loop. Hermes' `talaria-core/tests/` contains
four test files covering address resolution, state reduction, controller/tab logic, and
command dispatch against a fake `WebView`.

**Adopt this.** It is the difference between a browser whose logic can be verified by
CI and one that can only be verified by clicking.

### 2. Command / Event pattern with a stable tab identifier

Two plain enums carry everything across the boundary:

- **Commands** (UI → engine): navigate, back, forward, reload, pointer move, pointer
  button, scroll, focus change, keyboard input, IME input.
- **Events** (engine → UI): address changed, title changed, favicon changed, load state
  changed, history changed.

Every queued item carries a `TabId`. This matters: an asynchronous title/favicon update
from a background tab must land on the tab that produced it, not on whichever tab happens
to be active when the message is drained. A bare "current tab" pointer gets this wrong
the moment the user switches tabs mid-load.

**Adopt this.**

### 3. Commands are queued, not applied inline

The controller owns a `VecDeque` of commands. UI callbacks enqueue and request a redraw;
the frame loop drains them. Benefits:

- UI callbacks never touch engine objects, so the UI layer stays free of engine types.
- Engine work is naturally rate-limited to the frame loop.
- Commands can be **coalesced**: adjacent pointer-move, scroll, and focus commands for the
  same tab are merged (scroll deltas accumulate; pointer position is overwritten) rather
  than queueing hundreds of redundant events per frame.

That coalescing is a real idle-CPU and input-latency control, not a micro-optimisation.
**Adopt it.**

### 4. Bounded command drain per frame

Hermes drains at most `MAX_COMMANDS_PER_FRAME = 128` commands per frame, then requests
another redraw if work remains. A burst of input cannot monopolise a frame and starve
painting. **Adopt it.**

### 5. Paint only on demand, via a frame-ready signal

Servo calls `notify_new_frame_ready` on the webview delegate. Hermes sets a shared
"frame ready" flag and wakes the native event loop; the frame loop then calls `paint()`
only if the flag is set.

This prevents painting continuously when nothing changed. Hermes' docs make a sharp
point here worth copying: *URL and title callbacks do not prove that rendering
succeeded — only a delivered first frame does.* That is the correct signal to treat as
"the page is actually visible". **Adopt it.**

### 6. Lazy webview creation, model/tab separation

Creating a tab adds a model `Tab` with default state. The underlying engine `WebView` is
created later, on demand, at `about:blank`. Navigation starts only when a `Navigate`
command is dispatched.

Newly created webviews are hidden and left throttled until a later event-loop turn —
this deliberately avoids sending a throttle change before Servo has installed the
top-level browsing context. **Adopt the lazy creation; note the ordering hazard.**

### 7. Only one webview visible and unthrottled

On tab switch the previous webview is blurred, hidden, and `set_throttled(true)`; the new
one is unthrottled and shown. This is the cheap, correct first step toward the
"inactive tabs should use fewer resources" goal. **Adopt it.** (More aggressive tab
suspension is explicitly deferred, which matches our own plan.)

### 8. Resize through the webview, not the shared context

Each engine webview owns its resize notification. Resizing the shared rendering context
directly would skip the engine's CSS viewport update. **Adopt this** — it is a subtle,
easy-to-regress correctness detail.

### 9. Address classification is separated from the engine

`classify_address` decides "is this a URL or a search query?" using only `url` plus a
small rule set (recognise a parseable supported-scheme URL; otherwise try prefixing
`https://` and check the host looks like a host; otherwise treat as a search query).
The search engine is a constant. **Adopt it** — pure, testable, engine-free.

### 10. Keyboard shortcut handling sits above the webview

Browser shortcuts (`Ctrl+L`, `Ctrl+T`, `Ctrl+W`, `Ctrl+Tab`, `Ctrl+R`/`F5`, `Alt+Left`,
`Alt+Right`) are matched before the event is forwarded to the webview, and return a
"handled" flag that suppresses propagation to the engine. This is the correct place for
them: they are browser policy, not web content behaviour. **Adopt it.**

### 11. IME composition is pinned to the tab that started it

Composition state is tracked per tab, so switching tabs mid-composition does not send
the commit to the wrong page. **Adopt it.**

### 12. Multiprocess and sandbox are disabled — and *why* is documented

Hermes explicitly sets:

```text
multiprocess: false
sandbox: false
```

with the reason recorded in code and in `docs/developer-guide/servo-integration.md`:
Servo 0.4.0 runs script and networking in the child process, but the cross-process
Paint path does not deliver the display list to the embedder's rendering context.
Pages "finish loading and update their title while the WebView remains blank".

This is an important warning for us: **content-process isolation may render blank pages
in an embedding context.** We must test this on our checkout before enabling it, and we
must not enable it merely because the option exists. Security note: this trade-off
(isolation vs. visible output) needs an explicit, documented decision from us, not a
silent default.

### 13. Vendored patches are governed by an explicit rule set

Hermes documents each vendored crate, the reason, and a removal procedure. The stated
policy — vendor only for a narrow, testable fix; keep the diff minimal; add a regression
test; remove the fork when upstream absorbs it — is worth copying verbatim as project
policy.

### 14. Dev-time optimisation targeted at specific crates

Rather than optimising the whole dependency graph (which would rebuild SpiderMonkey and
ANGLE), Hermes pins `opt-level = 2` for named hot crates (script, layout, net, paint,
webrender, constellation, vello) while keeping its own crates debuggable. A good pattern
for keeping iteration fast with a huge engine dependency.

## What NOT to copy

| Hermes choice | Why we should not copy it |
| --- | --- |
| **Slint** UI (`slint`, `slint-build`, `.slint` markup) | The project brief specifies egui. Slint adds a declarative UI language plus a codegen build step. It is also a *reference* for layering only — we keep the layering, not the toolkit. |
| **adblock** + six bundled filter lists | Dependency creep. Ad blocking is not in the brief's milestones. Revisit only if explicitly requested. |
| **`=0.4.0` Servo pin** | Stale relative to our checkout. We track our own revision in `docs/servo-version.md`. |
| **Vendored `surfman` / `gaol` / `servo-media-gstreamer`** | Linux-specific fixes. On Windows they are likely irrelevant *and* harmful. Do not adopt without reproducing the failure locally. |
| **`rustls` + `gleam` + `glow` as direct dependencies** | Some of these exist only to satisfy Servo's `RenderingContext` plumbing. We should prefer Servo's own provided context types first and add a direct dependency only if a compile error forces it. |
| **A single 39 KB `main.rs`** | Directly against our "no giant files / no giant functions" rule. We want small modules with explicit ownership. |
| **Hardcoded placeholder UI strings** ("Waiting for WebView", "Servo runtime initialized") | Debug scaffolding presented as product UI. Do not copy. |
| **DuckDuckGo hardcoded** | Fine as a default, but it belongs in settings rather than a constant. |

## Hermes Servo APIs that must be re-verified against our checkout

Everything below is Hermes' claim about Servo `0.4.0`. **None of it is verified.**
Each must be checked against the real source in `servo/` before use.

- `ServoBuilder::default().opts(..).preferences(..).event_loop_waker(..).build()`
- `servo::Servo`, `Servo::spin_event_loop`, `Servo::set_delegate`, `Servo::setup_logging`
- `servo::ServoDelegate` with `load_web_resource`
- `servo::WebViewBuilder::new(&servo, rc_dyn_rendering_context).url(..).hidpi_scale_factor(..).delegate(..).build()`
- `servo::WebView` handles: `load`, `go_back`, `go_forward`, `reload`, `focus`, `blur`, `show`, `hide`, `set_throttled`, `resize`, `set_hidpi_scale_factor`, `paint`, `favicon`
- `servo::RenderingContext` trait shape: `read_to_image`, `size`, `resize`, `present`, `make_current`, `gleam_gl_api`, `glow_gl_api`, `connection`
- `servo::EventLoopWaker` requiring `clone_box` + `wake`
- Input types: `InputEvent::{MouseMove, MouseButton, Wheel, Keyboard, Ime}`, `WebViewPoint::Page`, `WheelDelta`/`WheelMode`, `KeyboardEvent::new_without_event`, `Modifiers`, `Key`, `Code`, `Location`
- Delegate callbacks used: `notify_url_changed`, `notify_page_title_changed`, `notify_favicon_changed`, `notify_load_status_changed`, `notify_history_changed`, `notify_new_frame_ready`, `notify_crashed`
- `servo::LoadStatus::{Started, HeadParsed, Complete}`, `servo::PixelFormat`, `servo::RgbaImage`, `servo::DeviceIntRect`
- `servo::run_content_process(token)`
- Preferences keys `dom_script_asynch`, `dom_canvas_backend` (value `"vello"`)

## Structural takeaway

Hermes' layering is the part we want:

```text
App composition (window, events, GPU surface)
  -> UI callbacks / presentation state
    -> engine-neutral core (tabs, state, commands, WebView trait)
      -> engine adapter (Servo runtime, webviews, delegates)
        -> Servo
```

The specific toolkit, the specific engine API calls, and the specific patch set are all
things we must re-derive from our own checkout.