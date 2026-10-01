//! Input translation: winit events to Servo events.
//!
//! Pure functions only. Everything here takes values and returns values, with no
//! reference to the app, the engine or any window handle â€” which is what makes it
//! unit testable, and coordinate translation is precisely the kind of thing that
//! is easy to get subtly wrong.
//!
//! `docs/architecture-research.md` records the trap this module exists to make
//! visible: `WebViewPoint` has `Device` and `Page` variants, the `Page` one is
//! expressed in CSS pixels *after* zoom, and passing it where device pixels are
//! expected is silently wrong. There is also a `From<Point2D<f32, _>>` impl for
//! each variant, so `.into()` does not compile and forces the choice. Every
//! conversion below therefore names its variant explicitly.

use egui::Rect;
use servo::{
    DevicePoint, Key, KeyState, Location, Modifiers, MouseButton, WebViewPoint,
};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, Ime, MouseButton as WinitMouseButton};
use winit::keyboard::Key as WinitKey;
use winit::keyboard::{NamedKey as WinitNamedKey, PhysicalKey};
use winit::keyboard::ModifiersState;

/// servoshell uses the same value (ports/servoshell/window.rs:25-27).
pub const SCROLL_LINE_HEIGHT: f64 = 76.0;

/// Translate a window-space cursor position into web-view device pixels.
///
/// The content rectangle is what makes this correct: the window's origin is not
/// the page's origin, so without subtracting the toolbar and tab strip a click
/// 40px into the chrome would register as a click at (40, 0) of the page.
/// servoshell does the equivalent at headed_window.rs:300-301.
///
/// `ppp` is egui's pixels-per-point. Passing 1.0 would be wrong on any display
/// that is not at 100% scaling, which is the kind of bug that only appears on
/// someone else's laptop.
pub fn page_point(
    position: PhysicalPosition<f64>,
    ppp: f32,
    content: Option<Rect>,
) -> WebViewPoint {
    let point = window_point(position, ppp);
    let Some(content) = content else {
        // No frame has been laid out yet, so there is no meaningful mapping.
        return WebViewPoint::Device(DevicePoint::new(point.x, point.y));
    };
    WebViewPoint::Device(DevicePoint::new(
        point.x - content.min.x,
        point.y - content.min.y,
    ))
}

/// Window-space cursor position in egui points, top-left origin.
pub fn window_point(position: PhysicalPosition<f64>, ppp: f32) -> egui::Pos2 {
    let ppp = if ppp > 0.0 { ppp as f64 } else { 1.0 };
    egui::pos2((position.x / ppp) as f32, (position.y / ppp) as f32)
}

/// Key translation via `FromStr` on the winit `Debug` representation.
///
/// `keyboard_types::Key` is `enum { Character(String), Named(NamedKey) }` and its
/// `FromStr` routes through `is_key_string`, which rejects multi-character ASCII
/// strings â€” so "Enter" parses to a *named* key rather than a literal character.
/// Characters are therefore handled explicitly.
///
/// This is a shortcut. servoshell carries a ~600 line exhaustive match table
/// (ports/servoshell/desktop/keyutils.rs); a full keyboard milestone should port
/// that rather than lean on Debug formatting.
pub fn keyboard_event(
    event: &winit::event::KeyEvent,
    mods: ModifiersState,
) -> servo::KeyboardEvent {
    let key = dom_key(&event.logical_key);
    let code = dom_code(event.physical_key);

    let mut modifiers = Modifiers::empty();
    modifiers.set(Modifiers::CONTROL, mods.control_key());
    modifiers.set(Modifiers::SHIFT, mods.shift_key());
    modifiers.set(Modifiers::ALT, mods.alt_key());
    modifiers.set(Modifiers::META, mods.super_key());

    servo::KeyboardEvent::new_without_event(
        match event.state {
            ElementState::Pressed => KeyState::Down,
            ElementState::Released => KeyState::Up,
        },
        key,
        code,
        Location::Standard,
        modifiers,
        event.repeat,
        false,
    )
}

