# Development Log

Chronological record of significant changes: **what** changed, **why**, the
**Servo APIs** involved, **problems** hit, and **how they were solved**.

This file exists so a future agent (human or AI) can pick up without repeating
work. **Read it before writing code.**

---

## 2026-09-29 — Entry 001: Research phase (no browser code written)

### What changed

Created the project scaffold and the research corpus. **No application code has
been written yet, and nothing has been built.**

Files created:

- `docs/architecture-research.md` — verified Servo embedding architecture
- `docs/hermes-research.md` — Hermes Browser architectural review
- `docs/dependencies.md` — dependency decisions
- `docs/servo-version.md` — pinned Servo revision and build provenance
- `docs/devlog.md` — this file
- `servo/` — Servo checkout at `b2ef57f642816e77e67e5a322ddccdbde6784f3c`
- Empty module directories under `browser/` (scaffolding only, no contents)

### Why

The project brief mandates: *"NEVER guess Servo APIs... The current Servo
checkout is the source of truth."* Nothing could be written until the actual
Servo source was present. The working directory was empty and no Servo checkout
existed anywhere on the machine.

### Servo APIs verified (all read from source, not memory)

| API | Location |
| --- | --- |
| `ServoBuilder::{build, opts, preferences, event_loop_waker, protocol_registry}` | `components/servo/servo.rs:1453,1457,1462,1467,1472` |
| `Servo::{spin_event_loop, setup_logging, set_delegate}` | `components/servo/servo.rs:1076,1080,1061` |
| `WebViewBuilder::{new, url, delegate, hidpi_scale_factor, user_content_manager, build}` | `components/servo/webview.rs:1082,1115,1109,1121,1132,1153` |
| `WebView::{load, reload, go_back, go_forward, resize, show, hide, set_focused, notify_input_event, paint}` | `components/servo/webview.rs:518,541,561,588,454,489,497,418,615,720` |
| `RenderingContext` trait (11 methods) | `components/shared/paint/rendering_context.rs:34-86` |
| `WindowRenderingContext::{new, offscreen_context}` | `rendering_context.rs:406,462` |
| `OffscreenRenderingContext::{parent_context, render_to_parent_callback}` | `rendering_context.rs:747,751` |
| `WebViewDelegate` (31 defaulted methods) | `components/servo/webview_delegate.rs:918-1092` |
| `ServoDelegate` | `components/servo/servo_delegate.rs:21-47` |
| `InputEvent` enum + constructors | `components/shared/embedder/input_events.rs:53` |
| `WebViewPoint` | `components/shared/embedder/lib.rs:61` |
| `embedder_traits::EventLoopWaker` (`clone_box` + `wake`) | `components/servo/examples/winit_minimal.rs:157-166` |
| `Opts` / `Preferences` | `components/config/opts.rs` |

### Problems encountered and how they were solved

**P1. No Servo source existed.** The working directory was empty. Temp dirs
named `servo-pkg/servo-0.6.0` and `egui-pkg/*` existed but contained **zero
files** — stale artifacts from a previous session, not a checkout. Resolved by
cloning `https://github.com/servo/servo` at `--depth 1` into `servo/`
(196,357 files, ~700 MB).

**P2. The crates.io `servo` crate IS Servo — but it is not our checkout.**
Initially mis-assumed these were unrelated; corrected by querying the crates.io
API. The `servo` crate on crates.io is published from `servo/servo` itself
(`repository: "https://github.com/servo/servo"`, trustpub-verified). The only
unrelated occupant of that name is `servo 0.0.1` (53 bytes, published 2014,
superseded).

The important consequence is **staleness, not identity**:

| | crates.io `servo 0.6.0` | our checkout |
| --- | --- | --- |
| Published / dated | 2026-09-25 | 2026-09-30 (`b2ef57f`) |
| Source sha | `c78d2c206f80a1c8b67eefa97f773bba513205d3` | `b2ef57f642816e77e67e5a322ddccdbde6784f3c` |
| `default` features | `["bundled", "clipboard", "js_jit"]` | `["bundled", "clipboard", "js_jit", "multiprocess"]` |

So a `servo = "0.6"` crates.io dependency would give us code **five days older**
than our checkout, missing the `multiprocess` default. **We depend on the path
`./servo/components/servo`**, which is also what makes Servo updatable by
changing one line. Note for the future: `0.1.x` versions on crates.io are from an
older versioning scheme and do **not** correspond to `0.6.0`.

**P3. Nearly every widely-circulated Servo embedding example is now wrong.**
The old `WindowMethods` / `WindowEvent` / `EmbedderMsg` / `Servo::new_webview`
API from `paulrouget/servo-embedding-example`-style tutorials **does not exist**
in 0.6.0. Verified absent by direct search:

| Removed / renamed in 0.6.0 | Replacement |
| --- | --- |
| `Servo::new_webview(url)` | `WebViewBuilder::new(&servo, ctx).url(url).build()` |
| `WebView::focus()` / `WebView::blur()` | `WebView::set_focused(bool)` |
| `WindowMethods`, `EmbedderDelegate`, `WindowEvent`, `EmbedderMsg` | `WebViewDelegate`, `ServoDelegate`, handle methods |
| `WindowEvent::{NewBrowser, SelectBrowser, Reload, ...}` | `WebView::{load, reload, go_back, ...}` |

Resolution: treat `components/servo/examples/winit_minimal.rs` (168 lines,
in-tree, current) as the canonical reference. Never use a blog post as an API
authority.

**P4. Hermes is one engine generation stale and contains wrong calls.** Hermes
pins `servo = "=0.4.0"`. Cross-checking its source against 0.6.0 found that it
calls **`WebView::set_throttled(bool)`**, which **does not exist** in 0.6.0
(`grep -i throttl` over `components/servo/*.rs` returns zero hits), and it uses
`focus()`/`blur()` which are now `set_focused(bool)`. Resolution: Hermes was used
for *architectural ideas only*; every API claim was re-verified against our
checkout. See `docs/hermes-research.md`.

**P5. Hermes uses Slint, not egui — so it is not a UI precedent.** Its workspace
pins `slint ~1.17.1` with `backend-winit` + `renderer-femtovg`. The brief asks
for egui. Resolution: keep Hermes' *layering* (engine-neutral core + adapter),
discard its toolkit.

**P6. Big surprise that resolves the egui question.** Servoshell **already uses
egui 0.34.3 + egui_glow, sharing Servo's exact `glow::Context`**
(`ports/servoshell/desktop/gui.rs:228-234`), and composes the chrome over web
content with an offscreen-FBO blit registered as an egui `PaintCallback`
(`gui.rs:666-678`). Servo's own doc comment recommends this strategy
(`rendering_context.rs:391-398`). Resolution: the brief's GUI choice needs no
justification and no new graphics abstraction — we reuse a proven upstream path.

**P7. Servo builds on Windows, but our workspace profile problem is real.**
Verified Windows is a supported, CI-covered target (`README.md:46-57`,
`.github/workflows/windows.yml`). However `servo/Cargo.toml:393-399` carries a
Windows-only workaround:

```toml
[profile.dev.package.servo-script]
codegen-units = 16   # otherwise the rlib exceeds 4 GB (rust-lang/rust#151184)
```

**Cargo does not inherit profiles across workspaces**, so a separate workspace
with a path dependency on Servo loses this. Resolution: not yet fixed — this is
the gating risk for the next step. Must be validated before writing browser code.

**P8. Windows requires the `no-wgl` feature.** Both `ports/servoshell` and
`ffi/capi` enable `servo/no-wgl` on Windows
(`components/servo/Cargo.toml:66` → ANGLE via `mozangle`). Without it, Windows
uses native WGL. Recorded in `docs/dependencies.md`.

**P9. Local machine is not fully bootstrapped for a Servo build.** Checked:

