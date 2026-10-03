#![deny(unsafe_code)]

use eframe::egui::{RichText, Ui};
use mousedrive::config::Config;
use mousedrive::control::{SetupInfo, Snapshot};
use mousedrive::input::DeviceFilter;
use mousedrive::status::{AppStatus, Connection, Health};

use super::logo;
use super::setup::owner_label;
use super::theme::{self, status_color};
use super::widgets::{Cx, chip, key_text, output_name, pill};
use crate::lang::{Lang, Strings, fill};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    OpenSetup,
    Reconnect,
    CancelBind,
    Start,
    Pause,
    EnableSink,
    UseAllMice,
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

pub fn action_for_snapshot(snap: &Snapshot) -> Action {
    if mouse_missing(snap) {
        Action::UseAllMice
    } else {
        action_for(snap.status)
    }
}

fn mouse_missing(snap: &Snapshot) -> bool {
    snap.status == AppStatus::NotReading && snap.device_filter == DeviceFilter::SelectedMissing
}

fn action_label(action: Action, s: &Strings, key: &str) -> String {
    match action {
        Action::OpenSetup => s.act_open_setup.into(),
        Action::Reconnect => s.act_reconnect.into(),
        Action::CancelBind => s.act_cancel_bind.into(),
        Action::Start => fill(s.act_start, &[("key", key)]),
        Action::Pause => fill(s.act_pause, &[("key", key)]),
        Action::EnableSink => s.act_enable_sink.into(),
        Action::UseAllMice => s.act_use_all_mice.into(),
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
        AppStatus::NotReading if mouse_missing(snap) => s.det_mouse_missing.into(),
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
    let action = action_for_snapshot(snap);
    let key = key_text(cx.s, cfg.capture_toggle_key);
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        logo::lockup(ui, theme::SURFACE);
        ui.label(
            RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                .small()
                .color(theme::MUTED),
        );
        ui.add_space(10.0);
        pill(ui, cx.s.status_labels[status.index()], status_color(status));
        let label = action_label(action, cx.s, &key);
        let response = if action == Action::Start
            || action == Action::OpenSetup
            || action == Action::Reconnect
        {
            theme::primary_button(ui, label)
        } else {
            ui.button(RichText::new(label).strong())
        };
        if response.clicked() {
            clicked = Some(action);
        }
        chips(ui, cx, snap, cfg);
    });
    theme::hint(ui, &detail(snap, setup, cfg, cx.s, cx.lang));
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
    chip(ui, &output_name(cx.s, cfg), Some(dot));
}
