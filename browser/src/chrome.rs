//! Browser chrome: navigation buttons and the address/search bar.
//!
//! Pure UI. It reads [`BrowserState`], emits [`Action`]s, and reports the
//! rectangle left over for web content. It never touches Servo.

use std::rc::Rc;

use egui::{Align, Button, Panel, Rect, TextEdit, Ui};

use crate::browser_state::BrowserState;

/// Something the user asked for by clicking or typing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Navigate(String),
    Back,
    Forward,
    Reload,
    /// Focus the address bar, e.g. from a Ctrl+L shortcut.
    FocusAddressBar,
}

/// What the chrome produced this frame.
pub struct Output {
    /// The area below the toolbar, in egui points, where the web view lives.
    pub content_rect: Rect,
    pub actions: Vec<Action>,
}

pub struct Chrome {
    state: Rc<BrowserState>,
    /// Set for one frame after a Ctrl+L style request, so the address bar can
    /// grab focus without needing a "please focus me" flag that never clears.
    focus_address_bar_next_frame: bool,
}

impl Chrome {
    pub fn new(state: Rc<BrowserState>) -> Self {
        Self {
            state,
            focus_address_bar_next_frame: false,
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
            });
            ui.add_space(6.0);
        });

        let screen = ui.ctx().screen_rect();
        let content_rect = Rect::from_min_max(
            egui::pos2(screen.min.x, toolbar.response.rect.max.y),
            screen.max,
        );

        Output {
            content_rect,
            actions,
        }
    }

    fn nav_buttons(&self, ui: &mut Ui, actions: &mut Vec<Action>) {
        // Buttons are disabled rather than hidden when unavailable: a button
        // that appears and disappears makes the toolbar jump around, and
        // disabled buttons still explain themselves via a tooltip.
        if ui
            .add_enabled(self.state.can_go_back.get(), Button::new("\u{2190}"))
            .on_hover_text("Back")
            .clicked()
        {
            actions.push(Action::Back);
        }

        if ui
            .add_enabled(self.state.can_go_forward.get(), Button::new("\u{2192}"))
            .on_hover_text("Forward")
            .clicked()
        {
            actions.push(Action::Forward);
        }

        if ui
            .add_enabled(true, Button::new("\u{21BB}"))
            .on_hover_text("Reload")
            .clicked()
        {
            actions.push(Action::Reload);
        }

        ui.add_space(8.0);
    }

    fn address_bar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let mut text = self.state.address_text.borrow_mut();

        let response = ui.add(
            TextEdit::singleline(&mut *text)
                .id_source("address_bar")
                .hint_text("Search or enter address")
                .desired_width(f32::INFINITY)
                .vertical_align(Align::Center),
        );

        // Focus tracking has to be explicit: egui grants focus during the frame
        // in which it was requested, and we need to know on *subsequent* frames
        // to decide whether keystrokes belong to the page or to this box.
        let focused = response.has_focus();
        self.state.address_focused.set(focused);

        if self.focus_address_bar_next_frame {
            response.request_focus();
            self.focus_address_bar_next_frame = false;
        }

        // Enter submits. Checked globally rather than on the TextEdit response
        // because a keypress that submits usually also mutates the text, and
        // relying on `changed()` alone misses Enter on an unchanged box.
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if focused && enter && !text.trim().is_empty() {
            actions.push(Action::Navigate(text.trim().to_owned()));
            response.request_focus(); // keep focus so the next keystroke replaces
        }
    }
}