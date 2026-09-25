#![deny(unsafe_code)]

use eframe::egui::{Color32, RichText, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, VLine};
use mousedrive::control::Shared;
use mousedrive::telemetry::{Marker, Sample, decimate_min_max};

use super::widgets::Cx;

const KEEP_MS: f64 = 30_000.0;
pub const WINDOWS_S: [u32; 3] = [5, 10, 30];
const DEFAULT_WINDOW_S: u32 = 10;
const PLOT_HEIGHT: f32 = 260.0;
const COLUMNS: usize = 400;
const LMB_Y: (f64, f64) = (-1.35, -1.15);
const RMB_Y: (f64, f64) = (-1.65, -1.45);

pub struct Monitor {
    samples: Vec<Sample>,
    last_seq: Option<u64>,
    frozen: bool,
    window_s: u32,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            samples: Vec::new(),
            last_seq: None,
            frozen: false,
            window_s: DEFAULT_WINDOW_S,
        }
    }
}

impl Monitor {
    pub fn pull(&mut self, shared: &Shared) {
        if self.frozen {
            return;
        }
        shared.copy_telemetry(self.last_seq, &mut self.samples);
        self.settle();
    }

    fn settle(&mut self) {
        let Some(last) = self.samples.last() else {
            return;
        };
        self.last_seq = Some(last.seq);
        let oldest = last.t_ms - KEEP_MS;
        let cut = self.samples.partition_point(|s| s.t_ms < oldest);
        self.samples.drain(..cut);
    }

    pub fn clear(&mut self) {
        self.samples = Vec::new();
        self.last_seq = None;
    }

    pub fn show(&mut self, ui: &mut Ui, cx: &Cx) {
        self.controls(ui, cx);
        let Some(latest) = self.samples.last().map(|s| s.t_ms) else {
            ui.label(RichText::new("…").weak());
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

    fn controls(&mut self, ui: &mut Ui, cx: &Cx) {
        ui.horizontal(|ui| {
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
        });
    }
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
        Line::new(trace(samples, window, |x| x.steering_raw))
            .color(weak)
            .name(s.tr_steer_raw),
        Line::new(trace(samples, window, |x| x.steering_out))
            .color(cx.pal.accent)
            .width(1.5_f32)
            .name(s.tr_steer_out),
        Line::new(trace(samples, window, |x| x.throttle_out))
            .color(cx.pal.throttle)
            .width(1.5_f32)
            .name(s.tr_throttle),
        Line::new(trace(samples, window, |x| x.brake_out))
            .color(cx.pal.brake)
            .width(1.5_f32)
            .style(LineStyle::dashed_loose())
            .name(s.tr_brake),
        Line::new(button_trace(samples, window, |x| x.throttle_pressed, LMB_Y))
            .color(cx.pal.throttle)
            .name(s.tr_lmb),
        Line::new(button_trace(samples, window, |x| x.brake_pressed, RMB_Y))
            .color(cx.pal.brake)
            .style(LineStyle::dashed_loose())
            .name(s.tr_rmb),
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
        .map(|(t, name)| VLine::new((t - latest) / 1000.0).name(name))
        .collect()
}
