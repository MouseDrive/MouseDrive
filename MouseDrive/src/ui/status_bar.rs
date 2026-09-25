#![deny(unsafe_code)]

use eframe::egui::{Align, Layout, Ui};
use mousedrive::config::Config;
use mousedrive::control::{SetupInfo, Snapshot};
use mousedrive::status::{AppStatus, Connection, Health};

use super::setup::owner_label;
use super::theme::status_color;
use super::widgets::{Cx, chip, key_text, pill};
use crate::lang::{Lang, Strings, fill};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenSetup,
    Reconnect,
    CancelBind,
    Start,
    Pause,
    EnableSink,
    Details,
}

pub fn action_for(status: AppStatus) -> Action {
    match status {
        AppStatus::SetupRequired | AppStatus::DeviceBusy => Action::OpenSetup,
        AppStatus::Lost => Action::Reconnect,
        AppStatus::Binding => Action::CancelBind,
        AppStatus::Paused => Action::Start,
        AppStatus::NotReading => Action::EnableSink,
        AppStatus::Degraded => Action::Details,
        AppStatus::Active => Action::Pause,
    }
}

fn action_label(action: Action, s: &Strings, key: &str) -> String {
    match action {
        Action::OpenSetup => s.act_open_setup.into(),
        Action::Reconnect => s.act_reconnect.into(),
        Action::CancelBind => s.act_cancel_bind.into(),
        Action::Start => fill(s.act_start, &[("key", key)]),
        Action::Pause => fill(s.act_pause, &[("key", key)]),
        Action::EnableSink => s.act_enable_sink.into(),
        Action::Details => s.act_details.into(),
    }
}

pub fn degraded_reasons(h: &Health, s: &Strings, lang: Lang) -> String {
    let reasons = [
        (h.write_failures > 0).then(|| s.reason_write_failures.to_string()),
        h.slow_loop()
            .then(|| fill(s.reason_slow_loop, &[("hz", &lang.num(h.loop_hz, 0))])),
        (h.clipped_ticks > 0).then(|| s.reason_clipped.to_string()),
        h.version_mismatch.then(|| s.reason_version.to_string()),
    ];
    reasons.into_iter().flatten().collect::<Vec<_>>().join(", ")
}

pub fn detail(snap: &Snapshot, setup: &SetupInfo, cfg: &Config, s: &Strings, lang: Lang) -> String {
    let key = key_text(s, cfg.capture_toggle_key);
    match snap.status {
        AppStatus::SetupRequired => s.det_setup_required.into(),
        AppStatus::DeviceBusy => {
            let owner = setup
                .report
                .owner
                .as_ref()
                .map_or_else(|| "?".to_string(), owner_label);
            let id = cfg.vjoy_device_id.to_string();
            fill(s.det_device_busy, &[("id", &id), ("owner", &owner)])
        }
        AppStatus::Lost => {
            let secs = snap.retry_in_ms.unwrap_or(0.0).max(0.0) / 1000.0;
            fill(s.det_lost, &[("secs", &lang.num(secs, 1))])
        }
        AppStatus::Binding => s.det_binding.into(),
        AppStatus::Paused => fill(s.det_paused, &[("key", &key)]),
        AppStatus::NotReading => s.det_not_reading.into(),
        AppStatus::Degraded => {
            let reasons = degraded_reasons(&snap.health, s, lang);
            fill(s.det_degraded, &[("reasons", &reasons)])
        }
        AppStatus::Active => s.det_active.into(),
    }
}

pub fn show(
    ui: &mut Ui,
    cx: &Cx,
    snap: &Snapshot,
    setup: &SetupInfo,
    cfg: &Config,
) -> Option<Action> {
    let status = snap.status;
    let action = action_for(status);
    let key = key_text(cx.s, cfg.capture_toggle_key);
    let mut clicked = None;
    ui.horizontal(|ui| {
        pill(ui, cx.s.status_labels[status.index()], status_color(status));
        if ui.button(action_label(action, cx.s, &key)).clicked() {
            clicked = Some(action);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            chips(ui, cx, snap, cfg);
        });
    });
    ui.label(detail(snap, setup, cfg, cx.s, cx.lang));
    clicked
}

fn chips(ui: &mut Ui, cx: &Cx, snap: &Snapshot, cfg: &Config) {
    if snap.mouse_hz > 0.0 {
        let hz = cx.lang.num(snap.mouse_hz, 0);
        chip(ui, &fill(cx.s.chip_mouse_hz, &[("hz", &hz)]), None);
    }
    let dot = match snap.connection {
        Connection::Connected => cx.pal.ok,
        _ => status_color(snap.status),
    };
    let id = cfg.vjoy_device_id.to_string();
    chip(ui, &fill(cx.s.chip_vjoy, &[("id", &id)]), Some(dot));
}