| Item | Status |
| --- | --- |
| VS 2022 Build Tools 14.44.35207 | present |
| Windows SDK 10.0.26100.0 | present (≥ 10.0.19041.0 required) |
| C++ ATL for v143 | **missing** (`atlmfc` absent) |
| clang / clang-cl / libclang.dll | present |
| CMake | present |
| Python 3.11 | present |
| `uv` | **missing** (build falls back to `python` and warns) |
| `ninja` | **missing** |
| moztools 4.0 | **missing** (`mach fetch` downloads it) |
| rustc 1.97.1 | installed |
| `rustc-dev` + `llvm-tools` for 1.97.1 | **missing** |

Not yet resolved. `mach bootstrap` fixes most of it but triggers a UAC elevation
prompt (it installs GStreamer MSIs) and permanently edits the user `PATH` to add
LLVM. That should be an explicit, confirmed decision, not an accident.

**P10. In-repo Servo docs are stubs.** `CONTRIBUTING.md` and
`docs/HACKING_QUICKSTART.md` are five-line redirects to `book.servo.org`, and the
book is not in the checkout. `README.md:46-57` is the only local Windows build
documentation. Consequence: for anything beyond the README, source code was the
authority.

**P11. A separate Cargo workspace will NOT inherit Servo's build configuration.**
This was the most important discovery of the research phase, and it was only
found by actually probing rather than assuming.

Cargo does not inherit `[profile.*]` tables or `.cargo/config.toml` across
workspaces. Servo depends on both. A browser crate in its own workspace that
path-depends on `servo/components/servo` would silently lose:

