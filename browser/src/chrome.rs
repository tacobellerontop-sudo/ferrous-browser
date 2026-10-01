//! Browser chrome: a single floating panel holding the tabs, the window
//! controls, the navigation buttons and the address/search bar.
//!
//! # Layout
//!
//! The page is a rounded **card** inset from the window edge, with the Mica
//! backdrop showing around it (see `backdrop` and `page_mask`). The chrome does
//! not take space from the page. It is an overlay that slides down over the top
//! of the card when wanted and away again when not:
//!
//! - it appears when the pointer touches the top edge of the window, when the
//!   address bar is asked for (Ctrl+L, Ctrl+T, a new tab), and while the
//!   address bar has focus;
//! - it stays while the pointer is on it, and hides a moment after it leaves.
//!
//! Keeping the page's size fixed is deliberate: resizing the web view every
//! time the toolbar came and went would re-lay out the page each time and make
//! the content jump.
//!
//! Pure UI. It reads the tab model, emits [`Action`]s and window commands, and
//! reports where the page goes. It never touches Servo.
//!
//! The tab set is taken as `&mut Tabs` for the whole frame rather than read
//! widget by widget. `BrowserState` holds the model behind a `RefCell`, and a
//! `RefCell` allows only one borrow at a time — a `tabs()` call here and an
//! `active()` call in the next widget would panic at runtime, not at compile time.
//!
//! Tabs and the title row are laid out by hand, with explicit rectangles and
//! [`Ui::interact`], rather than with `ui.horizontal`. Two earlier bugs (P43,
//! P44 in the devlog) came from egui's automatic spacing putting widgets a few
//! points away from where the geometry said, and the title-bar drag hit test
//! depends on knowing exactly where the tabs are.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use egui::epaint::{RectShape, Shadow};
use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{
    Align, Color32, CornerRadius, FontId, Galley, Id, LayerId, Layout, Margin, Order, Pos2, Rect,
    Sense, Shape, Stroke, StrokeKind, TextBuffer, TextEdit, Ui, UiBuilder, pos2,
    text_selection::CCursorRange, vec2,
};

use crate::browser_state::BrowserState;
use crate::homepage;
use crate::gpu_fx::{self, Corners, EdgeGlow, Glass, GlowParams, PixelRect};
use crate::icons;
use crate::tab::{Favicon, Tab, TabId, Tabs};
use crate::zoom;
use crate::theme;
use crate::titlebar::{self, CONTROL_WIDTH, TITLE_BAR_HEIGHT, WindowCommand};

/// Stable id for the address field. Explicit rather than auto-generated so that
/// [`TextEdit::load_state`] can find the widget's state and select its text;
/// an auto id would drift as the surrounding layout changes.
const ADDRESS_BAR_ID: fn() -> Id = || Id::new("ferrous.address_bar");

/// Space between the window edge and the page card, where the backdrop shows.
/// Also the window's resize grip, see `titlebar::RESIZE_BORDER`.
pub const GAP: f32 = 8.0;
/// Corner radius of the page card and the chrome panel.
const RADIUS: f32 = 10.0;

/// How far into the page, below the window's top edge, the pointer reveals the
/// chrome. Small, so reading near the top of a page does not keep summoning it.
const REVEAL_DEPTH: f32 = 6.0;
/// How long the chrome lingers after the pointer leaves it, in seconds. Long
/// enough that brushing past the edge does not make it flicker.
const HIDE_DELAY: f64 = 0.4;
/// The panel's motion: a damped spring rather than a fixed-duration curve.
/// Stiffness sets the speed (about 0.25s to settle); a damping ratio below 1
/// lets it overshoot by a few pixels and settle, which reads as physical
/// rather than mechanical. See [`Spring`].
const SPRING_STIFFNESS: f32 = 420.0;
const SPRING_DAMPING_RATIO: f32 = 0.72;
/// How quickly the loading glow fades in and out, in seconds.
const GLOW_FADE_TIME: f32 = 0.35;
/// Side of a tab's site icon, and the gap after it.
const FAVICON: f32 = 16.0;
const FAVICON_GAP: f32 = 7.0;
/// How long the zoom bubble stays after the zoom changes, and its fade-out.
const ZOOM_BUBBLE_SECS: f64 = 1.2;
const ZOOM_BUBBLE_FADE: f64 = 0.3;
/// Room at the right of the address pill for the zoom badge, when shown.
const ZOOM_BADGE_WIDTH: f32 = 50.0;
/// The blocker's shield: icon only, or icon plus a count.
const SHIELD_WIDTH: f32 = 28.0;
const SHIELD_WITH_COUNT_WIDTH: f32 = 50.0;

/// Toolbar row, below the title row.
const TOOLBAR_HEIGHT: f32 = 44.0;
const PANEL_HEIGHT: f32 = TITLE_BAR_HEIGHT + TOOLBAR_HEIGHT;
const NAV_BUTTON: f32 = 30.0;
const ADDRESS_HEIGHT: f32 = 30.0;
/// Room at the left of the address pill for the lock/search icon.
const ADDRESS_ICON_SLOT: f32 = 32.0;

