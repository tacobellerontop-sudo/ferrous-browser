//! Browser chrome: the tab strip, navigation buttons and the address/search bar.
//!
//! Pure UI. It reads the tab model, emits [`Action`]s, and reports the rectangle
//! left over for web content. It never touches Servo.
//!
//! The tab set is taken as `&mut Tabs` for the whole frame rather than read
//! widget by widget. `BrowserState` holds the model behind a `RefCell`, and a
//! `RefCell` allows only one borrow at a time — a `tabs()` call here and an
//! `active()` call in the next widget would panic at runtime, not at compile time.

use std::rc::Rc;

use egui::text::CCursor;
use egui::{
    Align, Button, Id, Panel, Rect, Sense, Spinner, TextEdit, Ui, text_selection::CCursorRange,
};

use crate::browser_state::BrowserState;
use crate::icons;
use crate::tab::{Tab, TabId, Tabs};

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

/// Tab strip metrics. A fixed width per tab rather than a flexible one: with a
/// flex layout, adding a tab resizes every other tab, which makes the strip jump
/// under the pointer and is genuinely annoying to click.
const TAB_HEIGHT: f32 = 28.0;
const TAB_WIDTH: f32 = 176.0;
const TAB_CLOSE_WIDTH: f32 = 22.0;

/// Something the user asked for by clicking or typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Navigate(String),
    Back,
    Forward,
    Reload,
    NewTab,
    /// Close a tab. Carries the id rather than an index so it cannot be applied
    /// to the wrong tab if the strip changes between the click and the drain.
    CloseTab(TabId),
    SelectTab(TabId),
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

    /// Draw the tab strip, toolbar, and report the content rectangle.
    ///
    /// The content rectangle is derived from the toolbar panel's own response
    /// rect rather than `Ui::available_rect_before_wrap()`. The latter is
    /// documented as "what is left on this row/column before wrapping" and, with
    /// a non-wrapping top-level layout, returns the full screen — it does not
    /// account for panels. servoshell likewise reads
    /// `outer.response.rect.max.y` (desktop/gui.rs:608).
    pub fn draw(&mut self, ui: &mut Ui, tabs: &mut Tabs) -> Output {
        let mut actions = Vec::new();

        self.tab_strip(ui, tabs, &mut actions);

        let frame = egui::Frame::new().fill(ui.visuals().panel_fill);
        let toolbar = Panel::top("toolbar").frame(frame).show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                self.nav_buttons(ui, tabs, &mut actions);
                self.address_bar(ui, tabs, &mut actions);
                self.loading_indicator(ui, tabs);
            });
            ui.add_space(6.0);
        });

        // `content_rect` rather than the deprecated `screen_rect`: this is the
        // area actually available to draw in, which is what the web view fills.
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

    /// One row of tabs, plus a new-tab button.
    fn tab_strip(&self, ui: &mut Ui, tabs: &Tabs, actions: &mut Vec<Action>) {
        let frame = egui::Frame::new().fill(ui.visuals().panel_fill);
        Panel::top("tabs").frame(frame).show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                for tab in tabs.iter() {
                    self.tab(ui, tab, tabs, actions);
                }

                // Plain ASCII rather than a symbol: egui's bundled font has no
                // Dingbats coverage, so U+2715 renders as a tofu box. Same
                // reason the nav icons are painted.
                if ui
                    .add_sized([BUTTON, TAB_HEIGHT], Button::new("+").frame(false))
                    .on_hover_text("New tab (Ctrl+T)")
                    .clicked()
                {
                    actions.push(Action::NewTab);
                }
                ui.add_space(4.0);
            });
        });
    }

    fn tab(&self, ui: &mut Ui, tab: &Tab, tabs: &Tabs, actions: &mut Vec<Action>) {
        let selected = tab.id == tabs.active().id;
        let label_width = (TAB_WIDTH - TAB_CLOSE_WIDTH).max(40.0);

        // No inter-item spacing inside a tab, for the same reason the title-bar
        // controls have none: it would make each tab 8pt wider than `TAB_WIDTH`,
        // so tabs would drift apart and the close button would not sit where the
        // geometry predicts. Found by a click test that missed by exactly 8pt.
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.horizontal(|ui| {
            // `Sense::click` plus manual painting rather than `Button`, so the
            // active tab can be tinted without the default frame fighting it.
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(label_width, TAB_HEIGHT), Sense::click());
            let visuals = ui.visuals().clone();
            let fill = if selected {
                visuals.selection.bg_fill
            } else {
                visuals.widgets.noninteractive.bg_fill
            };
            ui.painter().rect_filled(rect, 3.0, fill);

            // One line, clipped to the tab. `layout` would *wrap* a long title
            // onto three lines, and centring that block pushes the text out of
            // the strip and over the toolbar — which is exactly what the first
            // version did. The full URL is in the tooltip.
            let galley = ui.painter().layout_no_wrap(
                tab.label().to_owned(),
                egui::FontId::proportional(12.0),
                visuals.text_color(),
            );
            ui.painter()
                .with_clip_rect(rect.shrink(4.0))
                .galley(
                    egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0),
                    galley,
                    visuals.text_color(),
                );

            if response.clicked() {
                actions.push(Action::SelectTab(tab.id));
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            // The full URL as a tooltip; the label is clipped to a tab width.
            response.on_hover_text(&tab.url);

            let (close_rect, close_response) = ui.allocate_exact_size(
                egui::vec2(TAB_CLOSE_WIDTH, TAB_HEIGHT),
                Sense::click(),
            );
            if close_response.hovered() {
                ui.painter()
                    .rect_filled(close_rect, 3.0, visuals.widgets.hovered.bg_fill);
            }
            let cross = egui::Align2::CENTER_CENTER;
            ui.painter()
                .text(close_rect.center(), cross, "x", egui::FontId::proportional(12.0), visuals.weak_text_color());

            if close_response.clicked() {
                actions.push(Action::CloseTab(tab.id));
            }
            if close_response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            close_response.on_hover_text("Close tab (Ctrl+W)");
        });
    }

    fn nav_buttons(&self, ui: &mut Ui, tabs: &Tabs, actions: &mut Vec<Action>) {
        let size = egui::vec2(BUTTON, ICON_BUTTON);
        let (back, back_response) = ui.allocate_exact_size(size, Sense::click());
        let (forward, forward_response) = ui.allocate_exact_size(size, Sense::click());
        let (reload, reload_response) = ui.allocate_exact_size(size, Sense::click());

        // Disabled rather than hidden: a button that appears and disappears
        // makes the toolbar jump around, and a dimmed one still says "Back" in
        // its tooltip.
        let active = tabs.active();
        let (back_on, forward_on) = (active.can_go_back, active.can_go_forward);
        let on = ui.visuals().weak_text_color();
        let off = icons::dim(on);

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

    /// A spinner while the active page loads. The engine already reports load
    /// status, so this costs nothing but a widget.
    fn loading_indicator(&self, ui: &mut Ui, tabs: &Tabs) {
        if tabs.active().loading {
            ui.add(Spinner::new().size(14.0));
        }
    }

    fn address_bar(&mut self, ui: &mut Ui, tabs: &mut Tabs, actions: &mut Vec<Action>) {
        // Read the committed URL first: the borrow of `address_text` below lasts
        // for the whole widget and cannot overlap a second borrow of `tabs`.
        let committed = tabs.active().url.clone();

        let text = &mut tabs.active_mut().address_text;

        // Reserve room for the loading indicator. With
        // `desired_width(f32::INFINITY)` the field claims the entire row and
        // squeezes the spinner to zero width, so it would never be visible.
        let width = (ui.available_width() - SPINNER_SLOT).max(120.0);

        let response = ui.add_sized(
            [width, 22.0],
            TextEdit::singleline(text)
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
            select_all(ui.ctx(), text);
        }
        self.address_focused_last_frame = focused;

        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

        // Escape abandons the edit and restores the address of the page actually
        // loaded. Without this a half-typed URL sticks around and looks like the
        // browser's real location, which is worse than showing nothing.
        if focused && escape {
            *text = committed;
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
                if text.trim() != committed {
                    *text = committed;
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
