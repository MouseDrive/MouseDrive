#![deny(unsafe_code)]

use crate::output::{BUTTON_GEAR_DOWN, BUTTON_GEAR_UP, OutputFrame};

pub const COUNTDOWN_MS: f64 = 5000.0;
pub const SWEEP_MS: f64 = 4000.0;
const CYCLE_MS: f64 = 2000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindTarget {
    Steering,
    Throttle,
    Brake,
    GearUp,
    GearDown,
}

impl BindTarget {
    pub const ALL: [BindTarget; 5] = [
        BindTarget::Steering,
        BindTarget::Throttle,
        BindTarget::Brake,
        BindTarget::GearUp,
        BindTarget::GearDown,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BindPhase {
    Countdown(f64),
    Sweep(f64),
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BindHelper {
    pub target: BindTarget,
    elapsed_ms: f64,
}

impl BindHelper {
    pub fn new(target: BindTarget) -> Self {
        Self {
            target,
            elapsed_ms: 0.0,
        }
    }

    pub fn advance(&mut self, dt_ms: f64) {
        if dt_ms.is_finite() && dt_ms > 0.0 {
            self.elapsed_ms += dt_ms;
        }
    }

    pub fn phase(&self) -> BindPhase {
        if self.elapsed_ms < COUNTDOWN_MS {
            BindPhase::Countdown(COUNTDOWN_MS - self.elapsed_ms)
        } else if self.elapsed_ms < COUNTDOWN_MS + SWEEP_MS {
            BindPhase::Sweep((self.elapsed_ms - COUNTDOWN_MS) / SWEEP_MS)
        } else {
            BindPhase::Done
        }
    }

    pub fn frame(&self) -> OutputFrame {
        let BindPhase::Sweep(_) = self.phase() else {
            return OutputFrame::NEUTRAL;
        };
        let t = (self.elapsed_ms - COUNTDOWN_MS) % CYCLE_MS / CYCLE_MS;
        let tri = 1.0 - (2.0 * t - 1.0).abs();
        let pulse = t < 0.5;
        let base = OutputFrame::NEUTRAL;
        match self.target {
            BindTarget::Steering => OutputFrame {
                steering: (2.0 * std::f64::consts::PI * t).sin(),
                ..base
            },
            BindTarget::Throttle => OutputFrame {
                throttle: tri,
                ..base
            },
            BindTarget::Brake => OutputFrame { brake: tri, ..base },
            BindTarget::GearUp => OutputFrame {
                buttons: if pulse { BUTTON_GEAR_UP } else { 0 },
                ..base
            },
            BindTarget::GearDown => OutputFrame {
                buttons: if pulse { BUTTON_GEAR_DOWN } else { 0 },
                ..base
            },
        }
    }
}
