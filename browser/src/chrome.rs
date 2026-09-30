//! Browser chrome: navigation buttons and the address/search bar.
//!
//! Pure UI. It reads [`BrowserState`], emits [`Action`]s, and reports the
//! rectangle left over for web content. It never touches Servo.

use std::rc::Rc;

use egui::text::CCursor;
use egui::{
    Align, Id, Panel, Rect, Sense, Spinner, TextEdit, Ui, text_selection::CCursorRange,
};

use crate::browser_state::BrowserState;
use crate::icons;

/// Stable id for the address field. Explicit rather than auto-generated so that
/// [`TextEdit::load_state`] can find the widget's state and select its text;
/// an auto id would drift as the surrounding layout changes.
const ADDRESS_BAR_ID: fn() -> Id = || Id::new("ferrous.address_bar");

/// Horizontal space kept free at the end of the toolbar row for the loading
/// indicator.
const SPINNER_SLOT: f32 = 24.0;

/// Clickable size of each navigation icon.
const BUTTON: f32 = 24.0;
const ICON_BUTTON: f32 = 22.0;

/// Something the user asked for by clicking or typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Navigate(String),
    Back,
    Forward,
    Reload,
}

/// What the chrome produced this frame.
pub struct Output {
    /// The area below the toolbar, in egui points, where the web view lives.
    pub content_rect: Rect,
    pub actions: Vec<Action>,
}

pub struct Chrome {
    state: Rc<BrowserState>,
    /// Set for one frame after a focus request, so the address bar can grab
    /// focus without needing a "please focus me" flag that never clears.
    focus_address_bar_next_frame: bool,
    /// Whether the address bar had focus last frame. Used to detect the
    /// focus-*gained* edge.
    address_focused_last_frame: bool,
}

impl Chrome {
    pub fn new(state: Rc<BrowserState>) -> Self {
        Self {
            state,
            focus_address_bar_next_frame: false,
            address_focused_last_frame: false,
        }
    }

    pub fn request_focus_address_bar(&mut self) {
        self.focus_address_bar_next_frame = true;
    }

