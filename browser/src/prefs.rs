//! Engine feature preferences.
//!
//! # Why this exists
//!
//! Servo ships a large set of standard web APIs switched **off** by default.
//! They are off because they are incomplete, not because they are wrong — so a
//! site that uses them gets a half-rendered page and the embedder has no idea
//! why. YouTube is the clearest example: its shell rendered fine but its entire
//! video feed was blank, and the log showed CSS rules being dropped rather than
//! anything resembling an error.
//!
//! # Why these specific ones
//!
//! The list is servo shell's own. `ports/servoshell/prefs.rs:33-53` defines
//! `EXPERIMENTAL_PREFS`, the set servoshell turns on behind
//! `--enable-experimental-web-platform-features`. That is the engine author's
//! own statement of which pref-gated features are worth having on, so it is the
//! right starting point rather than a list invented here.
//!
//! Measured on this checkout (see `docs/devlog.md` entry 010) against a probe
//! page: with these off, `IntersectionObserver`, `@container`, `adoptedStyleSheets`
//! and `container-type` were all **absent**; `:has()`, dedicated `Worker` and
//! `MediaSource` were absent **with or without** these prefs, because they are
//! not pref-gated — they are simply unimplemented in Servo. No combination of
//! preferences will make YouTube's feed work; see the "Cannot be fixed here"
//! note below.
//!
//! # What is deliberately NOT enabled
//!
//! `dom_webgpu_enabled` (no WebGPU adapter is available under our ANGLE setup,
//! and it is heavy), `dom_notification_enabled` and `dom_permissions_enabled`
//! (they raise UI prompts this browser does not implement, so enabling them
//! would only produce dead ends), and the media stack. Enabling a pref the
//! embedder cannot service is worse than leaving it off.

use servo::{PrefValue, Preferences};

/// Pref-gated features enabled by default, each paired with the field it
/// controls.
///
/// The pairing is what makes this checkable. `Preferences::set_value` takes a
/// name and a value and *silently ignores names it does not recognise*, so a
/// typo would leave a feature off with nothing anywhere reporting an error.
/// Reading the field back is the only way to notice. It also means the field
/// names are validated by the compiler rather than only at runtime.
const WEB_COMPAT_PREFS: &[(&str, fn(&Preferences) -> bool)] = &[
    // servoshell's EXPERIMENTAL_PREFS...
    //
    // IntersectionObserver: YouTube's feed uses it to decide when to lay out
    // and fetch each thumbnail. Absent, the grid stays empty. This is the single
    // most load-bearing entry in this list.
    ("dom_intersection_observer_enabled", |p| p.dom_intersection_observer_enabled),
    // Container queries: YouTube sizes its video cards from the width of their
    // container, not the viewport. Absent, the card grid has no layout.
    ("layout_container_queries_enabled", |p| p.layout_container_queries_enabled),
    // IndexedDB: YouTube keeps feed and session state here.
    ("dom_indexeddb_enabled", |p| p.dom_indexeddb_enabled),
    ("dom_webgl2_enabled", |p| p.dom_webgl2_enabled),
    ("dom_offscreen_canvas_enabled", |p| p.dom_offscreen_canvas_enabled),
    // Custom and variable fonts: YouTube's icon font renders as blank space
    // without these, which is most of why its sidebar looked unstyled.
    ("dom_fontface_enabled", |p| p.dom_fontface_enabled),
    ("layout_variable_fonts_enabled", |p| p.layout_variable_fonts_enabled),
    ("dom_adoptedstylesheet_enabled", |p| p.dom_adoptedstylesheet_enabled),
    ("dom_web_animations_enabled", |p| p.dom_web_animations_enabled),
    ("dom_sanitizer_enabled", |p| p.dom_sanitizer_enabled),
    ("dom_storage_manager_api_enabled", |p| p.dom_storage_manager_api_enabled),
    ("dom_exec_command_enabled", |p| p.dom_exec_command_enabled),
    ("dom_async_clipboard_enabled", |p| p.dom_async_clipboard_enabled),
    ("layout_columns_enabled", |p| p.layout_columns_enabled),
    ("layout_css_ellipse_corners_enabled", |p| p.layout_css_ellipse_corners_enabled),
    ("layout_css_progress_function_enabled", |p| p.layout_css_progress_function_enabled),
    // ...plus one beyond servoshell's list.
    //
    // `dom_cookiestore_enabled` is not in EXPERIMENTAL_PREFS, but without the
    // Cookie Store API a site cannot enumerate or write its own cookies, which
    // breaks sign-in and consent state.
    ("dom_cookiestore_enabled", |p| p.dom_cookiestore_enabled),
];

