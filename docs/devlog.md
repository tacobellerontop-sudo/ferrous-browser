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
  itself is unconfirmed. Worth a glance on a slow connection.