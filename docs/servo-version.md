# Servo Version & Build Provenance

Single source of truth for which Servo revision this project is written against.

## Servo revision

| Field | Value |
| --- | --- |
| Repository | `https://github.com/servo/servo` |
| Commit | `b2ef57f642816e77e67e5a322ddccdbde6784f3c` |
| Commit date | 2026-09-30 |
| Commit subject | `android: Remove `RunCallback` abstraction (#48517)` |
| Workspace version | `0.6.0` |
| Clone location | `servo/` (shallow, `--depth 1`) |
| Local modifications | only the patches in `patches/servo/` (see "Local patches") |

Because the clone is shallow, upstream history is unavailable locally. To move to
a different revision: `git -C servo fetch --depth 1 origin <ref>` then
`git -C servo checkout FETCH_HEAD`, and update the table above.

## Toolchain

| Field | Value | Source |
| --- | --- | --- |
| Pinned toolchain | `1.97.1` | `servo/rust-toolchain.toml` |
| Components required | `clippy`, `llvm-tools`, `rustc-dev`, `rustfmt`, `rust-src` | `servo/rust-toolchain.toml` |
| Edition | `2024` | `servo/Cargo.toml:23` |
| MSRV | `1.88.0` | `servo/Cargo.toml:27` |
| Build Python | `3.11` | `servo/.python-version` |

`llvm-tools` and `rustc-dev` exist for Servo's `crown` binding generator.

## Relevant dependency versions (exact, from `servo/Cargo.lock`)

| Crate | Version | Role for us |
| --- | --- | --- |
| `servo` (path: `components/servo`) | `0.6.0` | the engine API we embed |
| `winit` | `0.30.13` | window + event loop |
| `egui` | `0.34.3` | browser UI |
| `egui-winit` | `0.34.3` | egui ↔ winit bridge |
| `egui_glow` | `0.34.3` | egui ↔ OpenGL bridge (feature `winit`) |
| `glow` | `0.17.0` | GL bindings shared by Servo and egui |
| `surfman` | `0.14.0` | GL context/surface management behind Servo |
| `webrender` | `0.70.0` | Servo's GPU renderer |
| `mozangle` | `0.7.1` | ANGLE, used by the `no-wgl` feature (Windows) |
| `gaol` | `0.2.1` | sandbox (**not built on Windows** — see below) |
| `url` | `2.5.8` | URL parsing |
| `euclid` | `0.22.14` | geometry types used across the Servo API |
| `dpi` | `0.1.2` | logical/physical size and scale |
| `keyboard-types` | `0.8.3` | `Key`, `Code`, `Modifiers`, `Location` |
| `rustls` | `0.23.45` | TLS (Servo installs the crypto provider) |
| `image` | `0.25.10` | favicon decoding |
| `accesskit` | `0.24.0` | accessibility |
| `arboard` | `3.6.1` | clipboard (servo `clipboard` feature) |
| `log` | `0.4.34` | logging facade |

### egui version policy

Servo pins `egui`/`egui-winit`/`egui_glow` **0.34.3**. We must use **0.34.3**, not
the 0.36.2 release that also exists in the local cargo cache. Using a different
egui minor would compile two incompatible egui trees and add binary size for no
benefit. Note `EguiGlow::new` takes `Arc<glow::Context>` from `glow 0.17`, so
`glow` must also stay on `0.17`.

## Why a path dependency and not `servo = "0.6"` from crates.io

The `servo` crate on crates.io *is* published from `servo/servo`, but it trails
our checkout:

| | crates.io `servo 0.6.0` | our checkout |
| --- | --- | --- |
| Date | published 2026-09-25 | commit 2026-09-30 |
| Source sha | `c78d2c206f80a1c8b67eefa97f773bba513205d3` | `b2ef57f642816e77e67e5a322ddccdbde6784f3c` |
| `default` features | `["bundled", "clipboard", "js_jit"]` | `["bundled", "clipboard", "js_jit", "multiprocess"]` |

A crates.io dependency would silently give us code five days older and missing
the `multiprocess` default. The path dependency also makes "update Servo" a
one-line change plus a re-verification of this document.

Caveat for future bumps: the `0.1.x` releases on crates.io come from an **older
versioning scheme** and do not correspond to `0.6.0`. Do not assume
`0.1.3` → `0.6.0` is a linear upgrade path.

## Local patches

Kept as patch files in `patches/servo/` and applied to the pinned checkout by
`scripts/apply-servo-patches.ps1` (CI runs it after cloning). Run it after any
fresh checkout or `git checkout` of `servo/`.

