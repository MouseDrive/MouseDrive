#![deny(unsafe_code)]

use std::fmt::Write as _;

use crate::config::{CONFIG_FILE, PORTABLE_MARKER, Portable, VERSION_KEY};
use crate::input::DeviceFilter;
#[cfg(target_os = "linux")]
use crate::platform::Environment;

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

const MAC_BYTES: usize = 6;
const MAC_MASKED_BYTES: usize = 4;
const MASKED_BYTE: &str = "xx";
const NO_SAMPLES: &str = "-";

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
    pub latency: LatencySummary,
    pub control_priority: String,
    pub input_priority: String,
    pub ok_writes: u64,
    pub failed_writes: u64,
    pub reconnects: u32,
    pub clipped_ticks: u64,
    pub telemetry_dropped: u64,
    pub mouse_rate_hz: f64,
    pub absolute_events: u64,
    pub ignored_events: u64,
    pub stuck_releases: u64,
    pub device_filter: DeviceFilter,
    pub selected_device: String,
    pub input_sink: bool,
    pub test_counter: bool,
    pub allow_injected: bool,
    pub instance_lock: bool,
    pub portable: Portable,
    #[cfg(target_os = "linux")]
    pub linux: LinuxInfo,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default)]
pub struct LinuxInfo {
    pub environment: Environment,
    pub pointer_readable: bool,
    pub denied_pointers: usize,
    pub dropped_reports: u64,
    pub reconcile_releases: u64,
    pub input_stopped: bool,
    pub devices: Vec<DeviceLine>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceLine {
    pub node: String,
    pub ids: String,
    pub bus: String,
    pub class: String,
    pub state: String,
    pub uniq: String,
    pub name: String,
}

#[derive(Clone, Copy)]
struct Mac {
    sep: Option<u8>,
}

impl Mac {
    fn stride(self) -> usize {
        if self.sep.is_some() { 3 } else { 2 }
    }

    fn len(self) -> usize {
        MAC_BYTES * self.stride() - usize::from(self.sep.is_some())
    }
}

fn mask_hardware_addresses(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        let Some(mac) = separated_mac_at(bytes, i).or_else(|| bare_mac_at(bytes, i)) else {
            i += 1;
            continue;
        };
        out.push_str(&text[copied..i]);
        for _ in 0..MAC_MASKED_BYTES {
            out.push_str(MASKED_BYTE);
            out.extend(mac.sep.map(char::from));
        }
        let kept = i + MAC_MASKED_BYTES * mac.stride();
        i += mac.len();
        out.push_str(&text[kept..i]);
        copied = i;
    }
    out.push_str(&text[copied..]);
    out
}

fn separated_mac_at(bytes: &[u8], start: usize) -> Option<Mac> {
    let sep = *bytes.get(start + 2)?;
    if !matches!(sep, b':' | b'-') {
        return None;
    }
    let mac = Mac { sep: Some(sep) };
    let candidate = bytes.get(start..start + mac.len())?;
    let well_formed = candidate.iter().enumerate().all(|(i, &b)| {
        if i % 3 == 2 {
            b == sep
        } else {
            b.is_ascii_hexdigit()
        }
    });
    let joined = |b: Option<&u8>| b.is_some_and(|&b| b.is_ascii_hexdigit() || b == sep);
    let alone = !joined(before(bytes, start)) && !joined(bytes.get(start + mac.len()));
    (well_formed && alone).then_some(mac)
}

fn bare_mac_at(bytes: &[u8], start: usize) -> Option<Mac> {
    let mac = Mac { sep: None };
    let candidate = bytes.get(start..start + mac.len())?;
    let joined = |b: Option<&u8>| b.is_some_and(u8::is_ascii_alphanumeric);
    let guid_group = before(bytes, start) == Some(&b'-');
    let alone = !joined(before(bytes, start)) && !joined(bytes.get(start + mac.len()));
    (candidate.iter().all(u8::is_ascii_hexdigit) && alone && !guid_group).then_some(mac)
}

fn before(bytes: &[u8], start: usize) -> Option<&u8> {
    start.checked_sub(1).and_then(|j| bytes.get(j))
}

fn opt(v: &Option<String>) -> &str {
    v.as_deref().unwrap_or("-")
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

pub fn format_report(i: &DiagnosticsInfo) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "MouseDrive diagnostics");
    let _ = writeln!(s, "version: {}", i.app_version);
    let _ = writeln!(s, "os: {}", i.os);
    #[cfg(target_os = "linux")]
    write_environment(&mut s, &i.linux.environment);
    let _ = writeln!(s, "status: {}", i.status);
    let _ = writeln!(s, "backend: {}", i.backend);
    write_output_driver(&mut s, i);
    write_process(&mut s, i);
    write_timing(&mut s, i);
    write_input(&mut s, i);
    #[cfg(target_os = "linux")]
    write_devices(&mut s, &i.linux);
    s
}

#[cfg(target_os = "linux")]
fn write_environment(s: &mut String, env: &Environment) {
    let known = |v: &Option<String>| v.as_deref().unwrap_or("?").to_owned();
    let yes_no = |set: bool| if set { "yes" } else { "no" };
    let _ = writeln!(s, "distro: {}", known(&env.distro));
    let _ = writeln!(s, "kernel: {}", known(&env.kernel));
    let _ = writeln!(s, "glibc: {}", known(&env.glibc));
    let _ = writeln!(
        s,
        "session: {}, desktop {}, wayland display {}, x11 display {}",
        known(&env.session),
        known(&env.desktop),
        yes_no(env.wayland),
        yes_no(env.x11)
    );
}