/// Prefs that are plausible but deliberately not enabled, with the reason.
///
/// Test-only: nothing in the browser reads this. It exists so that
/// `deferred_prefs_stay_off` has something to assert, and so the next person
/// does not have to rediscover each of these.
///
/// * `dom_serviceworker_enabled` — YouTube registers one. Servo's implementation
///   is partial, and a mis-serving worker can replace a working page with a
///   cached error. Worth an experiment; not worth enabling blind.
/// * `media_glvideo_enabled` — GL video decoding. servoshell forces it off for
///   headless windows (`prefs.rs:608-611`), implying it is only supported
///   headed, and there is no media backend on Windows.
/// * `dom_worklet_enabled` — AudioWorklet, needed for YouTube's audio path,
///   which is downstream of the missing `MediaSource`.
/// * `dom_webgpu_enabled` — no adapter available under our ANGLE context.
#[cfg(test)]
const DEFERRED_PREFS: &[(&str, fn(&Preferences) -> bool)] = &[
    ("dom_serviceworker_enabled", |p| p.dom_serviceworker_enabled),
    ("media_glvideo_enabled", |p| p.media_glvideo_enabled),
    ("dom_worklet_enabled", |p| p.dom_worklet_enabled),
    ("dom_webgpu_enabled", |p| p.dom_webgpu_enabled),
];

/// Build the engine preference set.
///
/// Starts from Servo's own defaults — which sets the user agent from the host
/// platform and picks up `http_proxy`, both of which matter — then turns on the
/// compatibility set.
pub fn web_compat() -> Preferences {
    let mut preferences = Preferences::default();
    for (name, _) in WEB_COMPAT_PREFS {
        preferences.set_value(name, PrefValue::Bool(true));
    }
    preferences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pref_name_is_real_and_takes_effect() {
        // Guards the silent-failure mode of `set_value`: an unrecognised name
        // does nothing and reports nothing.
        let prefs = web_compat();
        for (name, field) in WEB_COMPAT_PREFS {
            assert!(field(&prefs), "preference {name} did not take effect");
        }
    }

    #[test]
    fn the_compat_set_is_off_by_default_in_servo() {
        // If Servo ever flips one of these on upstream, this whole module becomes
        // unnecessary. Worth knowing, so assert the premise still holds.
        let defaults = Preferences::default();
        for (name, field) in WEB_COMPAT_PREFS {
            assert!(!field(&defaults), "{name} is already on upstream");
        }
    }

    #[test]
    fn pref_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for (name, _) in WEB_COMPAT_PREFS.iter().chain(DEFERRED_PREFS) {
            assert!(seen.insert(*name), "duplicate preference: {name}");
        }
    }

    #[test]
    fn deferred_prefs_stay_off() {
        let prefs = web_compat();
        for (name, field) in DEFERRED_PREFS {
            assert!(!field(&prefs), "{name} should not be enabled by default");
        }
    }

    #[test]
    fn defaults_survive() {
        // Enabling extras must not wipe anything Servo decided for us.
        let prefs = web_compat();
        assert!(!prefs.user_agent.is_empty(), "user agent must survive");
        assert!(prefs.dom_crypto_subtle_enabled);
    }
}
