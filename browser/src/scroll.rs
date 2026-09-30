//! Animated (smooth) scrolling.
//!
//! Servo has no built-in smooth scrolling: `Scroll` only offers
//! `Delta`/`Start`/`End` (`components/shared/embedder/lib.rs:173`) and there is no
//! `smooth_scroll` preference anywhere in the engine. A wheel notch therefore
//! arrives as one instantaneous jump, which feels abrupt on a trackpad and
//! worse on a mouse wheel.
//!
//! This module turns each wheel delta into a short decaying animation: a
//! fraction of the outstanding distance is delivered per frame, chosen so the
//! remaining distance decays exponentially. The result is a glide with a soft
//! stop rather than a hard stop.
//!
//! Pure logic — no Servo, no winit — so it is unit tested directly.

use std::time::Duration;

use euclid::{UnknownUnit, Vector2D};

/// Plain 2D vector in device pixels. Deliberately *not* a Servo type, so this
/// module stays independent of the engine and unit-testable on its own.
pub type Vec2 = Vector2D<f32, UnknownUnit>;

/// Time constant of the glide, in seconds. After this much time about 37% of the
/// outstanding distance is still left; the tail is cut off by `MIN_STEP_PX`.
const DECAY_SECONDS: f32 = 0.055;

/// Below this remaining distance the animation snaps to done. Without this the
/// exponential tail would keep waking the event loop forever, which directly
/// conflicts with the "low CPU while idle" goal.
const MIN_STEP_PX: f32 = 0.35;

/// Below this remaining distance a wheel delta is delivered immediately instead
/// of being animated. A 1px nudge should not produce a 200ms animation.
const MIN_ANIMATE_PX: f32 = 1.5;

#[derive(Debug, Default)]
pub struct SmoothScroll {
    /// Scroll distance not yet delivered to the engine, in device pixels.
    outstanding: Vec2,
    animating: bool,
}

/// Convert a winit wheel delta (already scaled to pixels) into a Servo scroll
/// delta.
///
/// **The sign must be inverted.** Servo negates the wheel delta itself when it
/// turns one into a scroll (`components/paint/webview_renderer.rs:1219-1222`):
/// "A scroll delta for a wheel event is the inverse of the wheel delta", with
/// positive meaning "reveal more content below".
///
/// Passing winit's value straight through scrolls the page the *wrong way*,
/// which is indistinguishable from "scrolling does nothing" whenever the page
/// happens to be sitting at the top — which is exactly where it starts.
pub fn scroll_delta_from_wheel(x: f64, y: f64) -> Vec2 {
    Vec2::new(-x as f32, -y as f32)
}

impl SmoothScroll {
    /// Queue a wheel delta.
    ///
    /// Returns `true` if this delta should be animated, and `false` if the
    /// caller should deliver it immediately instead (because it is too small to
    /// be worth animating).
    pub fn push(&mut self, delta: Vec2) -> bool {
        self.outstanding += delta;

        if !self.animating && self.outstanding.length() < MIN_ANIMATE_PX {
            return false;
        }
        self.animating = true;
        true
    }

    /// Advance the animation by `dt`, returning the distance to scroll now.
    ///
    /// Time-based rather than frame-based on purpose: a fixed fraction per frame
    /// would make the glide 2.4x faster on a 144Hz display than on a 60Hz one.
    pub fn step(&mut self, dt: Duration) -> Option<Vec2> {
        if !self.animating {
            return None;
        }

        // Exponential decay: how much of the remainder survives one dt.
        let retained = (-dt.as_secs_f32() / DECAY_SECONDS).exp();
        let step = self.outstanding * (1.0 - retained);

        if step.length() < MIN_STEP_PX {
            self.outstanding = Vec2::zero();
            self.animating = false;
            return None;
        }

        self.outstanding -= step;
        Some(step)
    }

    /// Finish immediately, discarding anything outstanding. Used when the user
    /// starts a new interaction that should not fight an in-flight glide.
    pub fn cancel(&mut self) {
        self.outstanding = Vec2::zero();
        self.animating = false;
    }

