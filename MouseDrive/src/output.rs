#![deny(unsafe_code)]

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputFrame {
    pub steering: f64,
    pub throttle: f64,
    pub brake: f64,
    pub buttons: u32,
}

pub const TEST_PATTERN_SPAN: u32 = 32767;

pub const BUTTON_GEAR_UP: u32 = 1 << 0;
pub const BUTTON_GEAR_DOWN: u32 = 1 << 1;

impl OutputFrame {
    pub const NEUTRAL: Self = Self {
        steering: 0.0,
        throttle: 0.0,
        brake: 0.0,
        buttons: 0,
    };

    pub fn test_pattern(counter: u64) -> Self {
        let step = counter % (u64::from(TEST_PATTERN_SPAN) + 1);
        let k = step as f64 / f64::from(TEST_PATTERN_SPAN);
        Self {
            steering: 2.0 * k - 1.0,
            throttle: k,
            brake: k,
            buttons: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisRange {
    pub min: i32,
    pub max: i32,
}

impl AxisRange {
    pub const DEFAULT: Self = Self { min: 0, max: 32767 };

    pub fn new(min: i32, max: i32) -> Option<Self> {
        (max > min).then_some(Self { min, max })
    }

    pub fn center(self) -> i32 {
        let sum = i64::from(self.min) + i64::from(self.max) + 1;
        (sum / 2) as i32
    }

    pub fn map_unit(self, v: f64) -> i32 {
        let v = if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let span = (i64::from(self.max) - i64::from(self.min)) as f64;
        (i64::from(self.min) + (v * span).round() as i64) as i32
    }

    pub fn map_signed(self, v: f64) -> i32 {
        let v = if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.map_unit((v + 1.0) / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisRanges {
    pub steering: AxisRange,
    pub throttle: AxisRange,
    pub brake: AxisRange,
}

impl Default for AxisRanges {
    fn default() -> Self {
        Self {
            steering: AxisRange::DEFAULT,
            throttle: AxisRange::DEFAULT,
            brake: AxisRange::DEFAULT,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteError;

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("output write failed")
    }
}

impl std::error::Error for WriteError {}

pub trait OutputBackend {
    fn write(&mut self, frame: &OutputFrame) -> Result<(), WriteError>;

    fn describe(&self) -> String;
}

pub const LOST_AFTER_FAILURES: u32 = 25;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteHealth {
    pub ok_writes: u64,
    pub failed_writes: u64,
    pub consecutive_failures: u32,
}

impl WriteHealth {
    pub fn record(&mut self, result: Result<(), WriteError>) {
        match result {
            Ok(()) => {
                self.ok_writes += 1;
                self.consecutive_failures = 0;
            }
            Err(WriteError) => {
                self.failed_writes += 1;
                self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            }
        }
    }

    pub fn is_lost(&self) -> bool {
        self.consecutive_failures >= LOST_AFTER_FAILURES
    }
}

#[derive(Debug, Default)]
pub struct MockBackend {
    pub frames: Vec<OutputFrame>,
    pub fail_next: u32,
}

impl OutputBackend for MockBackend {
    fn write(&mut self, frame: &OutputFrame) -> Result<(), WriteError> {
        if self.fail_next > 0 {
            self.fail_next -= 1;
            return Err(WriteError);
        }
        self.frames.push(*frame);
        Ok(())
    }

    fn describe(&self) -> String {
        "mock".to_string()
    }
}