/// Translate winit's logical key into the DOM `key` value.
///
/// Named keys are matched on the *inner* `NamedKey`. Formatting the whole
/// `winit::keyboard::Key` with `{:?}` gives `"Named(Enter)"`, which is not a key
/// name, so every named key used to reach the page as `Unidentified` â€” Enter
/// could not submit a form and Backspace could not delete. winit names its
/// variants after the W3C key values, so the inner name parses directly.
fn dom_key(key: &WinitKey) -> Key {
    match key {
        WinitKey::Character(text) => Key::Character(text.to_string()),
        // The one mismatch that matters: winit treats Space as a named key, but
        // in the DOM its `key` is the character " ". As `Unidentified` it was
        // silently dropped from every text field.
        WinitKey::Named(WinitNamedKey::Space) => Key::Character(" ".to_owned()),
        WinitKey::Named(named) => format!("{named:?}")
            .parse::<servo::NamedKey>()
            .map_or(Key::Named(servo::NamedKey::Unidentified), Key::Named),
        _ => Key::Named(servo::NamedKey::Unidentified),
    }
}

/// Translate winit's physical key into the DOM `code` value, for the same
/// reason as [`dom_key`]: `{:?}` of the wrapper is `"Code(KeyA)"`, not `"KeyA"`.
fn dom_code(physical: PhysicalKey) -> servo::Code {
    match physical {
        PhysicalKey::Code(code) => format!("{code:?}")
            .parse()
            .unwrap_or(servo::Code::Unidentified),
        PhysicalKey::Unidentified(_) => servo::Code::Unidentified,
    }
}

pub fn mouse_button(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Primary,
        WinitMouseButton::Right => MouseButton::Secondary,
        WinitMouseButton::Middle => MouseButton::Auxiliary,
        WinitMouseButton::Back => MouseButton::Back,
        WinitMouseButton::Forward => MouseButton::Forward,
        WinitMouseButton::Other(n) => MouseButton::Other(n as u16),
    }
}

