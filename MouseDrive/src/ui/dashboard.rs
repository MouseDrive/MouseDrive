#![deny(unsafe_code)]

use eframe::egui::{Color32, Grid, RichText, Ui};
use mousedrive::config::Config;
use mousedrive::control::Snapshot;
use mousedrive::logic::throttle_cut_factor;
use mousedrive::output::{BUTTON_GEAR_DOWN, BUTTON_GEAR_UP};

use super::brake_tab::phase_index;
use super::widgets::{Cx, bar, key_text, pill, steering_bar};
use crate::lang::fill;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ResetSteering,
    OpenBindHelper,
    OpenSetup,
    ToggleMonitor,
}

pub fn show(
    ui: &mut Ui,
    cx: &Cx,
    snap: &Snapshot,
    cfg: &Config,
    monitor_open: bool,
) -> Option<Action> {
    gauges(ui, cx, snap, cfg);
    ui.add_space(6.0);
    pills(ui, cx, snap, cfg);
    if snap.test_counter {
        ui.label(RichText::new(cx.s.test_counter_on).color(ui.visuals().warn_fg_color));
    }
    ui.add_space(6.0);
    actions(ui, cx, monitor_open)
}

fn gauges(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config) {
    let f = &snap.frame;
    let t = &cfg.tuning;
    let cut = t
        .throttle_cut_enabled
        .then(|| throttle_cut_factor(t, f.steering.abs()));
    let floor = snap.brake_pressed.then_some(snap.brake_floor);
    let pct = |v: f64| cx.lang.pct(v, 0);
    Grid::new("gauges")
        .num_columns(3)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label(cx.s.g_steering).on_hover_text(cx.s.steer_bar_tip);
            steering_bar(
                ui,
                cx.s.g_steering,
                (f.steering, snap.steering_raw),
                t.steering_deadzone,
                cx.pal.accent,
            )
            .on_hover_text(cx.s.steer_bar_tip);
            ui.label(pct(f.steering));
            ui.end_row();
            ui.label(cx.s.g_throttle)
                .on_hover_text(cx.s.throttle_gauge_tip);
            bar(ui, cx.s.g_throttle, f.throttle, cut, cx.pal.throttle)
                .on_hover_text(cx.s.throttle_gauge_tip);
            ui.label(pct(f.throttle));
            ui.end_row();
            ui.label(cx.s.g_brake).on_hover_text(cx.s.brake_gauge_tip);
            bar(ui, cx.s.g_brake, f.brake, floor, cx.pal.brake).on_hover_text(cx.s.brake_gauge_tip);
            ui.label(pct(f.brake));
            ui.end_row();
        });
    ui.horizontal(|ui| {
        ui.label(RichText::new(cx.s.steer_left).weak());
        ui.label(RichText::new("⬅ ➡").weak());
        ui.label(RichText::new(cx.s.steer_right).weak());
    });
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
        let phase = cx.s.phase_names[phase_index(snap.brake_phase)];
        ui.label(format!("{}: {phase}", cx.s.phase_prefix));
    });
}

fn actions(ui: &mut Ui, cx: &Cx, monitor_open: bool) -> Option<Action> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(cx.s.btn_reset_steering)
            .on_hover_text(cx.s.tip_reset_steering)
            .clicked()
        {
            action = Some(Action::ResetSteering);
        }
        if ui.button(cx.s.btn_bind_helper).clicked() {
            action = Some(Action::OpenBindHelper);
        }
        if ui.button(cx.s.btn_setup).clicked() {
            action = Some(Action::OpenSetup);
        }
        if ui
            .selectable_label(monitor_open, cx.s.monitor)
            .on_hover_text(cx.s.tip_monitor)
            .clicked()
        {
            action = Some(Action::ToggleMonitor);
        }
    });
    action
}