/// Tab strip metrics.
///
/// Tabs shrink to fit, between these bounds. The concern that originally kept
/// them fixed-width — the strip jumping under the pointer as tabs close — is
/// handled by [`Chrome::frozen_tab_width`], the same way Chrome does it.
const TAB_MAX_WIDTH: f32 = 220.0;
const TAB_MIN_WIDTH: f32 = 72.0;
const TAB_HEIGHT: f32 = 28.0;
/// Horizontal space between tabs: pills need air between them to read as pills.
const TAB_GAP: f32 = 4.0;
const TAB_STRIP_LEFT: f32 = 8.0;
const TAB_CLOSE: f32 = 18.0;
const NEW_TAB_BUTTON: f32 = 26.0;
/// Empty title row always left free for dragging the window, however many tabs
/// are open. Without it a full strip leaves no way to move the window.
const MIN_DRAG_WIDTH: f32 = 48.0;

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
    /// Back to 100%, from the zoom badge in the address bar.
    ResetZoom,
    /// Turn ad blocking off for the current site, or back on.
    ToggleBlocking,
}

/// What the chrome produced this frame.
pub struct Output {
    /// The page card, in egui points: where the web view lives.
    pub content_rect: Rect,
    /// Corner radius of the card, in egui points; 0 when maximised.
    pub content_radius: f32,
    pub actions: Vec<Action>,
    /// Minimize/maximize/close requests from the title-bar controls.
    pub window_commands: Vec<WindowCommand>,
    /// The title row, while the chrome is fully shown; [`Rect::NOTHING`]
    /// otherwise, so a press at the top of the page never drags the window.
    /// See [`titlebar::is_in_drag_area`].
    pub title_row: Rect,
    /// Where the tabs and the new-tab button are, likewise for the drag test.
    pub tab_strip: Rect,
    /// The part of the window the chrome owns for pointer input: the reveal
    /// band at the top and, while shown, the panel. Pointer moves anywhere else
    /// over the page can skip egui entirely.
    pub pointer_zone: Rect,
}

pub struct Chrome {
    state: Rc<BrowserState>,
    /// Set until the address bar exists to take it, so a focus request survives
    /// the frame where the hidden chrome is only starting to slide in.
    focus_address_bar_next_frame: bool,
    /// Whether the address bar had focus last frame. Used to detect the
    /// focus-*gained* edge.
    address_focused_last_frame: bool,
    /// Tab width held constant after a tab is closed with the mouse, until the
    /// pointer leaves the title row. Otherwise every remaining tab would widen
    /// and the next tab's close button would slide out from under the pointer,
    /// which makes closing several tabs in a row needlessly fiddly.
    frozen_tab_width: Option<f32>,
    /// egui time at which a lingering chrome finally hides, once the pointer
    /// has left it. `None` while something is holding it open.
    hide_at: Option<f64>,
    /// Where the panel was last frame, for the "pointer is on it" test.
    last_panel: Rect,
    /// The panel's position: 0 hidden, 1 shown, briefly a little past either
    /// end as it settles.
    reveal: Spring,
    /// egui time of the previous frame, for stepping the spring.
    last_time: Option<f64>,
    /// GPU effects; see `gpu_fx`.
    glass: Glass,
    glow: EdgeGlow,
    /// One uploaded texture per tab icon, with the icon version it holds, so
    /// an icon is uploaded once rather than every frame.
    favicons: HashMap<TabId, (u64, egui::TextureHandle)>,
    /// A zoom level to announce, and when it started showing (egui time; set
    /// on the first frame that draws it).
    zoom_bubble: Option<(f32, Option<f64>)>,
}

impl Chrome {
    pub fn new(state: Rc<BrowserState>) -> Self {
        Self {
            state,
            focus_address_bar_next_frame: false,
            address_focused_last_frame: false,
            frozen_tab_width: None,
            hide_at: None,
            last_panel: Rect::NOTHING,
            reveal: Spring::default(),
            last_time: None,
            glass: Glass::default(),
            glow: EdgeGlow::default(),
            favicons: HashMap::new(),
            zoom_bubble: None,
        }
    }

    pub fn request_focus_address_bar(&mut self) {
        self.focus_address_bar_next_frame = true;
    }

    /// Announce a new zoom level. The chrome is usually hidden, so a zoom
    /// change made from the keyboard would otherwise have no visible effect
    /// beyond the page reflowing.
    pub fn show_zoom(&mut self, zoom: f32) {
        self.zoom_bubble = Some((zoom, None));
    }