    /// Draw the toolbar and report the content rectangle.
    ///
    /// The content rectangle is derived from the toolbar panel's own response
    /// rect rather than `Ui::available_rect_before_wrap()`. The latter is
    /// documented as "what is left on this row/column before wrapping" and, with
    /// a non-wrapping top-level layout, returns the full screen — it does not
    /// account for panels. servoshell likewise reads
    /// `outer.response.rect.max.y` (desktop/gui.rs:608).
    pub fn draw(&mut self, ui: &mut Ui) -> Output {
        let mut actions = Vec::new();

        let frame = egui::Frame::new().fill(ui.visuals().panel_fill);
        let toolbar = Panel::top("toolbar").frame(frame).show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                self.nav_buttons(ui, &mut actions);
                self.address_bar(ui, &mut actions);
                self.loading_indicator(ui);
            });
            ui.add_space(6.0);
        });

        // `content_rect` rather than the deprecated `screen_rect`: this is the area
        // actually available to draw in, which is what the web view should fill.
        let content_bounds = ui.ctx().content_rect();
        let content_rect = Rect::from_min_max(
            egui::pos2(content_bounds.min.x, toolbar.response.rect.max.y),
            content_bounds.max,
        );

        Output {
            content_rect,
            actions,
        }
    }

    fn nav_buttons(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let size = egui::vec2(BUTTON, ICON_BUTTON);
        let (back, back_response) = ui.allocate_exact_size(size, Sense::click());
        let (forward, forward_response) = ui.allocate_exact_size(size, Sense::click());
        let (reload, reload_response) = ui.allocate_exact_size(size, Sense::click());

        // Disabled rather than hidden: a button that appears and disappears
        // makes the toolbar jump around, and a dimmed one still says "Back"
        // in its tooltip.
        let on = ui.visuals().weak_text_color();
        let off = icons::dim(on);
        let (back_on, forward_on) = (self.state.can_go_back.get(), self.state.can_go_forward.get());

        let painter = ui.painter().clone();
        icons::back(&painter, back.center(), if back_on { on } else { off });
        icons::forward(&painter, forward.center(), if forward_on { on } else { off });
        icons::reload(&painter, reload.center(), on);

        // `on_hover_text` consumes the `Response`, so the click and hover tests
        // have to happen first.
        let (back_hit, forward_hit, reload_hit) = (
            back_on && back_response.clicked(),
            forward_on && forward_response.clicked(),
            reload_response.clicked(),
        );
        for response in [&back_response, &forward_response, &reload_response] {
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        let _ = back_response.on_hover_text("Back");
        let _ = forward_response.on_hover_text("Forward");
        let _ = reload_response.on_hover_text("Reload");

        if back_hit {
            actions.push(Action::Back);
        }
        if forward_hit {
            actions.push(Action::Forward);
        }
        if reload_hit {
            actions.push(Action::Reload);
        }

        ui.add_space(8.0);
    }

    /// A spinner while the page loads. The engine already reports load status
    /// via `notify_load_status_changed`, so this costs nothing but a widget.
    fn loading_indicator(&self, ui: &mut Ui) {
        if self.state.loading.get() {
            ui.add(Spinner::new().size(14.0));
        }
    }

    fn address_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut text = self.state.address_text.borrow_mut();

        // Reserve room for the loading indicator. With
        // `desired_width(f32::INFINITY)` the field claims the entire row and
        // squeezes the spinner to zero width, so it would never be visible.
        let width = (ui.available_width() - SPINNER_SLOT).max(120.0);

        let response = ui.add_sized(
            [width, 22.0],
            TextEdit::singleline(&mut *text)
                .id(ADDRESS_BAR_ID())
                .hint_text("Search or enter address")
                .vertical_align(Align::Center),
        );

        let focused = response.has_focus();
        self.state.address_focused.set(focused);

        if self.focus_address_bar_next_frame {
            response.request_focus();
            self.focus_address_bar_next_frame = false;
        }

        // Select the existing URL whenever the bar *gains* focus, so that typing
        // replaces it instead of appending to it.
        //
        // This is detected as an edge rather than driven by the focus request,
        // because focus can also arrive from a plain mouse click, which never
        // goes through `request_focus_address_bar`. Keying off the request alone
        // meant clicking the bar appended to the URL instead of replacing it.
        //
        // egui 0.34 has no `select_all_on_focus` option, so the selection is set
        // through the widget's public `load_state`/`store_state`.
        if focused && !self.address_focused_last_frame {
            select_all(ui.ctx(), &text);
        }
        self.address_focused_last_frame = focused;

        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

        // Escape abandons the edit and restores the address of the page actually
        // loaded. Without this a half-typed URL sticks around and looks like the
        // browser's real location, which is worse than showing nothing.
        if focused && escape {
            *text = self.state.url.borrow().clone();
            response.surrender_focus();
            return;
        }

        // Enter submits.
        //
        // The submit condition is `lost_focus()`, NOT `has_focus()`. A singleline
        // TextEdit deliberately surrenders focus when Enter is pressed
        // (egui-0.34.3/src/widgets/text_edit/builder.rs:108), so by the time this
        // frame runs, `has_focus()` is already false. Testing `focused && enter`
        // therefore never fires — which is exactly why typing in the address bar
        // appeared to do nothing. egui documents the correct idiom at
        // builder.rs:35.
        if response.lost_focus() {
            let trimmed = text.trim().to_owned();
            if enter && !trimmed.is_empty() {
                actions.push(Action::Navigate(trimmed));
            } else {
                // Lost focus without submitting: put the real address back so the
                // bar never displays text the browser is not actually on.
                let current = self.state.url.borrow().clone();
                if text.trim() != current {
                    *text = current;
                }
            }
        }
    }
}

/// Select every character of the address field.
///
/// `CCursor` indexes by **character**, not byte. Using `str::len()` would panic
/// on any non-ASCII URL, so the count is taken with `chars()`.
fn select_all(ctx: &egui::Context, text: &str) {
    let Some(mut state) = TextEdit::load_state(ctx, ADDRESS_BAR_ID()) else {
        return;
    };
    let end = text.chars().count();
    state
        .cursor
        .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
    TextEdit::store_state(ctx, ADDRESS_BAR_ID(), state);
}