/// Translate an IME event for the page.
///
/// Enabled, preedit and commit become the start, update and end of a DOM
/// composition, which is what makes CJK input appear in a page's text field.
/// `Disabled` maps to nothing here: whether it means "the user dismissed the
/// IME" or "we turned it off ourselves" depends on state only the caller has.
/// Mirrors servoshell's handling (ports/servoshell/desktop/headed_window.rs).
pub fn composition_event(ime: &Ime) -> Option<servo::ImeEvent> {
    let (state, data) = match ime {
        Ime::Enabled => (servo::CompositionState::Start, String::new()),
        Ime::Preedit(text, _) => (servo::CompositionState::Update, text.clone()),
        Ime::Commit(text) => (servo::CompositionState::End, text.clone()),
        Ime::Disabled => return None,
    };
    Some(servo::ImeEvent::Composition(servo::CompositionEvent { state, data }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    fn content() -> Rect {
        Rect::from_min_max(egui::pos2(0.0, 60.0), egui::pos2(800.0, 700.0))
    }

    fn device(v: WebViewPoint) -> (f32, f32) {
        match v {
            // The variant is named explicitly on purpose: this test exists to
            // catch the Day-one mistake of passing `Page` where `Device` belongs.
            WebViewPoint::Device(p) => (p.x, p.y),
            WebViewPoint::Page(p) => panic!("expected Device, got Page {p:?}"),
        }
    }

    #[test]
    fn page_point_subtracts_the_chrome_height() {
        // A click at the very top of the page content must map to (0, 0).
        let p = page_point(PhysicalPosition::new(100.0, 60.0), 1.0, Some(content()));
        assert_eq!(device(p), (100.0, 0.0));
    }

    #[test]
    fn page_point_accounts_for_scale() {
        // At 150% scaling, 150 physical pixels is 100 egui points, and the
        // content rectangle starts 60 points down, so a physical y of 150 lands
        // 40 points into the page.
        let p = page_point(PhysicalPosition::new(150.0, 150.0), 1.5, Some(content()));
        assert_eq!(device(p), (100.0, 40.0));
    }

    #[test]
    fn page_point_at_100_percent_scale() {
        // Same arithmetic without the scale factor, as a control for the above.
        let p = page_point(PhysicalPosition::new(150.0, 150.0), 1.0, Some(content()));
        assert_eq!(device(p), (150.0, 90.0));
    }

    #[test]
    fn page_point_always_returns_device_not_page() {
        let p = page_point(PhysicalPosition::new(10.0, 70.0), 1.0, Some(content()));
        assert!(matches!(p, WebViewPoint::Device(_)));
    }

    #[test]
    fn page_point_before_layout_is_not_panicky() {
        // No frame drawn yet: there is no content rect to subtract. Must return
        // something rather than panicking, because mouse events arrive early.
        let p = page_point(PhysicalPosition::new(5.0, 5.0), 1.0, None);
        assert_eq!(device(p), (5.0, 5.0));
    }

    #[test]
    fn window_point_does_not_subtract_the_chrome() {
        let p = window_point(PhysicalPosition::new(100.0, 70.0), 1.0);
        assert_eq!((p.x, p.y), (100.0, 70.0));
    }

    #[test]
    fn a_zero_scale_factor_does_not_divide_by_zero() {
        // egui reports 1.0 before the first frame, but a defensive floor costs
        // nothing and turns a potential NaN into a usable point.
        let p = window_point(PhysicalPosition::new(50.0, 50.0), 0.0);
        assert_eq!((p.x, p.y), (50.0, 50.0));
    }

    #[test]
    fn scroll_line_height_matches_servoshell() {
        assert_eq!(SCROLL_LINE_HEIGHT, 76.0);
    }

    #[test]
    fn named_keys_reach_the_page_by_name() {
        // Regression: these all used to arrive as `Unidentified`.
        for (winit, dom) in [
            (WinitNamedKey::Enter, servo::NamedKey::Enter),
            (WinitNamedKey::Backspace, servo::NamedKey::Backspace),
            (WinitNamedKey::Tab, servo::NamedKey::Tab),
            (WinitNamedKey::ArrowLeft, servo::NamedKey::ArrowLeft),
            (WinitNamedKey::Escape, servo::NamedKey::Escape),
            (WinitNamedKey::Delete, servo::NamedKey::Delete),
        ] {
            assert_eq!(dom_key(&WinitKey::Named(winit)), Key::Named(dom));
        }
    }

    #[test]
    fn space_is_a_character_in_the_dom() {
        assert_eq!(dom_key(&WinitKey::Named(WinitNamedKey::Space)), Key::Character(" ".to_owned()));
    }

    #[test]
    fn characters_pass_through() {
        assert_eq!(dom_key(&WinitKey::Character("a".into())), Key::Character("a".to_owned()));
    }

    #[test]
    fn physical_codes_reach_the_page_by_name() {
        assert_eq!(dom_code(PhysicalKey::Code(KeyCode::KeyA)), servo::Code::KeyA);
        assert_eq!(dom_code(PhysicalKey::Code(KeyCode::Enter)), servo::Code::Enter);
        assert_eq!(dom_code(PhysicalKey::Code(KeyCode::Space)), servo::Code::Space);
    }

    fn composition(ime: Ime) -> (servo::CompositionState, String) {
        match composition_event(&ime) {
            Some(servo::ImeEvent::Composition(c)) => (c.state, c.data),
            other => panic!("expected a composition, got {other:?}"),
        }
    }

    #[test]
    fn ime_composition_maps_start_update_end() {
        assert_eq!(composition(Ime::Enabled), (servo::CompositionState::Start, String::new()));
        assert_eq!(
            composition(Ime::Preedit("にほ".into(), Some((0, 6)))),
            (servo::CompositionState::Update, "にほ".to_owned())
        );
        assert_eq!(
            composition(Ime::Commit("日本".into())),
            (servo::CompositionState::End, "日本".to_owned())
        );
    }

    #[test]
    fn ime_disabled_is_left_to_the_caller() {
        assert!(composition_event(&Ime::Disabled).is_none());
    }
}
