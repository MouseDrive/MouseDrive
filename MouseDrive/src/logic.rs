#![deny(unsafe_code)]

use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::config::Config;
use crate::input::{LEFT_BUTTON, MOUSE_DELTA_X, RIGHT_BUTTON};

pub const STEERING_RANGE: f64 = 16383.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SteeringMode {
    Linear,
    Expo,
    Filtered,
    SelfCenter,
}

impl SteeringMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Expo,
            2 => Self::Filtered,
            3 => Self::SelfCenter,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BrakeState {
    Idle,
    Press,
    PostHold,
    ReleaseHold,
    Release,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RampDir {
    Hold,
    Rising,
    Falling,
}

pub struct MouseDriveState {
    pub steering: f64,
    pub steering_filtered: f64,
    pub throttle: f64,
    pub throttle_target: f64,
    pub throttle_phase: f64,
    pub throttle_dir: RampDir,
    pub brake: f64,
    pub brake_state: BrakeState,
    pub brake_apply_phase: f64,
    pub brake_posthold_phase: f64,
    pub brake_press_start: Instant,
    pub brake_post_hold_start: Instant,
    pub brake_post_hold_start_value: f64,
    pub brake_release_hold_start: Instant,
    pub last_update: Instant,
    pub capture_enabled: bool,
    pub capture_key_prev: bool,
    pub w_key_pressed: bool,
    pub s_key_pressed: bool,
}

impl MouseDriveState {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            brake_press_start: now,
            brake_post_hold_start: now,
            brake_post_hold_start_value: 1.0,
            brake_release_hold_start: now,
            last_update: now,
            capture_enabled: true,
            steering: 0.0,
            steering_filtered: 0.0,
            throttle: 0.0,
            throttle_target: 0.0,
            throttle_phase: 0.0,
            throttle_dir: RampDir::Hold,
            brake: 0.0,
            brake_state: BrakeState::Idle,
            brake_apply_phase: 0.0,
            brake_posthold_phase: 0.0,
            capture_key_prev: false,
            w_key_pressed: false,
            s_key_pressed: false,
        }
    }

    pub fn update_steering(&mut self, config: &Config, delta_ms: f64) {
        let dx = MOUSE_DELTA_X.swap(0, Ordering::Relaxed) as f64;
        self.steering += dx * config.mouse_sens;
        self.steering = self.steering.clamp(-STEERING_RANGE, STEERING_RANGE);

        let mode = SteeringMode::from_i32(config.steering_mode);

        if mode == SteeringMode::SelfCenter {
            let k = config.steering_spring_strength.clamp(0.0, 5.0);
            let factor = (1.0 - (-k * (delta_ms / 1000.0)).exp()).clamp(0.0, 1.0);
            self.steering -= self.steering * factor;
        }

        let sat_ratio = config.steering_saturation.clamp(0.5, 1.0);
        let sat_range = STEERING_RANGE * sat_ratio;
        self.steering = self.steering.clamp(-sat_range, sat_range);

        let dz = config.steering_deadzone.clamp(0.0, 0.5) * STEERING_RANGE;
        let abs_steer = self.steering.abs();
        let sign = self.steering.signum();

        let after_dz = if abs_steer <= dz {
            0.0
        } else {
            (abs_steer - dz) * (sat_range / (sat_range - dz).max(1.0))
        };

        let shaped = match mode {
            SteeringMode::Expo => {
                let norm = (after_dz / sat_range).clamp(0.0, 1.0);
                sign * norm.powf(config.steering_expo) * sat_range
            }
            SteeringMode::Filtered => {
                let alpha = config.steering_filter_alpha.clamp(0.0, 1.0);
                let target = sign * after_dz;
                self.steering_filtered + (target - self.steering_filtered) * alpha
            }
            _ => sign * after_dz,
        };

        self.steering_filtered = shaped.clamp(-sat_range, sat_range);
    }

    pub fn update_throttle(&mut self, config: &Config, time_scale: f64) {
        let start_cut = config.throttle_cut_start.clamp(0.0, 0.99);
        let max_cut = config.throttle_cut_max.clamp(start_cut + 0.01, 1.0);

        if LEFT_BUTTON.load(Ordering::Acquire) {
            let effective_ratio = self.steering_filtered.abs() / (STEERING_RANGE * 0.30);

            let modf = if effective_ratio <= start_cut {
                1.0
            } else if effective_ratio >= max_cut {
                config.throttle_min_cut_at_full
            } else {
                let normalized =
                    ((effective_ratio - start_cut) / (max_cut - start_cut)).clamp(0.0, 1.0);
                let shaped = normalized.powf(config.throttle_curve_exp);
                1.0 - shaped * (1.0 - config.throttle_min_cut_at_full)
            };
            self.throttle_target = modf;
        } else {
            self.throttle_target = 0.0;
        }

        let dt_ms = config.thread_interval_ms.max(1) as f64 * time_scale;
        let desired = self.throttle_target;
        self.ramp_toward(config, desired, dt_ms);
    }

    fn ramp_toward(&mut self, config: &Config, desired: f64, dt_ms: f64) {
        const EPS: f64 = 1e-6;

        if desired > self.throttle + EPS {
            let rise = &config.throttle_rise_curve;
            if self.throttle_dir != RampDir::Rising {
                self.throttle_phase = rise.inverse_eval(self.throttle);
                self.throttle_dir = RampDir::Rising;
            }
            self.throttle_phase =
                (self.throttle_phase + dt_ms / config.throttle_ramp_ms.max(1) as f64).min(1.0);
            let mut y = rise.eval(self.throttle_phase);
            if y >= desired {
                y = desired;
                self.throttle_phase = rise.inverse_eval(desired);
            }
            self.throttle = y;
        } else if desired < self.throttle - EPS {
            let fall = &config.throttle_fall_curve;
            if self.throttle_dir != RampDir::Falling {
                self.throttle_phase = fall.inverse_eval(self.throttle);
                self.throttle_dir = RampDir::Falling;
            }
            self.throttle_phase =
                (self.throttle_phase - dt_ms / config.throttle_drop_ms.max(1) as f64).max(0.0);
            let mut y = fall.eval(self.throttle_phase);
            if y <= desired {
                y = desired;
                self.throttle_phase = fall.inverse_eval(desired);
            }
            self.throttle = y;
        } else {
            self.throttle = desired;
            self.throttle_dir = RampDir::Hold;
        }
        self.throttle = self.throttle.clamp(0.0, 1.0);
    }

    pub fn update_brake(&mut self, config: &Config, now: Instant, time_scale: f64) {
        let dyn_min = if config.brake_trail_enabled {
            let shaped = (self.steering_filtered.abs() / STEERING_RANGE)
                .clamp(0.0, 1.0)
                .powf(config.brake_curve_exp);
            config.brake_min_ratio_base
                + (config.brake_min_ratio_max - config.brake_min_ratio_base) * shaped
        } else {
            config.brake_min_ratio_base
        };

        let dt_ms = config.thread_interval_ms.max(1) as f64 * time_scale;
        let right_pressed = RIGHT_BUTTON.load(Ordering::Acquire);

        if right_pressed {
            if matches!(
                self.brake_state,
                BrakeState::Idle | BrakeState::Release | BrakeState::ReleaseHold
            ) {
                self.brake_state = BrakeState::Press;
                self.brake_press_start = now;
                self.brake_apply_phase = config.brake_apply_curve.inverse_eval(self.brake);
            }

            let elapsed_ms = now.duration_since(self.brake_press_start).as_millis() as i32;

            if elapsed_ms < config.brake_hold_ms {
                self.brake_apply_phase = (self.brake_apply_phase
                    + dt_ms / config.brake_fast_apply_ms.max(1) as f64)
                    .min(1.0);
                self.brake = config.brake_apply_curve.eval(self.brake_apply_phase);
            } else {
                if self.brake_state == BrakeState::Press {
                    self.brake_state = BrakeState::PostHold;
                    self.brake_post_hold_start = now;
                    self.brake_post_hold_start_value = self.brake;
                }

                let t_rel = now.duration_since(self.brake_post_hold_start).as_millis() as f64;
                let progress = (t_rel / config.brake_release_total_ms as f64).clamp(0.0, 1.0);
                let pre = progress.powf(config.brake_release_accel_exp);
                self.brake_posthold_phase = pre;
                let shaped = config.brake_posthold_curve.eval(pre);
                let target = self.brake_post_hold_start_value
                    - shaped * (self.brake_post_hold_start_value - dyn_min);
                self.brake = target.max(dyn_min);
            }
        } else {
            if matches!(self.brake_state, BrakeState::Press | BrakeState::PostHold) {
                let press_elapsed = now.duration_since(self.brake_press_start).as_millis() as i32;
                if press_elapsed < config.brake_tap_ms {
                    self.brake_state = BrakeState::Release;
                } else {
                    self.brake_state = BrakeState::ReleaseHold;
                    self.brake_release_hold_start = now;
                    self.brake = config.brake_after_release_hold_ratio;
                }
            }

            if self.brake_state == BrakeState::ReleaseHold {
                let rel_elapsed = now
                    .duration_since(self.brake_release_hold_start)
                    .as_millis() as i32;
                if rel_elapsed >= config.brake_after_release_hold_ms {
                    self.brake_state = BrakeState::Release;
                } else {
                    self.brake = config.brake_after_release_hold_ratio;
                }
            }

            if self.brake_state == BrakeState::Release {
                self.brake =
                    (self.brake - dt_ms / config.brake_fast_release_ms.max(1) as f64).max(0.0);
                if self.brake <= 0.0 {
                    self.brake = 0.0;
                    self.brake_state = BrakeState::Idle;
                }
            }

            if self.brake_state == BrakeState::Idle {
                self.brake = 0.0;
            }
        }

        self.brake = self.brake.clamp(0.0, 1.0);
    }
}
