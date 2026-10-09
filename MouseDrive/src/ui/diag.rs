#![deny(unsafe_code)]

use eframe::egui::{RichText, ScrollArea, Ui};
use mousedrive::config::Config;
use mousedrive::control::{SetupInfo, Snapshot};
use mousedrive::diagnostics::{DiagnosticsInfo, Spread, format_report};
use mousedrive::priority::ThreadPriority;
use mousedrive::setup::format_version;

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
    DiagnosticsInfo {
        app_version: env!("CARGO_PKG_VERSION").into(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        status: strings(Lang::En).status_labels[snap.status.index()].into(),
        backend: setup.backend.clone().unwrap_or_else(|| "-".into()),
        dll_path: r.dll_path.as_ref().map(|p| p.display().to_string()),
        dll_version: r.versions.map(|v| format_version(v.dll)),
        driver_version: r.versions.map(|v| format_version(v.driver)),
        target_hz: snap.target_hz,
        loop_summary: snap.loop_summary,
        ok_writes: snap.writes.ok_writes,
        failed_writes: snap.writes.failed_writes,
        reconnects: snap.reconnects,
        clipped_ticks: snap.clipped_ticks,
        telemetry_dropped: snap.telemetry_dropped,
        mouse_rate_hz: snap.mouse_hz,
        absolute_events: snap.absolute_events,
        mouse_filter: format!("{:?}", snap.device_filter),
        input_sink: cfg.input_sink_enabled,
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
