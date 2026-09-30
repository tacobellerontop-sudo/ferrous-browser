# Proposed First Implementation Step

Status: **PROPOSED — not started.** Nothing in `browser/` contains code yet.

---

## The proposal

Build a single throwaway binary inside the Servo workspace that opens a window,
creates one Servo `WebView` on an offscreen rendering context, paints it, and
forwards mouse and keyboard input. **No browser abstractions. No tabs, no
address bar, no engine-neutral core, no module structure.**

Then throw it away and rebuild properly on a proven foundation.

## Why this is the smallest honest first step

Milestone 1 says *"application launches, window opens, Servo initializes, a
page loads, page renders, mouse input works, keyboard input works."*

Each of those six depends on facts that are **currently unverified on this
machine**:

1. Servo compiles at all on Windows here (missing C++ ATL, `uv`, `ninja`,
   moztools).
2. A separate Cargo workspace can depend on Servo *and still build it*, given
   the config Servo relies on (see below).
3. ANGLE builds and a GL context is obtainable on this GPU/driver.
4. Servo renders actual pixels through an offscreen context inside a larger
   window (the Hermes cross-process finding suggests caution).

Writing a tab manager, an address bar, an engine-neutral core, and a command
queue *before* any of those is known is how projects end up rewriting
architecture three times. A 150-line spike de-risks all four in one shot.

The spike should be based on two verified references, in this order:

- `servo/components/servo/examples/winit_minimal.rs` (168 lines) — the canonical
  minimal embedder.
- `servo/ports/servoshell/desktop/headed_window.rs` + `gui.rs` — the real
  offscreen + egui composition path.

Never an external tutorial. See `docs/devlog.md` P3.

## Workspace configuration this requires

Servo does **not** build correctly from an arbitrary directory. A separate
workspace must replicate the following, because **Cargo does not inherit
profiles or `.cargo/config.toml` across workspaces.** All four items were read
from `servo/Cargo.toml` and `servo/.cargo/config.toml`.

```toml
# <our-workspace>/.cargo/config.toml
[target.x86_64-pc-windows-msvc]        # servo/.cargo/config.toml:16
linker = "lld-link.exe"                # servo/.cargo/config.toml:17 — Servo does NOT use MSVC link.exe

[env]
# servo/.cargo/config.toml:33 — inert unless the `crown` feature is enabled
# (all `#![feature(..)]` attrs in Servo are `#![cfg_attr(crown, ..)]`).
# Replicated for fidelity; should not be extended to our own crates.
RUSTC_BOOTSTRAP = "crown,script,script_webgpu,script_bindings,style_tests,mozjs,mozjs_sys"
```

```toml
# <our-workspace>/Cargo.toml
# servo/Cargo.toml:393-399 — REQUIRED ON WINDOWS.
# Without it the servo-script rlib exceeds 4 GB and the build fails with
# rust-lang/rust#151184.
[profile.dev.package.servo-script]
codegen-units = 16

# servo/Cargo.toml:390-391
[profile.dev.package.crypto-bigint]
opt-level = 3
```

Also needed:

- A `rust-toolchain.toml` pinning **1.97.1**, matching `servo/rust-toolchain.toml`,
  because Servo's CI builds with that exact toolchain.
- The `servo` dependency with feature **`no-wgl`** (Windows → ANGLE).

**Manifest-level validation already done:** a throwaway crate with
`servo = { path = ".../servo/components/servo", features = ["no-wgl"] }` resolves
successfully via `cargo metadata` from an unrelated directory. Cargo accepts a
path dependency into another workspace without complaint. What remains
unverified is whether the *compilation* succeeds with the config above.

## Machine prerequisites still missing

| Item | Why | How |
| --- | --- | --- |
| **C++ ATL for v143** | listed as required by `servo/README.md:54` | Visual Studio Installer |
| **moztools 4.0** | needed to build SpiderMonkey on Windows | `.\mach fetch` |
| `uv` | Servo's preferred Python runner; build falls back to `python` with a warning | standalone installer |
| `ninja` | winget-installed by `mach bootstrap`; only strictly needed for cross-compiling | `winget install Ninja-build.Ninja` |
| `rustc-dev`, `llvm-tools` (1.97.1) | only if the `crown` feature is used | `rustup component add --toolchain 1.97.1 rustc-dev llvm-tools` |

`.\mach bootstrap` would handle most of this but **installs GStreamer MSIs via a
UAC elevation prompt and permanently edits the user `PATH` to add LLVM.** That is
an invasive, machine-wide change and should be an explicit decision, not an
accident. A more surgical alternative is to install only what the build actually
errors on.

## Success criteria

The spike is done when all of these are observed on screen:

1. A window opens.
2. A URL loads and **actual page pixels are visible** (not a white/blank frame).
3. The mouse can hover a link (cursor changes) and click it (navigation occurs).
4. Keyboard input reaches the page (type into a text field).
5. The process does not require a `--no-sandbox`-style escape hatch.

If (2) fails, stop and report. A blank window with a correct title and URL means
the failure mode Hermes documented — we must not paper over it.

## Explicitly NOT in this step

- No `browser/` module structure. No trait abstractions.
- No tabs, address bar, history, bookmarks, downloads, settings.
- No egui chrome yet. The offscreen-context + egui-blit composition is
  Milestone 1's *second half*; prove the plain `WindowRenderingContext` path first.
- No multiprocess/sandbox changes. Default to `Opts::default()` and revisit only
  after pixels are proven.
- No vendored patches. Upstream's `[patch.crates-io]` is empty; keep it that way.
- No performance claims. Nothing is "lightweight" until it is measured
  (Milestone 5).

## What comes after the spike

Once the engine provably renders:

1. Add egui chrome on the offscreen context using servoshell's exact
   `PaintCallback` + `render_to_parent_callback` composition.
2. Then introduce the engine-neutral core (`WebView` trait, commands, events,
   `TabId`) — Hermes' layering — **with unit tests that never touch Servo.**
3. Only then tabs, then the features in Milestones 2–4.

Ordering matters: the abstraction is cheap to add later, expensive to retrofit.

## Decision needed before proceeding

1. **Bootstrapping method.** Full `.\mach bootstrap` (UAC prompt + permanent PATH
   edit + GStreamer install) versus surgical installs of only what the build
   demands? The surgical route is slower per iteration but far less invasive.
2. **Workspace location.** Keep our browser in its own workspace (clean
   separation, but we own the config replication above and must re-check it on
   every Servo bump) versus adding the crate inside `servo/ports/` (inherits all
   config automatically, but couples us to Servo's repo layout).

Recommendation: **own workspace + surgical bootstrap.** The separation is the
entire point of the project, and the config set is small, well understood, and
documented in `docs/servo-version.md` for re-verification on each bump.