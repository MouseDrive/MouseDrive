#![deny(unsafe_code)]

use eframe::egui::{CollapsingHeader, DragValue, RichText, Ui};
use mousedrive::config::{Config, Tuning};
use mousedrive::ergonomics::{cm_for_full_lock, counts_for_full_lock, sens_for_cm};
use mousedrive::logic::SteeringMode;

use super::widgets::{Cx, Fmt, Param, combo_row, grid, param_row};
use crate::lang::{fill, parse_num};

const SENS_RANGE: std::ops::RangeInclusive<f64> = 0.5..=10.0;
const CM_RANGE: std::ops::RangeInclusive<f64> = 1.0..=100.0;

pub fn show(ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
    let d = Tuning::default();
    let s = cx.s;
    grid(ui, "steer_sens", |ui| {
        let p = Param::new(s.sens, s.tip_sens, d.mouse_sens, SENS_RANGE, Fmt::Num(2));
        param_row(ui, cx, &p, &mut cfg.tuning.mouse_sens);
    });
    ergonomics(ui, cx, cfg);
    ui.add_space(6.0);
    let t = &mut cfg.tuning;
    grid(ui, "steer_mode", |ui| {
        combo_row(
            ui,
            cx,
            (s.mode, ""),
            &mut t.steering_mode,
            &s.mode_names,
            d.steering_mode,
        );
    });
    let mode = SteeringMode::from_i32(t.steering_mode);
    ui.label(RichText::new(s.mode_desc[mode.to_i32() as usize]).weak());
    grid(ui, "steer_mode_rows", |ui| mode_rows(ui, cx, t, &d, mode));
    grid(ui, "steer_shape", |ui| shape_rows(ui, cx, t, &d));
    CollapsingHeader::new(s.advanced)
        .id_salt("steer_advanced")
        .show(ui, |ui| advanced(ui, cx, cfg));
    ui.add_space(8.0);
    if ui.button(s.btn_tab_defaults).clicked() {
        cfg.tuning = steering_defaults(&cfg.tuning);
    }
}

fn ergonomics(ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
    let (s, lang) = (cx.s, cx.lang);
    let (scale, sat, dpi) = (
        cfg.mouse_dpi_scale,
        cfg.tuning.steering_saturation,
        cfg.mouse_dpi,
    );
    let Some(cm) = cm_for_full_lock(cfg.tuning.mouse_sens, scale, sat, dpi) else {
        if let Some(counts) = counts_for_full_lock(cfg.tuning.mouse_sens, scale, sat) {
            let text = fill(s.counts_per_lock, &[("counts", &lang.num(counts, 0))]);
            ui.label(RichText::new(text).weak());
        }
        return;
    };
    let text = fill(
        s.cm_per_lock,
        &[("cm", &lang.num(cm, 1)), ("dpi", &dpi.to_string())],
    );
    ui.label(RichText::new(text).weak());
    ui.horizontal(|ui| {
        let label = ui.label(s.cm_input).on_hover_text(s.tip_cm_input);
        let mut value = cm;
        let edited = ui
            .add(
                DragValue::new(&mut value)
                    .range(CM_RANGE)
                    .speed(0.1)
                    .custom_formatter(move |v, _| lang.num(v, 1))
                    .custom_parser(parse_num),
            )
            .labelled_by(label.id)
            .on_hover_text(s.tip_cm_input)
            .changed();
        if let Some(sens) = sens_for_cm(value, scale, sat, dpi).filter(|_| edited) {
            cfg.tuning.mouse_sens = sens.clamp(*SENS_RANGE.start(), *SENS_RANGE.end());
        }
    });
}

fn mode_rows(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning, mode: SteeringMode) {
    let s = cx.s;
    match mode {
        SteeringMode::Linear => {}
        SteeringMode::Expo => {
            let p = Param::new(s.expo, s.tip_expo, d.steering_expo, 0.5..=3.0, Fmt::Num(2));
            param_row(ui, cx, &p, &mut t.steering_expo);
        }
        SteeringMode::Filtered => {
            let p = Param::new(
                s.smoothing,
                s.tip_smoothing,
                d.steering_smoothing_ms,
                0.0..=200.0,
                Fmt::Num(1),
            );
            param_row(ui, cx, &p, &mut t.steering_smoothing_ms);
        }
        SteeringMode::SelfCenter => {
            let p = Param::new(
                s.spring,
                s.tip_spring,
                d.steering_spring_strength,
                0.0..=1.0,
                Fmt::Num(2),
            );
            param_row(ui, cx, &p, &mut t.steering_spring_strength);
        }
        SteeringMode::Adaptive => {
            let cutoff = Param::new(
                s.adaptive_cutoff,
                s.tip_adaptive_cutoff,
                d.steering_adaptive_min_cutoff_hz,
                0.5..=30.0,
                Fmt::Num(1),
            );
            param_row(ui, cx, &cutoff, &mut t.steering_adaptive_min_cutoff_hz);
            let beta = Param::new(
                s.adaptive_beta,
                s.tip_adaptive_beta,
                d.steering_adaptive_beta,
                0.0..=50.0,
                Fmt::Num(1),
            );
            param_row(ui, cx, &beta, &mut t.steering_adaptive_beta);
        }
    }
}

fn shape_rows(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    let dz = Param::new(
        s.deadzone,
        s.tip_deadzone,
        d.steering_deadzone,
        0.0..=0.5,
        Fmt::PCT1,
    );
    param_row(ui, cx, &dz, &mut t.steering_deadzone);
    let sat = Param::new(
        s.saturation,
        s.tip_saturation,
        d.steering_saturation,
        0.5..=1.0,
        Fmt::PCT0,
    );
    param_row(ui, cx, &sat, &mut t.steering_saturation);
}

fn advanced(ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
    let (s, d) = (cx.s, Config::default());
    ui.label(RichText::new(s.shared_note).weak());
    grid(ui, "steer_advanced_rows", |ui| {
        let rate = Param::new(
            s.max_rate,
            s.tip_max_rate,
            d.steering_max_rate_pct_s,
            0.0..=10000.0,
            Fmt::Num(0),
        );
        param_row(ui, cx, &rate, &mut cfg.steering_max_rate_pct_s);
        let scale = Param::new(
            s.dpi_scale,
            s.tip_dpi_scale,
            d.mouse_dpi_scale,
            0.5..=2.0,
            Fmt::Num(2),
        );
        param_row(ui, cx, &scale, &mut cfg.mouse_dpi_scale);
    });
}

pub fn steering_defaults(t: &Tuning) -> Tuning {
    let d = Tuning::default();
    Tuning {
        mouse_sens: d.mouse_sens,
        steering_mode: d.steering_mode,
        steering_expo: d.steering_expo,
        steering_smoothing_ms: d.steering_smoothing_ms,
        steering_adaptive_min_cutoff_hz: d.steering_adaptive_min_cutoff_hz,
        steering_adaptive_beta: d.steering_adaptive_beta,
        steering_deadzone: d.steering_deadzone,
        steering_saturation: d.steering_saturation,
        steering_spring_strength: d.steering_spring_strength,
        ..t.clone()
    }
}
