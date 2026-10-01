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
//! just for a dozen shapes.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

/// Half-length of the Back/Forward arrow shafts, and the reach of their heads.
const ARROW_SHAFT: f32 = 5.5;
const ARROW_HEAD: f32 = 4.5;
const STROKE: f32 = 1.6;

/// Dimmed colour for a control that is present but not currently available.
pub fn dim(color: Color32) -> Color32 {
    let alpha = (f32::from(color.a()) * 0.4) as u8;
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

fn stroke(color: Color32) -> Stroke {
    Stroke::new(STROKE, color)
}

/// Points along a circular arc, `sweep` radians from `start`.
fn arc(center: Pos2, radius: f32, start: f32, sweep: f32, segments: usize) -> Vec<Pos2> {
    (0..=segments)
        .map(|i| {
            let angle = start + (i as f32 / segments as f32) * sweep;
            center + vec2(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

/// Left-pointing arrow, for the Back control.
pub fn back(painter: &Painter, center: Pos2, color: Color32) {
    let tip = center + vec2(-ARROW_SHAFT, 0.0);
    painter.line_segment([tip, center + vec2(ARROW_SHAFT, 0.0)], stroke(color));
    painter.add(Shape::line(
        vec![tip + vec2(ARROW_HEAD, -ARROW_HEAD), tip, tip + vec2(ARROW_HEAD, ARROW_HEAD)],
        stroke(color),
    ));
}

/// Right-pointing arrow, for the Forward control.
pub fn forward(painter: &Painter, center: Pos2, color: Color32) {
    let tip = center + vec2(ARROW_SHAFT, 0.0);
    painter.line_segment([center - vec2(ARROW_SHAFT, 0.0), tip], stroke(color));
    painter.add(Shape::line(
        vec![tip + vec2(-ARROW_HEAD, -ARROW_HEAD), tip, tip + vec2(-ARROW_HEAD, ARROW_HEAD)],
        stroke(color),
    ));
}

/// Circular arrow, for the Reload control.
///
/// An *open* arc plus a filled arrowhead at the end of the sweep. The first
/// version drew the arc with `Shape::closed_line`, which joins the two ends with
/// a chord — the icon looked like a round bomb rather than a reload arrow.
pub fn reload(painter: &Painter, center: Pos2, color: Color32) {
    const RADIUS: f32 = 5.5;
    /// Arc sweep, in radians. Short of a full turn so the head has a gap.
    const SWEEP: f32 = 4.8;
    /// Where the arc ends: upper right, like the reload icon in other browsers.
    /// Angles grow clockwise on screen because y points down.
    const END: f32 = -0.7;

    painter.add(Shape::line(arc(center, RADIUS, END - SWEEP, SWEEP, 20), stroke(color)));

    // The head sits at the end of the sweep and points the way the arc travels.
    let end = center + vec2(END.cos() * RADIUS, END.sin() * RADIUS);
    let tangent = vec2(-END.sin(), END.cos());
    let outward = vec2(END.cos(), END.sin());
    painter.add(Shape::convex_polygon(
        vec![end + outward * 2.8, end + tangent * 3.4, end - outward * 2.8],
        color,
        Stroke::NONE,
    ));
}

/// A padlock, shown in the address bar for HTTPS pages.
pub fn lock(painter: &Painter, center: Pos2, color: Color32) {
    let body = Rect::from_center_size(center + vec2(0.0, 2.0), vec2(9.0, 7.0));
    painter.rect_filled(body, 1.5, color);
    let shackle_center = pos2(center.x, body.top());
    painter.add(Shape::line(
        arc(shackle_center, 2.8, std::f32::consts::PI, std::f32::consts::PI, 10)
            .into_iter()
            .chain([pos2(shackle_center.x + 2.8, body.top())])
            .collect(),
        Stroke::new(1.4_f32, color),
    ));
}

/// A circled "i", shown in the address bar for pages that are not HTTPS.
pub fn info(painter: &Painter, center: Pos2, color: Color32) {
    painter.circle_stroke(center, 6.0, Stroke::new(1.3_f32, color));
    painter.circle_filled(center + vec2(0.0, -2.8), 0.9, color);
    painter.line_segment([center + vec2(0.0, -0.8), center + vec2(0.0, 3.2)], Stroke::new(1.4_f32, color));
}

/// A magnifying glass, shown in the address bar while typing a query.
pub fn search(painter: &Painter, center: Pos2, color: Color32) {
    let lens = center + vec2(-1.2, -1.2);
    painter.circle_stroke(lens, 3.8, Stroke::new(1.5_f32, color));
    let d = std::f32::consts::FRAC_1_SQRT_2;
    painter.line_segment(
        [lens + vec2(d, d) * 3.8, lens + vec2(d, d) * 7.6],
        Stroke::new(1.8_f32, color),
    );
}

/// A shield, for the ad blocker. `active` fills it; inactive draws the outline
/// struck through.
pub fn shield(painter: &Painter, center: Pos2, color: Color32, active: bool) {
    let outline: Vec<Pos2> = [
        (-5.5, -5.5),
        (0.0, -7.5),
        (5.5, -5.5),
        (5.5, -0.5),
        (3.6, 3.8),
        (0.0, 7.0),
        (-3.6, 3.8),
        (-5.5, -0.5),
    ]
    .into_iter()
    .map(|(x, y)| center + vec2(x, y))
    .collect();
    let fill = if active { color.gamma_multiply(0.28) } else { Color32::TRANSPARENT };
    painter.add(Shape::convex_polygon(outline, fill, Stroke::new(1.4_f32, color)));
    if !active {
        painter.line_segment([center + vec2(-6.5, 6.5), center + vec2(6.5, -6.5)], Stroke::new(1.4_f32, color));
    }
}

/// A clock face, for history suggestions.
pub fn clock(painter: &Painter, center: Pos2, color: Color32) {
    painter.circle_stroke(center, 6.0, Stroke::new(1.3_f32, color));
    painter.add(Shape::line(
        vec![center + vec2(0.0, -3.6), center, center + vec2(2.6, 1.6)],
        Stroke::new(1.3_f32, color),
    ));
}

/// A plus sign, for the New Tab control.
pub fn plus(painter: &Painter, center: Pos2, color: Color32) {
    const ARM: f32 = 5.0;
    painter.line_segment([center - vec2(ARM, 0.0), center + vec2(ARM, 0.0)], stroke(color));
    painter.line_segment([center - vec2(0.0, ARM), center + vec2(0.0, ARM)], stroke(color));
}

/// A small cross, for a tab's close button.
pub fn cross(painter: &Painter, center: Pos2, color: Color32) {
    const ARM: f32 = 3.5;
    let s = Stroke::new(1.4_f32, color);
    painter.line_segment([center + vec2(-ARM, -ARM), center + vec2(ARM, ARM)], s);
    painter.line_segment([center + vec2(ARM, -ARM), center + vec2(-ARM, ARM)], s);
}

/// A rotating arc, for a tab whose page is loading.
///
/// Painted here rather than using `egui::Spinner` so that it can sit at an exact
/// point inside a hand-laid-out tab without allocating layout space. The caller
/// is responsible for requesting a repaint while it is visible.
pub fn spinner(painter: &Painter, center: Pos2, time: f64, color: Color32) {
    const RADIUS: f32 = 5.5;
    let start = (time * 5.0).rem_euclid(std::f64::consts::TAU) as f32;
    painter.circle_stroke(center, RADIUS, Stroke::new(1.8_f32, dim(color)));
    painter.add(Shape::line(arc(center, RADIUS, start, 2.0, 12), Stroke::new(1.8_f32, color)));
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
