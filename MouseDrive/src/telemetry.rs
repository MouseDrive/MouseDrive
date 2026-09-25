#![deny(unsafe_code)]

use crate::logic::BrakePhase;

pub const DEFAULT_CAPACITY: usize = 250 * 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Marker {
    #[default]
    None,
    CaptureOn,
    CaptureOff,
    ProfileChanged,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub seq: u64,
    pub t_ms: f64,
    pub counts_x: i32,
    pub steering_raw: f32,
    pub steering_out: f32,
    pub throttle_pressed: bool,
    pub throttle_target: f32,
    pub throttle_out: f32,
    pub brake_pressed: bool,
    pub brake_floor: f32,
    pub brake_out: f32,
    pub brake_phase: BrakePhase,
    pub capture: bool,
    pub marker: Marker,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            seq: 0,
            t_ms: 0.0,
            counts_x: 0,
            steering_raw: 0.0,
            steering_out: 0.0,
            throttle_pressed: false,
            throttle_target: 0.0,
            throttle_out: 0.0,
            brake_pressed: false,
            brake_floor: 0.0,
            brake_out: 0.0,
            brake_phase: BrakePhase::Idle,
            capture: false,
            marker: Marker::None,
        }
    }
}

pub struct TelemetryRing {
    buf: Vec<Sample>,
    next: usize,
    len: usize,
    next_seq: u64,
}

impl TelemetryRing {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: vec![Sample::default(); capacity.max(1)],
            next: 0,
            len: 0,
            next_seq: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, sample: Sample) -> u64 {
        let seq = self.next_seq;
        self.buf[self.next] = Sample { seq, ..sample };
        self.next = (self.next + 1) % self.buf.len();
        self.len = (self.len + 1).min(self.buf.len());
        self.next_seq += 1;
        seq
    }

    pub fn latest_seq(&self) -> Option<u64> {
        self.next_seq.checked_sub(1).filter(|_| self.len > 0)
    }

    pub fn copy_since(&self, after: Option<u64>, out: &mut Vec<Sample>) {
        let cap = self.buf.len();
        let oldest_idx = (self.next + cap - self.len) % cap;
        let oldest_seq = self.next_seq - self.len as u64;
        let skip = match after {
            None => 0,
            Some(seq) => (seq + 1).saturating_sub(oldest_seq).min(self.len as u64) as usize,
        };
        out.extend((skip..self.len).map(|i| self.buf[(oldest_idx + i) % cap]));
    }
}

pub fn decimate_min_max(
    samples: &[Sample],
    t0: f64,
    t1: f64,
    columns: usize,
    value: impl Fn(&Sample) -> f32,
) -> Vec<Option<(f32, f32)>> {
    let mut out = vec![None; columns];
    if columns == 0 || t1 <= t0 {
        return out;
    }
    let scale = columns as f64 / (t1 - t0);
    for s in samples.iter().filter(|s| s.t_ms >= t0 && s.t_ms <= t1) {
        let col = (((s.t_ms - t0) * scale) as usize).min(columns - 1);
        let v = value(s);
        out[col] = Some(match out[col] {
            None => (v, v),
            Some((lo, hi)) => (lo.min(v), hi.max(v)),
        });
    }
    out
}
