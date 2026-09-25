#![deny(unsafe_code)]

use serde::{Deserialize, Serialize};

pub const MAX_POINTS: usize = 8;
pub const MIN_T_SPACING: f64 = 0.02;
pub const MIN_V_SPACING: f64 = 0.005;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CurveMode {
    Linear,
    Smooth,
}

impl CurveMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::Smooth,
            _ => Self::Linear,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum CurvePreset {
    Linear,
    SCurve,
    Aggressive,
    Progressive,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(default)]
pub struct Curve {
    pub mode: i32,
    pub points: Vec<[f64; 2]>,
}

impl Default for Curve {
    fn default() -> Self {
        Self {
            mode: 0,
            points: vec![[0.0, 0.0], [1.0, 1.0]],
        }
    }
}

impl Curve {
    pub fn eval(&self, t: f64) -> f64 {
        let n = self.points.len().min(MAX_POINTS);
        if n < 2 {
            return t.clamp(0.0, 1.0);
        }
        let t = t.clamp(0.0, 1.0);

        let mut i = 0;
        while i + 2 < n && t > self.points[i + 1][0] {
            i += 1;
        }
        let [x0, y0] = self.points[i];
        let [x1, y1] = self.points[i + 1];
        let h = (x1 - x0).max(1e-9);
        let u = ((t - x0) / h).clamp(0.0, 1.0);

        match CurveMode::from_i32(self.mode) {
            CurveMode::Linear => y0 + (y1 - y0) * u,
            CurveMode::Smooth => {
                let m = self.tangents();
                let (m0, m1) = (m[i], m[i + 1]);
                let u2 = u * u;
                let u3 = u2 * u;
                let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
                let h10 = u3 - 2.0 * u2 + u;
                let h01 = -2.0 * u3 + 3.0 * u2;
                let h11 = u3 - u2;
                (h00 * y0 + h10 * h * m0 + h01 * y1 + h11 * h * m1).clamp(0.0, 1.0)
            }
        }
    }

    pub fn inverse_eval(&self, v: f64) -> f64 {
        let n = self.points.len().min(MAX_POINTS);
        if n < 2 {
            return v.clamp(0.0, 1.0);
        }
        let v = v.clamp(0.0, 1.0);

        let mut i = 0;
        while i + 2 < n && v > self.points[i + 1][1] {
            i += 1;
        }
        let [x0, y0] = self.points[i];
        let [x1, y1] = self.points[i + 1];

        match CurveMode::from_i32(self.mode) {
            CurveMode::Linear => {
                let dy = (y1 - y0).max(1e-9);
                x0 + (v - y0).clamp(0.0, dy) / dy * (x1 - x0)
            }
            CurveMode::Smooth => {
                let (mut lo, mut hi) = (x0, x1);
                for _ in 0..40 {
                    let mid = (lo + hi) * 0.5;
                    if self.eval(mid) < v {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                    if hi - lo < 1e-9 {
                        break;
                    }
                }
                (lo + hi) * 0.5
            }
        }
    }

    pub fn is_identity(&self) -> bool {
        self.points.len() == 2 && self.points[0] == [0.0, 0.0] && self.points[1] == [1.0, 1.0]
    }

    fn tangents(&self) -> [f64; MAX_POINTS] {
        let n = self.points.len().min(MAX_POINTS);
        let mut d = [0.0; MAX_POINTS];
        for (dk, w) in d.iter_mut().zip(self.points.windows(2)).take(n - 1) {
            let dx = (w[1][0] - w[0][0]).max(1e-9);
            *dk = (w[1][1] - w[0][1]) / dx;
        }
        let mut m = [0.0; MAX_POINTS];
        m[0] = d[0];
        m[n - 1] = d[n - 2];
        for k in 1..n - 1 {
            m[k] = if d[k - 1] * d[k] <= 0.0 {
                0.0
            } else {
                (d[k - 1] + d[k]) * 0.5
            };
        }
        for k in 0..n - 1 {
            if d[k].abs() < 1e-12 {
                m[k] = 0.0;
                m[k + 1] = 0.0;
            } else {
                let a = m[k] / d[k];
                let b = m[k + 1] / d[k];
                let s = a * a + b * b;
                if s > 9.0 {
                    let tau = 3.0 / s.sqrt();
                    m[k] = tau * a * d[k];
                    m[k + 1] = tau * b * d[k];
                }
            }
        }
        m
    }

    pub fn validate(&mut self) -> u32 {
        let mode = self.fix_mode();
        if self.is_broken() {
            self.points = Curve::default().points;
            return mode + 1;
        }
        mode + self.drop_extra_points()
            + self.clamp_coordinates()
            + self.sort_by_t()
            + self.pin_ends()
            + self.enforce_t_spacing()
            + self.enforce_v_monotone()
    }

    fn fix_mode(&mut self) -> u32 {
        if (0..=1).contains(&self.mode) {
            return 0;
        }
        self.mode = self.mode.clamp(0, 1);
        1
    }

    fn is_broken(&self) -> bool {
        self.points.len() < 2
            || self
                .points
                .iter()
                .any(|p| !p[0].is_finite() || !p[1].is_finite())
    }

    fn drop_extra_points(&mut self) -> u32 {
        let extra = self.points.len().saturating_sub(MAX_POINTS);
        for _ in 0..extra {
            let idx = self.points.len() - 2;
            self.points.remove(idx);
        }
        extra as u32
    }

    fn clamp_coordinates(&mut self) -> u32 {
        let mut corrected = 0;
        for p in &mut self.points {
            let c = [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)];
            if c != *p {
                *p = c;
                corrected += 1;
            }
        }
        corrected
    }

