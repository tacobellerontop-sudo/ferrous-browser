//! Hand-painted UI icons.
//!
//! Every icon here is drawn with [`egui::Painter`] rather than rendered as text.
//! That is not a stylistic choice: **egui's bundled font has no coverage for
//! Arrows (U+2190..), Misc Symbols (U+2600..), Geometric Shapes (U+25A0..) or
//! Dingbats (U+2700..)**, so a `Button::new("\u{2190}")` renders as a `.notdef`
//! tofu box. This was confirmed on screen with the navigation buttons and the
//! window controls, not assumed.
//!
//! Painting them also keeps them crisp at any DPI and avoids shipping a font
//! just for six shapes.

use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, vec2};

/// Side of the arrow heads and general icon scale, in points.
const ARROW_HEAD: f32 = 4.0;
const ARROW_SHAFT: f32 = 3.0;
const STROKE: f32 = 1.6;

/// Dimmed colour for a control that is present but not currently available.
pub fn dim(color: Color32) -> Color32 {
    let alpha = (f32::from(color.a()) * 0.4) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn stroke(color: Color32) -> Stroke {
    Stroke::new(STROKE, color)
}

/// Left-pointing arrow, for the Back control.
pub fn back(painter: &Painter, center: Pos2, color: Color32) {
    let tip = center + vec2(-ARROW_SHAFT, 0.0);
    painter.line_segment([tip, center + vec2(ARROW_SHAFT, 0.0)], stroke(color));
    painter.line_segment([tip, tip + vec2(ARROW_SHAFT, -ARROW_HEAD)], stroke(color));
    painter.line_segment([tip, tip + vec2(ARROW_SHAFT, ARROW_HEAD)], stroke(color));
}

/// Right-pointing arrow, for the Forward control.
pub fn forward(painter: &Painter, center: Pos2, color: Color32) {
    let tip = center + vec2(ARROW_SHAFT, 0.0);
    painter.line_segment([center - vec2(ARROW_SHAFT, 0.0), tip], stroke(color));
    painter.line_segment([tip, center + vec2(0.0, -ARROW_HEAD)], stroke(color));
    painter.line_segment([tip, center + vec2(0.0, ARROW_HEAD)], stroke(color));
}

/// Circular arrow, for the Reload control.
///
/// Drawn as an arc that stops short of a full circle, plus two short lines
/// forming the head at the open end — the same construction Windows and Firefox
/// use, so it reads as "reload" without a glyph.
pub fn reload(painter: &Painter, center: Pos2, color: Color32) {
    const RADIUS: f32 = 4.5;
    /// Arc sweep, in radians. Short of a full turn so the head has a gap.
    const SWEEP: f32 = 4.9;
    const START: f32 = -0.4;
    const SEGMENTS: usize = 16;

    let points = (0..=SEGMENTS)
        .map(|i| {
            let angle = START + (i as f32 / SEGMENTS as f32) * SWEEP;
            center + vec2(angle.cos() * RADIUS, angle.sin() * RADIUS)
        })
        .collect();
    painter.add(egui::Shape::closed_line(points, stroke(color)));

    // The arc head sits at the *end* of the sweep, not the start.
    let end_angle = START + SWEEP;
    let end = center + vec2(end_angle.cos() * RADIUS, end_angle.sin() * RADIUS);
    let tangent = vec2(-end_angle.sin(), end_angle.cos());
    let outward = vec2(end_angle.cos(), end_angle.sin());
    painter.line_segment([end, end + tangent * 2.2], stroke(color));
    painter.line_segment([end, end + outward * 2.2], stroke(color));
}

/// The window-control icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowIcon {
    Minimize,
    Maximize,
    Restore,
    Close,
}

const WINDOW_ICON_SIZE: f32 = 10.0;

/// Windows tints the close control red on hover; the others just brighten.
/// Keeping the convention makes the destructive button findable without a
/// tooltip.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

pub fn window(painter: &Painter, center: Pos2, icon: WindowIcon, color: Color32) {
    let half = WINDOW_ICON_SIZE / 2.0;
    let outline = Stroke::new(1.2_f32, color);
    let square = Rect::from_center_size(center, vec2(WINDOW_ICON_SIZE, WINDOW_ICON_SIZE));
    // `Inside` keeps the stroke inside the button, so a maximised window with no
    // visible frame does not get a bright outline painted half off-screen.
    let kind = StrokeKind::Inside;

    match icon {
        WindowIcon::Minimize => {
            painter.rect_filled(
                Rect::from_center_size(center + vec2(0.0, half), vec2(WINDOW_ICON_SIZE, 1.2)),
                0.0,
                color,
            );
        }
        WindowIcon::Maximize => {
            painter.rect_stroke(square, 0.0, outline, kind);
        }
        WindowIcon::Restore => {
            // Two offset outlines. Masking the overlap would mean repainting
            // the button background; leaving it reads as the familiar restore
            // icon and costs nothing.
            painter.rect_stroke(square.translate(vec2(3.0, -3.0)), 0.0, outline, kind);
            painter.rect_stroke(square, 0.0, outline, kind);
        }
        WindowIcon::Close => {
            let d = vec2(half, half);
            painter.line_segment([center - d, center + d], outline);
            painter.line_segment([center + vec2(d.x, -d.y), center + vec2(-d.x, d.y)], outline);
        }
    }
}
