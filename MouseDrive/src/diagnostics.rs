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

    pub fn record(&mut self, period_ms: f64) {
        if !period_ms.is_finite() || period_ms < 0.0 {
            return;
        }
        self.periods[self.idx] = period_ms;
        self.idx = (self.idx + 1) % self.periods.len();
        self.filled = (self.filled + 1).min(self.periods.len());
    }

    pub fn summary(&mut self) -> LoopSummary {
        if self.filled == 0 {
            return LoopSummary::default();
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(&self.periods[..self.filled]);
        self.scratch.sort_by(f64::total_cmp);
        let n = self.scratch.len();
        let mean = self.scratch.iter().sum::<f64>() / n as f64;
        LoopSummary {
            hz: if mean > 0.0 { 1000.0 / mean } else { 0.0 },
            p50_ms: percentile(&self.scratch, 0.50),
            p99_ms: percentile(&self.scratch, 0.99),
            max_ms: self.scratch[n - 1],
            samples: n,
        }
    }
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    let idx = ((sorted.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize;
    sorted[idx]
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
        "loop: target {:.0} Hz, measured {:.1} Hz, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms (n={})",
        i.target_hz, l.hz, l.p50_ms, l.p99_ms, l.max_ms, l.samples
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
