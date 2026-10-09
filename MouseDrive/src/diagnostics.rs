#![deny(unsafe_code)]

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LoopSummary {
    pub hz: f64,
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub samples: usize,
}

pub struct LoopStats {
    periods: Vec<f64>,
    idx: usize,
    filled: usize,
    scratch: Vec<f64>,
}

impl LoopStats {
    pub fn new(window: usize) -> Self {
        let window = window.max(1);
        Self {
            periods: vec![0.0; window],
            idx: 0,
            filled: 0,
            scratch: Vec::with_capacity(window),
        }
    }

    pub fn clear(&mut self) {
        self.idx = 0;
        self.filled = 0;
    }

    pub fn record(&mut self, period_ms: f64) {
        if !period_ms.is_finite() || period_ms < 0.0 {
            return;
        }
        self.periods[self.idx] = period_ms;
        self.idx = (self.idx + 1) % self.periods.len();
        self.filled = (self.filled + 1).min(self.periods.len());
    }

    pub fn summary(&mut self) -> LoopSummary {
        let sorted = self.sorted();
        let Some(spread) = spread_of(sorted) else {
            return LoopSummary::default();
        };
        let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
        LoopSummary {
            hz: if mean > 0.0 { 1000.0 / mean } else { 0.0 },
            p50_ms: spread.p50_ms,
            p99_ms: spread.p99_ms,
            max_ms: spread.max_ms,
            samples: spread.samples,
        }
    }

    pub fn spread(&mut self) -> Spread {
        spread_of(self.sorted()).unwrap_or_default()
    }

    fn sorted(&mut self) -> &[f64] {
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.periods[..self.filled]);
        self.scratch.sort_by(f64::total_cmp);
        &self.scratch
    }
}

fn spread_of(sorted: &[f64]) -> Option<Spread> {
    let max_ms = *sorted.last()?;
    Some(Spread {
        p50_ms: percentile(sorted, 0.50),
        p99_ms: percentile(sorted, 0.99),
        max_ms,
        samples: sorted.len(),
    })
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Spread {
    pub p50_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub samples: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LatencySummary {
    pub wake_late: Spread,
    pub work: Spread,
    pub input_age: Spread,
}

pub struct LatencyStats {
    wake_late: LoopStats,
    work: LoopStats,
    input_age: LoopStats,
}

impl LatencyStats {
    pub fn new(window: usize) -> Self {
        Self {
            wake_late: LoopStats::new(window),
            work: LoopStats::new(window),
            input_age: LoopStats::new(window),
        }
    }

    pub fn record_wake_late(&mut self, ms: f64) {
        record_clamped(&mut self.wake_late, ms);
    }

    pub fn record_work(&mut self, ms: f64) {
        record_clamped(&mut self.work, ms);
    }

    pub fn record_input_age(&mut self, ms: f64) {
        record_clamped(&mut self.input_age, ms);
    }

    pub fn clear(&mut self) {
        self.wake_late.clear();
        self.work.clear();
        self.input_age.clear();
    }

    pub fn summary(&mut self) -> LatencySummary {
        LatencySummary {
            wake_late: self.wake_late.spread(),
            work: self.work.spread(),
            input_age: self.input_age.spread(),
        }
    }
}

fn record_clamped(stats: &mut LoopStats, ms: f64) {
    if ms.is_finite() {
        stats.record(ms.max(0.0));
    }
}

const RATE_WINDOW_MS: f64 = 250.0;
const RATE_WINDOWS: usize = 20;

#[derive(Clone, Debug, Default)]
pub struct RateMeter {
    acc_events: u64,
    acc_ms: f64,
    windows: [f64; RATE_WINDOWS],
    idx: usize,
}

impl RateMeter {
    pub fn update(&mut self, events: u64, dt_ms: f64) {
        self.acc_events += events;
        self.acc_ms += dt_ms.max(0.0);
        if self.acc_ms >= RATE_WINDOW_MS {
            self.windows[self.idx] = self.acc_events as f64 * 1000.0 / self.acc_ms;
            self.idx = (self.idx + 1) % RATE_WINDOWS;
            self.acc_events = 0;
            self.acc_ms = 0.0;
        }
    }

    pub fn peak_hz(&self) -> f64 {
        self.windows.iter().copied().fold(0.0, f64::max)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DiagnosticsInfo {
    pub app_version: String,
    pub os: String,
    pub status: String,
    pub backend: String,
    pub dll_path: Option<String>,
    pub dll_version: Option<String>,
    pub driver_version: Option<String>,
    pub target_hz: f64,
    pub loop_summary: LoopSummary,
    pub ok_writes: u64,
    pub failed_writes: u64,
    pub reconnects: u32,
    pub clipped_ticks: u64,
    pub telemetry_dropped: u64,
    pub mouse_rate_hz: f64,
    pub absolute_events: u64,
    pub mouse_filter: String,
    pub input_sink: bool,
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("-")
}

pub fn format_report(i: &DiagnosticsInfo) -> String {
    let l = &i.loop_summary;
    let mut s = String::new();
    let _ = writeln!(s, "MouseDrive diagnostics");
    let _ = writeln!(s, "version: {}", i.app_version);
    let _ = writeln!(s, "os: {}", i.os);
    let _ = writeln!(s, "status: {}", i.status);
    let _ = writeln!(s, "backend: {}", i.backend);
    if cfg!(windows) {
        let _ = writeln!(
            s,
            "vjoy dll: {} ({})",
            opt(&i.dll_version),
            opt(&i.dll_path)
        );
        let _ = writeln!(s, "vjoy driver: {}", opt(&i.driver_version));
    } else {
        let _ = writeln!(s, "uinput: {}", opt(&i.dll_path));
    }
    let _ = writeln!(
        s,
        "loop: target {:.0} Hz, measured {:.1} Hz, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms (n={}), skipped telemetry {}",
        i.target_hz, l.hz, l.p50_ms, l.p99_ms, l.max_ms, l.samples, i.telemetry_dropped
    );
    let _ = writeln!(
        s,
        "writes: {} ok / {} failed, reconnects {}",
        i.ok_writes, i.failed_writes, i.reconnects
    );
    let _ = writeln!(
        s,
        "mouse: ~{:.0} Hz, clipped ticks {}, absolute events {}, filter {}, background capture {}",
        i.mouse_rate_hz,
        i.clipped_ticks,
        i.absolute_events,
        if i.mouse_filter.is_empty() {
            "all mice"
        } else {
            &i.mouse_filter
        },
        if i.input_sink { "on" } else { "off" }
    );
    s
}
