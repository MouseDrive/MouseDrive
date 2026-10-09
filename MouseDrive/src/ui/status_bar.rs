#![deny(unsafe_code)]

use eframe::egui::{RichText, Ui};
use mousedrive::config::Config;
use mousedrive::control::{SetupInfo, Snapshot};
use mousedrive::input::{self, DeviceFilter, MovedMouse};
use mousedrive::status::{AppStatus, Connection, Health, SessionState};

use super::logo;
use super::setup::owner_label;
use super::theme::{self, status_color};
use super::widgets::{Cx, chip, key_text, output_name, pill};
use crate::lang::{Lang, Strings, fill};

const STALL_INTERVALS: f64 = 5.0;
const STALL_MIN_MS: f64 = 250.0;
const STALL_STARTUP_MS: f64 = 3000.0;

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
    SaveMovedMouse,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputHints {
    pub moved: Option<MovedMouse>,
    pub grabbed: bool,
}

impl InputHints {
    pub fn read(selected: &str) -> Self {
        Self::new(input::moved_mouse(), input::grab_suspected(), selected)
    }

    fn new(moved: Option<MovedMouse>, grabbed: bool, selected: &str) -> Self {
        Self {
            moved: moved.filter(|m| m.old.eq_ignore_ascii_case(selected)),
            grabbed: grabbed && !selected.is_empty(),
        }
    }
}

pub fn moved_text(moved: &MovedMouse, s: &Strings) -> String {
    fill(
        s.det_mouse_moved,
        &[
            ("name", moved.name()),
            ("old", moved.old_port()),
            ("new", moved.new_port()),
        ],
    )
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
    } else if session_paused(snap) {
        Action::Details
    } else {
        action_for(snap.status)
    }
}

fn session_paused(snap: &Snapshot) -> bool {
    snap.status == AppStatus::Paused && !snap.session.is_active()
}

fn paused_detail(session: SessionState, s: &Strings, key: &str) -> String {
    match session {
        SessionState::Locked => s.det_session_locked.into(),
        SessionState::Inactive => s.det_session_inactive.into(),
        SessionState::Active => fill(s.det_paused, &[("key", key)]),
    }
}

fn mouse_missing(snap: &Snapshot) -> bool {
    snap.status == AppStatus::NotReading && snap.device_filter == DeviceFilter::SelectedMissing
}

fn pointer_unreadable(snap: &Snapshot, setup: &SetupInfo) -> bool {
    snap.status == AppStatus::SetupRequired
        && snap.connection == Connection::Connected
        && !snap.registration_ok
        && !setup.report.has_failures()
}

fn stall_after_ms(tick: u64, interval_ms: f64) -> f64 {
    if tick == 0 {
        return STALL_STARTUP_MS;
    }
    (STALL_INTERVALS * interval_ms).max(STALL_MIN_MS)
}

#[derive(Default)]
pub struct LoopWatch {
    last_tick: Option<u64>,
    changed_at_ms: f64,
}

impl LoopWatch {
    pub fn stalled(&mut self, tick: u64, interval_ms: f64, now_ms: f64) -> bool {
        if self.last_tick != Some(tick) {
            self.last_tick = Some(tick);
            self.changed_at_ms = now_ms;
            return false;
        }
        now_ms - self.changed_at_ms > stall_after_ms(tick, interval_ms)
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
        Action::UseAllMice => s.act_use_all_mice.into(),
        Action::Details => s.act_details.into(),
        Action::SaveMovedMouse => s.btn_save.into(),
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
        AppStatus::SetupRequired if pointer_unreadable(snap, setup) => s.det_no_pointer.into(),
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
        AppStatus::Paused => paused_detail(snap.session, s, &key),
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
    stalled: bool,
    hints: &InputHints,
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
    let warn = ui.visuals().warn_fg_color;
    if stalled {
        ui.label(RichText::new(cx.s.det_loop_stalled).color(warn));
    } else {
        theme::hint(ui, &detail(snap, setup, cfg, cx.s, cx.lang));
    }
    if let Some(text) = input_lost_hint(snap, cx.s) {
        ui.label(RichText::new(text).color(warn));
    }
    if hints.grabbed {
        ui.label(RichText::new(cx.s.det_mouse_grabbed).color(warn));
    }
    if let Some(moved) = &hints.moved {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(moved_text(moved, cx.s)).color(warn));
            if ui
                .button(action_label(Action::SaveMovedMouse, cx.s, &key))
                .clicked()
            {
                clicked = Some(Action::SaveMovedMouse);
            }
        });
    }
    clicked
}

fn input_lost_hint<'a>(snap: &Snapshot, s: &'a Strings) -> Option<&'a str> {
    (snap.drops_since_center > 0).then_some(s.det_input_lost)
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