    pub fn is_animating(&self) -> bool {
        self.animating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    fn vec(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn idle_scroller_delivers_nothing() {
        let mut s = SmoothScroll::default();
        assert_eq!(s.step(FRAME), None);
        assert!(!s.is_animating());
    }

    #[test]
    fn tiny_deltas_are_passed_through_unanimated() {
        let mut s = SmoothScroll::default();
        // Below MIN_ANIMATE_PX: caller should send this itself.
        assert!(!s.push(vec(0.0, 1.0)));
        assert!(!s.is_animating());
    }

    #[test]
    fn large_deltas_are_animated() {
        let mut s = SmoothScroll::default();
        assert!(s.push(vec(0.0, 400.0)));
        assert!(s.is_animating());
    }

    #[test]
    fn animation_delivers_the_full_distance_and_then_stops() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, 400.0));

        let mut total = 0.0;
        let mut frames = 0;
        while let Some(step) = s.step(FRAME) {
            total += step.y;
            frames += 1;
            assert!(frames < 600, "animation failed to terminate");
        }

        // Everything queued is delivered except a sub-pixel tail, which is snapped away
        // so the animation terminates. Coming up fractions of a pixel short is
        // imperceptible; an animation that eases forever would keep waking the
        // event loop indefinitely, which is exactly the idle-CPU cost this
        // project is trying to avoid.
        assert!(total <= 400.0, "delivered {total}, more than was queued");
        assert!(400.0 - total < 2.0, "dropped {}px of tail", 400.0 - total);
        assert!(!s.is_animating());
    }

    #[test]
    fn velocity_decays_monotonically() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, 400.0));

        let first = s.step(FRAME).expect("first step").y;
        let second = s.step(FRAME).expect("second step").y;
        assert!(second < first, "expected deceleration, got {second} after {first}");
    }

    #[test]
    fn animation_converges_quickly() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, 400.0));

        let mut frames = 0;
        while s.step(FRAME).is_some() {
            frames += 1;
        }
        // At a 60Hz-ish frame time this should settle in well under half a second.
        assert!(frames < 40, "took {frames} frames to settle");
    }

    #[test]
    fn direction_is_preserved() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, -250.0));
        let step = s.step(FRAME).expect("step");
        assert!(step.y < 0.0, "scroll-up should stay negative, got {}", step.y);
    }

    #[test]
    fn horizontal_and_vertical_are_independent() {
        let mut s = SmoothScroll::default();
        s.push(vec(60.0, 0.0));
        let step = s.step(FRAME).expect("step");
        assert!(step.x > 0.0);
        assert_eq!(step.y, 0.0);
    }

    #[test]
    fn cancel_drops_outstanding_distance() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, 400.0));
        s.cancel();
        assert!(!s.is_animating());
        assert_eq!(s.step(FRAME), None);
    }

    #[test]
    fn consecutive_deltas_accumulate_rather_than_replace() {
        let mut s = SmoothScroll::default();
        s.push(vec(0.0, 100.0));
        s.push(vec(0.0, 100.0));

        let mut total = 0.0;
        while let Some(step) = s.step(FRAME) {
            total += step.y;
        }
        assert!(total <= 200.0, "delivered {total}, more than was queued");
        assert!(200.0 - total < 2.0, "dropped {}px of tail", 200.0 - total);
    }

    // The sign convention below caused a real bug: scrolling appeared to do
    // nothing because every wheel event moved the page the wrong way, and the
    // start page is already at the top.

    #[test]
    fn downward_wheel_becomes_a_positive_scroll_delta() {
        // winit reports a forward/downward wheel as negative Y.
        let delta = scroll_delta_from_wheel(0.0, -76.0);
        assert!(delta.y > 0.0, "downward wheel must scroll down, got {}", delta.y);
        assert_eq!(delta.x, 0.0);
    }

    #[test]
    fn upward_wheel_becomes_a_negative_scroll_delta() {
        assert!(scroll_delta_from_wheel(0.0, 76.0).y < 0.0);
    }

    #[test]
    fn horizontal_wheel_is_inverted_too() {
        assert!(scroll_delta_from_wheel(-40.0, 0.0).x > 0.0);
        assert!(scroll_delta_from_wheel(40.0, 0.0).x < 0.0);
    }

    #[test]
    fn magnitudes_are_preserved() {
        assert_eq!(scroll_delta_from_wheel(0.0, -76.0).y, 76.0);
    }
}