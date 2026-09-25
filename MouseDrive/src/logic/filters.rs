use std::f64::consts::PI;

pub const MAX_SMOOTHING_MS: f64 = 200.0;

pub fn ema_alpha(dt_ms: f64, tau_ms: f64) -> f64 {
    if tau_ms <= 0.0 {
        return 1.0;
    }
    if dt_ms <= 0.0 {
        return 0.0;
    }
    1.0 - (-dt_ms / tau_ms).exp()
}

pub fn tau_from_legacy_alpha(alpha: f64, interval_ms: f64) -> f64 {
    if !alpha.is_finite() || alpha >= 1.0 {
        return 0.0;
    }
    if alpha <= 0.0 {
        return MAX_SMOOTHING_MS;
    }
    (-interval_ms.max(1.0) / (1.0 - alpha).ln()).clamp(0.0, MAX_SMOOTHING_MS)
}

const DERIVATIVE_CUTOFF_HZ: f64 = 1.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OneEuro {
    x_hat: f64,
    dx_hat: f64,
    initialized: bool,
}

impl OneEuro {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn filter(&mut self, x: f64, dt_s: f64, min_cutoff_hz: f64, beta: f64) -> f64 {
        if !self.initialized {
            *self = Self {
                x_hat: x,
                dx_hat: 0.0,
                initialized: true,
            };
            return x;
        }
        if dt_s <= 0.0 {
            return self.x_hat;
        }
        let dx = (x - self.x_hat) / dt_s;
        self.dx_hat += smoothing_factor(dt_s, DERIVATIVE_CUTOFF_HZ) * (dx - self.dx_hat);
        let cutoff = min_cutoff_hz.max(1e-3) + beta.max(0.0) * self.dx_hat.abs();
        self.x_hat += smoothing_factor(dt_s, cutoff) * (x - self.x_hat);
        self.x_hat
    }
}

fn smoothing_factor(dt_s: f64, cutoff_hz: f64) -> f64 {
    let tau = 1.0 / (2.0 * PI * cutoff_hz);
    1.0 / (1.0 + tau / dt_s)
}