fn write_output_driver(s: &mut String, i: &DiagnosticsInfo) {
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
}

fn write_process(s: &mut String, i: &DiagnosticsInfo) {
    let instance = if i.instance_lock {
        "single-instance lock held"
    } else {
        "no single-instance lock"
    };
    let _ = writeln!(s, "instance: {instance}");
    let _ = writeln!(s, "portable: {}", portable_label(i.portable));
    let _ = writeln!(
        s,
        "options: test counter {}, allow injected {}",
        on_off(i.test_counter),
        on_off(i.allow_injected)
    );
}

fn portable_label(portable: Portable) -> String {
    let foreign = format!("off ({CONFIG_FILE} next to the exe is not MouseDrive's");
    match portable {
        Portable::Off => "off".into(),
        Portable::Marker => format!("on ({PORTABLE_MARKER} marker)"),
        Portable::VersionKey => format!("on ({CONFIG_FILE} with {VERSION_KEY})"),
        Portable::IgnoredForeign { marked: true } => format!("{foreign}, marker ignored)"),
        Portable::IgnoredForeign { marked: false } => format!("{foreign})"),
        Portable::IgnoredUnmarked => {
            format!("off ({CONFIG_FILE} next to the exe has no marker or {VERSION_KEY})")
        }
    }
}

fn write_timing(s: &mut String, i: &DiagnosticsInfo) {
    let l = &i.loop_summary;
    let _ = writeln!(
        s,
        "loop: target {:.0} Hz, measured {:.1} Hz, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms (n={}), skipped telemetry {}",
        i.target_hz, l.hz, l.p50_ms, l.p99_ms, l.max_ms, l.samples, i.telemetry_dropped
    );
    let _ = writeln!(
        s,
        "latency ms (p50/p99/max): wake-up {}, tick work {}, input age {}",
        spread_text(&i.latency.wake_late),
        spread_text(&i.latency.work),
        spread_text(&i.latency.input_age)
    );
    let _ = writeln!(
        s,
        "priority: control {}, input {}",
        i.control_priority, i.input_priority
    );
}

fn spread_text(spread: &Spread) -> String {
    if spread.samples == 0 {
        return NO_SAMPLES.into();
    }
    format!(
        "{:.2}/{:.2}/{:.2}",
        spread.p50_ms, spread.p99_ms, spread.max_ms
    )
}

fn write_input(s: &mut String, i: &DiagnosticsInfo) {
    let _ = writeln!(
        s,
        "writes: {} ok / {} failed, reconnects {}",
        i.ok_writes, i.failed_writes, i.reconnects
    );
    let _ = writeln!(
        s,
        "mouse: ~{:.0} Hz, clipped ticks {}, absolute events {}, background capture {}",
        i.mouse_rate_hz,
        i.clipped_ticks,
        i.absolute_events,
        on_off(i.input_sink)
    );
    let _ = writeln!(s, "mouse filter: {}", filter_label(i));
    let _ = writeln!(
        s,
        "input: ignored events {}, stuck-button releases {}{}",
        i.ignored_events,
        i.stuck_releases,
        evdev_details(i)
    );
}

#[cfg(target_os = "linux")]
fn evdev_details(i: &DiagnosticsInfo) -> String {
    let linux = &i.linux;
    let thread = if linux.input_stopped {
        "stopped"
    } else {
        "running"
    };
    format!(
        ", lost reports {}, input-check releases {}, thread {thread}",
        linux.dropped_reports, linux.reconcile_releases
    )
}

#[cfg(not(target_os = "linux"))]
fn evdev_details(_: &DiagnosticsInfo) -> String {
    String::new()
}

fn filter_label(i: &DiagnosticsInfo) -> String {
    let label = match i.device_filter {
        DeviceFilter::AllMice => return "all mice".into(),
        DeviceFilter::Selected => "selected",
        DeviceFilter::SelectedMissing => "selected device missing",
    };
    let device = i.selected_device.trim();
    if device.is_empty() {
        return label.into();
    }
    format!("{label} ({})", mask_hardware_addresses(device))
}

#[cfg(target_os = "linux")]
fn write_devices(s: &mut String, linux: &LinuxInfo) {
    let readable = if linux.pointer_readable { "yes" } else { "no" };
    let _ = writeln!(
        s,
        "pointer readable: {readable}, denied pointer devices {}",
        linux.denied_pointers
    );
    if linux.devices.is_empty() {
        let _ = writeln!(s, "devices: none found");
        return;
    }
    let _ = writeln!(s, "devices ({}):", linux.devices.len());
    for d in &linux.devices {
        let uniq = match d.uniq.trim() {
            "" => "-".to_owned(),
            uniq => mask_hardware_addresses(uniq),
        };
        let _ = writeln!(
            s,
            "  {}: {}, {}, {} {}, uniq {uniq}, {}",
            d.node, d.class, d.state, d.bus, d.ids, d.name
        );
    }
}
