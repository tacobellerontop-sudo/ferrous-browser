//! The chrome's colour palette and egui style.
//!
//! A quiet, "zen" arrangement: the window itself is the Windows Mica backdrop
//! (see `backdrop`), the page sits on it as a rounded card, and the chrome is a
//! single translucent panel that floats over the page only while it is needed.
//!
//! Surfaces are told apart by **hairline outlines and faint white washes**
//! rather than by solid fills of different greys, so they read as layers of
//! the same glass instead of stacked grey boxes. Every wash is white at a low
//! alpha, which keeps it correct over both the dark Mica tint and the panel.
//!
//! The same style is installed for egui's dark *and* light themes. egui follows
//! the OS theme by default, and a half-applied palette — our hand-painted
//! surfaces on top of egui's light widgets — is worse than either.

use egui::{Color32, CornerRadius, Stroke, Style};

const fn white(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const(255, 255, 255, alpha)
}

/// Window background when the Mica backdrop is unavailable (Windows 10).
pub const BACKDROP_FALLBACK: Color32 = Color32::from_rgb(0x17, 0x18, 0x1c);

/// The floating chrome panel's tint, laid over the frosted (blurred) page.
/// Translucent enough for the frost to show, dark enough that text on it stays
/// readable even over a white page: over white it works out to about #4e4f52.
pub const PANEL: Color32 = Color32::from_rgba_unmultiplied_const(0x1b, 0x1c, 0x21, 0xc4);
/// The light catching the panel's top edge.
pub const RIM_LIGHT: Color32 = white(34);

/// Hairline around the panel, the page card, the address bar and the active tab.
pub const OUTLINE: Color32 = white(26);
/// The same hairline on something under the pointer.
pub const OUTLINE_STRONG: Color32 = white(46);

/// Washes for interactive things: hovered, pressed, and the selected tab.
pub const HOVER: Color32 = white(14);
pub const PRESS: Color32 = white(24);
pub const SELECTED: Color32 = white(20);

/// The address bar's well.
pub const FIELD: Color32 = white(8);
pub const FIELD_HOVER: Color32 = white(13);

pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xec);
pub const TEXT_WEAK: Color32 = Color32::from_rgb(0x9a, 0x9f, 0xa8);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(0x62, 0x67, 0x6f);

/// Focus outline, text selection, loading indicators.
pub const ACCENT: Color32 = Color32::from_rgb(0x7c, 0xac, 0xf8);
/// The second colour of the loading glow, which drifts between the two.
pub const ACCENT_ALT: Color32 = Color32::from_rgb(0xb4, 0x8c, 0xff);

/// Windows tints the close control red on hover; keeping the convention makes
/// the destructive button findable without a tooltip.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc4, 0x2b, 0x1c);

/// Install the palette. Call once, before the first frame.
pub fn install(ctx: &egui::Context) {
    ctx.all_styles_mut(apply);
}

fn apply(style: &mut Style) {
    let v = &mut style.visuals;
    v.dark_mode = true;
    // Nothing egui draws by default should cover the Mica backdrop.
    v.panel_fill = Color32::TRANSPARENT;
    v.extreme_bg_color = Color32::TRANSPARENT;
    v.text_edit_bg_color = Some(Color32::TRANSPARENT);
    v.override_text_color = Some(TEXT);
    v.weak_text_color = Some(TEXT_WEAK);
    v.hyperlink_color = ACCENT;

    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0_f32, TEXT);
    v.text_cursor.stroke = Stroke::new(1.5_f32, ACCENT);

    // Tooltips: a small floating panel in the same style as the chrome.
    v.window_fill = Color32::from_rgb(0x1f, 0x20, 0x25);
    v.window_stroke = Stroke::new(1.0_f32, OUTLINE);
    v.window_corner_radius = CornerRadius::same(8);

    for widget in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        widget.corner_radius = CornerRadius::same(8);
        widget.fg_stroke.color = TEXT;
        widget.bg_stroke = Stroke::new(1.0_f32, OUTLINE);
    }
    v.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_fill = FIELD;
    v.widgets.inactive.weak_bg_fill = FIELD;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.active.bg_fill = PRESS;
    v.widgets.active.weak_bg_fill = PRESS;
}