    /// Draw the chrome and report where the page goes.
    ///
    /// `pointer` is the window-relative pointer position the frame loop tracks
    /// itself, not egui's: pointer moves over the page bypass egui (see
    /// `main.rs`), so egui's idea of the pointer can be stale there.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        tabs: &mut Tabs,
        maximized: bool,
        pointer: Option<Pos2>,
    ) -> Output {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let gap = if maximized { 0.0 } else { GAP };
        let radius = if maximized { 0.0 } else { RADIUS };
        let card = screen.shrink(gap);

        // Where the panel sits when fully shown: across the top of the card.
        let panel = Rect::from_min_size(card.min, vec2(card.width(), PANEL_HEIGHT));
        let reveal_zone = Rect::from_min_max(screen.min, pos2(screen.max.x, card.top() + REVEAL_DEPTH));

        // --- Should the chrome be showing? ------------------------------------
        let now = ctx.input(|i| i.time);
        let on_panel = pointer.is_some_and(|p| self.last_panel.expand(6.0).contains(p));
        let held = pointer.is_some_and(|p| reveal_zone.contains(p))
            || on_panel
            || self.focus_address_bar_next_frame
            || self.address_focused_last_frame;
        let shown_before = self.last_panel != Rect::NOTHING;
        let wanted = if held {
            self.hide_at = None;
            true
        } else if shown_before {
            let hide_at = *self.hide_at.get_or_insert(now + HIDE_DELAY);
            let remaining = hide_at - now;
            if remaining > 0.0 {
                ctx.request_repaint_after(Duration::from_secs_f64(remaining));
            }
            remaining > 0.0
        } else {
            false
        };
        let dt = self.last_time.replace(now).map_or(0.0, |last| (now - last) as f32);
        self.reveal.target = if wanted { 1.0 } else { 0.0 };
        if self.reveal.step(dt) {
            ctx.request_repaint();
        }
        // Position follows the spring, overshoot included; opacity cannot go
        // past fully opaque, and fades a little faster than the panel moves
        // so it is gone before it reaches the top edge on the way out.
        let t = self.reveal.position;
        let opacity = (t * 1.4).clamp(0.0, 1.0);

        // The page card's hairline, over the page, under the chrome.
        if radius > 0.0 {
            ctx.layer_painter(LayerId::new(Order::Middle, Id::new("ferrous.card")))
                .rect_stroke(card, radius, Stroke::new(1.0_f32, theme::OUTLINE), StrokeKind::Outside);
        }
        self.loading_glow(&ctx, tabs, card, radius);
        self.zoom_bubble(&ctx, card, opacity);
        // Forget textures for tabs that closed or lost their icon.
        self.favicons.retain(|id, _| {
            tabs.iter().any(|tab| tab.id == *id && tab.favicon.is_some())
        });

        let mut output = Output {
            content_rect: card,
            content_radius: radius,
            actions: Vec::new(),
            window_commands: Vec::new(),
            title_row: Rect::NOTHING,
            tab_strip: Rect::NOTHING,
            pointer_zone: reveal_zone,
        };

        if self.reveal.is_hidden() {
            self.last_panel = Rect::NOTHING;
            self.address_focused_last_frame = false;
            self.state.address_focused.set(false);
            return output;
        }

        // Slide up out of view as `t` falls, fading at the same time.
        // Whole points, so the hairlines stay crisp mid-slide.
        let shown = panel.translate(vec2(0.0, (-(1.0 - t) * (PANEL_HEIGHT + gap + 8.0)).round()));
        self.last_panel = shown;
        output.pointer_zone = reveal_zone.union(shown.expand(6.0));

        egui::Area::new(Id::new("ferrous.chrome"))
            .order(Order::Foreground)
            .fixed_pos(shown.min)
            .constrain(false)
            .show(&ctx, |ui| {
                ui.set_opacity(opacity);
                // Claims the whole panel for pointer input, so clicks on its
                // empty parts do not fall through to the page underneath.
                let _ = ui.allocate_exact_size(shown.size(), Sense::hover());

                let corners = if maximized {
                    CornerRadius { nw: 0, ne: 0, sw: RADIUS as u8, se: RADIUS as u8 }
                } else {
                    CornerRadius::same(RADIUS as u8)
                };
                let painter = ui.painter();
                painter.add(
                    Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(90) }
                        .as_shape(shown, corners),
                );
                // Frosted glass: the page under the panel, blurred on the GPU.
                // Drawn before the panel's own fill, which tints it.
                let glass = self.glass.clone();
                let ppp = ctx.pixels_per_point();
                let glass_corners = Corners {
                    top_left: f32::from(corners.nw) * ppp,
                    top_right: f32::from(corners.ne) * ppp,
                    bottom_right: f32::from(corners.se) * ppp,
                    bottom_left: f32::from(corners.sw) * ppp,
                };
                painter.add(egui::PaintCallback {
                    rect: shown,
                    callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                        glass.draw(&info, painter.gl(), glass_corners, opacity);
                    })),
                });
                painter.add(RectShape::new(
                    shown,
                    corners,
                    theme::PANEL,
                    Stroke::new(1.0_f32, theme::OUTLINE),
                    StrokeKind::Inside,
                ));
                // A faint highlight along the top edge, as light catching the
                // rim of a sheet of glass.
                painter.hline(
                    (shown.left() + f32::from(corners.nw) + 2.0)..=(shown.right() - f32::from(corners.ne) - 2.0),
                    shown.top() + 1.5,
                    Stroke::new(1.0_f32, theme::RIM_LIGHT),
                );

                let row = Rect::from_min_size(shown.min, vec2(shown.width(), TITLE_BAR_HEIGHT));
                output.tab_strip = self.tab_strip(ui, row, tabs, &mut output.actions);
                output.window_commands = titlebar::controls(ui, titlebar::controls_rect(row), maximized);
                // Dragging only once the panel has settled: a window drag
                // started on a moving target is surprising.
                if self.reveal.is_settled_open() {
                    output.title_row = row;
                }

                let toolbar = Rect::from_min_max(
                    pos2(shown.left() + 6.0, row.bottom()),
                    pos2(shown.right() - 8.0, shown.bottom() - 6.0),
                );
                ui.scope_builder(
                    UiBuilder::new().max_rect(toolbar).layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        self.nav_buttons(ui, tabs, &mut output.actions);
                        ui.add_space(6.0);
                        self.address_bar(ui, tabs, &mut output.actions);
                    },
                );
            });

        output
    }

    /// A small pill over the top of the page announcing the zoom level, shown
    /// for a moment after it changes. Skipped while the panel is mostly
    /// visible, since the address bar shows the level there.
    fn zoom_bubble(&mut self, ctx: &egui::Context, card: Rect, panel_opacity: f32) {
        let now = ctx.input(|i| i.time);
        let Some((zoom, since)) = &mut self.zoom_bubble else {
            return;
        };
        let zoom = *zoom;
        let age = now - *since.get_or_insert(now);
        if age >= ZOOM_BUBBLE_SECS {
            self.zoom_bubble = None;
            return;
        }
        let fade = ((ZOOM_BUBBLE_SECS - age) / ZOOM_BUBBLE_FADE).min(1.0) as f32;
        let alpha = fade * (1.0 - panel_opacity);
        ctx.request_repaint();
        if alpha <= 0.0 {
            return;
        }
        let text = format!("{}%", zoom::percent(zoom));
        let painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("ferrous.zoom")));
        let galley = painter.layout_no_wrap(text, FontId::proportional(14.0), theme::TEXT);
        let size = galley.size() + vec2(28.0, 14.0);
        let pill = Rect::from_center_size(pos2(card.center().x, card.top() + 28.0 + size.y / 2.0), size);
        painter.add(
            Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha((70.0 * alpha) as u8) }
                .as_shape(pill, size.y / 2.0),
        );
        painter.add(RectShape::new(
            pill,
            size.y / 2.0,
            Color32::from_rgb(0x1f, 0x20, 0x25).gamma_multiply(alpha),
            Stroke::new(1.0_f32, theme::OUTLINE.gamma_multiply(alpha)),
            StrokeKind::Inside,
        ));
        let pos = pill.center() - galley.size() / 2.0;
        painter.galley_with_override_text_color(pos, galley, theme::TEXT.gamma_multiply(alpha));
    }

    /// The blocker's shield: lit while blocking is on for `site`, with the
    /// number of requests blocked on this page; struck through while the site
    /// is allowed. Clicking toggles it.
    fn shield(&self, ui: &mut Ui, rect: Rect, site: &str, blocked: u32, actions: &mut Vec<Action>) {
        let allowed = self.state.blocker.is_allowed(site);
        let response = ui.interact(rect, Id::new("ferrous.shield"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(rect, rect.height() / 2.0, theme::HOVER);
        }
        let icon_center = pos2(rect.left() + SHIELD_WIDTH / 2.0, rect.center().y);
        let color = if allowed { theme::TEXT_FAINT } else { theme::ACCENT };
        icons::shield(ui.painter(), icon_center, color, !allowed);
        if blocked > 0 && !allowed {
            ui.painter().text(
                pos2(rect.left() + SHIELD_WIDTH - 2.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                if blocked > 99 { "99+".to_owned() } else { blocked.to_string() },
                FontId::proportional(11.5),
                theme::TEXT_WEAK,
            );
        }
        if response.clicked() {
            actions.push(Action::ToggleBlocking);
        }
        response.on_hover_text(if allowed {
            format!("Ad and tracker blocking is off for {site}.\nClick to turn it back on.")
        } else {
            let what = match blocked {
                0 => "Nothing blocked on this page yet.".to_owned(),
                1 => "Blocked 1 ad or tracker on this page.".to_owned(),
                n => format!("Blocked {n} ads and trackers on this page."),
            };
            format!("{what}\nClick to allow them on {site}.")
        });
    }

    /// The texture for `tab`'s icon, uploading it the first time a given icon
    /// version is seen.
    fn favicon_texture(&mut self, ctx: &egui::Context, tab: &Tab, icon: &Favicon) -> egui::TextureId {
        if let Some((version, texture)) = self.favicons.get(&tab.id)
            && *version == icon.version
        {
            return texture.id();
        }
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        );
        // Mipmapped: sites often serve 128-256px icons, which alias badly when
        // drawn at 16px without them.
        let options = egui::TextureOptions {
            mipmap_mode: Some(egui::TextureFilter::Linear),
            ..egui::TextureOptions::LINEAR
        };
        let texture = ctx.load_texture(format!("ferrous.favicon.{}", tab.id.get()), image, options);
        let id = texture.id();
        self.favicons.insert(tab.id, (icon.version, texture));
        id
    }

    /// While the active page loads, a soft light sweeps around the edge of the
    /// card, drawn by a shader (`gpu_fx::EdgeGlow`). It fades in and out rather
    /// than switching, so a fast load is a brief shimmer, not a flash.
    fn loading_glow(&self, ctx: &egui::Context, tabs: &Tabs, card: Rect, radius: f32) {
        let strength = ctx.animate_bool_with_time(
            Id::new("ferrous.loading.glow"),
            tabs.active().loading,
            GLOW_FADE_TIME,
        );
        if strength <= 0.0 {
            return;
        }
        let glow = self.glow.clone();
        let ppp = ctx.pixels_per_point();
        let time = (ctx.input(|i| i.time) % 1000.0) as f32;
        let area = card.expand(gpu_fx::GLOW_REACH);
        let reach_px = (gpu_fx::GLOW_REACH * ppp).round() as i32;
        ctx.layer_painter(LayerId::new(Order::Middle, Id::new("ferrous.loading")))
            .add(egui::PaintCallback {
                rect: area,
                callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
                    // The callback's viewport is the card plus the glow's reach
                    // on every side; the card itself is that, shrunk back.
                    let viewport = PixelRect::from_viewport(&info);
                    let card = PixelRect {
                        x: viewport.x + reach_px,
                        y: viewport.y + reach_px,
                        width: viewport.width - 2 * reach_px,
                        height: viewport.height - 2 * reach_px,
                    };
                    glow.draw(
                        painter.gl(),
                        &GlowParams {
                            card,
                            corners: Corners::uniform(radius * ppp),
                            time,
                            strength,
                            pixels_per_point: ppp,
                            color_a: theme::ACCENT.into(),
                            color_b: theme::ACCENT_ALT.into(),
                        },
                    );
                })),
            });
        // A shader animation: one quad per frame, so running it at the display
        // rate costs the GPU next to nothing.
        ctx.request_repaint();
    }

    /// The tabs and the new-tab button, laid out along the left of the title
    /// `row`. Returns the rectangle they occupy.
    fn tab_strip(&mut self, ui: &mut Ui, row: Rect, tabs: &Tabs, actions: &mut Vec<Action>) -> Rect {
        let top = row.center().y - TAB_HEIGHT / 2.0;
        let left = row.left() + TAB_STRIP_LEFT;
        let room = (row.width() - TAB_STRIP_LEFT - CONTROL_WIDTH - NEW_TAB_BUTTON - MIN_DRAG_WIDTH)
            .max(TAB_MIN_WIDTH);

        let fitted = (room / tabs.len() as f32).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);
        if !ui.rect_contains_pointer(row) {
            self.frozen_tab_width = None;
        }
        let width = self.frozen_tab_width.map_or(fitted, |frozen| frozen.min(fitted));

        // Anything past `room` is clipped rather than painted over the window
        // controls. Those tabs stay reachable with Ctrl+Tab; a scrolling strip is
        // a separate piece of work.
        let strip_right = left + (width * tabs.len() as f32).min(room);
        let painter = ui
            .painter()
            .with_clip_rect(Rect::from_min_max(pos2(left, row.top()), pos2(strip_right, row.bottom())));

        let active = tabs.active().id;
        for (i, tab) in tabs.iter().enumerate() {
            let slot = Rect::from_min_size(pos2(left + i as f32 * width, top), vec2(width, TAB_HEIGHT));
            // The pill leaves `TAB_GAP` between neighbours; the slot itself
            // stays contiguous so there is no dead gap to click into.
            let pill = slot.shrink2(vec2(TAB_GAP / 2.0, 0.0));
            let response = ui.interact(slot, Id::new(("ferrous.tab", tab.id)), Sense::click());
            let selected = tab.id == active;
            let hovered = response.hovered();

            if selected {
                painter.add(RectShape::new(
                    pill,
                    8.0,
                    theme::SELECTED,
                    Stroke::new(1.0_f32, theme::OUTLINE),
                    StrokeKind::Inside,
                ));
            } else if hovered {
                painter.rect_filled(pill, 8.0, theme::HOVER);
            }

            let center_y = pill.center().y;
            let mut text_left = pill.left() + 10.0;
            // The icon slot: a spinner while loading, otherwise the site's
            // icon, or the Ferrous mark on the built-in homepage.
            let icon_rect = Rect::from_center_size(pos2(text_left + FAVICON / 2.0, center_y), vec2(FAVICON, FAVICON));
            let has_icon = if tab.loading {
                icons::spinner(&painter, icon_rect.center(), ui.input(|i| i.time), theme::ACCENT);
                // ~30 fps is smooth for a 12pt spinner and half the cost of
                // repainting at the display rate.
                ui.ctx().request_repaint_after(Duration::from_millis(33));
                true
            } else if homepage::is_homepage(&tab.url) {
                painter.rect_filled(icon_rect.shrink(2.0), 4.0, theme::BRAND);
                true
            } else if let Some(icon) = &tab.favicon {
                let texture = self.favicon_texture(ui.ctx(), tab, icon);
                let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                painter.image(texture, icon_rect, uv, Color32::WHITE);
                true
            } else {
                false
            };
            if has_icon {
                text_left += FAVICON + FAVICON_GAP;
            }

            // The close button is always on the active tab, on the hovered tab,
            // and on every tab when there is room for it. On narrow tabs it
            // would otherwise leave no room for the title.
            let show_close = selected || hovered || width >= 120.0;
            let close_rect = Rect::from_center_size(
                pos2(pill.right() - 6.0 - TAB_CLOSE / 2.0, center_y),
                vec2(TAB_CLOSE, TAB_CLOSE),
            );
            let text_right = if show_close { close_rect.left() - 2.0 } else { pill.right() - 8.0 };

            // One line, clipped to the tab. `layout` would *wrap* a long title
            // onto three lines and push the text out of the strip (P43).
            let color = if selected || hovered { theme::TEXT } else { theme::TEXT_WEAK };
            let galley = painter.layout_no_wrap(tab.label().to_owned(), FontId::proportional(12.5), color);
            let overflows = galley.size().x > text_right - text_left;
            let text_clip = Rect::from_x_y_ranges(text_left..=text_right, pill.y_range())
                .intersect(painter.clip_rect());
            // A title that does not fit fades out instead of stopping
            // mid-letter. The panel is translucent, so the fade is done by
            // fading the text itself rather than by painting a gradient of
            // some background colour over it.
            let text_painter = painter.with_clip_rect(text_clip);
            if overflows {
                fade_text(&text_painter, pos2(text_left, center_y - galley.size().y / 2.0), &galley, color, text_right);
            } else {
                text_painter.galley(pos2(text_left, center_y - galley.size().y / 2.0), galley, color);
            }

            // Middle-click closes, as in every other browser.
            if response.clicked() {
                actions.push(Action::SelectTab(tab.id));
            }
            if response.middle_clicked() {
                actions.push(Action::CloseTab(tab.id));
                self.frozen_tab_width = Some(width);
            }
            let tooltip = if tab.title.is_empty() {
                tab.url.clone()
            } else {
                format!("{}\n{}", tab.title, tab.url)
            };
            response.on_hover_text(tooltip);

            if show_close && close_rect.right() <= strip_right {
                // Registered after the tab itself, so it sits on top and wins
                // the hit test where the two overlap.
                let close = ui.interact(close_rect, Id::new(("ferrous.tab_close", tab.id)), Sense::click());
                if close.hovered() {
                    let fill = if close.is_pointer_button_down_on() { theme::PRESS } else { theme::HOVER };
                    painter.circle_filled(close_rect.center(), TAB_CLOSE / 2.0, fill);
                }
                let cross = if close.hovered() || selected { theme::TEXT } else { theme::TEXT_WEAK };
                icons::cross(&painter, close_rect.center(), cross);
                if close.clicked() {
                    actions.push(Action::CloseTab(tab.id));
                    self.frozen_tab_width = Some(width);
                }
                close.on_hover_text("Close tab (Ctrl+W)");
            }
        }

        let new_tab = Rect::from_center_size(
            pos2(strip_right + 4.0 + NEW_TAB_BUTTON / 2.0, row.center().y),
            vec2(NEW_TAB_BUTTON, NEW_TAB_BUTTON),
        );
        let response = ui.interact(new_tab, Id::new("ferrous.new_tab"), Sense::click());
        if response.hovered() {
            let fill = if response.is_pointer_button_down_on() { theme::PRESS } else { theme::HOVER };
            ui.painter().circle_filled(new_tab.center(), NEW_TAB_BUTTON / 2.0, fill);
        }
        let color = if response.hovered() { theme::TEXT } else { theme::TEXT_WEAK };
        icons::plus(ui.painter(), new_tab.center(), color);
        if response.clicked() {
            actions.push(Action::NewTab);
        }
        response.on_hover_text("New tab (Ctrl+T)");

        Rect::from_min_max(row.min, pos2(new_tab.right(), row.bottom()))
    }

    fn nav_buttons(&self, ui: &mut Ui, tabs: &Tabs, actions: &mut Vec<Action>) {
        // Disabled rather than hidden: a button that appears and disappears
        // makes the toolbar jump around, and a dimmed one still says "Back" in
        // its tooltip.
        let active = tabs.active();
        if icon_button(ui, active.can_go_back, "Back (Alt+Left)", icons::back) {
            actions.push(Action::Back);
        }
        if icon_button(ui, active.can_go_forward, "Forward (Alt+Right)", icons::forward) {
            actions.push(Action::Forward);
        }
        if icon_button(ui, true, "Reload (F5)", icons::reload) {
            actions.push(Action::Reload);
        }
    }

    fn address_bar(&mut self, ui: &mut Ui, tabs: &mut Tabs, actions: &mut Vec<Action>) {
        // Read the committed URL first: the borrow of `address_text` below lasts
        // for the whole widget and cannot overlap a second borrow of `tabs`.
        let committed = tabs.active().url.clone();

        let page_zoom = tabs.active().zoom;
        let zoomed = zoom::percent(page_zoom) != 100;
        let blocked = tabs.active().blocked;
        // The shield only means something on web pages, not the homepage.
        let site = url::Url::parse(&committed)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .and_then(|u| u.host_str().map(str::to_owned));

        let text = &mut tabs.active_mut().address_text;

        let (pill, pill_response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ADDRESS_HEIGHT), Sense::click());
        // The pill is painted *under* the text field but depends on its focus,
        // which is only known once the field has been added. Reserve the slot
        // now and fill it in afterwards.
        let background = ui.painter().add(Shape::Noop);

        let was_focused = ui.memory(|m| m.has_focus(ADDRESS_BAR_ID()));
        let font = FontId::proportional(13.5);
        let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _wrap_width: f32| -> Arc<Galley> {
            let job = address_layout(buffer.as_str(), font.clone(), !was_focused);
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };

        // Badges at the right end, outermost first: the blocker's shield, then
        // the zoom level while the page is not at 100%.
        let shield_width = match (&site, blocked) {
            (None, _) => 0.0,
            (Some(_), 0) => SHIELD_WIDTH,
            (Some(_), _) => SHIELD_WITH_COUNT_WIDTH,
        };
        let zoom_width = if zoomed { ZOOM_BADGE_WIDTH } else { 0.0 };
        let badges = shield_width + zoom_width;
        let right_inset = if badges > 0.0 { badges + 10.0 } else { 12.0 };
        let field = Rect::from_min_max(pill.min + vec2(ADDRESS_ICON_SLOT, 0.0), pill.max - vec2(right_inset, 0.0));
        let response = ui.put(
            field,
            TextEdit::singleline(text)
                .id(ADDRESS_BAR_ID())
                .hint_text("Search or enter address")
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .desired_width(field.width())
                .vertical_align(Align::Center)
                .layouter(&mut layouter),
        );

        // Clicks on the pill's padding or icon focus the field too; otherwise
        // the left 34 points of an obvious text box would be dead.
        if pill_response.clicked() {
            response.request_focus();
        }

        let focused = response.has_focus();
        let hovered = pill_response.hovered() || response.hovered();
        let (fill, stroke) = if focused {
            (theme::FIELD, Stroke::new(1.5_f32, theme::ACCENT))
        } else if hovered {
            (theme::FIELD_HOVER, Stroke::new(1.0_f32, theme::OUTLINE_STRONG))
        } else {
            (theme::FIELD, Stroke::new(1.0_f32, theme::OUTLINE))
        };
        let radius = CornerRadius::same((ADDRESS_HEIGHT / 2.0) as u8);
        ui.painter().set(
            background,
            RectShape::new(pill, radius, fill, stroke, StrokeKind::Inside),
        );

        if let Some(site) = &site {
            let shield = Rect::from_min_size(
                pos2(pill.right() - shield_width - 4.0, pill.top() + 4.0),
                vec2(shield_width, pill.height() - 8.0),
            );
            self.shield(ui, shield, site, blocked, actions);
        }

        if zoomed {
            let badge = Rect::from_min_size(
                pos2(pill.right() - shield_width - ZOOM_BADGE_WIDTH - 6.0, pill.top() + 4.0),
                vec2(ZOOM_BADGE_WIDTH, pill.height() - 8.0),
            );
            let response = ui.interact(badge, Id::new("ferrous.zoom_badge"), Sense::click());
            let fill = if response.hovered() { theme::HOVER } else { theme::FIELD };
            ui.painter().add(RectShape::new(
                badge,
                badge.height() / 2.0,
                fill,
                Stroke::new(1.0_f32, theme::OUTLINE),
                StrokeKind::Inside,
            ));
            ui.painter().text(
                badge.center(),
                egui::Align2::CENTER_CENTER,
                format!("{}%", zoom::percent(page_zoom)),
                FontId::proportional(11.5),
                theme::TEXT_WEAK,
            );
            if response.clicked() {
                actions.push(Action::ResetZoom);
            }
            response.on_hover_text("Reset zoom (Ctrl+0)");
        }

        let icon_center = pos2(pill.left() + 18.0, pill.center().y);
        if focused || text.is_empty() {
            icons::search(ui.painter(), icon_center, theme::TEXT_WEAK);
        } else if committed.starts_with("https://") {
            icons::lock(ui.painter(), icon_center, theme::TEXT_WEAK);
        } else {
            icons::info(ui.painter(), icon_center, theme::TEXT_WEAK);
        }
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