    fn sort_by_t(&mut self) -> u32 {
        if self.points.windows(2).all(|w| w[0][0] <= w[1][0]) {
            return 0;
        }
        self.points
            .sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap_or(std::cmp::Ordering::Equal));
        1
    }

    fn pin_ends(&mut self) -> u32 {
        let last = self.points.len() - 1;
        let mut corrected = 0;
        for (idx, end) in [(0, [0.0, 0.0]), (last, [1.0, 1.0])] {
            if self.points[idx] != end {
                self.points[idx] = end;
                corrected += 1;
            }
        }
        corrected
    }

    fn enforce_t_spacing(&mut self) -> u32 {
        let mut corrected = 0;
        let mut i = 1;
        while i < self.points.len() - 1 {
            let remaining_right = self.points.len() - 1 - i;
            let too_left = self.points[i][0] < self.points[i - 1][0] + MIN_T_SPACING;
            let too_right = self.points[i][0] > 1.0 - MIN_T_SPACING * remaining_right as f64;
            if too_left || too_right {
                self.points.remove(i);
                corrected += 1;
            } else {
                i += 1;
            }
        }
        corrected
    }

    fn enforce_v_monotone(&mut self) -> u32 {
        let mut corrected = 0;
        let mut i = 1;
        while i < self.points.len() - 1 {
            let last = self.points.len() - 1;
            let lo = self.points[i - 1][1] + MIN_V_SPACING;
            let hi = 1.0 - MIN_V_SPACING * (last - i) as f64;
            if lo > hi {
                self.points.remove(i);
                corrected += 1;
                continue;
            }
            let v = self.points[i][1].clamp(lo, hi);
            if v != self.points[i][1] {
                self.points[i][1] = v;
                corrected += 1;
            }
            i += 1;
        }
        corrected
    }

    fn point_bounds(&self, i: usize) -> Option<(f64, f64, f64, f64)> {
        if i == 0 || i + 1 >= self.points.len() {
            return None;
        }
        let (prev, next) = (self.points[i - 1], self.points[i + 1]);
        let b = (
            prev[0] + MIN_T_SPACING,
            next[0] - MIN_T_SPACING,
            prev[1] + MIN_V_SPACING,
            next[1] - MIN_V_SPACING,
        );
        (b.0 <= b.1 && b.2 <= b.3).then_some(b)
    }

    pub fn move_point(&mut self, i: usize, t: f64, v: f64) -> bool {
        let Some((t_lo, t_hi, v_lo, v_hi)) = self.point_bounds(i) else {
            return false;
        };
        if !t.is_finite() || !v.is_finite() {
            return false;
        }
        let next = [t.clamp(t_lo, t_hi), v.clamp(v_lo, v_hi)];
        if self.points[i] == next {
            return false;
        }
        self.points[i] = next;
        true
    }

    pub fn insert_point(&mut self, t: f64, v: f64) -> Option<usize> {
        let n = self.points.len();
        if !(2..MAX_POINTS).contains(&n) || !t.is_finite() || !v.is_finite() {
            return None;
        }
        let idx = self
            .points
            .iter()
            .position(|p| p[0] > t)
            .unwrap_or(n - 1)
            .max(1);
        let (prev, next) = (self.points[idx - 1], self.points[idx]);
        let (t_lo, t_hi) = (prev[0] + MIN_T_SPACING, next[0] - MIN_T_SPACING);
        let (v_lo, v_hi) = (prev[1] + MIN_V_SPACING, next[1] - MIN_V_SPACING);
        if t_lo > t_hi || v_lo > v_hi {
            return None;
        }
        self.points
            .insert(idx, [t.clamp(t_lo, t_hi), v.clamp(v_lo, v_hi)]);
        Some(idx)
    }

    pub fn remove_point(&mut self, i: usize) -> bool {
        if i == 0 || i + 1 >= self.points.len() {
            return false;
        }
        self.points.remove(i);
        true
    }

    pub fn preset(p: CurvePreset) -> Self {
        match p {
            CurvePreset::Linear => Self::default(),
            CurvePreset::SCurve => Self {
                mode: 1,
                points: vec![[0.0, 0.0], [0.25, 0.1], [0.75, 0.9], [1.0, 1.0]],
            },
            CurvePreset::Aggressive => Self {
                mode: 1,
                points: vec![[0.0, 0.0], [0.2, 0.55], [0.5, 0.85], [1.0, 1.0]],
            },
            CurvePreset::Progressive => Self {
                mode: 1,
                points: vec![[0.0, 0.0], [0.5, 0.15], [0.8, 0.5], [1.0, 1.0]],
            },
        }
    }
}
