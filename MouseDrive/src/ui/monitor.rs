#![deny(unsafe_code)]

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui::{Button, Color32, RichText, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, VLine};
use mousedrive::config::Tuning;
use mousedrive::control::Shared;
use mousedrive::disk::{DiskWriter, JobState, Pending};
use mousedrive::ioerr::IoFailure;
use mousedrive::telemetry::{Marker, Sample, decimate_min_max};
use mousedrive::{fsutil, log};

use super::profiles::open_in_file_manager;
use super::widgets::Cx;
use crate::lang::fill;

const KEEP_MS: f64 = 30_000.0;
pub const WINDOWS_S: [u32; 3] = [5, 10, 30];
const DEFAULT_WINDOW_S: u32 = 10;
const PLOT_HEIGHT: f32 = 320.0;
const COLUMNS: usize = 400;
const LMB_Y: (f64, f64) = (-1.35, -1.15);
const RMB_Y: (f64, f64) = (-1.65, -1.45);
const CSV_COLUMNS: &str = "seq,t_ms,counts_x,steering_raw,steering_out,throttle_pressed,\
throttle_target,throttle_out,brake_pressed,brake_floor,brake_out,brake_phase,capture,marker";
const EXPORT_PREFIX: &str = "mousedrive-monitor-";

struct CsvMeta<'a> {
    version: &'a str,
    profile: &'a str,
    unsaved: bool,
    tuning_toml: &'a str,
}

pub struct Export<'a> {
    pub disk: &'a DiskWriter,
    pub profile: &'a str,
    pub unsaved: bool,
    pub tuning: &'a Tuning,
}

pub struct Monitor {
    samples: Vec<Sample>,
    last_seq: Option<u64>,
    lost: u64,
    gap_end_seq: Option<u64>,
    frozen: bool,
    window_s: u32,
    changed_at_ms: Option<f64>,
    export: Option<Pending<io::Result<PathBuf>>>,
    exported: Option<Result<PathBuf, IoFailure>>,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            last_seq: None,
            lost: 0,
            gap_end_seq: None,
            frozen: false,
            window_s: DEFAULT_WINDOW_S,
            changed_at_ms: None,
            export: None,
            exported: None,
        }
    }
}

type Plotted = (f32, f32, f32, f32, bool, bool, bool);

fn plotted(s: &Sample) -> Plotted {
    (
        s.steering_raw,
        s.steering_out,
        s.throttle_out,
        s.brake_out,
        s.throttle_pressed,
        s.brake_pressed,
        s.marker != Marker::None,
    )
}

impl Monitor {
    pub fn pull(&mut self, shared: &Shared) {
        if self.frozen {
            return;
        }
        let lost = shared.copy_telemetry(self.last_seq, &mut self.samples);
        self.record_loss(lost);
        self.settle();
    }

    fn record_loss(&mut self, lost: u64) {
        if lost == 0 {
            return;
        }
        self.lost = self.lost.saturating_add(lost);
        self.gap_end_seq = self
            .last_seq
            .map(|seq| seq.saturating_add(lost).saturating_add(1));
    }

    fn settle(&mut self) {
        self.note_changes();
        let Some(last) = self.samples.last() else {
            return;
        };
        self.last_seq = Some(last.seq);
        let oldest = last.t_ms - KEEP_MS;
        let cut = self.samples.partition_point(|s| s.t_ms < oldest);
        self.samples.drain(..cut);
        self.forget_gap_out_of_view();
    }

    fn note_changes(&mut self) {
        let seen = self.last_seq;
        let first_new = self
            .samples
            .partition_point(|s| seen.is_some_and(|last| s.seq <= last));
        let changed = self.samples[first_new.saturating_sub(1)..]
            .windows(2)
            .rev()
            .find(|pair| plotted(&pair[0]) != plotted(&pair[1]))
            .map(|pair| pair[1].t_ms);
        if changed.is_some() {
            self.changed_at_ms = changed;
        }
    }

    pub fn moving(&self) -> bool {
        let window_ms = f64::from(self.window_s) * 1000.0;
        match (self.frozen, self.changed_at_ms, self.samples.last()) {
            (false, Some(at), Some(last)) => last.t_ms - at <= window_ms,
            _ => false,
        }
    }

    fn forget_gap_out_of_view(&mut self) {
        let first = self.samples.first().map(|s| s.seq);
        if self
            .gap_end_seq
            .is_some_and(|end| first.is_some_and(|seq| seq >= end))
        {
            self.lost = 0;
            self.gap_end_seq = None;
        }
    }

    pub fn release(&mut self) {
        if self.frozen {
            return;
        }
        self.samples = Vec::new();
        self.last_seq = None;
        self.lost = 0;
        self.gap_end_seq = None;
        self.changed_at_ms = None;
    }