/// An underdamped spring driving a value towards `target`.
///
/// Used for the panel instead of a fixed-length easing curve because it stays
/// continuous when the target flips mid-flight (the pointer brushing past and
/// leaving), and because the small overshoot reads as physical. It is stepped
/// with real elapsed time in fixed sub-steps, so it moves at the same speed at
/// 60 Hz, 120 Hz or a stuttering frame rate.
#[derive(Debug, Clone, Copy, Default)]
struct Spring {
    position: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    /// Sub-step length. Small enough to stay stable at this stiffness.
    const STEP: f32 = 1.0 / 480.0;
    /// Close enough to rest to snap and stop asking for frames.
    const REST: f32 = 0.0015;

    /// Advance by `dt` seconds. Returns true while still moving.
    fn step(&mut self, dt: f32) -> bool {
        // After a long idle gap, do not replay seconds of motion in one frame.
        let mut remaining = dt.clamp(0.0, 1.0 / 20.0);
        let damping = 2.0 * SPRING_DAMPING_RATIO * SPRING_STIFFNESS.sqrt();
        while remaining > 0.0 {
            let h = remaining.min(Self::STEP);
            let force = SPRING_STIFFNESS * (self.target - self.position) - damping * self.velocity;
            self.velocity += force * h;
            self.position += self.velocity * h;
            remaining -= h;
        }
        let at_rest = (self.target - self.position).abs() < Self::REST
            && self.velocity.abs() < Self::REST * 10.0;
        if at_rest {
            self.position = self.target;
            self.velocity = 0.0;
        }
        !at_rest
    }

