#![deny(unsafe_code)]

use eframe::egui::{CollapsingHeader, Color32, RichText, Ui};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotPoints, Points, Polygon, VLine};
use mousedrive::config::Tuning;
use mousedrive::control::Snapshot;
use mousedrive::curve::Curve;
use mousedrive::logic::BrakePhase;
use mousedrive::preview::{BrakeTimeline, PhaseSpan, brake_timeline, full_press_ms};

use super::widgets::{Cx, Fmt, Param, check_row, grid, param_row, param_row_i32, section};
use crate::curve_editor::{CurveDisplay, curve_editor};

const PLOT_HEIGHT: f32 = 150.0;
const STEER_BUCKETS: f64 = 50.0;
const SPAN_ALPHA: u8 = 36;

pub fn phase_index(p: BrakePhase) -> usize {
    match p {
        BrakePhase::Idle => 0,
        BrakePhase::Fill => 1,
        BrakePhase::Full => 2,
        BrakePhase::Decay => 3,
        BrakePhase::ShortHold => 4,
        BrakePhase::Release => 5,
    }
}

fn phase_tint(p: BrakePhase) -> Color32 {
    let [r, g, b] = match p {
        BrakePhase::Idle => [128, 128, 128],
        BrakePhase::Fill => [0xE6, 0x9F, 0x00],
        BrakePhase::Full => [0xD5, 0x5E, 0x00],
        BrakePhase::Decay => [0xCC, 0x79, 0xA7],
        BrakePhase::ShortHold => [0x56, 0xB4, 0xE9],
        BrakePhase::Release => [0x00, 0x9E, 0x73],
    };
    Color32::from_rgba_unmultiplied(r, g, b, SPAN_ALPHA)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BrakeClock {
    since: Option<f64>,
}

impl BrakeClock {
    pub fn update(&mut self, pressed: bool, now_ms: f64) {
        self.since = match (pressed, self.since) {
            (true, None) => Some(now_ms),
            (true, since) => since,
            (false, _) => None,
        };
    }

    pub fn elapsed(&self, now_ms: f64) -> Option<f64> {
        self.since.map(|s| (now_ms - s).max(0.0))
    }
}

#[derive(Default)]
pub struct BrakeTab {
    clock: BrakeClock,
    cache: Option<(Tuning, u32, BrakeTimeline)>,
}

impl BrakeTab {
    pub fn tick(&mut self, snap: &Snapshot, now_ms: f64) {
        self.clock.update(snap.brake_pressed, now_ms);
    }

    fn timeline(&mut self, t: &Tuning, steer_abs: f64) -> &BrakeTimeline {
        let bucket = (steer_abs.clamp(0.0, 1.0) * STEER_BUCKETS).round() as u32;
        let fresh = matches!(&self.cache, Some((ct, cb, _)) if ct == t && *cb == bucket);
        if !fresh {
            self.cache = None;
        }
        let (_, _, timeline) = self.cache.get_or_insert_with(|| {
            let steer = f64::from(bucket) / STEER_BUCKETS;
            (
                t.clone(),
                bucket,
                brake_timeline(t, full_press_ms(t), steer),
            )
        });
        timeline
    }

    pub fn show(&mut self, ui: &mut Ui, cx: &Cx, t: &mut Tuning, snap: &Snapshot, now_ms: f64) {
        let (s, d) = (cx.s, Tuning::default());
        grid(ui, "brake_ceiling", |ui| {
            let p = Param::new(
                s.brake_ceiling,
                s.tip_brake_ceiling,
                d.brake_max_output,
                0.1..=1.0,
                Fmt::PCT0,
            );
            param_row(ui, cx, &p, &mut t.brake_max_output);
        });
        self.plot(ui, cx, t, snap, now_ms);
        fill_and_full(ui, cx, t, &d);
        decay(ui, cx, t, &d);
        after_release(ui, cx, t, &d);
        ui.add_space(8.0);
        if ui.button(s.btn_tab_defaults).clicked() {
            *t = brake_defaults(t);
        }
    }

    fn plot(&mut self, ui: &mut Ui, cx: &Cx, t: &Tuning, snap: &Snapshot, now_ms: f64) {
        let steer = if t.brake_trail_enabled {
            snap.frame.steering.abs()
        } else {
            0.0
        };
        let live = self.clock.elapsed(now_ms).map(|ms| [ms, snap.frame.brake]);
        let timeline = self.timeline(t, steer);
        ui.label(RichText::new(cx.s.timeline).strong())
            .on_hover_text(cx.s.tip_timeline);
        let lang = cx.lang;
        Plot::new("brake_timeline")
            .height(PLOT_HEIGHT)
            .allow_zoom(false)
            .allow_drag(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .allow_double_click_reset(false)
            .include_y(0.0)
            .include_y(1.0)
            .x_axis_label(cx.s.axis_ms)
            .y_axis_label(cx.s.axis_brake_pct)
            .y_axis_formatter(move |m, _| lang.pct(m.value, 0))
            .legend(Legend::default())
            .show(ui, |plot| {
                for span in &timeline.spans {
                    plot.polygon(span_polygon(span, cx));
                }
                let points = PlotPoints::new(timeline.points.clone());
                let style = LineStyle::dashed_loose();
                plot.line(
                    Line::new(points)
                        .color(cx.pal.brake)
                        .width(2.0_f32)
                        .style(style),
                );
                plot.vline(VLine::new(timeline.release_ms).name(cx.s.release_marker));
                if let Some(point) = live {
                    let marker = Points::new(vec![point])
                        .radius(4.0_f32)
                        .color(cx.pal.accent);
                    plot.points(marker.name(cx.s.now_marker));
                }
            });
    }
}

fn span_polygon<'a>(span: &PhaseSpan, cx: &Cx) -> Polygon<'a> {
    let (a, b) = (span.start_ms, span.end_ms);
    let corners = vec![[a, 0.0], [b, 0.0], [b, 1.0], [a, 1.0]];
    let tint = phase_tint(span.phase);
    Polygon::new(PlotPoints::new(corners))
        .fill_color(tint)
        .stroke((0.0, tint))
        .name(cx.s.phase_names[phase_index(span.phase)])
}

fn advanced_curve(
    ui: &mut Ui,
    cx: &Cx,
    id: &str,
    (title, curve): (&str, &mut Curve),
    display: CurveDisplay,
) {
    CollapsingHeader::new(cx.s.advanced)
        .id_salt(id)
        .show(ui, |ui| {
            curve_editor(ui, title, curve, display, cx.s, cx.lang);
        });
}

fn fill_and_full(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    section(ui, s.phase_heads[0]);
    grid(ui, "brake_fill", |ui| {
        let p = Param::new(
            s.fill_ms,
            s.tip_fill_ms,
            f64::from(d.brake_fast_apply_ms),
            1.0..=200.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &p, &mut t.brake_fast_apply_ms);
    });
    advanced_curve(
        ui,
        cx,
        "brake_fill_adv",
        (s.curve_apply, &mut t.brake_apply_curve),
        CurveDisplay::Normal,
    );
    section(ui, s.phase_heads[1]);
    grid(ui, "brake_full", |ui| {
        let p = Param::new(
            s.hold_ms,
            s.tip_hold_ms,
            f64::from(d.brake_hold_ms),
            100.0..=3000.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &p, &mut t.brake_hold_ms);
    });
}

fn decay(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    section(ui, s.phase_heads[2]);
    grid(ui, "brake_decay", |ui| {
        let ms = Param::new(
            s.decay_ms,
            s.tip_decay_ms,
            f64::from(d.brake_release_total_ms),
            200.0..=5000.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &ms, &mut t.brake_release_total_ms);
        let exp = Param::new(
            s.decay_exp,
            s.tip_decay_exp,
            d.brake_release_accel_exp,
            0.5..=4.0,
            Fmt::Num(2),
        );
        param_row(ui, cx, &exp, &mut t.brake_release_accel_exp);
        let base = Param::new(
            s.floor_base,
            s.tip_floor_base,
            d.brake_min_ratio_base,
            0.0..=1.0,
            Fmt::PCT0,
        );
        param_row(ui, cx, &base, &mut t.brake_min_ratio_base);
        trail_rows(ui, cx, t, d);
    });
    advanced_curve(
        ui,
        cx,
        "brake_decay_adv",
        (s.curve_decay, &mut t.brake_posthold_curve),
        CurveDisplay::MirrorY,
    );
}

fn trail_rows(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    check_row(
        ui,
        cx,
        (s.trail, s.tip_trail),
        &mut t.brake_trail_enabled,
        d.brake_trail_enabled,
    );
    if !t.brake_trail_enabled {
        return;
    }
    let max = Param::new(
        s.floor_max,
        s.tip_floor_max,
        d.brake_min_ratio_max,
        0.0..=1.0,
        Fmt::PCT0,
    );
    param_row(ui, cx, &max, &mut t.brake_min_ratio_max);
    let exp = Param::new(
        s.floor_exp,
        s.tip_floor_exp,
        d.brake_curve_exp,
        0.5..=4.0,
        Fmt::Num(2),
    );
    param_row(ui, cx, &exp, &mut t.brake_curve_exp);
}

fn after_release(ui: &mut Ui, cx: &Cx, t: &mut Tuning, d: &Tuning) {
    let s = cx.s;
    section(ui, s.phase_heads[3]);
    grid(ui, "brake_short_hold", |ui| {
        let ratio = Param::new(
            s.post_hold_ratio,
            s.tip_post_hold_ratio,
            d.brake_after_release_hold_ratio,
            0.0..=0.5,
            Fmt::PCT0,
        );
        param_row(ui, cx, &ratio, &mut t.brake_after_release_hold_ratio);
        let ms = Param::new(
            s.post_hold_ms,
            s.tip_post_hold_ms,
            f64::from(d.brake_after_release_hold_ms),
            0.0..=3000.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &ms, &mut t.brake_after_release_hold_ms);
        let tap = Param::new(
            s.tap_ms,
            s.tip_tap_ms,
            f64::from(d.brake_tap_ms),
            10.0..=500.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &tap, &mut t.brake_tap_ms);
    });
    section(ui, s.phase_heads[4]);
    grid(ui, "brake_release", |ui| {
        let p = Param::new(
            s.release_ms,
            s.tip_release_ms,
            f64::from(d.brake_fast_release_ms),
            10.0..=500.0,
            Fmt::Num(0),
        );
        param_row_i32(ui, cx, &p, &mut t.brake_fast_release_ms);
    });
}

pub fn brake_defaults(t: &Tuning) -> Tuning {
    let d = Tuning::default();
    Tuning {
        brake_max_output: d.brake_max_output,
        brake_fast_apply_ms: d.brake_fast_apply_ms,
        brake_hold_ms: d.brake_hold_ms,
        brake_release_total_ms: d.brake_release_total_ms,
        brake_release_accel_exp: d.brake_release_accel_exp,
        brake_fast_release_ms: d.brake_fast_release_ms,
        brake_tap_ms: d.brake_tap_ms,
        brake_min_ratio_base: d.brake_min_ratio_base,
        brake_min_ratio_max: d.brake_min_ratio_max,
        brake_curve_exp: d.brake_curve_exp,
        brake_trail_enabled: d.brake_trail_enabled,
        brake_after_release_hold_ratio: d.brake_after_release_hold_ratio,
        brake_after_release_hold_ms: d.brake_after_release_hold_ms,
        brake_apply_curve: d.brake_apply_curve,
        brake_posthold_curve: d.brake_posthold_curve,
        ..t.clone()
    }
}
