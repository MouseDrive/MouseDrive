#![deny(unsafe_code)]

use eframe::egui::{
    Button, Color32, Context, CornerRadius, FontId, Frame, Margin, Response, RichText, Stroke,
    TextStyle, Ui, Visuals, pos2, vec2,
};
use mousedrive::status::{AppStatus, status_rgb};

pub const ACCENT: Color32 = Color32::from_rgb(255, 40, 0);
pub const PRIMARY: Color32 = Color32::from_rgb(197, 30, 22);
pub const BACKGROUND: Color32 = Color32::from_rgb(12, 13, 13);
pub const SURFACE: Color32 = Color32::from_rgb(21, 22, 22);
pub const ELEVATED: Color32 = Color32::from_rgb(33, 33, 32);
pub const BORDER: Color32 = Color32::from_rgb(65, 62, 58);
pub const TEXT: Color32 = Color32::from_rgb(244, 237, 225);
pub const MUTED: Color32 = Color32::from_rgb(193, 185, 173);
pub const SELECTED: Color32 = Color32::from_rgb(102, 18, 14);
pub const WARN: Color32 = Color32::from_rgb(0xE6, 0x9F, 0x00);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub accent: Color32,
    pub throttle: Color32,
    pub brake: Color32,
    pub ok: Color32,
    pub update: Color32,
}

impl Palette {
    pub fn new(colorblind: bool) -> Self {
        let (throttle, brake) = if colorblind {
            (
                Color32::from_rgb(0x56, 0xB4, 0xE9),
                Color32::from_rgb(0xD5, 0x5E, 0x00),
            )
        } else {
            (TEXT, ACCENT)
        };
        Self {
            accent: ACCENT,
            throttle,
            brake,
            ok: TEXT,
            update: PRIMARY,
        }
    }
}

pub fn status_color(status: AppStatus) -> Color32 {
    let [r, g, b] = status_rgb(status);
    Color32::from_rgb(r, g, b)
}

pub fn apply(ctx: &Context, zoom: f64) {
    let mut visuals = Visuals::dark();
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = BACKGROUND;
    visuals.faint_bg_color = ELEVATED;
    visuals.override_text_color = Some(TEXT);
    visuals.window_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.bg_fill = ELEVATED;
    visuals.widgets.inactive.weak_bg_fill = ELEVATED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_fill = SELECTED;
    visuals.widgets.hovered.weak_bg_fill = SELECTED;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.active.bg_fill = SELECTED;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke.color = TEXT;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);
    visuals.widgets.open.corner_radius = CornerRadius::same(2);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(2);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.widgets.active.bg_fill = PRIMARY;
    visuals.widgets.active.weak_bg_fill = PRIMARY;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    visuals.window_corner_radius = CornerRadius::same(3);
    visuals.menu_corner_radius = CornerRadius::same(2);
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = eframe::egui::style::HandleShape::Rect { aspect_ratio: 0.38 };
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = ACCENT;
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(10.0, 10.0);
        style.spacing.button_padding = vec2(12.0, 8.0);
        style.spacing.interact_size = vec2(36.0, 36.0);
        style.spacing.slider_width = 180.0;
        style.spacing.slider_rail_height = 7.0;
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(25.0));
        style.animation_time = 0.12;
    });
    ctx.set_zoom_factor(zoom as f32);
}

pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width();
    let result = Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .corner_radius(2.0)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width((width - 34.0).max(0.0));
            add(ui)
        });
    let r = result.response.rect;
    ui.painter().line_segment(
        [
            pos2(r.left() + 1.0, r.top()),
            pos2(r.left() + 45.0, r.top()),
        ],
        Stroke::new(2.0_f32, ACCENT),
    );
    result.inner
}

pub fn nav_button(ui: &mut Ui, label: &str, selected: bool, width: f32) -> Response {
    let response = ui.add_sized(
        [width, 40.0],
        Button::new(RichText::new(label).strong()).selected(selected),
    );
    if selected {
        let r = response.rect;
        ui.painter().line_segment(
            [
                pos2(r.left() + 1.0, r.top() + 1.0),
                pos2(r.left() + 1.0, r.bottom() - 1.0),
            ],
            Stroke::new(3.0_f32, ACCENT),
        );
    }
    response
}

pub fn primary_button(ui: &mut Ui, label: impl Into<String>) -> Response {
    ui.add(
        Button::new(RichText::new(label.into()).strong().color(TEXT))
            .fill(PRIMARY)
            .stroke(Stroke::new(1.0_f32, ACCENT)),
    )
}

pub fn heading(ui: &mut Ui, title: &str) {
    ui.label(RichText::new(title).size(26.0).strong().color(TEXT));
}

pub fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED));
}

fn channel(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.039_28 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Color32) -> f64 {
    0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
}

pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub fn text_on(bg: Color32) -> Color32 {
    if contrast(bg, BACKGROUND) >= contrast(bg, TEXT) {
        BACKGROUND
    } else {
        TEXT
    }
}
