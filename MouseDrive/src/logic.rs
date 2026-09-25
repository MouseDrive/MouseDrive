#![deny(unsafe_code)]

pub mod filters;

use crate::config::{Config, Tuning};
use crate::output::OutputFrame;
use filters::{OneEuro, ema_alpha};

pub const STEERING_RANGE: f64 = 16383.0;

pub const MAX_TICK_DT_MS: f64 = 50.0;

pub const THROTTLE_CUT_REFERENCE: f64 = 0.30;

pub const COUNT_ACCUM_LIMIT: i64 = 1 << 24;

pub fn accumulate_counts(acc: i64, dx: i32) -> i64 {
    acc.saturating_add(i64::from(dx))
        .clamp(-COUNT_ACCUM_LIMIT, COUNT_ACCUM_LIMIT)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SteeringMode {
    Linear,
    Expo,
    Filtered,
    SelfCenter,
    Adaptive,
}

impl SteeringMode {
    pub const ALL: [SteeringMode; 5] = [
        SteeringMode::Linear,
        SteeringMode::Expo,
        SteeringMode::Filtered,
        SteeringMode::SelfCenter,
        SteeringMode::Adaptive,
    ];

    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Expo,
            2 => Self::Filtered,
            3 => Self::SelfCenter,
            4 => Self::Adaptive,
            _ => Self::Linear,
        }
    }

    pub fn to_i32(self) -> i32 {
        match self {
            Self::Linear => 0,
            Self::Expo => 1,
            Self::Filtered => 2,
            Self::SelfCenter => 3,
            Self::Adaptive => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrakeState {
    Idle,
    Press,
    PostHold,
    ReleaseHold,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrakePhase {
    Idle,
    Fill,
    Full,
    Decay,
    ShortHold,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RampDir {
    Hold,
    Rising,
    Falling,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TickInput {
    pub counts_x: i64,
    pub throttle_pressed: bool,
    pub brake_pressed: bool,
    pub dt_ms: f64,
}

pub fn throttle_cut_bounds(t: &Tuning) -> (f64, f64) {
    let start = t.throttle_cut_start.clamp(0.0, 0.99);
    let max = t.throttle_cut_max.clamp(start + 0.01, 1.0);
    (start, max)
}

pub fn throttle_cut_factor(t: &Tuning, steer_abs: f64) -> f64 {
    if !t.throttle_cut_enabled {
        return 1.0;
    }
    let (start, max) = throttle_cut_bounds(t);
    let ratio = steer_abs / THROTTLE_CUT_REFERENCE;
    if ratio <= start {
        1.0
    } else if ratio >= max {
        t.throttle_min_cut_at_full
    } else {
        let n = ((ratio - start) / (max - start)).clamp(0.0, 1.0);
        1.0 - n.powf(t.throttle_curve_exp) * (1.0 - t.throttle_min_cut_at_full)
    }
}

pub fn brake_floor(t: &Tuning, steer_abs: f64) -> f64 {
    if !t.brake_trail_enabled {
        return t.brake_min_ratio_base;
    }
    let shaped = steer_abs.clamp(0.0, 1.0).powf(t.brake_curve_exp);
    t.brake_min_ratio_base + (t.brake_min_ratio_max - t.brake_min_ratio_base) * shaped
}

const DEADZONE_MAX_SHARE: f64 = 0.9;

fn decay_shape_changed(old: &Tuning, new: &Tuning) -> bool {
    old.brake_posthold_curve != new.brake_posthold_curve
        || old.brake_release_total_ms != new.brake_release_total_ms
        || old.brake_release_accel_exp != new.brake_release_accel_exp
}

fn apply_deadzone(steering: f64, deadzone: f64, sat_range: f64) -> f64 {
    let dz = (deadzone.clamp(0.0, 0.5) * STEERING_RANGE).min(sat_range * DEADZONE_MAX_SHARE);
    let abs = steering.abs();
    if abs <= dz {
        0.0
    } else {
        steering.signum() * (abs - dz) * (sat_range / (sat_range - dz).max(1.0))
    }
}

#[derive(Clone, Debug)]
pub struct MouseDriveState {
    pub steering: f64,
    pub steering_filtered: f64,
    adaptive: OneEuro,
    pub clipped_ticks: u64,

    pub throttle: f64,
    pub throttle_target: f64,
    pub throttle_phase: f64,
    pub throttle_dir: RampDir,

    pub brake: f64,
    pub brake_state: BrakeState,
    pub brake_floor: f64,
    pub brake_apply_phase: f64,
    pub brake_posthold_phase: f64,
    brake_press_start_ms: f64,
    brake_post_hold_start_ms: f64,
    brake_post_hold_start_value: f64,
    brake_release_hold_start_ms: f64,

    clock_ms: f64,
    pub last_input: TickInput,
}

impl Default for MouseDriveState {
    fn default() -> Self {
        Self::new()
    }
}

impl MouseDriveState {
    pub fn new() -> Self {
        Self {
            steering: 0.0,
            steering_filtered: 0.0,
            adaptive: OneEuro::default(),
            clipped_ticks: 0,
            throttle: 0.0,
            throttle_target: 0.0,
            throttle_phase: 0.0,
            throttle_dir: RampDir::Hold,
            brake: 0.0,
            brake_state: BrakeState::Idle,
            brake_floor: 0.0,
            brake_apply_phase: 0.0,
            brake_posthold_phase: 0.0,
            brake_press_start_ms: 0.0,
            brake_post_hold_start_ms: 0.0,
            brake_post_hold_start_value: 1.0,
            brake_release_hold_start_ms: 0.0,
            clock_ms: 0.0,
            last_input: TickInput::default(),
        }
    }

    pub fn tick(&mut self, config: &Config, input: TickInput) {
        let dt_ms = if input.dt_ms.is_finite() {
            input.dt_ms.clamp(0.0, MAX_TICK_DT_MS)
        } else {
            0.0
        };
        self.clock_ms += dt_ms;
        self.last_input = input;
        self.update_steering(config, input.counts_x, dt_ms);
        self.update_throttle(&config.tuning, input.throttle_pressed, dt_ms);
        self.update_brake(&config.tuning, input.brake_pressed, dt_ms);
    }

    pub fn reset(&mut self) {
        *self = Self {
            clipped_ticks: self.clipped_ticks,
            ..Self::new()
        };
    }

    pub fn reset_steering(&mut self) {
        self.steering = 0.0;
        self.steering_filtered = 0.0;
        self.adaptive.reset();
    }

    pub fn reseed_after_config_change(&mut self, old: &Tuning, new: &Tuning) {
        if old.throttle_rise_curve != new.throttle_rise_curve
            || old.throttle_fall_curve != new.throttle_fall_curve
        {
            self.throttle_dir = RampDir::Hold;
        }
        if old.brake_apply_curve != new.brake_apply_curve && self.brake_state == BrakeState::Press {
            self.brake_apply_phase = new.brake_apply_curve.inverse_eval(self.brake);
        }
        if self.brake_state == BrakeState::PostHold && decay_shape_changed(old, new) {
            self.brake_post_hold_start_ms = self.clock_ms;
            self.brake_post_hold_start_value = self.brake;
        }
    }

    pub fn pedals_idle(&self) -> bool {
        self.brake_state == BrakeState::Idle
            && !self.last_input.throttle_pressed
            && !self.last_input.brake_pressed
    }

    pub fn steering_output(&self) -> f64 {
        (self.steering_filtered / STEERING_RANGE).clamp(-1.0, 1.0)
    }

    fn steering_abs(&self) -> f64 {
        self.steering_output().abs()
    }

    pub fn brake_output(&self, t: &Tuning) -> f64 {
        (self.brake * t.brake_max_output).clamp(0.0, 1.0)
    }

    pub fn output_frame(&self, t: &Tuning, buttons: u32) -> OutputFrame {
        OutputFrame {
            steering: self.steering_output(),
            throttle: self.throttle.clamp(0.0, 1.0),
            brake: self.brake_output(t),
            buttons,
        }
    }

    pub fn brake_phase(&self) -> BrakePhase {
        match self.brake_state {
            BrakeState::Idle => BrakePhase::Idle,
            BrakeState::Press if self.brake_apply_phase < 1.0 => BrakePhase::Fill,
            BrakeState::Press => BrakePhase::Full,
            BrakeState::PostHold => BrakePhase::Decay,
            BrakeState::ReleaseHold => BrakePhase::ShortHold,
            BrakeState::Release => BrakePhase::Release,
        }
    }

    fn update_steering(&mut self, config: &Config, counts: i64, dt_ms: f64) {
        let t = &config.tuning;
        let raw = counts as f64 * config.mouse_dpi_scale * t.mouse_sens;
        let delta = self.limit_rate(raw, config.steering_max_rate_pct_s, dt_ms);
        self.steering = (self.steering + delta).clamp(-STEERING_RANGE, STEERING_RANGE);

        let mode = SteeringMode::from_i32(t.steering_mode);
        if mode == SteeringMode::SelfCenter {
            let k = t.steering_spring_strength.clamp(0.0, 5.0);
            let factor = (1.0 - (-k * (dt_ms / 1000.0)).exp()).clamp(0.0, 1.0);
            self.steering -= self.steering * factor;
        }
        if mode != SteeringMode::Adaptive {
            self.adaptive.reset();
        }

        let sat_range = STEERING_RANGE * t.steering_saturation.clamp(0.5, 1.0);
        self.steering = self.steering.clamp(-sat_range, sat_range);
        let target = apply_deadzone(self.steering, t.steering_deadzone, sat_range);
        let shaped = self.shape_steering(mode, t, target, sat_range, dt_ms);
        self.steering_filtered = shaped.clamp(-sat_range, sat_range);
    }

    fn limit_rate(&mut self, delta: f64, max_rate_pct_s: f64, dt_ms: f64) -> f64 {
        if max_rate_pct_s <= 0.0 {
            return delta;
        }
        let max_step = max_rate_pct_s / 100.0 * STEERING_RANGE * dt_ms.max(0.0) / 1000.0;
        if delta.abs() <= max_step {
            return delta;
        }
        self.clipped_ticks += 1;
        delta.signum() * max_step
    }

    fn shape_steering(
        &mut self,
        mode: SteeringMode,
        t: &Tuning,
        target: f64,
        sat_range: f64,
        dt_ms: f64,
    ) -> f64 {
        match mode {
            SteeringMode::Expo => {
                let norm = (target.abs() / sat_range).clamp(0.0, 1.0);
                target.signum() * norm.powf(t.steering_expo) * sat_range
            }
            SteeringMode::Filtered => {
                let alpha = ema_alpha(dt_ms, t.steering_smoothing_ms);
                self.steering_filtered + (target - self.steering_filtered) * alpha
            }
            SteeringMode::Adaptive => {
                let y = self.adaptive.filter(
                    target / STEERING_RANGE,
                    dt_ms / 1000.0,
                    t.steering_adaptive_min_cutoff_hz,
                    t.steering_adaptive_beta,
                );
                y * STEERING_RANGE
            }
            SteeringMode::Linear | SteeringMode::SelfCenter => target,
        }
    }

    fn update_throttle(&mut self, t: &Tuning, pressed: bool, dt_ms: f64) {
        self.throttle_target = if pressed {
            throttle_cut_factor(t, self.steering_abs())
        } else {
            0.0
        };
        self.ramp_toward(t, self.throttle_target, dt_ms);
    }

    fn ramp_toward(&mut self, t: &Tuning, desired: f64, dt_ms: f64) {
        const EPS: f64 = 1e-6;

        if desired > self.throttle + EPS {
            let rise = &t.throttle_rise_curve;
            if self.throttle_dir != RampDir::Rising {
                self.throttle_phase = rise.inverse_eval(self.throttle);
                self.throttle_dir = RampDir::Rising;
            }
            self.throttle_phase =
                (self.throttle_phase + dt_ms / f64::from(t.throttle_ramp_ms.max(1))).min(1.0);
            let y = rise.eval(self.throttle_phase);
            self.throttle = if y >= desired {
                self.throttle_phase = rise.inverse_eval(desired);
                desired
            } else {
                y
            };
        } else if desired < self.throttle - EPS {
            let fall = &t.throttle_fall_curve;
            if self.throttle_dir != RampDir::Falling {
                self.throttle_phase = fall.inverse_eval(self.throttle);
                self.throttle_dir = RampDir::Falling;
            }
            self.throttle_phase =
                (self.throttle_phase - dt_ms / f64::from(t.throttle_drop_ms.max(1))).max(0.0);
            let y = fall.eval(self.throttle_phase);
            self.throttle = if y <= desired {
                self.throttle_phase = fall.inverse_eval(desired);
                desired
            } else {
                y
            };
        } else {
            self.throttle = desired;
            self.throttle_dir = RampDir::Hold;
        }
        self.throttle = self.throttle.clamp(0.0, 1.0);
    }

    fn update_brake(&mut self, t: &Tuning, pressed: bool, dt_ms: f64) {
        self.brake_floor = brake_floor(t, self.steering_abs());
        if pressed {
            self.brake_pressed_tick(t, dt_ms);
        } else {
            self.brake_released_tick(t, dt_ms);
        }
        self.brake = self.brake.clamp(0.0, 1.0);
    }

    fn brake_pressed_tick(&mut self, t: &Tuning, dt_ms: f64) {
        let now = self.clock_ms;
        if matches!(
            self.brake_state,
            BrakeState::Idle | BrakeState::Release | BrakeState::ReleaseHold
        ) {
            self.brake_state = BrakeState::Press;
            self.brake_press_start_ms = now;
            self.brake_apply_phase = t.brake_apply_curve.inverse_eval(self.brake);
        }

        if now - self.brake_press_start_ms < f64::from(t.brake_hold_ms) {
            self.brake_apply_phase =
                (self.brake_apply_phase + dt_ms / f64::from(t.brake_fast_apply_ms.max(1))).min(1.0);
            self.brake = t.brake_apply_curve.eval(self.brake_apply_phase);
            return;
        }

        if self.brake_state == BrakeState::Press {
            self.brake_state = BrakeState::PostHold;
            self.brake_post_hold_start_ms = now;
            self.brake_post_hold_start_value = self.brake;
        }
        let t_rel = now - self.brake_post_hold_start_ms;
        let progress = (t_rel / f64::from(t.brake_release_total_ms.max(1))).clamp(0.0, 1.0);
        let pre = progress.powf(t.brake_release_accel_exp);
        self.brake_posthold_phase = pre;
        let shaped = t.brake_posthold_curve.eval(pre);
        let start = self.brake_post_hold_start_value;
        let floor = self.brake_floor;
        self.brake = (start - shaped * (start - floor)).max(floor);
    }

    fn brake_released_tick(&mut self, t: &Tuning, dt_ms: f64) {
        let now = self.clock_ms;
        if matches!(self.brake_state, BrakeState::Press | BrakeState::PostHold) {
            if now - self.brake_press_start_ms < f64::from(t.brake_tap_ms) {
                self.brake_state = BrakeState::Release;
            } else {
                self.brake_state = BrakeState::ReleaseHold;
                self.brake_release_hold_start_ms = now;
                self.brake = t.brake_after_release_hold_ratio;
            }
        }

        if self.brake_state == BrakeState::ReleaseHold {
            if now - self.brake_release_hold_start_ms >= f64::from(t.brake_after_release_hold_ms) {
                self.brake_state = BrakeState::Release;
            } else {
                self.brake = t.brake_after_release_hold_ratio;
            }
        }

        if self.brake_state == BrakeState::Release {
            self.brake = (self.brake - dt_ms / f64::from(t.brake_fast_release_ms.max(1))).max(0.0);
            if self.brake <= 0.0 {
                self.brake_state = BrakeState::Idle;
            }
        }

        if self.brake_state == BrakeState::Idle {
            self.brake = 0.0;
        }
    }
}
