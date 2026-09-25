#![deny(unsafe_code)]

use eframe::egui::{Color32, Context, Visuals};
use mousedrive::status::{AppStatus, status_rgb};

pub const ACCENT: Color32 = Color32::from_rgb(55, 138, 221);

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
                Color32::from_rgb(0x00, 0x72, 0xB2),
                Color32::from_rgb(0xE6, 0x9F, 0x00),
            )
        } else {
            (
                Color32::from_rgb(99, 153, 34),
                Color32::from_rgb(226, 75, 74),
            )
        };
        Self {
            accent: ACCENT,
            throttle,
            brake,
            ok: Color32::from_rgb(29, 158, 117),
            update: Color32::from_rgb(35, 134, 54),
        }
    }
}

pub fn status_color(status: AppStatus) -> Color32 {
    let [r, g, b] = status_rgb(status);
    Color32::from_rgb(r, g, b)
}

pub fn apply(ctx: &Context, zoom: f64) {
    let mut visuals = Visuals::dark();
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke.color = Color32::WHITE;
    ctx.set_visuals(visuals);
    ctx.set_zoom_factor(zoom as f32);
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
    if contrast(bg, Color32::BLACK) >= contrast(bg, Color32::WHITE) {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}