    fn is_hidden(&self) -> bool {
        self.position <= 0.0 && self.target == 0.0
    }

    fn is_settled_open(&self) -> bool {
        self.position == 1.0 && self.velocity == 0.0
    }
}

/// A round toolbar button with a painted icon. Returns true when clicked.
///
/// A disabled button still takes hover, so its tooltip explains what it is.
fn icon_button(
    ui: &mut Ui,
    enabled: bool,
    tooltip: &str,
    paint: fn(&egui::Painter, Pos2, Color32),
) -> bool {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(vec2(NAV_BUTTON, NAV_BUTTON), sense);

    if enabled && response.hovered() {
        let fill = if response.is_pointer_button_down_on() { theme::PRESS } else { theme::HOVER };
        ui.painter().circle_filled(rect.center(), NAV_BUTTON / 2.0, fill);
    }
    let color = match (enabled, response.hovered()) {
        (false, _) => theme::TEXT_FAINT,
        (true, true) => theme::TEXT,
        (true, false) => theme::TEXT_WEAK,
    };
    paint(ui.painter(), rect.center(), color);

    // `on_hover_text` consumes the `Response`, so the click test comes first.
    let clicked = enabled && response.clicked();
    response.on_hover_text(tooltip);
    clicked
}

/// Paint a one-line `galley` whose end fades out towards `right`.
///
/// Drawn as the solid part plus a few narrow slices at falling opacity. A
/// gradient overlay would need to know the colour behind the text, and the
/// panel is translucent, so there is no single colour to fade into.
fn fade_text(painter: &egui::Painter, pos: Pos2, galley: &Arc<Galley>, color: Color32, right: f32) {
    const FADE: f32 = 24.0;
    const STEPS: usize = 8;
    let clip = painter.clip_rect();
    let solid_end = right - FADE;
    let slice = |from: f32, to: f32| Rect::from_x_y_ranges(from..=to, clip.y_range()).intersect(clip);
    painter
        .with_clip_rect(slice(clip.left(), solid_end))
        .galley(pos, galley.clone(), color);
    for step in 0..STEPS {
        let from = solid_end + FADE * step as f32 / STEPS as f32;
        let to = solid_end + FADE * (step + 1) as f32 / STEPS as f32;
        let alpha = 1.0 - (step as f32 + 0.5) / STEPS as f32;
        painter.with_clip_rect(slice(from, to)).galley_with_override_text_color(
            pos,
            galley.clone(),
            color.gamma_multiply(alpha),
        );
    }
}

