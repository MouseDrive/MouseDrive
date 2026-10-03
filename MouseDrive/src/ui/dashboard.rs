#![deny(unsafe_code)]

use eframe::egui::{Align, Color32, Frame, Layout, Margin, RichText, Stroke, Ui};
use mousedrive::config::Config;
use mousedrive::control::Snapshot;
use mousedrive::logic::throttle_cut_factor;
use mousedrive::output::{BUTTON_GEAR_DOWN, BUTTON_GEAR_UP};

use super::brake_tab::phase_index;
use super::theme;
use super::widgets::{Cx, bar, key_text, pill, steering_bar};
use crate::lang::{Lang, fill};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ResetSteering,
    OpenMonitor,
}

pub fn show(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config) -> Option<Action> {
    theme::card(ui, |ui| {
        ui.label(RichText::new(cx.s.live_title).size(18.0).strong());
        theme::hint(ui, cx.s.live_desc);
        ui.add_space(8.0);
        if ui.available_width() >= 560.0 {
            ui.columns(3, |columns| {
                for (i, column) in columns.iter_mut().enumerate() {
                    instrument(column, cx, snap, cfg, i);
                }
            });
        } else {
            for i in 0..3 {
                instrument(ui, cx, snap, cfg, i);
            }
        }
        ui.add_space(12.0);
        pills(ui, cx, snap, cfg);
        if snap.test_counter {
            ui.label(RichText::new(cx.s.test_counter_on).color(ui.visuals().warn_fg_color));
        }
        ui.add_space(6.0);
        actions(ui, cx)
    })
}

pub fn compact(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config) {
    ui.label(RichText::new(cx.s.live_title).strong());
    theme::hint(ui, cx.s.live_desc);
    for i in 0..3 {
        ui.add_space(10.0);
        theme::card(ui, |ui| gauge(ui, cx, snap, cfg, i));
    }
}

fn instrument(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config, axis: usize) {
    let width = ui.available_width();
    Frame::new()
        .fill(theme::BACKGROUND)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(2.0)
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width((width - 22.0).max(0.0));
            gauge(ui, cx, snap, cfg, axis);
        });
}

fn gauge(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config, axis: usize) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let f = &snap.frame;
    let t = &cfg.tuning;
    let cut = t
        .throttle_cut_enabled
        .then(|| throttle_cut_factor(t, f.steering.abs()));
    let floor = snap.brake_pressed.then_some(snap.brake_floor);
    let (name, value, color, tip) = match axis {
        0 => (
            cx.s.g_steering,
            f.steering,
            cx.pal.accent,
            cx.s.steer_bar_tip,
        ),
        1 => (
            cx.s.g_throttle,
            f.throttle,
            cx.pal.throttle,
            cx.s.throttle_gauge_tip,
        ),
        _ => (cx.s.g_brake, f.brake, cx.pal.brake, cx.s.brake_gauge_tip),
    };
    ui.label(RichText::new(name).size(14.0).strong().color(theme::MUTED))
        .on_hover_text(tip);
    ui.label(
        RichText::new(signed_pct(cx.lang, value))
            .monospace()
            .size(34.0)
            .strong()
            .color(theme::TEXT),
    );
    match axis {
        0 => {
            steering_bar(
                ui,
                cx.s.g_steering,
                (f.steering, snap.steering_raw),
                t.steering_deadzone,
                color,
            )
            .on_hover_text(tip);
            ui.horizontal(|ui| {
                ui.label(RichText::new(cx.s.steer_left).small().color(theme::MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(cx.s.steer_right).small().color(theme::MUTED));
                });
            });
        }
        1 => {
            bar(ui, name, value, cut, color).on_hover_text(tip);
        }
        _ => {
            bar(ui, name, value, floor, color).on_hover_text(tip);
        }
    }
}

fn signed_pct(lang: Lang, value: f64) -> String {
    let shown = lang.pct(value.abs(), 0);
    let nonzero = shown.chars().any(|c| c.is_ascii_digit() && c != '0');
    if value < 0.0 && nonzero {
        format!("−{shown}")
    } else {
        shown
    }
}

fn pills(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config) {
    let idle = ui.visuals().widgets.inactive.bg_fill;
    let color = |on: bool, active: Color32| if on { active } else { idle };
    ui.horizontal_wrapped(|ui| {
        pill(
            ui,
            cx.s.pill_lmb,
            color(snap.throttle_pressed, cx.pal.throttle),
        );
        pill(ui, cx.s.pill_rmb, color(snap.brake_pressed, cx.pal.brake));
        if cfg.gear_keys_enabled {
            let buttons = snap.frame.buttons;
            let up = fill(
                cx.s.pill_gear_up,
                &[("key", &key_text(cx.s, cfg.gear_up_key))],
            );
            let down = fill(
                cx.s.pill_gear_down,
                &[("key", &key_text(cx.s, cfg.gear_down_key))],
            );
            pill(ui, &up, color(buttons & BUTTON_GEAR_UP != 0, cx.pal.accent));
            pill(
                ui,
                &down,
                color(buttons & BUTTON_GEAR_DOWN != 0, cx.pal.accent),
            );
        }
        if snap.brake_pressed || snap.frame.brake > 0.0 {
            let phase = cx.s.phase_names[phase_index(snap.brake_phase)];
            ui.label(format!("{}: {phase}", cx.s.phase_prefix));
        }
    });
}

fn actions(ui: &mut Ui, cx: &Cx) -> Option<Action> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(cx.s.btn_reset_steering)
            .on_hover_text(cx.s.tip_reset_steering)
            .clicked()
        {
            action = Some(Action::ResetSteering);
        }
        if ui
            .button(cx.s.monitor)
            .on_hover_text(cx.s.tip_monitor)
            .clicked()
        {
            action = Some(Action::OpenMonitor);
        }
    });
    action
}
