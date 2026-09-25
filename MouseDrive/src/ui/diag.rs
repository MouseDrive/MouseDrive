#![deny(unsafe_code)]

use eframe::egui::{Align, Layout, RichText, ScrollArea, Ui};
use mousedrive::config::Config;
use mousedrive::control::{SetupInfo, Snapshot};
use mousedrive::diagnostics::{DiagnosticsInfo, format_report};
use mousedrive::setup::format_version;

use super::widgets::Cx;
use crate::lang::{Lang, Strings, fill, strings};

const COPIED_VISIBLE_MS: f64 = 2000.0;
const DETAILS_MAX_HEIGHT: f32 = 160.0;

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

pub fn warnings(snap: &Snapshot, s: &Strings) -> Vec<String> {
    let absolute = (snap.absolute_events > 0)
        .then(|| fill(s.diag_absolute, &[("n", &snap.absolute_events.to_string())]));
    let stuck = (snap.stuck_releases > 0)
        .then(|| fill(s.diag_stuck, &[("n", &snap.stuck_releases.to_string())]));
    let registration = (!snap.registration_ok && snap.loop_summary.samples > 0)
        .then(|| s.diag_registration.to_string());
    [absolute, stuck, registration]
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
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(summary_line(snap, cx.s, cx.lang))
                    .small()
                    .weak(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.toggle_value(&mut self.open, cx.s.act_details);
                if ui.small_button(cx.s.btn_copy_diag).clicked() {
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
        });
        let warn = ui.visuals().warn_fg_color;
        for w in warnings(snap, cx.s) {
            ui.label(RichText::new(format!("⚠ {w}")).small().color(warn));
        }
        if self.open {
            let report = format_report(&report_info(snap, setup, cfg));
            ScrollArea::vertical()
                .max_height(DETAILS_MAX_HEIGHT)
                .show(ui, |ui| ui.label(RichText::new(report).monospace().small()));
        }
    }
}
