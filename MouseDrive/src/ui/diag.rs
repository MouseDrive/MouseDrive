#![deny(unsafe_code)]

use eframe::egui::{RichText, ScrollArea, Ui};
use mousedrive::config::{self, Config};
use mousedrive::control::{SetupInfo, Snapshot};
#[cfg(target_os = "linux")]
use mousedrive::diagnostics::{DeviceLine, LinuxInfo};
use mousedrive::diagnostics::{DiagnosticsInfo, Spread, format_report};
#[cfg(target_os = "linux")]
use mousedrive::input::{Class, DeviceReport};
#[cfg(target_os = "linux")]
use mousedrive::platform::{self, Environment};
use mousedrive::priority::ThreadPriority;
use mousedrive::setup::format_version;
use mousedrive::{input, instance};

use super::theme;
use super::widgets::{Cx, output_name};
use crate::lang::{Lang, Strings, fill, strings};

const COPIED_VISIBLE_MS: f64 = 2000.0;
const DETAILS_MAX_HEIGHT: f32 = 160.0;
const NO_SAMPLES: &str = "–";

pub fn summary_line(snap: &Snapshot, s: &Strings, lang: Lang) -> String {
    let l = &snap.loop_summary;
    let line = fill(
        s.diag_line,
        &[
            ("ok", &snap.writes.ok_writes.to_string()),
            ("fail", &snap.writes.failed_writes.to_string()),
            ("hz", &lang.num(l.hz, 0)),
            ("p99", &lang.num(l.p99_ms, 1)),
            ("clipped", &snap.clipped_ticks.to_string()),
            ("reconnects", &snap.reconnects.to_string()),
        ],
    );
    let mouse = fill(s.diag_mouse, &[("hz", &lang.num(snap.mouse_hz, 0))]);
    format!("{line} · {mouse}")
}

pub fn detail_lines(snap: &Snapshot, s: &Strings, lang: Lang) -> [String; 2] {
    let p = &snap.priorities;
    let priority = fill(
        s.diag_priority,
        &[
            ("control", &s.thread_priority(p.control)),
            ("input", &s.thread_priority(p.input)),
        ],
    );
    let l = &snap.latency;
    let latency = fill(
        s.diag_latency,
        &[
            ("wake", &spread_text(&l.wake_late, lang)),
            ("work", &spread_text(&l.work, lang)),
            ("age", &spread_text(&l.input_age, lang)),
        ],
    );
    [priority, latency]
}

fn spread_text(spread: &Spread, lang: Lang) -> String {
    if spread.samples == 0 {
        return NO_SAMPLES.into();
    }
    [spread.p50_ms, spread.p99_ms, spread.max_ms]
        .map(|ms| lang.num(ms, 2))
        .join("/")
}

pub fn footer_line(snap: &Snapshot, cfg: &Config, s: &Strings, lang: Lang) -> String {
    let l = &snap.loop_summary;
    fill(
        s.footer_line,
        &[
            ("output", &output_name(s, cfg)),
            ("hz", &lang.num(l.hz, 0)),
            ("p99", &lang.num(l.p99_ms, 1)),
        ],
    )
}

pub fn footer_issues(snap: &Snapshot, s: &Strings) -> Option<String> {
    let count = |text: &str, n: u64| (n > 0).then(|| fill(text, &[("n", &n.to_string())]));
    let parts = [
        count(s.footer_write_errors, snap.writes.failed_writes),
        count(s.footer_reconnects, u64::from(snap.reconnects)),
    ];
    let text = parts.into_iter().flatten().collect::<Vec<_>>().join(" · ");
    (!text.is_empty()).then_some(text)
}