    pub fn show(&mut self, ui: &mut Ui, cx: &Cx, export: &Export) {
        self.controls(ui, cx, export);
        self.export_result(ui, cx);
        let Some(latest) = self.samples.last().map(|s| s.t_ms) else {
            super::theme::hint(ui, cx.s.monitor_empty);
            return;
        };
        let t0 = latest - f64::from(self.window_s) * 1000.0;
        let visible = &self.samples[self.samples.partition_point(|s| s.t_ms < t0)..];
        let window = (t0, latest);
        let weak = ui.visuals().weak_text_color();
        Plot::new("input_monitor")
            .height(PLOT_HEIGHT)
            .allow_zoom(false)
            .allow_drag(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .include_x(-f64::from(self.window_s))
            .include_x(0.0)
            .include_y(LMB_Y.0.min(RMB_Y.0))
            .include_y(1.05)
            .x_axis_label(cx.s.axis_seconds)
            .legend(Legend::default())
            .show(ui, |plot| {
                for line in traces(visible, window, cx, weak) {
                    plot.line(line);
                }
                for vline in markers(visible, latest, cx) {
                    plot.vline(vline);
                }
            });
    }

    fn controls(&mut self, ui: &mut Ui, cx: &Cx, export: &Export) {
        ui.horizontal_wrapped(|ui| {
            let label = if self.frozen {
                cx.s.resume
            } else {
                cx.s.freeze
            };
            if ui.button(label).clicked() {
                self.frozen = !self.frozen;
            }
            ui.label(cx.s.window_secs);
            for w in WINDOWS_S {
                ui.selectable_value(&mut self.window_s, w, format!("{w} {}", cx.s.axis_seconds));
            }
            let ready = !self.samples.is_empty() && self.export.is_none();
            let save = ui
                .add_enabled(ready, Button::new(cx.s.btn_save_csv))
                .on_hover_text(cx.s.tip_save_csv);
            if save.clicked() {
                self.start_export(export);
            }
            if self.export.is_some() {
                ui.spinner();
                ui.label(RichText::new(cx.s.saving).weak());
            }
        });
        if self.lost > 0 {
            let text = fill(cx.s.monitor_lost, &[("n", &self.lost.to_string())]);
            super::theme::hint(ui, &text);
        }
    }

    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    fn start_export(&mut self, export: &Export) {
        let tuning_toml = export.tuning.to_toml().unwrap_or_else(|e| {
            log::warn(&format!("profil ayarları CSV başlığına yazılamadı: {e}"));
            String::new()
        });
        let meta = CsvMeta {
            version: env!("CARGO_PKG_VERSION"),
            profile: export.profile,
            unsaved: export.unsaved,
            tuning_toml: &tuning_toml,
        };
        let text = csv_text(&self.samples, &meta);
        let name = export_file_name(&log::timestamp(SystemTime::now()));
        self.exported = None;
        self.export = Some(export.disk.submit(move || write_export(&name, &text)));
    }

    pub fn collect_export(&mut self) {
        let Some(pending) = &self.export else {
            return;
        };
        self.exported = match pending.state() {
            JobState::Running => return,
            JobState::Done(result) => Some(result.map_err(IoFailure::from)),
            JobState::Lost => Some(Err(IoFailure::from_kind(io::ErrorKind::Interrupted))),
        };
        self.export = None;
    }

    fn export_result(&self, ui: &mut Ui, cx: &Cx) {
        match &self.exported {
            None => {}
            Some(Ok(path)) => saved_line(ui, cx, path),
            Some(Err(error)) => {
                let text = fill(cx.s.csv_failed, &[("error", &cx.s.io_failure(*error))]);
                ui.label(RichText::new(text).color(ui.visuals().error_fg_color));
            }
        }
    }
}

fn saved_line(ui: &mut Ui, cx: &Cx, path: &Path) {
    ui.horizontal_wrapped(|ui| {
        let shown = path.display().to_string();
        ui.label(RichText::new(fill(cx.s.csv_saved, &[("path", &shown)])).weak());
        if let Some(dir) = path.parent()
            && ui.small_button(cx.s.btn_open_folder_short).clicked()
            && let Err(e) = open_in_file_manager(dir)
        {
            log::warn(&format!("klasör açılamadı ({}): {e}", dir.display()));
        }
    });
}

fn write_export(name: &str, text: &str) -> io::Result<PathBuf> {
    let dir = log::log_dir().ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    match fsutil::write_new(&path, text.as_bytes(), None) {
        Ok(()) => {
            log::info(&format!(
                "input monitor CSV olarak kaydedildi: {}",
                path.display()
            ));
            Ok(path)
        }
        Err(e) => {
            log::error(&format!(
                "input monitor CSV kaydedilemedi ({}): {e}",
                path.display()
            ));
            Err(e)
        }
    }
}

fn export_file_name(timestamp: &str) -> String {
    let safe: String = timestamp
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("{EXPORT_PREFIX}{safe}.csv")
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}

fn marker_label(marker: Marker) -> String {
    match marker {
        Marker::None => String::new(),
        other => format!("{other:?}"),
    }
}

fn csv_text(samples: &[Sample], meta: &CsvMeta) -> String {
    let unsaved = if meta.unsaved {
        " (unsaved changes)"
    } else {
        ""
    };
    let mut out = format!(
        "# MouseDrive v{} input monitor\n# profile: {}{unsaved}\n# profile settings:\n",
        meta.version, meta.profile
    );
    for line in meta.tuning_toml.lines() {
        let _ = writeln!(out, "# {line}");
    }
    out.push_str(CSV_COLUMNS);
    out.push('\n');
    for s in samples {
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{},{},{:?},{},{}",
            s.seq,
            s.t_ms,
            s.counts_x,
            s.steering_raw,
            s.steering_out,
            flag(s.throttle_pressed),
            s.throttle_target,
            s.throttle_out,
            flag(s.brake_pressed),
            s.brake_floor,
            s.brake_out,
            s.brake_phase,
            flag(s.capture),
            marker_label(s.marker)
        );
    }
    out
}