| Patch | File | Why | Remove when |
| --- | --- | --- | --- |
| `0001-root-timers-while-firing.patch` | `components/script/event_loop/timers.rs` | **Crash (use-after-free) on YouTube's first load.** `OneshotTimers::fire_timer` moved every due timer into a local `Vec` before running them. A `TracedCallback` is only safe in a GC-traced location, so when one callback triggered a GC, the callbacks still waiting in the `Vec` were freed or moved, and the next one crashed in `JS::GetNonCCWObjectGlobal` reading poisoned memory (`0x4b4b…`). The patch keeps the batch in a traced field, `timers_to_run`, and takes each timer out only immediately before it runs. Found with a minidump; see devlog entry 015. | Upstream `fire_timer` stops holding timers in an untraced local (still present on Servo `main` as of 2026-10-01). Worth reporting upstream. |
| `0002-video-frames-without-relayout.patch` | `components/script/dom/html/embedded_content/htmlmediaelement.rs` | **Video plays at half rate and freezes heavy pages.** `playback_video_frame_updated` marked the `<video>` node dirty for every decoded frame, forcing a restyle and relayout of the whole page per frame, although the frame had already been sent as an in-place update of the same WebRender image key. On a page with a large DOM, a 24 fps clip showed 10.6 frames/s and the page ran at 11 fps; with the patch, 24-25 frames/s and 41-44 fps. When the size is unchanged it now requests a new frame the way animated images do (`set_has_pending_animated_image_update`). A size change still takes the old path. Devlog entry 015. | Upstream stops dirtying the node per video frame. Worth reporting upstream. |

For reference, upstream Servo's own `[patch.crates-io]` section
(`servo/Cargo.toml:444+`) is entirely commented out — upstream currently patches
nothing.

If we ever need a patch, record it here with: why it is needed, which file
changed, what upstream functionality is missing, and whether it can be removed
once upstream absorbs it. Follow the policy in `docs/hermes-research.md` §13.

## Windows-specific configuration

| Fact | Value | Source |
| --- | --- | --- |
| Feature to enable | `servo/no-wgl` | `components/servo/Cargo.toml:66` |
| Expands to | `mozangle/egl`, `mozangle/build_dlls`, `surfman/sm-angle-default`, `paint_api/no-wgl` | same |
| servoshell enables it | yes | `ports/servoshell/Cargo.toml:154` |
| libservo C API enables it | yes | `ffi/capi/Cargo.toml:35` |
| Allocator | system allocator; jemalloc is **not** compiled on Windows | `components/allocator/Cargo.toml:22-30` |

## Profile settings we must replicate

Servo's root `Cargo.toml` defines build tuning that applies **only inside the
Servo workspace**. Cargo does not inherit profiles across workspaces, so our
workspace must reproduce the Windows-critical entries:

```toml
# servo/Cargo.toml:390-405
[profile.dev.package.crypto-bigint]
opt-level = 3

# REQUIRED ON WINDOWS: without this the servo-script rlib exceeds 4 GB
# (rust-lang/rust#151184) and the build fails.
[profile.dev.package.servo-script]
codegen-units = 16

[profile.dev.package.tikv-jemalloc-sys]
opt-level = 1
```

Servo also defines non-standard profiles we may want to mirror:
`medium`, `checked-release`, `production`, `production-stripped`, `profiling`,
`coverage` (`servo/Cargo.toml:407-443`).

## Servo crate feature flags

Default (`components/servo/Cargo.toml:22`):
`["bundled", "clipboard", "js_jit", "multiprocess"]`

Relevant groups:
- `default_web_features` = brotli, clipboard, webcrypto, webgpu, webgl, webxr
- `bundled` = baked-in-resources + bundled freetype
- `no-wgl` = ANGLE-based GL (**required on Windows**)
- `media-gstreamer` = real media pipeline (needs a native GStreamer install)
- `vello` = GPU canvas backend
- `multiprocess` = content-process isolation (see the open question below)
- `tracing` = tracing instrumentation

`servoshell` uses `default-features = false` and composes its own set
(`ports/servoshell/Cargo.toml:41-76`). We should start closer to Servo's defaults
and trim later, measuring first.

Note that the **cargo feature** `multiprocess` and the **runtime** option
`Opts::multiprocess` are different. `Opts::default().multiprocess` is `false`
(`components/config/opts.rs:249`) even though the feature is on by default.

## Open questions blocking later milestones

1. **Does cross-process paint work in an embedded rendering context on 0.6.0?**
   Hermes reports it does not on 0.4.0 (blank pages). If it is still broken we
   must ship single-process and document the security trade-off explicitly.
2. **What replaces `WebView::set_throttled`?** No public throttling API exists in
   0.6.0. Needed for cheap background tabs in Milestone 5.
3. **Does our own workspace + path dependency build Servo on Windows at all?**
   See the profile caveat above.