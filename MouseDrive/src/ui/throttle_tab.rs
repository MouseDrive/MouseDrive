#![deny(unsafe_code)]

use eframe::egui::{CollapsingHeader, RichText, Ui};
use egui_plot::{Line, Plot, PlotPoints, Points};
use mousedrive::config::Tuning;
use mousedrive::logic::throttle_cut_factor;
use mousedrive::preview::{cut_curve, cut_summary};

use super::widgets::{Cx, Fmt, Param, check_row, grid, param_row, param_row_i32};
use crate::curve_editor::{CurveDisplay, curve_editor};
use crate::lang::fill;

const CUT_FMT: Fmt = Fmt::Pct {
    scale: mousedrive::logic::THROTTLE_CUT_REFERENCE,
    decimals: 1,
};
const PLOT_HEIGHT: f32 = 120.0;
const PLOT_STEPS: usize = 80;

pub fn show(ui: &mut Ui, cx: &Cx, t: &mut Tuning, steer_abs: f64) {
    let (s, d) = (cx.s, Tuning::default());
    grid(ui, "thr_cut_toggle", |ui| {
        let label = (s.cut_enabled, s.tip_cut_enabled);
        check_row(
            ui,
            cx,
            label,
            &mut t.throttle_cut_enabled,
            d.throttle_cut_enabled,
        );
    });
    ui.label(RichText::new(summary(cx, t)).weak());
    if t.throttle_cut_enabled {
        cut_plot(ui, cx, t, steer_abs);
    }
    grid(ui, "thr_ramp", |ui| ramp_rows(ui, cx, t, &d));
    let enabled = t.throttle_cut_enabled;
    ui.add_enabled_ui(enabled, |ui| {
        grid(ui, "thr_cut", |ui| cut_rows(ui, cx, t, &d))
    });
    CollapsingHeader::new(s.advanced)
        .id_salt("thr_advanced")
        .show(ui, |ui| {
            curve_editor(
                ui,
                s.curve_rise,
                &mut t.throttle_rise_curve,
                CurveDisplay::Normal,
                s,
                cx.lang,
            );
            ui.add_space(8.0);
            curve_editor(
                ui,
                s.curve_fall,
                &mut t.throttle_fall_curve,
                CurveDisplay::MirrorX,
                s,
                cx.lang,
            );
        });
    ui.add_space(8.0);
    if ui.button(s.btn_tab_defaults).clicked() {
        *t = throttle_defaults(t);
    }
}

pub fn summary(cx: &Cx, t: &Tuning) -> String {
    let Some(c) = cut_summary(t) else {
        return cx.s.cut_off.to_string();
    };
    let pct = |v: f64| cx.lang.pct(v, 1);
    fill(
        cx.s.cut_summary,
        &[
            ("start", &pct(c.start)),
            ("end", &pct(c.floor_at)),
            ("floor", &cx.lang.pct(c.floor, 0)),
        ],
    )
}

fn plot_x_max(t: &Tuning) -> f64 {
    cut_summary(t).map_or(0.35, |c| (c.floor_at * 1.5).clamp(0.1, 1.0))
}

fn cut_plot(ui: &mut Ui, cx: &Cx, t: &Tuning, steer_abs: f64) {
    let x_max = plot_x_max(t);
    let points = cut_curve(t, x_max, PLOT_STEPS);
    let now = [steer_abs.min(x_max), throttle_cut_factor(t, steer_abs)];
    let lang = cx.lang;
    Plot::new("throttle_cut_plot")
        .height(PLOT_HEIGHT)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .allow_double_click_reset(false)
        .include_x(0.0)
        .include_x(x_max)
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label(cx.s.axis_steering_pct)
        .y_axis_label(cx.s.axis_max_throttle)
        .x_axis_formatter(move |m, _| lang.pct(m.value, 0))
        .y_axis_formatter(move |m, _| lang.pct(m.value, 0))
        .show(ui, |plot| {
            plot.line(
                Line::new(PlotPoints::new(points))
                    .color(cx.pal.throttle)
                    .width(2.0_f32),
            );
            plot.points(
                Points::new(vec![now])
                    .radius(4.0_f32)
                    .color(cx.pal.accent)
                    .name(cx.s.now_marker),
            );
        });
}

fn ramp_rows(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    let ramp = Param::new(
        s.ramp,
        s.tip_ramp,
        f64::from(d.throttle_ramp_ms),
        10.0..=1000.0,
        Fmt::Num(0),
    );
    param_row_i32(ui, cx, &ramp, &mut t.throttle_ramp_ms);
    let drop = Param::new(
        s.drop,
        s.tip_drop,
        f64::from(d.throttle_drop_ms),
        5.0..=200.0,
        Fmt::Num(0),
    );
    param_row_i32(ui, cx, &drop, &mut t.throttle_drop_ms);
}

fn cut_rows(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    let start = Param::new(
        s.cut_start,
        s.tip_cut_start,
        d.throttle_cut_start,
        0.0..=0.5,
        CUT_FMT,
    );
    param_row(ui, cx, &start, &mut t.throttle_cut_start);
    let end = Param::new(
        s.cut_end,
        s.tip_cut_end,
        d.throttle_cut_max,
        0.3..=1.0,
        CUT_FMT,
    );
    param_row(ui, cx, &end, &mut t.throttle_cut_max);
    let floor = Param::new(
        s.cut_floor,
        s.tip_cut_floor,
        d.throttle_min_cut_at_full,
        0.3..=0.95,
        Fmt::PCT0,
    );
    param_row(ui, cx, &floor, &mut t.throttle_min_cut_at_full);
    let exp = Param::new(
        s.cut_exp,
        s.tip_cut_exp,
        d.throttle_curve_exp,
        0.5..=4.0,
        Fmt::Num(2),
    );
    param_row(ui, cx, &exp, &mut t.throttle_curve_exp);
}

pub fn throttle_defaults(t: &Tuning) -> Tuning {
    let d = Tuning::default();
    Tuning {
        throttle_cut_enabled: d.throttle_cut_enabled,
        throttle_curve_exp: d.throttle_curve_exp,
        throttle_min_cut_at_full: d.throttle_min_cut_at_full,
        throttle_cut_start: d.throttle_cut_start,
        throttle_cut_max: d.throttle_cut_max,
        throttle_ramp_ms: d.throttle_ramp_ms,
        throttle_drop_ms: d.throttle_drop_ms,
        throttle_rise_curve: d.throttle_rise_curve,
        throttle_fall_curve: d.throttle_fall_curve,
        ..t.clone()
    }
}