| Lost setting | Source | Consequence if missing |
| --- | --- | --- |
| `[target.x86_64-pc-windows-msvc] linker = "lld-link.exe"` | `servo/.cargo/config.toml:16-17` | Servo would link with the default MSVC `link.exe` instead of LLD |
| `[env] RUSTC_BOOTSTRAP = "crown,script,script_webgpu,script_bindings,style_tests,mozjs,mozjs_sys"` | `servo/.cargo/config.toml:33` | breaks only if the `crown` feature is enabled — see below |
| `[profile.dev.package.servo-script] codegen-units = 16` | `servo/Cargo.toml:393-399` | **build fails**: rlib exceeds 4 GB (rust-lang/rust#151184) |
| `[profile.dev.package.crypto-bigint] opt-level = 3` | `servo/Cargo.toml:390-391` | slower build |

Follow-up investigation on `RUSTC_BOOTSTRAP`: a recursive search for `#![feature`
across `servo/components/` returned **zero** unconditional hits. Every hit is
`#![cfg_attr(crown, feature(...))]` (e.g.
`components/script_bindings/lib.rs:5`, `components/script/lib.rs:5`), i.e. the
nightly features activate only when the `crown` cfg is set, and `crown` is **not**
in Servo's default feature set. So `RUSTC_BOOTSTRAP` is currently inert for a
default build. We replicate it anyway for fidelity, at zero cost — but it should
not be treated as load-bearing, and it should not be extended to our own crates.

Resolution: `docs/next-step.md` specifies the exact config our workspace must
carry, and `docs/servo-version.md` records it as a per-bump re-verification item.

**P12. Manifest-level workspace integration is confirmed working.** Verified with
a throwaway crate outside the project: `servo = { path = ".../servo/components/servo",
features = ["no-wgl"] }` resolves cleanly under `cargo metadata --no-deps`, and
Cargo reports our crate as the sole workspace member with `servo` as a path
dependency. **There is no "member of the wrong workspace" error.** So a separate
workspace is viable; what remains unverified is only whether the *compilation*
succeeds with the P11 config replicated. Probe crate has been deleted.

### Open questions carried forward

1. Does cross-process paint reach an embedded rendering context on 0.6.0?
   (Hermes reports "no" on 0.4.0 → blank pages.) **Must be tested before
   enabling multiprocess.** Security-relevant; do not guess.
2. What replaces `WebView::set_throttled` for background-tab cost reduction?
   No public throttling API exists in 0.6.0.
3. Can a separate Cargo workspace build Servo on Windows given the profile
   caveat (P7)?
4. `notify_animating_changed` is the only public signal that a webview is
   animating. Is it sufficient to drive our event loop, or do we need our own
   scheduler?

### Next step (proposed, not started)

**Validate the build before writing any browser code.** See
`docs/next-step.md`.

---

## 2026-09-29 — Entry 002: Build spike scaffolding

### What changed

Created the workspace and the Milestone 1 spike:

```text
Ferrous Browser/
  .cargo/config.toml        (replicated from servo/.cargo/config.toml)
  rust-toolchain.toml       (pinned to 1.97.1)
  .gitignore
  browser/
    Cargo.toml              <-- WORKSPACE ROOT + package
    src/main.rs             <-- the spike
  docs/
  servo/                    (dependency checkout, not a member)
```

Also ran `.\mach.ps1 bootstrap --yes` on this machine (UAC-approved by the user),
and installed `uv 0.12.21` to `C:\Users\User\.local\bin`, which `mach` requires.

### Why

The user approved both pending decisions: full `mach bootstrap`, and a separate
workspace at the project root. Entry 001's next-step proposal was to spike before
writing browser code, because all six Milestone 1 criteria depend on unverified
facts.

### Problems encountered and how they were solved

**P13. A workspace root at the project root CANNOT path-depend on the Servo
checkout. The workspace root had to move inside `browser/`.**

This was the blocking defect, and it only appeared when the manifest was actually
parsed rather than reasoned about.

With `Ferrous Browser/Cargo.toml` as the workspace root and `browser/` as a
member, `cargo metadata` failed:

```text
failed to load manifest for workspace member `...\browser`
  caused by: failed to load manifest for dependency `servo`
  caused by: failed to parse manifest at `...\servo\components\servo\Cargo.toml`
  caused by: error inheriting `edition` from workspace root manifest's
             `workspace.package.edition`
  caused by: `workspace.package.edition` was not defined
```

Mechanism: Servo's crate manifests use `edition.workspace = true`,
`version.workspace = true`, `license.workspace = true`, and friends. Cargo
resolves those against the nearest ancestor manifest declaring `[workspace]`.
Because our root was an **ancestor directory** of `servo/`, Cargo bound Servo's
manifest to *our* `[workspace]` — and our manifest has no `[workspace.package]` at
all.

Resolution: move the workspace root into `browser/`, making it a **sibling** of
`servo/` rather than an ancestor. Servo's own `Cargo.toml` is then found and its
`[workspace.package]` resolves.

Verified empirically with three throwaway probes (since removed):

| Layout | Result |
| --- | --- |
| No workspace root; package alone with path dep to Servo | resolves |
| Workspace root that is **not** an ancestor of the Servo path | resolves |
| Workspace root that **is** an ancestor of the Servo path | fails as above |

Note `.cargo/config.toml` and `rust-toolchain.toml` stay at
`Ferrous Browser/` and still apply: both cargo and rustup discover them by walking
*up* from the current directory, so running cargo from `browser/` or from the
project root finds them either way.

**P14. `keyboard_types::Key` has no `Unidentified` constant.** First draft of the
spike used `Key::Unidentified`. `Key` is actually
`enum { Character(String), Named(NamedKey) }`
(keyboard-types-0.8.3/src/key.rs:13-22); the unidentified state is
`Key::Named(NamedKey::Unidentified)`. Fixed.

Also confirmed while fixing it that `Key`'s `FromStr` is safe for named keys: it
routes through `is_key_string`, which rejects any multi-character ASCII string
(`key.rs:150-152`), so `"Enter"` parses to `Key::Named(NamedKey::Enter)` rather
than a bogus `Character("Enter")`.

**P15. Modifier state was being dropped.** The spike passed
`ModifiersState::default()` for every key event, so Shift/Ctrl would never reach
the page. Fixed by tracking `WindowEvent::ModifiersChanged` in `AppState` and
threading the live state into `keyboard_event`.

**P16. Pointer events were gated on redraw.** A first draft only forwarded
`MouseMove` when a repaint was also pending. Since repaints are driven by
`notify_new_frame_ready`, hovering would never reach the engine and the
cursor-change-over-links behaviour would be dead. Fixed: `CursorMoved` is
delivered immediately; `last_pointer` exists only to give wheel events a position.

**P17. New WebViews are visible by default — no `show()` needed.** Worth
recording because it is not documented on `WebView::show()`.
`WebViewRenderer::new` initialises `hidden: false`
(`components/paint/webview_renderer.rs:161`), so the spike omitting `show()`
(correctly following `winit_minimal.rs`) is right.

### Servo APIs exercised by the spike

All previously verified in Entry 001. The spike additionally relies on:

- `WindowRenderingContext::new(display_handle, window_handle, size)`
- `WebView::resize(PhysicalSize<u32>)`, `WebView::paint()`
- `MouseMoveEvent::new(WebViewPoint)`, `MouseButtonEvent::new(action, button, point)`
- `WheelEvent::new(WheelDelta, WebViewPoint)`
- `KeyboardEvent::new_without_event(state, key, code, location, modifiers, repeat, composing)`
- `WebView::notify_input_event(InputEvent) -> InputEventId`

### Open questions carried forward

1. **Does Servo actually compile here?** Bootstrap is still running. A full
   `cargo check` on the spike is the next action and is expected to be slow
   (SpiderMonkey native build).
2. Does cross-process paint reach an embedded rendering context on 0.6.0?
3. What replaces `WebView::set_throttled` for background-tab cost reduction?
4. Is `notify_animating_changed` sufficient to drive the event loop, or do we
   need our own scheduler?

---

## 2026-09-29 — Entry 003: Disk reclamation, build profile, CI prep

### What changed

- Freed **~11.1 GB** (55.1 → 65.8 GB free): stale `%TEMP%` including a 2.71 GB
  prior-session probe build, `npm-cache`, Windows Update download cache, and
  WSL `Ubuntu` unregistered (7.75 GB, at the user's explicit instruction).
- Switched the build to Servo's `medium` profile.
- Added `scripts/copy-angle-dlls.ps1`.
- Added `.github/workflows/windows.yml` (private repo; **not pushed**).

### Problems encountered and how they were solved

**P18. `mach build` CANNOT build our binary. The user-selected "use mach as the
single source of truth" is only half-achievable.**

`mach build` calls `run_cargo_build_like_command("rustc", opts, ...)` with no
`-p` and no package selection (`python/servo/build_commands.py:164`). Servo's
workspace sets `default-members = ["ports/servoshell"]` (`servo/Cargo.toml:8`),
so `mach build` builds **servoshell**. Our crate lives in a different workspace
(`browser/`) and is invisible to `mach`.

Resolution — split the responsibilities honestly rather than pretending one tool
does everything:
- **Servo-side environment** (moztools, MSVC redist, resource copying) → `mach`.
- **Our crate** → `cargo build`.
- **ANGLE DLL staging** → our own script (P19).

**P19. Root cause of the ANGLE DLL problem (found, not assumed).**

We enable `servo/no-wgl` on Windows, which routes GL through ANGLE
(`components/servo/Cargo.toml:66`). Tracing `mozangle-0.7.1/build.rs`:

```rust
build_windows_dll(&build_data::GLESv2, "libGLESv2", ...);
build_windows_dll(&build_data::EGL, "libEGL", ...);
let out = env::var("OUT_DIR").unwrap();
println!("cargo:rustc-link-search={out}");
println!("cargo:rustc-link-lib=libEGL");
```

The build script **compiles ANGLE's C++ itself** and emits the DLLs into its own
`OUT_DIR`. Our binary is *linked* against `libEGL` but the DLL is never placed
beside the executable, so Windows cannot load it at startup.

Upstream fixes this in `copy_windows_dlls_to_build_directory`
(`python/servo/build_commands.py:327-345`) — but it runs under
`mach run-post-build-tasks` and targets the servoshell binary only.

Resolution: `scripts/copy-angle-dlls.ps1` performs the same copy for our binary.

Consequence worth stating plainly: **a bare `cargo build` works on this machine
only because `mach bootstrap` previously ran here.** On a clean machine it
produces a binary that cannot start. That is now fixed, and the CI workflow
enforces it.

**P20. moztools is a hard build prerequisite, not a nicety.** SpiderMonkey's build
needs moztools 4.0, which only `mach fetch`/`mach bootstrap` downloads (into
`servo/target/dependencies/moztools`). CI runs `mach fetch` explicitly for this.

**P21. Custom profiles do NOT inherit `profile.dev.package.*` overrides.** Adding
`[profile.medium] inherits = "dev"` silently dropped the Windows-critical
`servo-script codegen-units = 16` override. Fixed by duplicating it under
`[profile.medium.package.servo-script]`. This would have been an invisible,
intermittent failure.

### Build progress at time of writing

SpiderMonkey (`mozjs_sys v153.3.0`) compiled successfully — the single largest
unknown is resolved. ANGLE's C++ compile is running inside mozangle's build
script right now. 566 crates, **0 errors**. `browser/Cargo.lock` now exists,
which the CI workflow pins via `--locked`.

---

## 2026-09-29 — Entry 004: MILESTONE 1 ACHIEVED

### Outcome

The spike compiles, runs, and renders real web content on this machine.

| Milestone 1 criterion | Status | Evidence |
| --- | --- | --- |
| Application launches | PASS | process starts, 57 threads |
| Window opens | PASS | titled "Ferrous spike" |
| Servo initializes | PASS | no panics, no stderr |
| A webpage loads | PASS | `https://servo.org` over TLS |
| **Page renders** | **PASS** | screenshot: SVG logo, text, CSS-styled buttons — not a blank frame |
| **Mouse input** | **PASS** | synthetic click navigated to the Contributing page |
| Keyboard input | **UNVERIFIED** | no text input on the test page; not yet exercised |

Build: 117.8 MB executable, 450 MB PDB, 186 MB working set at idle-with-a-page.

### Compile errors hit in our own crate (10, all in the 346-line spike)

The engine compiled clean first time. All errors were in code we wrote:

1. **Double-bound pattern binding.** `WindowEvent::MouseInput { state: button, button, .. }`
   bound `button` twice (E0416).
2. **Wrong coordinates for `WebViewPoint`.** Used `WebViewPoint::Page`, which is
   *CSS pixels after page-zoom and pinch-zoom* — only equal to window coordinates
   in the degenerate case of no chrome and zoom 1.0. Corrected to
   `WebViewPoint::Device`, matching servoshell (`geometry.rs:13`,
   `headed_window.rs:326`).
3. **`CursorMoved` yields `PhysicalPosition<f64>`** in winit 0.30, not logical.
4. **`run_app` returns `Result<(), EventLoopError>`**, not `Box<dyn Error>`.
5. **Scroll type mismatch.** winit line deltas are `f32`; Servo's `WheelDelta` is
   `f64`.
6. **Ambiguous `.into()`.** Servo provides *two* `From<Point2D<f32, _>>` impls for
   `WebViewPoint` (CSSPixel and DevicePixel), so inference fails (E0283). Now
   spelled explicitly.
7. **`Rc` is read-only.** `last_pointer` and `modifiers` needed `Cell` because
   `AppState` is shared with the delegate as `Rc<dyn WebViewDelegate>` (E0594).
8. **Pattern shadowing.** Renaming `state: button` to `state` shadowed the outer
   `Rc<AppState>` inside the match arm (E0609).

The most valuable of these is #2: `Page` vs `Device` is a silent correctness bug,
not a compile error, and it would have broken hit-testing the instant browser
chrome was added.

### P19 confirmed empirically

`scripts/copy-angle-dlls.ps1` found the ANGLE DLLs in
`target/medium/build/mozangle-56c8504a11462356/out/` — present but **not** beside
the executable. Without the copy step the binary would have failed to start with
a missing-DLL error. This is now a hard prerequisite, enforced in CI.

### What is still unproven

- Keyboard input (needs a page with a text field).
- Resize handling under real user resize.
- Whether cross-process paint works when embedded (still defaulting to
  single-process via `Opts::default()`).
- Anything at all about tabs, chrome, or navigation UI.

---

## 2026-09-29 — Entry 005: MILESTONE 2 — browser shell

### What changed

Split the single 346-line spike into four focused modules and added the browser
shell: back / forward / reload buttons plus an address-and-search bar, composed
over the live web view.

```text
browser/src/
  main.rs           window, event loop, frame orchestration, input routing
  chrome.rs         egui toolbar; returns Action list + content rect
  browser_state.rs  state shared between UI and engine delegate
  engine.rs         the ONLY module that knows Servo types exist
  address.rs        URL-vs-search classification (pure, 9 unit tests)
```

Also added `LICENSE` (MPL-2.0 — matches Servo, and we will likely need to patch
Servo, which MPL-2.0 obliges us to publish).

### The architecture that made this work

`App` **owns** `AppState`; only the delegate holds `Rc`s. My first attempt put
`EguiGlow` and `Chrome` behind an `Rc` and reached them through `unsafe` pointer
casts to get `&mut`. That was wrong — it is exactly the kind of unsafe hack the
project rules forbid — and it was unnecessary, because the delegate only ever
needs `Rc<BrowserState>` and a way to request a redraw.

With `App` owning `AppState`, `draw_frame` destructures the fields so that
`egui` is borrowed mutably at the same time as `engine`/`chrome`. Disjoint field
borrows, no unsafe.

### Problems encountered and how they were solved

**P22. `Ui::available_rect_before_wrap()` does not account for panels.** The
first working build logged `content_rect = [0,0]-[1100,820]` — the full window,
meaning the toolbar consumed zero height and the page covered it completely.

The method is documented as "what is left on this row/column before wrapping";
with a non-wrapping top-level layout it returns the full screen. There is no
`Context` equivalent either.

Fixed by reading the toolbar panel's own `response.rect` and constructing the
content rect from it, which is what servoshell does (`desktop/gui.rs:608`).

**P23. `Panel::show` is deprecated in egui 0.34.3** (`#[deprecated = "Use
show_inside() instead"]`, epaint/panel.rs:1048). Using `show_inside(ui, ..)`
against the root `Ui` both silences the deprecation and is what makes the
panel participate in layout correctly.

**P24. `winit::window::Window` is no longer `Clone` in winit 0.30** (it was in
0.29). The delegate needs the window to request repaints, so `AppState` holds
`Rc<Window>` and passes a clone of the `Rc`.

**P25. The blit callback needs euclid geometry, not egui rects.**
`render_to_parent` expects `Rect<i32, UnknownUnit>` in device pixels with a
bottom-left origin. Passing an `egui::Rect` (f32 points, top-left) is a type
error; the imports are aliased (`EuclidRect`, `EuclidPoint`, `EuclidSize`) so the
two `Rect` types cannot be confused.

**P26. The ANGLE copy script copied files onto themselves** once the DLLs were
already staged. It searched recursively and found the staged copy first. Now it
searches only `target/*/build/mozangle-*/out/`, which is where mozangle's
`build.rs` actually emits them.

**P27. Cursor coordinates need translating out of the toolbar.** A raw winit
cursor position is in *window* coordinates; the page's origin is below the
toolbar. Without subtracting the content rect, clicking the toolbar would
register as a click at the top of the page. `to_page()` does this conversion.

### Verified working (screenshots)

- Window opens with toolbar; back/forward correctly **disabled** with no history.
- Page renders beneath the chrome — egui and Servo share one GL context, and the
  offscreen FBO blit composites correctly.
- Real GPU: `Renderer: ANGLE (AMD, AMD Radeon 780M Graphics Direct3D11,
  OpenGL ES 3.0)`.
- **Mouse hover** — links highlight.
- **Mouse click** — opened the "Get Involved" dropdown.
- **Address bar updates live** from `notify_url_changed` (picked up `#`).
- Input routing confirmed by instrumentation: correct `content_rect`, and
  keyboard events correctly *withheld* from the page while the address bar has
  focus.

### NOT verified

- Back / forward / reload buttons actually navigating.
- Address-bar navigation (typing + Enter).
- Keyboard input reaching page content.

Synthetic mouse clicks kept missing because the window moves between
`SetForegroundWindow` and the click, and synthetic `SendInput` did not reliably
reach the TextEdit. I stopped rather than keep guessing at screen coordinates.
**These need a human at the keyboard.** They are wired (`apply_action` handles
each variant, `address::resolve` is unit tested) but "wired" is not "verified".

### Also worth noting

An earlier observation that the address bar appeared focused at startup was an
artifact of my own test clicks, not a bug: instrumented logs show
`address_focused=false` in normal operation.

---

## 2026-09-30 — Entry 006: Fix address bar (user-reported)

### Symptom

User reported the search bar did not work, while reload and the back/forward
buttons did.

### Root cause

The submit condition was inverted. `chrome.rs` had:

```rust
if focused && enter && !text.trim().is_empty() { /* navigate */ }
```

A **singleline** egui `TextEdit` deliberately surrenders focus when Enter is
pressed — `egui-0.34.3/src/widgets/text_edit/builder.rs:108`:

> No newlines (`\n`) allowed. Pressing enter key will result in the
> `TextEdit` losing focus (`response.lost_focus`).

So by the time the frame ran, `response.has_focus()` was already `false`, and
`focused && enter` could never be true. Enter did reach egui and did blur the
box; the navigation branch was simply unreachable.

egui documents the correct idiom at `builder.rs:35`:

```rust
if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) { /* .. */ }
```

Fixed to `if response.lost_focus() && enter && ...`.

**General lesson:** for widgets that react to a key by surrendering focus, the
submit signal is "lost focus this frame", not "has focus". Checking focus *after*
the widget has processed the key is a race with itself.

### How it was found

Not by reading. Instrumented `address_bar` to log
`rect / focused / lost_focus / enter / text` every frame, which immediately
showed `focused=false enter=false` after Enter — and, crucially, showed Enter was
being received at all. That ruled out input-injection problems and pointed at the
condition itself.

Two input-injection problems were also ruled out along the way, so as not to
chase them later:

- A hand-rolled `SendInput` `INPUT` struct with the union at offset 0. On x64
  Windows the union sits at **offset 8**; with the wrong layout nothing is
  delivered at all and the app simply draws no new frames.
- `keybd_event` with `[byte][char]'.'` (0x2E) is **VK_DOWN**, not a period, so
  injected URLs silently lose their dots. Only affects test harnesses.

### Verified after the fix

Screenshot shows the full DuckDuckGo results page for `examplecom`, with the
address bar reading `https://duckduckgo.com/?q=examplecom&ia=web`, and the
back/forward buttons now **enabled** — confirming `notify_history_changed` is
driving toolbar state correctly.

Also confirmed: `address::resolve` classified a bare word as a search query and
redirected to the search endpoint, while `https://…` input passes through as a
URL. That split is unit tested.

### Milestone 2 status

| Feature | Status |
| --- | --- |
| Toolbar renders over the page | verified |
| Reload | verified by user |
| Back / forward | verified by user |
| Address bar: accept input | verified |
| Address bar: Enter navigates | verified (this fix) |
| Search queries -> DuckDuckGo | verified |
| URL passthrough (typed URL) | unit tested, not clicked by hand |
| Keyboard reaching page content | still unverified |
| Resize handling | still unverified |

---

## 2026-09-30 — Entry 007: QoL pass (select-all, escape, spinner, title)

### What changed

Four additions, all driven by state the browser **already tracked** and none
adding a dependency:

1. **Select-all when the address bar gains focus**, so typing replaces the URL
   instead of appending to it.
2. **Escape abandons an edit** and restores the address of the page actually
   loaded.
3. **Loading spinner** in the toolbar, bound to `notify_load_status_changed`.
4. **Window title reflects the page title** (`<title> - Ferrous`).

Plus a safety net: losing focus without pressing Enter restores the real address,
so the bar never displays text the browser is not actually on.

Dead code found by the compiler and removed rather than kept: `Action::FocusAddressBar`
(never constructed — Ctrl+L calls `Chrome::request_focus_address_bar` directly),
`Engine::can_go_back`/`can_go_forward`/`page_title` (superseded by the delegate's
history events), and the unused `engine::Pointer` struct.

### Problems encountered and how they were solved

**P28. Select-all initially only worked for Ctrl+L, not for clicks.** The first
implementation armed `select_all_pending` inside the `request_focus_address_bar`
path, so a plain mouse click — which reaches egui's own focus handling and never
calls our request — never selected anything. Typing appended.

Verified by screenshot: address bar read `https://servo.org/z` instead of `z`.

Fixed by detecting the focus-*gained* **edge** (`focused && !focused_last_frame`)
rather than driving it from the focus request. Now `text="z"` after typing one
character — the URL is genuinely replaced.

**P29. egui 0.34.3 has no `select_all_on_focus`.** The full `TextEdit` option
list was enumerated to confirm this; the selection has to be set through the
widget's public `TextEdit::load_state` / `store_state` plus
`TextCursorState::set_char_range(CCursorRange::two(..))`.

This required giving the field an **explicit** `Id`, because `load_state` needs
to find the same widget the TextEdit uses and an auto-generated id drifts as the
surrounding layout changes.

Footgun worth remembering: `CCursor` indexes by **character**, not byte. Using
`str::len()` panics on any non-ASCII URL, so the count is taken with `chars()`.

**P30. `desired_width(f32::INFINITY)` starved the spinner of width.** The
address field claimed the whole toolbar row, leaving the indicator zero width,
so it could never have been seen. Replaced with `ui.add_sized([available - 24, 22])`,
reserving a slot.

**P31. `Context::screen_rect()` is deprecated** in favour of `content_rect()`
(egui 0.34 deprecation notice: *"screen_rect has been split into viewport_rect()
and content_rect()"*). Switched; `content_rect()` is also the semantically
correct one for "the area actually available to draw in".

### Verified

- Select-all on focus: typed one character, `text="z"` (was
  `https://servo.org/z`).
- Escape: text reverted to `https://servo.org/`, focus surrendered.
- Window title: `Wikipedia - Ferrous`, then
  `Servo aims to empower developers... - Ferrous`.
- Ctrl+L + select-all + URL entry navigates to a **URL** (wikipedia.org), not a
  search — confirming `address::resolve`'s scheme-passthrough branch.
- Search entry still routes to DuckDuckGo.
- Address bar now reserves trailing space for the indicator.
- 9 unit tests pass; smoke test clean; no warnings from our own crate.

### NOT verified

- **The spinner was never caught in a screenshot.** Every page tested loaded
  faster than the capture. The layout fix is confirmed (the field visibly ends
  short of the row), and the binding is one line, but the rendered spinner
  itself is unconfirmed. Worth a glance on a slow connection.---

## 2026-09-30 — Entry 008: Smooth scrolling and a custom frameless title bar

### What changed

- `browser/src/scroll.rs` — **new.** `SmoothScroll`, an engine-free, unit-tested
  exponential-decay animation, plus `scroll_delta_from_wheel`.
- `browser/src/titlebar.rs` — **new.** Title-bar drawing, resize-border hit
  testing, and the Win32 interop.
- `browser/src/icons.rs` — **new.** Hand-painted icons for both the window
  controls and the navigation buttons.
- `browser/src/chrome.rs` — nav buttons are now painted shapes, not text glyphs.
- `browser/src/engine.rs` — added `notify_scroll_event`.
- `browser/src/main.rs` — `with_decorations(false)`, title-bar panel, scroll
  pump, resize-border routing, `hwnd` extraction, `seen_url` navigation check.

### Why

Two requests: smooth scrolling, and replace the OS title bar. Neither exists in
Servo or winit, so both had to be built.

### Servo APIs verified (read from source)

| Fact | Location |
| --- | --- |
| `Scroll` has only `Delta`/`Start`/`End` — **no animated variant** | `components/shared/embedder/lib.rs:173` |
| There is **no `smooth_scroll` preference anywhere in Servo** | grep over `components/` returns nothing |
| `WebView::notify_scroll_event(&self, Scroll, WebViewPoint)` | `components/servo/webview.rs:603` |
| `Scroll::Delta(WebViewVector)`, and `WebViewVector::Device(DeviceVector2D)` | `components/shared/embedder/lib.rs:138` |
| **"A scroll delta for a wheel event is the inverse of the wheel delta."** | `components/paint/webview_renderer.rs:1219-1222` |
| servoshell sets `with_decorations(false)` but implements **no** window dragging or controls | `ports/servoshell/desktop/headed_window.rs:114-124` |

So smooth scrolling is ours to build, and servoshell gives no reference for the
frameless window.

### winit 0.30.13 gaps (why Win32 interop is unavoidable)

Checked directly in the winit source:

- **No** `Window::start_drag`
- **No** `Window::begin_resize_drag`
- **No** `maximize` / `unmaximize` — only `is_maximized() -> bool`
- **No** `Window::close()` at all, not under any name

A frameless window without those is a window the user cannot move, resize,
maximise or close. That is worse than keeping the native title bar, so the drag,
the edge resize and the maximise/restore are handed back to Windows with
`ReleaseCapture` + `SendMessageW(WM_NCLBUTTONDOWN, HT*)`. Windows then runs its
own modal loop, which keeps edge snapping, Aero Shake and double-click-to-maximise
working — none of which a hand-rolled "track the cursor, call `set_outer_position`"
loop gets right.

Closing is the exception: with no `Window::close`, the close button sets a flag
that `draw_frame` returns, and `window_event` calls `event_loop.exit()`.

### This is the project's first `unsafe`

One block, confined to `titlebar.rs::platform`, genuine platform interop rather
than a type-erasure workaround: three `extern "system"` declarations and one
`GetCursorPos`. Every call site carries its own `SAFETY` note, and the Win32
constants (`WM_NCLBUTTONDOWN = 161`, `HTLEFT = 10` … `HTBOTTOMRIGHT = 17`,
`SW_MAXIMIZE = 3`, `SW_RESTORE = 9`) were read out of `windows-sys 0.45.0` in the
registry rather than written from memory.

### Problems hit

**P32. The smooth-scroll tests caught the implementation, not the reverse.**
They asserted the full queued distance was delivered; the implementation
deliberately discards a sub-pixel tail so the exponential decay terminates.
The tests were wrong, not the code — an animation that eases forever would keep
waking the event loop indefinitely, which is precisely the idle-CPU cost this
project is trying to avoid. Rewrote the tests to state the real contract.

**P33. egui's bundled font has no Arrows, Geometric Shapes or Dingbats.**
`Button::new("\u{2190}")` renders as a `.notdef` tofu box. Discovered by zooming
into a screenshot: the window controls and the *pre-existing* navigation buttons
were all unreadable squares. Every icon is now painted with `egui::Painter`,
which also makes them DPI-crisp and avoids shipping a font for six shapes.

**P34. `ui.horizontal` inserts item spacing between widgets.** The title bar
reserved `3 * BUTTON_WIDTH` for the controls but the layout spent
`3 * BUTTON_WIDTH + 2 * spacing`, pushing Close off the right edge — visible only
after zooming into the corner. Fixed by zeroing `item_spacing.x` inside the
title bar's own `Ui`.

**P35. Window controls were in reverse order.** `ui.horizontal` lays out in array
order, so iterating `[Close, Maximize, Minimize]` produced close-maximize-minimize.
Windows convention is minimize, maximize, close.

**P36. `resize_edge_at` had no bounds check.** A point to the *left* of the
window has a *negative* distance to the right edge, which satisfies
`<= RESIZE_BORDER` and was reported as a right-edge resize. winit reports cursor
positions relative to the client area, and those fall outside it during a drag.
Added an explicit bounds check plus a regression test.

**P37. `lParam` must carry the real cursor position.** Sending `0` looks
harmless and a *bottom*-edge resize even works, but Windows validates the point
against the window bounds — with `0, 0` a left, right or caption drag is
silently discarded while bottom still goes through. This asymmetry is what made
it look like "some edges work". Fixed by packing `GetCursorPos` into an LPARAM.

**P38. Scrolling was silently inverted.** The single worst bug of the entry.
Servo negates the wheel delta internally
(`components/paint/webview_renderer.rs:1219-1222`); passing winit's value
straight through scrolled every page the *wrong way*. Because the start page is
already at the top, the symptom was "scrolling does nothing" — which looks like
a missing feature rather than a sign error. Only found by screenshotting before
and after. Fixed in `scroll::scroll_delta_from_wheel`, with four tests that
encode the convention so it cannot regress silently.

**P39. egui API details that differ from the obvious guess.**
`Ui::allocate_exact_size` returns `(Rect, Response)` in egui 0.34, not
`(Response, Rect)`. `Painter::rect_stroke` takes a fourth `StrokeKind` argument.
`Response::on_hover_text` *consumes* the `Response` and returns an `Option`, so
the click test must happen before it. Edition 2024 requires `unsafe extern`
blocks.

**P40. `Panel::top` + `Panel::top` leave no gap**, so drawing the title-bar panel
before the toolbar panel is sufficient to push the content rectangle below both —
no manual offset arithmetic needed.

### Verified on screen

- Title bar shows the page `<title>`, with minimize / maximize / close painted,
  in the correct order, inside the window bounds. Maximised state switches the
  middle glyph to the restore form.
- **Maximize** button maximises (`IsZoomed` true, 1936x1048).
- **Restore** button restores (1100x820).
- **Minimize** button minimises (`IsIconic` true).
- **Close** button exits the process cleanly.
- **Title-bar drag** moves the window.
- **Left, top and bottom** edge resize all move the correct edge.
- Navigation buttons (back / forward / reload) are now legible painted arrows.
- **Smooth scrolling** works and goes the right way: 8 notches down scrolls well
  down the page, 14 notches up returns to the top.
- **Idle CPU is 0 ms over an 8-second window** — the `is_animating()` guard
  means the event loop is not kept awake after a glide finishes.
- Working set 174 MB. 28 unit tests pass. No warnings from our crate.

### NOT verified — needs a human

- **Right-edge resize.** It behaved inconsistently under synthetic input: two
  attempts produced two *different* wrong answers (once the left edge followed,
  once both edges shifted). Left, top, bottom and the caption drag all pass
  through the identical `send_non_client` path, so the mechanism is sound, but
  the right edge specifically could not be confirmed. Suspect the test harness
  rather than the code: the drag is synthesised with `SetCursorPos` *inside*
  Windows' own modal loop, which is not how a real mouse behaves. **Must be
  checked with a real mouse before this is called done.**

- Smooth scrolling has been verified as *functional and correctly directed*, but
  not *judged as feeling smooth* — that needs a human hand on a real wheel, and
  ideally a trackpad, since `PixelDelta` and `LineDelta` take different paths.

### Still open from earlier entries

- Loading spinner never caught in a screenshot (P30/layout confirmed only).
- Keyboard input reaching page content never confirmed with a text field.
- `Opts::default().multiprocess` is still `false`; cross-process paint untested.
---

## 2026-09-30 — Entry 009: Fix title-bar drag wedging the UI (user-reported)

### Symptom

Reported after Entry 008 landed:

> the custom title bar makes it really really laggy. i now cant click on the
> buttons and the link address bar. i can use it to drag the window

### How it was reproduced

Not by eye — by measurement. CPU time over a fixed window, before and after one
synthetic title-bar drag:

| | CPU over the following seconds |
| --- | --- |
| at rest | 0 ms |
| after **one** drag | **609 ms over 4 s** |

One drag left the process permanently busy. That is the lag, measured.

### Root cause

`titlebar::draw` gave the drag strip `Sense::click_and_drag()` and, every frame
it was dragged, pushed a `WindowCommand::DragTitleBar`. `draw_frame` applied
those commands after `present()`:

```
draw_frame -> apply_window_command -> SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)
```

`SendMessageW` is **synchronous and pumps messages**. So the sequence was:

1. `WM_NCLBUTTONDOWN` blocks, waiting for the gesture to end.
2. While waiting it pumps messages, so `RedrawRequested` arrives.
3. `draw_frame` runs again. Windows has the mouse captured, so the window never
   receives the release — `dragged()` is still true, so it queues **another**
   `DragTitleBar`.
4. That nested frame applies another `SendMessageW`, which pumps again...

Unbounded nesting. It never unwound because the mouse-up that would have ended
the first drag was consumed by the inner loops. Every other click then landed in
a process that was re-entering this loop, which is why the address bar and the
buttons stopped responding. Dragging "working" was the one path that happened to
complete.

Worth noting the bug was invisible to the obvious test: four plain *clicks* on
the title bar measured 0 ms, because `clicked()` settles within a frame. Only a
real *drag* wedged it.

### The fix

Three changes, all removing a way for the modal loop to re-enter:

1. **The strip is no longer an egui widget.** `Sense::hover()` instead of
   `Sense::click_and_drag()`. It is painted only, so no drag state can ever
   accumulate in egui. The buttons keep `Sense::click()` and are unaffected.
2. **The drag is started from the mouse-down handler**, before egui and before
   the page ever see the event, by geometric hit test
   (`titlebar::is_in_drag_area`). Nothing is queued through a frame, so there is
   nothing for the pumped messages to duplicate.
3. **`run_modal_gesture` refuses re-entry.** `WM_NCLBUTTONDOWN` pumps messages, so
   a second press can genuinely arrive while the first is on the stack; a
   `Cell<bool>` makes a nested loop impossible rather than merely unlikely.

`WindowCommand::DragTitleBar` is gone. Minimize, maximize and close do not block,
so they stay deferred to the end of the frame.

### Consequence for the layout

`titlebar::is_in_drag_area` hit tests against `drag_rect`, which assumes the
painted strip starts at the window origin. egui's default `Frame` has an 8pt
inner margin, so the hit test and the pixels would have disagreed by 8pt and
presses would land slightly off. The title-bar panel is therefore created with
**zero inner and outer margin**, and `drag_rect` is the single source of truth
for the geometry. Five unit tests cover it, including that a window narrower than
the controls clamps the strip to zero width instead of going negative.

### Verified after the fix

| Check | Before | After |
| --- | --- | --- |
| CPU over 4 s after one drag | 609 ms | **0 ms** |
| CPU over 4 s after four drags | — | **0 ms** |
| CPU at rest, 5 s | — | **0 ms** |
| Title-bar drag moves the window | did not | **exact**, e.g. 4 consecutive drags each moved it precisely (-25, +18) |
| Address bar click + type + Enter | unresponsive | **works** — accepted the click, took the text, navigated |
| Reload button | unresponsive | **works** |
| Maximize / restore / minimize | — | **all work** after drags |
| Close | — | **works** |

32 unit tests pass; our crate is warning-free.

The window drag is also *more* accurate than before the fix — in the buggy build
the drag that wedged the app did not actually move the window.

### Not re-verified

Right-edge resize was already unverified in Entry 008 and is still is. The
resize path shares `send_non_client` and the new `run_modal_gesture` guard with
the drag, so it is no worse, but it is not confirmed.
---

## 2026-09-30 — Entry 010: YouTube investigation, and enabling Servo's pref-gated features

### The report

> YouTube doesn't load very well. Maybe it's because of Servo but could you
> investigate?

### What YouTube actually did

Loaded `https://www.youtube.com/` and waited 60 s. The page was **not blank**:

- header, logo, hamburger, search box, mic, overflow menu, Sign in — all rendered
- sidebar rendered
- the body showed YouTube's **logged-out empty state**: *"Try searching to get
  started — Start watching videos to help us build a feed of videos you'll love."*

So the shell worked and the **feed** was missing. No crash, no hang, no error
page. Working set 373–413 MB, CPU slowly ticking (~2% of one core).

### Turning up the logging

`Servo::setup_logging` uses `env_logger::Env::default()` (servo.rs:1080-1090),
which filters at **Error** unless `RUST_LOG` is set. That is why the first run
produced an empty stderr file — not because nothing was wrong.

With `RUST_LOG=info` the log grew to **3.3 MB** and showed:

- **no** JS errors (one `fail to evaluate module`, which servo.org also produces)
- **no** network errors, no TLS failures
- thousands of dropped CSS rules, of three kinds:
  - ~73 vendor-prefixed scrollbar/pseudo rules (`::-webkit-scrollbar`) — cosmetic
  - 29 `view-transition-*` rules — View Transitions is unimplemented
  - **`17 :has()` rules**, including
    `#content.ytd-rich-section-renderer:has(>grid-shelf-view-model)` — the feed
    container's own layout rule

### A probe page, because CSS.supports is not enough

Rather than infer from the log, served a 41-feature probe page over HTTP and
read the answers off the screen. Before any change:

| Present | Missing |
| --- | --- |
| `customElements`, `attachShadow`, `MutationObserver`, `ResizeObserver`, `fetch`, `WebAssembly`, `Proxy`, `Intl.Segmenter`, `:is()`, `:where()`, `@layer` | **`:has()`**, **`IntersectionObserver`**, **`@container`**, **`adoptedStyleSheets`**, **`Worker`**, **`SharedWorker`**, **`MediaSource`**, **`navigator.serviceWorker`**, `requestIdleCallback`, `navigator.mediaDevices`, `crypto.randomUUID`, `view-transition-name`, `container-type`, `aspect-ratio`, `backdrop-filter`, `content-visibility` |

### The fix that was available: Servo's own experimental preferences

Servo ships these features switched **off** by default. The authoritative list is
servoshell's `EXPERIMENTAL_PREFS` (ports/servoshell/prefs.rs:33-53), the set it
enables behind `--enable-experimental-web-platform-features`. Applied that via
`ServoBuilder::preferences`, plus `dom_cookiestore_enabled`, which is not on that
list but without which a site cannot write its own cookies.

`browser/src/prefs.rs` now does this. The important wrinkle: `Preferences::set_value`
**silently ignores a name it does not recognise**, so a typo would leave a feature
off with nothing reporting it. Each name is therefore paired with a getter for
the field it controls, and a test reads the field back — the names are checked by
the compiler and by a test, not by hope.

### Verified effect of the preferences

Re-ran the probe with them on: **23 → 25 of 41**, the two additions being
`IntersectionObserver` and `adoptedStyleSheets`, both previously absent.

No regression: servo.org renders identically, working set 174 → 163 MB.
YouTube working set 413 → 373 MB.

### YouTube still does not work — and why

The feed is still empty. The decisive evidence is in the network log:

```
POST https://www.youtube.com/youtubei/v1/log_event   x6   -> 200
POST https://www.youtube.com/youtubei/v1/guide              -> 200
POST https://www.youtube.com/youtubei/v1/feedback           -> 200
(no youtubei/v1/browse at all)
```

The Polymer app **boots and runs** — it logs telemetry, fetches the guide, posts
feedback, all successfully. It simply never dispatches the `browse` request that
loads the home feed. So this is not a network, TLS, or CORS problem.

Cross-check on the wire: fetching the same URL from PowerShell with a normal
browser UA returns 873 KB containing `ytInitialData` and `richGridRenderer` but
**zero `videoId`s**. The feed is genuinely fetched client-side, after boot.

The features that are missing are the plausible causes, and the two decisive ones
are **not preference-gated**:

- **dedicated `Worker`** — absent with prefs on *and* off. YouTube's data layer
  uses web workers.
- **`:has()`** — still dropped 17 times with all prefs enabled.
- **`MediaSource`** — absent either way. Even a working feed could not play video,
  since YouTube streams via MSE.

### Verdict

**This is Servo, not our integration.** There is no embedder-side change that
makes YouTube's home feed work on this checkout, and video playback is out of
reach regardless because `MediaSource` does not exist here. The preferences are
still worth shipping — they are a real, measurable compatibility improvement
(`IntersectionObserver` and `adoptedStyleSheets` are common on the modern web)
and they cost nothing in memory.

Sites that were already fine are unaffected: servo.org and DuckDuckGo both render
correctly with them on. DuckDuckGo in particular — a fairly modern page — renders
essentially perfectly, which is a useful reminder that "Servo can't do modern
sites" is too strong a claim.

### Deliberately not enabled

- `dom_serviceworker_enabled` — YouTube registers one, but Servo's is partial and
  a mis-serving worker can replace a good page with a cached error. Wants a
  dedicated experiment, not a blind default.
- `media_glvideo_enabled` — servoshell forces it off for headless windows
  (prefs.rs:608-611), so it is headed-only, and there is no media backend on
  Windows anyway.
- `dom_worklet_enabled` — downstream of the missing `MediaSource`.
- `dom_webgpu_enabled` — no adapter under our ANGLE context.
- `dom_notification_enabled`, `dom_permissions_enabled` — these raise UI prompts
  this browser does not implement, so enabling them only produces dead ends.

### Side findings

- **Ctrl+V paste does not work**; `dom_async_clipboard_enabled` was false and is
  now on, but paste is still unverified.
- servo.org itself logs one `fail to evaluate module`.
- Synthetic `keybd_event` typing drops characters; clipboard paste via
  `Ctrl+V` did not register either. Test-harness limitation, not a browser bug —
  but it means address-bar tests need a human or a different input path.
---

## 2026-09-30 — Entry 011: MILESTONE 3 ACHIEVED — tabs

### What changed

- `browser/src/tab.rs` — **new.** `TabId`, `Tab`, `Tabs`: the tab model.
  Engine-neutral, no Servo, no egui, 25 unit tests.
- `browser/src/browser_state.rs` — now owns the `Tabs` plus a queue of pending
  engine callbacks. The flat `url`/`title`/`can_go_back`/`can_go_forward`/`loading`
  fields moved onto `Tab`, so each tab has its own.
- `browser/src/engine.rs` — one `WebView` per tab, `HashMap<TabId, WebView>`.
  Per-tab delegate. Only the active web view is painted, resized and given input.
- `browser/src/chrome.rs` — tab strip: title/address label, close button, new-tab
  button, active-tab tint.
- `browser/src/input.rs` — **new.** winit→Servo translation, extracted from
  `main.rs` and made pure so it could be tested.
- `browser/src/main.rs` — wiring and shortcuts.

Adopted from `docs/hermes-research.md`: idea 1 (engine-neutral core), idea 2
(Command/Event carrying a stable `TabId`), idea 3 (queue rather than apply
inline), idea 7 (only one web view visible), idea 8 (resize through the web view,
not the shared context), idea 10 (shortcuts above the web view).

### Servo facts this was built on (all read from the checkout)

| Fact | Location |
| --- | --- |
| `WebView` is `Rc<RefCell<WebViewInner>>`, derives `Clone`, `PartialEq` by id | `components/servo/webview.rs:84` |
| **`WebViewInner` implements `Drop`** → sends `CloseWebView` + `remove_webview`. Dropping the last clone is the *only* way a web view is destroyed; there is no `close()` | `webview.rs:144-151` |
| **`hidden` defaults to `false`** — a new web view is VISIBLE, so a background tab must be hidden explicitly or it paints over the active one | `components/paint/webview_renderer.rs:161` |
| `show()` / `hide()` exist; hidden web views are never "animating" and the constellation is told to stop work on them | `webview.rs:489,497`; `webview_renderer.rs:188,197` |
| **`set_throttled` does not exist** in this version — it was removed. Hiding is the only background-tab saving available | method list, `webview.rs:291-731` |
| `WebViewBuilder::new(&servo, Rc<dyn RenderingContext>)` — many web views may share one rendering context | `webview.rs:1082` |

### Design decisions worth recording

**`TabId` is never reused.** Engine callbacks arrive asynchronously on Servo's
own threads. A title change for a background tab must land on *that* tab, not on
whichever is active when the queue drains. If ids were recycled, a late callback
from a closed tab could be applied to a brand new one. A monotonically
increasing counter makes that impossible rather than unlikely. Tested.

**Callbacks are queued, never applied inline.** The delegate only pushes a
`TabEvent`; `drain_events()` applies them once per frame before anything reads
the model. Writing straight into the model from the callback would mean a
`RefCell` borrow held across a call that can re-enter the delegate — `paint()`
and `spin()` both can — which is a runtime panic waiting for the wrong frame.

**The engine does not know which tab is "active" as state.** It is handed a
`TabId` and told to activate it. Selection policy lives in `Tabs`, which is
testable; the engine only knows how to show and hide.

**Fixed tab width.** A flexible layout resizes every tab when one is added, which
makes the strip jump under the pointer. Constant width is less pretty and
actually clickable.

### Problems hit

**P41. `RefCell already mutably borrowed` at startup — app crashed immediately.**
`draw_frame` took one `tabs_mut()` for the whole egui closure, then read the model
again later in the same closure (the seen-URL check, the window title, and
`apply_action` itself). `RefCell` permits one borrow at a time, so the first
`tabs()` inside panicked. Fixed by scoping the borrow tightly around
`chrome.draw`.

**P42. `index out of bounds: the len is 0 but the index is 0`.** Closing the last
tab empties the model, and every accessor indexes directly. The guard at the top
of `draw_frame` did not help because `apply_action` runs *inside* the frame and
emptied it before the later reads. Fixed by making `apply_action` the last thing
that touches the model and guarding the one read after it.

**P43. Tab labels overflowed the strip and painted over the toolbar.**
`Painter::layout` *wraps*, so a long page title became three lines and centring
that block pushed it out of a 28pt strip. Fixed with `layout_no_wrap` plus
`with_clip_rect`.

**P44. The close button was 8pt from where the geometry said.** `ui.horizontal`
inserts inter-item spacing, so each tab was 184pt wide rather than the 176 the
constants implied. Found because a click test missed by exactly 8pt. Fixed by
zeroing `item_spacing` inside the tab, as was already done for the title bar.

**P45. Test-harness trap, worth remembering.** `Process.MainWindowHandle` was
returning a 16x16 winit helper window ("Winit Thread Event Target"), not the
browser, so screenshots were of the wrong window and every click missed. The
helper now enumerates windows and picks the largest visible one per PID. Several
minutes were lost to chasing a "black window" that was a terminal.

**P46. A test asserted the wrong arithmetic.** `page_point` at 150% scale:
I expected y=20 for physical y=90, but 90/1.5 = 60 egui points and the content
rect starts at 60, so the answer is 0. The implementation was right. Added a
100%-scale control test so the two cases can be compared.

### Verified on screen

| Behaviour | Result |
| --- | --- |
| Tab strip renders, one tab, correct label and close button | yes |
| New tab via `+` and via Ctrl+T | `opened tab N`, `activated tab N` |
| Switch by clicking a tab | `activated tab N` |
| Ctrl+Tab cycles forward, wraps | 3 → 1 |
| Ctrl+Shift+Tab cycles backward, wraps | 1 → 2 |
| Close via the × | `closed tab N`, correct tab activated |
| Closing the active tab selects the one to its right | yes |
| Closing the rightmost active tab falls back to the left | `closed tab 3` → `activated tab 2` |
| Closing the last tab shuts the window down | process exits |
| Ctrl+W on the only tab shuts down | process exits |
| **Per-tab scroll position retained across switches** | scrolled tab 2 by 8, switched away, back — identical position |

That last one is the real proof that the tabs are independent: each has its own
`WebView`, and the screenshots before and after the round trip are the same.

**Idle CPU with 2 tabs: 0 ms over 6 s.** Background tabs cost nothing, which was
the point of hiding them. Working set 163 MB for one tab, 227 MB for two.

72 unit tests pass; our crate is warning-free. `main.rs` went from 933 to 852
lines with the input extraction.

### Not verified

- **Background tabs are hidden but not throttled.** `set_throttled` no longer
  exists, so `hide()` is the only lever. Servo does tell the constellation the
  web view is hidden, which stops animation-driven redraws, but it is not a
  suspension mechanism. A heavy background tab still consumes CPU in its own
  process. Measured as 0 ms here because servo.org is static, not because
  suspension works.
- **Ctrl+W with several tabs** was exercised via the × buttons; the shortcut path
  shares `close_tab` with it and was only tested on the single-tab case.
- Right-edge window resize is still unconfirmed (Entry 008/009).
- No tab reordering by dragging, and no tab overflow scrolling when there are more
  tabs than fit. Both are deliberate omissions, not bugs.

---

## 2026-10-01 — Entry 012: YouTube's blank pages were `document.all`, not missing features

Entry 010 concluded that YouTube's blank pages were Servo's missing `:has()`,
dedicated `Worker` and `MediaSource`, and that nothing on the embedder side
could fix them. The first two parts were wrong. Search results and watch pages
now render; only video playback (`MediaSource`) remains out of reach.

### How it was found

Search became testable once Enter and Space reached pages (the key-translation
fix in the UI overhaul). The results page rendered its chips but no results. A
probe run through `WebView::evaluate_javascript`, temporarily, showed:

- 15-19 `ytd-video-renderer` elements **in the DOM**, with correct titles, and no
  console errors: YouTube's scripts worked.
- every one with a 0x0 box. Walking the ancestors found
  `ytd-two-column-search-results-renderer` with `hidden` set, so `display: none`.
- the template binds it as `hidden="[[data.hideContents]]"`, normally undefined.
  Bindings were processed (only one raw `[[...]]` attribute on the whole page).
- a user script wrapping the `hidden` setter caught Polymer assigning an object
  whose string is `"zClosurez"`: polymer-resin's "innocuous" Trusted Types
  placeholder, i.e. resin had rejected the binding.

### The cause

polymer-resin (Google's Polymer sanitizer, installed as
`Polymer.sanitizeDOMValue`) lets falsy values through with:

```js
if (!v && v !== document.all) return v;
```

Servo has no `document.all` (commented out in `Document.webidl`, servo/servo#7396),
so it evaluates to `undefined`, `undefined !== undefined` is false, and resin
replaces `undefined` with its truthy placeholder. Every element bound that way
is hidden.

### The fix

`browser/src/compat.rs`: site interventions, the mechanism Firefox ships as
webcompat interventions. One user script, attached to every tab and guarded on
`location.hostname`, defines `document.all` as `NaN` on youtube.com only. NaN is
falsy and unequal to everything, so all three of resin's `document.all` checks
behave exactly as in Chrome; YouTube uses `document.all` nowhere else. It should
be deleted when Servo implements `document.all`.

### Corrections to entry 010

- **Dedicated `Worker` is present.** `Worker.webidl` has no `Pref`; the probe was
  wrong.
- **`:has()` is pref-gated, not unimplemented**, by the Stylo static pref
  `layout.css.has-selector.enabled` (false), which Servo's `prefs::set` never
  maps. Enabling it did not change YouTube, so it stays off.
- **`requestIdleCallback`** is genuinely absent; a polyfill did not change
  YouTube either.
- The **home feed's** "Try searching to get started" is YouTube's own signed-out
  empty state: the server response carries no video ids for it.

### Side finding: site data was written to the working directory

With no `Opts::config_dir`, Servo writes IndexedDB "bottles" relative to the
working directory; the YouTube runs left 11 MB of them in `browser/`. Site data
now lives in `%APPDATA%\Ferrous\profile`, which also makes cookies and storage
persist between runs.

### Verified on screen

Search results with thumbnails, titles, channels, durations and the Shorts
shelf; sidebar and search-button icons (also previously hidden); a watch page
with player controls, title, channel, description, comment count and related
videos, showing "Your browser can't play this video".

### Not working

- Video playback: no `MediaSource` in Servo.
- Clicking a result title did not navigate; loading the watch URL directly does.
  YouTube handles those clicks itself (client-side navigation); not yet
  investigated.
