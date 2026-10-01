//! Opt-in frame timing, for checking animation smoothness on a real display.
//!
//! Set `FERROUS_FRAME_STATS=1` before launching. Every frame then records how
//! long the browser spent building it on the CPU, how long `present()` blocked
//! waiting for the swap chain, and the time since the previous frame. On exit
//! the raw rows go to `%TEMP%\ferrous-frames.csv` and a summary to
//! `%TEMP%\ferrous-frames-summary.txt`.
//!
//! Why this exists: measured inside a sandboxed session, building a frame cost
//! under 1ms of CPU and about 2ms of GPU, yet frames arrived 15-45ms apart
//! because `present()` blocked on the compositor. Whether that is the session's
//! virtual display or something real can only be told on the user's own
//! desktop, and this is the tool for it.
//!
//! Disabled, it costs one branch per frame.

use std::cell::{Cell, RefCell};
use std::fmt::Write as _;
use std::time::Instant;

/// Frames closer together than this are treated as one animation burst when
/// summarising frame spacing; longer gaps are just the browser being idle.
const BURST_GAP_MS: f32 = 100.0;

/// One frame: CPU build time, present wait, and gap since the previous frame,
/// all in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub cpu: f32,
    pub present: f32,
    pub interval: f32,
}

#[derive(Default)]
pub struct FrameStats {
    enabled: bool,
    frames: RefCell<Vec<Frame>>,
    last_start: Cell<Option<Instant>>,
}

impl FrameStats {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var_os("FERROUS_FRAME_STATS").is_some_and(|v| v != "0"),
            ..Self::default()
        }
    }

    /// Record a frame that started at `start`, finished its own work at
    /// `before_present` and returned from `present()` at `end`.
    pub fn record(&self, start: Instant, before_present: Instant, end: Instant) {
        if !self.enabled {
            return;
        }
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f32() * 1000.0;
        let interval = self.last_start.replace(Some(start)).map_or(f32::INFINITY, |last| ms(last, start));
        self.frames.borrow_mut().push(Frame {
            cpu: ms(start, before_present),
            present: ms(before_present, end),
            interval,
        });
    }

    /// Write the CSV and summary. Called once, on exit.
    pub fn finish(&self) {
        if !self.enabled {
            return;
        }
        let frames = self.frames.borrow();
        let dir = std::env::temp_dir();
        let mut csv = String::from("cpu_ms,present_ms,interval_ms\n");
        for f in frames.iter() {
            let _ = writeln!(csv, "{:.3},{:.3},{:.3}", f.cpu, f.present, f.interval);
        }
        let summary = summarise(&frames);
        let _ = std::fs::write(dir.join("ferrous-frames.csv"), csv);
        let _ = std::fs::write(dir.join("ferrous-frames-summary.txt"), &summary);
        log::info!("frame stats:\n{summary}");
    }
}

/// Average, 95th percentile and maximum of `values`.
fn spread(mut values: Vec<f32>) -> Option<(f32, f32, f32)> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    let average = values.iter().sum::<f32>() / values.len() as f32;
    let p95 = values[((values.len() - 1) as f32 * 0.95).round() as usize];
    Some((average, p95, *values.last()?))
}

pub fn summarise(frames: &[Frame]) -> String {
    let mut out = format!("frames: {}\n", frames.len());
    let mut line = |name: &str, values: Vec<f32>| {
        if let Some((average, p95, max)) = spread(values) {
            let _ = writeln!(out, "{name:<34} avg {average:6.2}  p95 {p95:6.2}  max {max:6.2} ms");
        }
    };
    line("cpu (build frame)", frames.iter().map(|f| f.cpu).collect());
    line("present (wait for swap chain)", frames.iter().map(|f| f.present).collect());
    line(
        "interval within animations",
        frames.iter().map(|f| f.interval).filter(|&i| i < BURST_GAP_MS).collect(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(cpu: f32, present: f32, interval: f32) -> Frame {
        Frame { cpu, present, interval }
    }

    #[test]
    fn spread_reports_average_p95_and_max() {
        let values: Vec<f32> = (1..=100).map(|v| v as f32).collect();
        let (average, p95, max) = spread(values).unwrap();
        assert_eq!(average, 50.5);
        assert_eq!(p95, 95.0);
        assert_eq!(max, 100.0);
        assert_eq!(spread(Vec::new()), None);
    }

    #[test]
    fn idle_gaps_are_left_out_of_animation_spacing() {
        let frames = [
            frame(1.0, 8.0, f32::INFINITY),
            frame(1.0, 8.0, 8.3),
            frame(1.0, 8.0, 8.3),
            frame(1.0, 8.0, 2500.0),
        ];
        let summary = summarise(&frames);
        assert!(summary.contains("frames: 4"));
        let spacing = summary.lines().find(|l| l.starts_with("interval")).unwrap();
        assert!(spacing.contains("max   8.30"), "{spacing}");
    }

    #[test]
    fn disabled_stats_record_nothing() {
        let stats = FrameStats::default();
        let now = Instant::now();
        stats.record(now, now, now);
        assert!(stats.frames.borrow().is_empty());
    }
}