/// Lay out the address text, with the host emphasised when `emphasise_host` is
/// set: `https://` and the path are dimmed, `en.wikipedia.org` is not. That is
/// what makes a lookalike such as `paypal.com.evil.example/...` stand out.
///
/// Only while the field is not being edited — mid-edit the text is whatever the
/// user is typing, and colouring half a hostname is just noise.
fn address_layout(text: &str, font: FontId, emphasise_host: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let format = |color| TextFormat { font_id: font.clone(), color, ..Default::default() };
    match host_range(text).filter(|_| emphasise_host) {
        Some((start, end)) => {
            job.append(&text[..start], 0.0, format(theme::TEXT_WEAK));
            job.append(&text[start..end], 0.0, format(theme::TEXT));
            job.append(&text[end..], 0.0, format(theme::TEXT_WEAK));
        }
        None => job.append(text, 0.0, format(theme::TEXT)),
    }
    job
}

/// Byte range of the host in something that looks like a URL, or `None` for
/// anything else (a search query, a half-typed address).
///
/// Deliberately a string scan rather than `url::Url::parse`: this runs during
/// text layout every frame, and all it needs is where to change colour. Any
/// userinfo (`user:pass@`) is left out of the emphasised part, since it is
/// exactly the trick a lookalike URL would use.
fn host_range(text: &str) -> Option<(usize, usize)> {
    let authority_start = text.find("://")? + 3;
    let rest = &text[authority_start..];
    let authority_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_len];
    let host_start = authority_start + authority.rfind('@').map_or(0, |at| at + 1);
    let host_end = authority_start + authority_len;
    (host_start < host_end).then_some((host_start, host_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(text: &str) -> Option<&str> {
        host_range(text).map(|(start, end)| &text[start..end])
    }

    #[test]
    fn host_is_found_in_ordinary_urls() {
        assert_eq!(host("https://servo.org/"), Some("servo.org"));
        assert_eq!(host("https://en.wikipedia.org/wiki/Servo"), Some("en.wikipedia.org"));
        assert_eq!(host("http://localhost:8000?q=1"), Some("localhost:8000"));
        assert_eq!(host("https://example.com#top"), Some("example.com"));
        assert_eq!(host("https://example.com"), Some("example.com"));
    }

    #[test]
    fn userinfo_is_not_part_of_the_host() {
        assert_eq!(host("https://paypal.com@evil.example/login"), Some("evil.example"));
    }

    #[test]
    fn non_urls_have_no_host() {
        assert_eq!(host("rust borrow checker"), None);
        assert_eq!(host("servo.org"), None);
        assert_eq!(host("https://"), None);
        assert_eq!(host(""), None);
    }

    #[test]
    fn non_ascii_urls_split_on_char_boundaries() {
        // Slicing at a byte offset inside a multi-byte character would panic.
        let text = "https://日本.example/パス";
        let (start, end) = host_range(text).unwrap();
        assert_eq!(&text[start..end], "日本.example");
        let job = address_layout(text, FontId::proportional(13.0), true);
        assert_eq!(job.text, text);
    }

    #[test]
    fn layout_keeps_the_text_intact() {
        for text in ["https://servo.org/a?b#c", "just a query", ""] {
            for emphasise in [true, false] {
                let job = address_layout(text, FontId::proportional(13.0), emphasise);
                assert_eq!(job.text, text);
            }
        }
    }

    fn run(spring: &mut Spring, seconds: f32) -> (f32, f32) {
        let (mut min, mut max) = (f32::MAX, f32::MIN);
        let mut t = 0.0;
        while t < seconds {
            spring.step(1.0 / 120.0);
            min = min.min(spring.position);
            max = max.max(spring.position);
            t += 1.0 / 120.0;
        }
        (min, max)
    }

    #[test]
    fn the_spring_opens_with_a_small_overshoot_and_settles() {
        let mut spring = Spring { target: 1.0, ..Spring::default() };
        let (_, max) = run(&mut spring, 1.0);
        assert!(max > 1.0 && max < 1.06, "a few percent of overshoot, got {max}");
        assert!(spring.is_settled_open(), "settled within a second: {spring:?}");
    }

    #[test]
    fn the_spring_is_frame_rate_independent() {
        let mut fast = Spring { target: 1.0, ..Spring::default() };
        let mut slow = Spring { target: 1.0, ..Spring::default() };
        for _ in 0..12 {
            fast.step(1.0 / 120.0);
        }
        for _ in 0..3 {
            slow.step(1.0 / 30.0);
        }
        assert!((fast.position - slow.position).abs() < 0.01, "{fast:?} vs {slow:?}");
    }

    #[test]
    fn reversing_mid_flight_is_continuous() {
        let mut spring = Spring { target: 1.0, ..Spring::default() };
        for _ in 0..10 {
            spring.step(1.0 / 120.0);
        }
        let before = spring.position;
        spring.target = 0.0;
        spring.step(1.0 / 120.0);
        assert!((spring.position - before).abs() < 0.05, "no jump when the target flips");
        run(&mut spring, 1.0);
        assert!(spring.is_hidden());
    }

    #[test]
    fn a_long_idle_gap_does_not_teleport() {
        let mut spring = Spring { target: 1.0, ..Spring::default() };
        spring.step(5.0);
        assert!(spring.position < 0.9, "one frame after a stall should not finish the slide");
    }
}