pub fn envelope_points(cols: &[Option<(f32, f32)>], t0: f64, t1: f64) -> Vec<[f64; 2]> {
    if cols.is_empty() {
        return Vec::new();
    }
    let width = (t1 - t0) / cols.len() as f64;
    (0..)
        .zip(cols)
        .filter_map(|(i, c)| c.map(|(lo, hi)| (f64::from(i), lo, hi)))
        .flat_map(|(i, lo, hi)| {
            let x = (t0 + (i + 0.5) * width - t1) / 1000.0;
            [[x, f64::from(lo)], [x, f64::from(hi)]]
        })
        .collect()
}

fn trace<'a>(
    samples: &[Sample],
    (t0, t1): (f64, f64),
    value: impl Fn(&Sample) -> f32,
) -> PlotPoints<'a> {
    PlotPoints::new(envelope_points(
        &decimate_min_max(samples, t0, t1, COLUMNS, value),
        t0,
        t1,
    ))
}

fn button_trace<'a>(
    samples: &[Sample],
    window: (f64, f64),
    pressed: fn(&Sample) -> bool,
    (off, on): (f64, f64),
) -> PlotPoints<'a> {
    let (t0, t1) = window;
    let cols = decimate_min_max(
        samples,
        t0,
        t1,
        COLUMNS,
        |s| if pressed(s) { 1.0 } else { 0.0 },
    );
    let points = envelope_points(&cols, t0, t1)
        .into_iter()
        .map(|[x, v]| [x, off + v * (on - off)])
        .collect();
    PlotPoints::new(points)
}

fn traces<'a>(samples: &[Sample], window: (f64, f64), cx: &Cx, weak: Color32) -> Vec<Line<'a>> {
    let s = cx.s;
    vec![
        Line::new(s.tr_steer_raw, trace(samples, window, |x| x.steering_raw)).color(weak),
        Line::new(s.tr_steer_out, trace(samples, window, |x| x.steering_out))
            .color(cx.pal.accent)
            .width(1.5_f32),
        Line::new(s.tr_throttle, trace(samples, window, |x| x.throttle_out))
            .color(cx.pal.throttle)
            .width(1.5_f32),
        Line::new(s.tr_brake, trace(samples, window, |x| x.brake_out))
            .color(cx.pal.brake)
            .width(1.5_f32)
            .style(LineStyle::dashed_loose()),
        Line::new(
            s.tr_lmb,
            button_trace(samples, window, |x| x.throttle_pressed, LMB_Y),
        )
        .color(cx.pal.throttle),
        Line::new(
            s.tr_rmb,
            button_trace(samples, window, |x| x.brake_pressed, RMB_Y),
        )
        .color(cx.pal.brake)
        .style(LineStyle::dashed_loose()),
    ]
}

fn marker_name(marker: Marker, cx: &Cx) -> Option<&'static str> {
    match marker {
        Marker::None => None,
        Marker::CaptureOn => Some(cx.s.mk_capture_on),
        Marker::CaptureOff => Some(cx.s.mk_capture_off),
        Marker::ProfileChanged => Some(cx.s.mk_profile),
    }
}

fn markers(samples: &[Sample], latest: f64, cx: &Cx) -> Vec<VLine> {
    samples
        .iter()
        .filter_map(|s| marker_name(s.marker, cx).map(|name| (s.t_ms, name)))
        .map(|(t, name)| VLine::new(name, (t - latest) / 1000.0))
        .collect()
}
