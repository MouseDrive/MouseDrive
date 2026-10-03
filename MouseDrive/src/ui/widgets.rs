#![deny(unsafe_code)]

use std::ops::RangeInclusive;

use eframe::egui::{
    Color32, ComboBox, Frame, Grid, Margin, Rect, Response, RichText, Sense, Slider, Stroke,
    StrokeKind, Ui, WidgetInfo, WidgetType, pos2, vec2,
};
use mousedrive::config::Config;
use mousedrive::keys::{KeyLabel, key_label};

use super::theme::{self, Palette, text_on};
use crate::lang::{Lang, Strings, fill, parse_num};

#[derive(Clone, Copy)]
pub struct Cx {
    pub s: &'static Strings,
    pub lang: Lang,
    pub pal: Palette,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fmt {
    Num(usize),
    Pct { scale: f64, decimals: usize },
}

impl Fmt {
    pub const PCT0: Fmt = Fmt::Pct {
        scale: 1.0,
        decimals: 0,
    };
    pub const PCT1: Fmt = Fmt::Pct {
        scale: 1.0,
        decimals: 1,
    };

    pub fn text(self, lang: Lang, v: f64) -> String {
        match self {
            Fmt::Num(d) => lang.num(v, d),
            Fmt::Pct { scale, decimals } => lang.pct(v * scale, decimals),
        }
    }

    pub fn parse(self, text: &str) -> Option<f64> {
        let n = parse_num(text)?;
        Some(match self {
            Fmt::Num(_) => n,
            Fmt::Pct { scale, .. } => n / (100.0 * scale),
        })
    }

    fn step(self) -> f64 {
        let unit = |d: usize| 10f64.powi(-(d.min(6) as i32));
        match self {
            Fmt::Num(d) => unit(d),
            Fmt::Pct { scale, decimals } => unit(decimals) / (100.0 * scale),
        }
    }
}

pub struct Param<'a> {
    pub label: &'a str,
    pub tip: &'a str,
    pub default: f64,
    pub range: RangeInclusive<f64>,
    pub fmt: Fmt,
}

impl<'a> Param<'a> {
    pub fn new(
        label: &'a str,
        tip: &'a str,
        default: f64,
        range: RangeInclusive<f64>,
        fmt: Fmt,
    ) -> Self {
        Self {
            label,
            tip,
            default,
            range,
            fmt,
        }
    }
}

fn tooltip(cx: &Cx, p: &Param) -> String {
    let range = fill(
        cx.s.tip_range,
        &[
            ("min", &p.fmt.text(cx.lang, *p.range.start())),
            ("max", &p.fmt.text(cx.lang, *p.range.end())),
            ("default", &p.fmt.text(cx.lang, p.default)),
        ],
    );
    format!("{}\n\n{range}", p.tip)
}

pub fn param_row(ui: &mut Ui, cx: &Cx, p: &Param, value: &mut f64) -> bool {
    let tip = tooltip(cx, p);
    let label = ui.label(p.label).on_hover_text(&tip);
    let (fmt, lang) = (p.fmt, cx.lang);
    let slider = Slider::new(value, p.range.clone())
        .step_by(fmt.step())
        .custom_formatter(move |v, _| fmt.text(lang, v))
        .custom_parser(move |t| fmt.parse(t));
    let mut changed = ui
        .add(slider)
        .labelled_by(label.id)
        .on_hover_text(&tip)
        .changed();
    let differs = (*value - p.default).abs() > 1e-9;
    if reset_cell(ui, cx, differs, &p.fmt.text(cx.lang, p.default)) {
        *value = p.default;
        changed = true;
    }
    ui.end_row();
    changed
}

pub fn param_row_i32(ui: &mut Ui, cx: &Cx, p: &Param, value: &mut i32) -> bool {
    let mut v = f64::from(*value);
    let changed = param_row(ui, cx, p, &mut v);
    if changed {
        *value = v.round() as i32;
    }
    changed
}

pub fn check_row(
    ui: &mut Ui,
    cx: &Cx,
    (label, tip): (&str, &str),
    value: &mut bool,
    default: bool,
) -> bool {
    let l = ui.label(label).on_hover_text(tip);
    let mut changed = ui
        .checkbox(value, "")
        .labelled_by(l.id)
        .on_hover_text(tip)
        .changed();
    let default_text = if default { "✔" } else { "–" };
    if reset_cell(ui, cx, *value != default, default_text) {
        *value = default;
        changed = true;
    }
    ui.end_row();
    changed
}

pub fn combo_row(
    ui: &mut Ui,
    cx: &Cx,
    (label, tip): (&str, &str),
    value: &mut i32,
    names: &[&str],
    default: i32,
) -> bool {
    let name_of = |v: i32| usize::try_from(v).ok().and_then(|i| names.get(i)).copied();
    let l = ui.label(label).on_hover_text(tip);
    let before = *value;
    ComboBox::from_id_salt(label)
        .selected_text(name_of(*value).unwrap_or("?"))
        .show_ui(ui, |ui| {
            for (i, name) in (0..).zip(names) {
                ui.selectable_value(value, i, *name);
            }
        })
        .response
        .labelled_by(l.id)
        .on_hover_text(tip);
    let mut changed = *value != before;
    if reset_cell(ui, cx, *value != default, name_of(default).unwrap_or("?")) {
        *value = default;
        changed = true;
    }
    ui.end_row();
    changed
}

