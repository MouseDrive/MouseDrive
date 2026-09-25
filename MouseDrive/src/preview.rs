#![deny(unsafe_code)]

use crate::config::{Config, Tuning};
use crate::logic::{
    BrakePhase, MouseDriveState, STEERING_RANGE, SteeringMode, THROTTLE_CUT_REFERENCE, TickInput,
    throttle_cut_bounds, throttle_cut_factor,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutSummary {
    pub start: f64,
    pub floor_at: f64,
    pub floor: f64,
}

pub fn cut_summary(t: &Tuning) -> Option<CutSummary> {
    t.throttle_cut_enabled.then(|| {
        let (start, max) = throttle_cut_bounds(t);
        CutSummary {
            start: start * THROTTLE_CUT_REFERENCE,
            floor_at: max * THROTTLE_CUT_REFERENCE,
            floor: t.throttle_min_cut_at_full,
        }
    })
}

pub fn cut_curve(t: &Tuning, max_steer: f64, steps: usize) -> Vec<[f64; 2]> {
    let steps = steps.max(1);
    let max_steer = max_steer.clamp(0.0, 1.0);
    (0..=steps)
        .map(|i| {
            let steer = max_steer * i as f64 / steps as f64;
            [steer, throttle_cut_factor(t, steer)]
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhaseSpan {
    pub phase: BrakePhase,
    pub start_ms: f64,
    pub end_ms: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BrakeTimeline {
    pub points: Vec<[f64; 2]>,
    pub spans: Vec<PhaseSpan>,
    pub release_ms: f64,
}

const SIM_STEP_MS: f64 = 1.0;
const MAX_TAIL_MS: f64 = 10_000.0;
const FLOOR_VISIBLE_MS: f64 = 300.0;
const POINT_MAX_GAP_MS: f64 = 20.0;
const POINT_MIN_DELTA: f64 = 0.004;

pub fn full_press_ms(t: &Tuning) -> f64 {
    f64::from(t.brake_hold_ms) + f64::from(t.brake_release_total_ms) + FLOOR_VISIBLE_MS
}

pub fn brake_timeline(tuning: &Tuning, press_ms: f64, steer_abs: f64) -> BrakeTimeline {
    let config = preview_config(tuning);
    let press_ms = if press_ms.is_finite() {
        press_ms.max(0.0)
    } else {
        0.0
    };
    let steer_counts = (steer_abs.clamp(0.0, 1.0) * STEERING_RANGE).round() as i64;
    let mut state = MouseDriveState::new();
    let mut rec = Recorder::new();
    let mut t_ms = 0.0;
    loop {
        let pressed = t_ms < press_ms;
        let input = TickInput {
            counts_x: if t_ms == 0.0 { steer_counts } else { 0 },
            throttle_pressed: false,
            brake_pressed: pressed,
            dt_ms: SIM_STEP_MS,
        };
        state.tick(&config, input);
        t_ms += SIM_STEP_MS;
        let phase = state.brake_phase();
        rec.record(t_ms, state.brake_output(tuning), phase);
        if !pressed && (phase == BrakePhase::Idle || t_ms > press_ms + MAX_TAIL_MS) {
            break;
        }
    }
    rec.finish(press_ms)
}

fn preview_config(tuning: &Tuning) -> Config {
    Config::default().with_tuning(Tuning {
        mouse_sens: 1.0,
        steering_mode: SteeringMode::Linear.to_i32(),
        steering_deadzone: 0.0,
        steering_saturation: 1.0,
        ..tuning.clone()
    })
}

struct Recorder {
    points: Vec<[f64; 2]>,
    spans: Vec<PhaseSpan>,
    last_phase: BrakePhase,
    skipped: Option<[f64; 2]>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            points: vec![[0.0, 0.0]],
            spans: Vec::new(),
            last_phase: BrakePhase::Idle,
            skipped: None,
        }
    }

    fn record(&mut self, t_ms: f64, value: f64, phase: BrakePhase) {
        let point = [t_ms, value];
        if self.should_keep(point, phase) {
            if let Some(prev) = self.skipped.take() {
                self.points.push(prev);
            }
            self.points.push(point);
        } else {
            self.skipped = Some(point);
        }
        self.extend_span(t_ms, phase);
        self.last_phase = phase;
    }

    fn should_keep(&self, [t_ms, value]: [f64; 2], phase: BrakePhase) -> bool {
        let Some(&[last_t, last_v]) = self.points.last() else {
            return true;
        };
        phase != self.last_phase
            || t_ms - last_t >= POINT_MAX_GAP_MS
            || (value - last_v).abs() >= POINT_MIN_DELTA
    }

    fn extend_span(&mut self, t_ms: f64, phase: BrakePhase) {
        if phase == BrakePhase::Idle {
            return;
        }
        match self.spans.last_mut() {
            Some(span) if span.phase == phase && phase == self.last_phase => span.end_ms = t_ms,
            _ => self.spans.push(PhaseSpan {
                phase,
                start_ms: t_ms - SIM_STEP_MS,
                end_ms: t_ms,
            }),
        }
    }

    fn finish(mut self, release_ms: f64) -> BrakeTimeline {
        if let Some(last) = self.skipped.take() {
            self.points.push(last);
        }
        BrakeTimeline {
            points: self.points,
            spans: self.spans,
            release_ms,
        }
    }
}