pub fn warnings(snap: &Snapshot, s: &Strings) -> Vec<String> {
    let count = |text: &str, n: u64| (n > 0).then(|| fill(text, &[("n", &n.to_string())]));
    let stopped = snap.input_stopped.then(|| s.diag_input_stopped.to_string());
    let registration = (!snap.registration_ok && snap.loop_summary.samples > 0)
        .then(|| s.diag_registration.to_string());
    let p = &snap.priorities;
    let demoted = [p.control, p.input]
        .contains(&ThreadPriority::Demoted)
        .then(|| s.diag_demoted.to_string());
    [
        stopped,
        count(s.diag_dropped, snap.dropped_reports),
        count(s.diag_reconciled, snap.reconcile_releases),
        count(s.diag_absolute, snap.absolute_events),
        count(s.diag_stuck, snap.stuck_releases),
        registration,
        demoted,
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn report_info(snap: &Snapshot, setup: &SetupInfo, cfg: &Config) -> DiagnosticsInfo {
    let r = &setup.report;
    let en = strings(Lang::En);
    DiagnosticsInfo {
        app_version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        status: en.status_labels[snap.status.index()].into(),
        backend: setup.backend.clone().unwrap_or_else(|| "-".into()),
        dll_path: r.dll_path.as_ref().map(|p| p.display().to_string()),
        dll_version: r.versions.map(|v| format_version(v.dll)),
        driver_version: r.versions.map(|v| format_version(v.driver)),
        target_hz: snap.target_hz,
        loop_summary: snap.loop_summary,
        latency: snap.latency,
        control_priority: en.thread_priority(snap.priorities.control),
        input_priority: en.thread_priority(snap.priorities.input),
        ok_writes: snap.writes.ok_writes,
        failed_writes: snap.writes.failed_writes,
        reconnects: snap.reconnects,
        clipped_ticks: snap.clipped_ticks,
        telemetry_dropped: snap.telemetry_dropped,
        mouse_rate_hz: snap.mouse_hz,
        absolute_events: snap.absolute_events,
        ignored_events: snap.ignored_events,
        stuck_releases: snap.stuck_releases,
        device_filter: snap.device_filter,
        selected_device: cfg.mouse_device.clone(),
        input_sink: cfg.input_sink_enabled,
        test_counter: snap.test_counter,
        allow_injected: input::allow_injected(),
        instance_lock: instance::lock_acquired(),
        portable: config::portable_mode(),
        #[cfg(target_os = "linux")]
        linux: linux_info(snap, &input::devices(), platform::environment()),
    }
}

#[cfg(target_os = "linux")]
fn linux_info(snap: &Snapshot, devices: &[DeviceReport], environment: &Environment) -> LinuxInfo {
    let en = strings(Lang::En);
    LinuxInfo {
        environment: environment.clone(),
        pointer_readable: snap.registration_ok,
        denied_pointers: devices
            .iter()
            .filter(|d| d.denied_as(Class::is_pointer))
            .count(),
        dropped_reports: snap.dropped_reports,
        reconcile_releases: snap.reconcile_releases,
        input_stopped: snap.input_stopped,
        devices: devices.iter().map(|d| device_line(d, en)).collect(),
    }
}

#[cfg(target_os = "linux")]
fn device_line(d: &DeviceReport, s: &Strings) -> DeviceLine {
    let [node, ids, bus, class, state, name] = crate::cli::device_row(d, s);
    DeviceLine {
        node,
        ids,
        bus,
        class,
        state,
        uniq: d.uniq.clone(),
        name,
    }
}

#[derive(Default)]
pub struct DiagUi {
    pub open: bool,
    copied_at: Option<f64>,
}

impl DiagUi {
    pub fn show(
        &mut self,
        ui: &mut Ui,
        cx: &Cx,
        (snap, setup, cfg): (&Snapshot, &SetupInfo, &Config),
        now_ms: f64,
    ) {
        let warn = ui.visuals().warn_fg_color;
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(footer_line(snap, cfg, cx.s, cx.lang))
                    .small()
                    .color(theme::MUTED),
            );
            if let Some(issues) = footer_issues(snap, cx.s) {
                ui.label(RichText::new(issues).small().color(warn));
            }
            ui.toggle_value(&mut self.open, cx.s.system_details);
            if self.open && ui.small_button(cx.s.btn_copy_diag).clicked() {
                ui.ctx()
                    .copy_text(format_report(&report_info(snap, setup, cfg)));
                self.copied_at = Some(now_ms);
            }
            if self
                .copied_at
                .is_some_and(|t| now_ms - t < COPIED_VISIBLE_MS)
            {
                ui.label(RichText::new(cx.s.copied).small());
            }
        });
        for w in warnings(snap, cx.s) {
            ui.label(RichText::new(format!("⚠ {w}")).small().color(warn));
        }
        if self.open {
            ui.label(RichText::new(summary_line(snap, cx.s, cx.lang)).small());
            for line in detail_lines(snap, cx.s, cx.lang) {
                ui.label(RichText::new(line).small());
            }
            let report = format_report(&report_info(snap, setup, cfg));
            ScrollArea::vertical()
                .max_height(DETAILS_MAX_HEIGHT)
                .show(ui, |ui| ui.label(RichText::new(report).monospace().small()));
        }
    }
}