fn reset_cell(ui: &mut Ui, cx: &Cx, differs: bool, default_text: &str) -> bool {
    if !differs {
        ui.label("");
        return false;
    }
    let tip = fill(cx.s.tip_reset_default, &[("v", default_text)]);
    ui.horizontal(|ui| {
        ui.label(RichText::new("•").color(cx.pal.accent))
            .on_hover_text(cx.s.tip_changed);
        let button = ui.small_button("↺").on_hover_text(&tip);
        button.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &tip));
        button.clicked()
    })
    .inner
}

pub fn grid(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    ui.spacing_mut().slider_width = (ui.available_width() * 0.32).clamp(100.0, 200.0);
    Grid::new(id)
        .num_columns(3)
        .max_col_width((ui.available_width() * 0.44).clamp(120.0, 260.0))
        .spacing([10.0, 6.0])
        .show(ui, add);
}

pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong().size(15.0));
    ui.separator();
}

pub fn chip(ui: &mut Ui, text: &str, dot: Option<Color32>) -> Response {
    Frame::NONE
        .fill(theme::BACKGROUND)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(2.0)
        .inner_margin(Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(c) = dot {
                    let (r, p) = ui.allocate_painter(vec2(9.0, 9.0), Sense::hover());
                    p.rect_filled(r.rect.shrink(2.0), 0.0, c);
                }
                ui.label(
                    RichText::new(text)
                        .monospace()
                        .size(12.0)
                        .color(theme::TEXT),
                );
            });
        })
        .response
}

pub fn pill(ui: &mut Ui, text: &str, bg: Color32) -> Response {
    Frame::NONE
        .fill(bg)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(2.0)
        .inner_margin(Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .strong()
                    .monospace()
                    .size(12.0)
                    .color(text_on(bg)),
            );
        })
        .response
}

fn progress_info(label: &str, value: f64) -> WidgetInfo {
    let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, label);
    info.value = Some((value * 100.0).round());
    info
}

const BAR_MIN_WIDTH: f32 = 40.0;

fn segmented_track(p: &eframe::egui::Painter, rect: Rect, start: f64, end: f64, color: Color32) {
    p.rect_filled(rect, 0.0, theme::BACKGROUND);
    p.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0_f32, theme::BORDER),
        StrokeKind::Inside,
    );
    let r = rect.shrink(3.0);
    let count = ((r.width() / 10.0) as usize).clamp(4, 24);
    let gap = 2.0;
    let width = (r.width() - (count - 1) as f32 * gap) / count as f32;
    let a = r.left() + r.width() * start.clamp(0.0, 1.0) as f32;
    let b = r.left() + r.width() * end.clamp(0.0, 1.0) as f32;
    for i in 0..count {
        let left = r.left() + i as f32 * (width + gap);
        let cell = Rect::from_min_size(pos2(left, r.top()), vec2(width, r.height()));
        p.rect_filled(cell, 0.0, theme::ELEVATED);
        let lo = left.max(a);
        let hi = cell.right().min(b);
        if hi > lo {
            p.rect_filled(Rect::from_x_y_ranges(lo..=hi, r.y_range()), 0.0, color);
        }
    }
}

pub fn bar(ui: &mut Ui, label: &str, value: f64, marker: Option<f64>, color: Color32) -> Response {
    let (resp, p) = ui.allocate_painter(
        vec2(ui.available_width().max(BAR_MIN_WIDTH), 20.0),
        Sense::hover(),
    );
    let r = resp.rect;
    segmented_track(&p, r, 0.0, value, color);
    if let Some(m) = marker {
        let inner = r.shrink(3.0);
        let x = inner.left() + inner.width() * m.clamp(0.0, 1.0) as f32;
        p.line_segment(
            [pos2(x, r.top() - 2.0), pos2(x, r.bottom() + 2.0)],
            Stroke::new(2.0_f32, theme::TEXT),
        );
    }
    resp.widget_info(|| progress_info(label, value));
    resp
}

pub fn steering_bar(
    ui: &mut Ui,
    label: &str,
    (out, raw): (f64, f64),
    deadzone: f64,
    color: Color32,
) -> Response {
    let (resp, p) = ui.allocate_painter(
        vec2(ui.available_width().max(BAR_MIN_WIDTH), 20.0),
        Sense::hover(),
    );
    let r = resp.rect;
    let inner = r.shrink(3.0);
    let cx = inner.center().x;
    let half = inner.width() * 0.5;
    let x_at = |v: f64| cx + half * v.clamp(-1.0, 1.0) as f32;
    let end = (out.clamp(-1.0, 1.0) + 1.0) * 0.5;
    segmented_track(&p, r, end.min(0.5), end.max(0.5), color);
    if deadzone > 0.0 {
        let band = Rect::from_x_y_ranges(x_at(-deadzone)..=x_at(deadzone), inner.y_range());
        p.rect_stroke(
            band,
            0.0,
            Stroke::new(1.0_f32, theme::MUTED),
            StrokeKind::Inside,
        );
    }
    p.line_segment(
        [pos2(cx, r.top()), pos2(cx, r.bottom())],
        Stroke::new(1.0_f32, theme::TEXT),
    );
    let rx = x_at(raw);
    p.line_segment(
        [pos2(rx, r.top() - 2.0), pos2(rx, r.bottom() + 2.0)],
        Stroke::new(1.5_f32, theme::TEXT),
    );
    resp.widget_info(|| progress_info(label, out));
    resp
}

pub fn key_text(s: &Strings, vk: i32) -> String {
    match key_label(vk) {
        KeyLabel::Unbound => s.key_none.to_string(),
        KeyLabel::Mouse(n) => fill(s.mouse_button, &[("n", &n.to_string())]),
        KeyLabel::Key(name) => name,
    }
}

pub fn output_name(s: &Strings, cfg: &Config) -> String {
    fill(s.chip_vjoy, &[("id", &cfg.vjoy_device_id.to_string())])
}
