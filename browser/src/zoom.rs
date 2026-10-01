//! Page zoom levels.
//!
//! Zoom moves through a fixed ladder of comfortable values rather than by a
//! fixed percentage: +10% steps are too fine at 300% and too coarse at 50%.
//! These are the levels Chrome and Edge use, so Ctrl+Plus lands where people
//! expect. Servo itself clamps page zoom to 10%..1000%.

/// The ladder, ascending. 1.0 is 100%.
const LEVELS: [f32; 17] = [
    0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0,
];

/// Slack for comparing a current zoom against the ladder, so a value that is
/// 1.1 to within float error counts as being on the 110% rung.
const EPSILON: f32 = 0.001;

/// The next level up from `current`, or `current` if already at the top.
pub fn zoom_in(current: f32) -> f32 {
    LEVELS
        .iter()
        .copied()
        .find(|&level| level > current + EPSILON)
        .unwrap_or(current)
}

/// The next level down from `current`, or `current` if already at the bottom.
pub fn zoom_out(current: f32) -> f32 {
    LEVELS
        .iter()
        .rev()
        .copied()
        .find(|&level| level < current - EPSILON)
        .unwrap_or(current)
}

/// `current` as a whole percentage, for display.
pub fn percent(current: f32) -> u32 {
    (current * 100.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_ladder() {
        assert_eq!(zoom_in(1.0), 1.1);
        assert_eq!(zoom_in(1.1), 1.25);
        assert_eq!(zoom_out(1.0), 0.9);
        assert_eq!(zoom_out(0.9), 0.8);
    }

    #[test]
    fn the_ends_of_the_ladder_hold() {
        assert_eq!(zoom_in(5.0), 5.0);
        assert_eq!(zoom_out(0.25), 0.25);
    }

    #[test]
    fn off_ladder_values_snap_to_the_next_rung() {
        // A zoom set some other way (a future pinch, say) still steps sensibly.
        assert_eq!(zoom_in(1.03), 1.1);
        assert_eq!(zoom_out(1.03), 1.0);
    }

    #[test]
    fn float_error_does_not_skip_a_rung() {
        assert_eq!(zoom_in(1.1 - 0.0001), 1.25);
        assert_eq!(zoom_out(0.67 + 0.0001), 0.5);
    }

    #[test]
    fn percentages_round_for_display() {
        assert_eq!(percent(1.0), 100);
        assert_eq!(percent(0.67), 67);
        assert_eq!(percent(1.25), 125);
    }
}
