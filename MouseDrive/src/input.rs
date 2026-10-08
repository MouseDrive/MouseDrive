use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread::JoinHandle;

use crate::logic::{COUNT_ACCUM_LIMIT, accumulate_counts};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
use linux as sys;
#[cfg(windows)]
use win as sys;

#[cfg(target_os = "linux")]
pub use linux::denied_mice;

const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;

const NO_MOUSE: isize = isize::MIN;

const BUTTON_DOWN: u8 = 1;
const BUTTON_PRESSED: u8 = 2;
const BUTTON_RELEASED: u8 = 4;
const BUTTON_REPORTED: u8 = 8;

fn tick_value(bits: u8) -> bool {
    let down = bits & BUTTON_DOWN != 0;
    if bits & BUTTON_REPORTED != 0 {
        down && bits & BUTTON_RELEASED == 0
    } else {
        down || bits & BUTTON_PRESSED != 0
    }
}

fn after_take(bits: u8) -> u8 {
    let reported = if tick_value(bits) { BUTTON_REPORTED } else { 0 };
    (bits & BUTTON_DOWN) | reported
}

struct ButtonLatch(AtomicU8);

impl ButtonLatch {
    const fn new() -> Self {
        Self(AtomicU8::new(0))
    }

    fn set(&self, down: bool) {
        let edge = if down {
            BUTTON_DOWN | BUTTON_PRESSED
        } else {
            BUTTON_RELEASED
        };
        self.0.update(Ordering::AcqRel, Ordering::Acquire, |bits| {
            let level = if down { bits } else { bits & !BUTTON_DOWN };
            level | edge
        });
    }

    fn level(&self) -> bool {
        self.0.load(Ordering::Acquire) & BUTTON_DOWN != 0
    }

    fn take(&self) -> bool {
        let previous = self
            .0
            .update(Ordering::AcqRel, Ordering::Acquire, after_take);
        tick_value(previous)
    }

    fn force_release(&self) -> bool {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |bits| {
                (bits & BUTTON_DOWN != 0).then_some(bits & !(BUTTON_DOWN | BUTTON_PRESSED))
            })
            .is_ok()
    }

    fn clear(&self) {
        self.0.store(0, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Report {
    dx: i32,
    dy: i32,
    buttons: u16,
    absolute: bool,
    time_ns: u64,
}

pub struct InputState {
    counts_x: AtomicI64,
    move_time_ns: AtomicU64,
    left: ButtonLatch,
    right: ButtonLatch,
    middle_clicked: AtomicBool,
    move_events: AtomicU64,
    absolute_events: AtomicU64,
    ignored_events: AtomicU64,
    dropped_reports: AtomicU64,
    reconcile_releases: AtomicU64,
    thread_stopped: AtomicBool,
    last_device: AtomicIsize,
    accepted_device: AtomicIsize,
    allow_injected: AtomicBool,
    input_sink: AtomicBool,
    filter_path: Mutex<String>,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            counts_x: AtomicI64::new(0),
            move_time_ns: AtomicU64::new(0),
            left: ButtonLatch::new(),
            right: ButtonLatch::new(),
            middle_clicked: AtomicBool::new(false),
            move_events: AtomicU64::new(0),
            absolute_events: AtomicU64::new(0),
            ignored_events: AtomicU64::new(0),
            dropped_reports: AtomicU64::new(0),
            reconcile_releases: AtomicU64::new(0),
            thread_stopped: AtomicBool::new(false),
            last_device: AtomicIsize::new(0),
            accepted_device: AtomicIsize::new(0),
            allow_injected: AtomicBool::new(false),
            input_sink: AtomicBool::new(true),
            filter_path: Mutex::new(String::new()),
        }
    }

    fn accepts(&self, device: isize) -> bool {
        let wanted = self.accepted_device.load(Ordering::Relaxed);
        if wanted == 0 {
            return true;
        }
        if device == 0 {
            return self.allow_injected.load(Ordering::Relaxed);
        }
        device == wanted
    }

    fn process(&self, device: isize, report: Report) {
        let moved = !report.absolute && (report.dx != 0 || report.dy != 0);
        if moved && device != 0 {
            self.last_device.store(device, Ordering::Relaxed);
        }
        if !self.accepts(device) {
            self.ignored_events.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if report.absolute {
            self.absolute_events.fetch_add(1, Ordering::Relaxed);
        } else if moved {
            let _ = self
                .counts_x
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |acc| {
                    Some(accumulate_counts(acc, report.dx))
                });
            self.note_move_time(report.time_ns);
            self.move_events.fetch_add(1, Ordering::Relaxed);
        }
        self.apply_buttons(report.buttons);
    }

    fn note_move_time(&self, time_ns: u64) {
        if time_ns != 0 {
            let _ =
                self.move_time_ns
                    .compare_exchange(0, time_ns, Ordering::AcqRel, Ordering::Relaxed);
        }
    }

    fn take_motion(&self) -> Motion {
        let time_ns = self.move_time_ns.swap(0, Ordering::AcqRel);
        let counts = self
            .counts_x
            .swap(0, Ordering::Relaxed)
            .clamp(-COUNT_ACCUM_LIMIT, COUNT_ACCUM_LIMIT);
        Motion {
            counts,
            oldest_ns: (time_ns != 0).then_some(time_ns),
        }
    }

    fn apply_buttons(&self, bf: u16) {
        let updates = [
            (RI_MOUSE_LEFT_BUTTON_DOWN, &self.left, true),
            (RI_MOUSE_LEFT_BUTTON_UP, &self.left, false),
            (RI_MOUSE_RIGHT_BUTTON_DOWN, &self.right, true),
            (RI_MOUSE_RIGHT_BUTTON_UP, &self.right, false),
        ];
        for (mask, latch, down) in updates {
            if bf & mask != 0 {
                latch.set(down);
            }
        }
        if bf & RI_MOUSE_MIDDLE_BUTTON_DOWN != 0 {
            self.middle_clicked.store(true, Ordering::Release);
        }
    }

    fn resolve_filter(&self, find: impl Fn(&str) -> Option<isize>) {
        let path = self
            .filter_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let accepted = if path.is_empty() {
            0
        } else {
            find(&path).unwrap_or(NO_MOUSE)
        };
        let previous = self.accepted_device.swap(accepted, Ordering::AcqRel);
        if previous != accepted {
            self.left.clear();
            self.right.clear();
        }
    }

    fn buttons(&self) -> (bool, bool) {
        (self.left.level(), self.right.level())
    }

    fn take_buttons(&self) -> (bool, bool) {
        (self.left.take(), self.right.take())
    }

    fn force_release(&self, button: MouseButton) -> bool {
        match button {
            MouseButton::Left => self.left.force_release(),
            MouseButton::Right => self.right.force_release(),
        }
    }

    fn clear(&self) {
        self.counts_x.store(0, Ordering::Relaxed);
        self.move_time_ns.store(0, Ordering::Release);
        self.left.clear();
        self.right.clear();
        self.middle_clicked.store(false, Ordering::Release);
    }

    fn filter(&self) -> DeviceFilter {
        match self.accepted_device.load(Ordering::Acquire) {
            0 => DeviceFilter::AllMice,
            NO_MOUSE => DeviceFilter::SelectedMissing,
            _ => DeviceFilter::Selected,
        }
    }

    fn counters(&self) -> InputCounters {
        InputCounters {
            move_events: self.move_events.load(Ordering::Relaxed),
            absolute_events: self.absolute_events.load(Ordering::Relaxed),
            ignored_events: self.ignored_events.load(Ordering::Relaxed),
            dropped_reports: self.dropped_reports.load(Ordering::Relaxed),
            reconcile_releases: self.reconcile_releases.load(Ordering::Relaxed),
            thread_stopped: self.thread_stopped.load(Ordering::Acquire),
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

static STATE: InputState = InputState::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputCounters {
    pub move_events: u64,
    pub absolute_events: u64,
    pub ignored_events: u64,
    pub dropped_reports: u64,
    pub reconcile_releases: u64,
    pub thread_stopped: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Motion {
    pub counts: i64,
    pub oldest_ns: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceFilter {
    #[default]
    AllMice,
    Selected,
    SelectedMissing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MouseInfo {
    pub path: String,
    pub label: String,
}

pub fn start(
    input_sink: bool,
    device_path: &str,
    allow_injected: bool,
) -> std::io::Result<JoinHandle<()>> {
    STATE.input_sink.store(input_sink, Ordering::Release);
    STATE
        .allow_injected
        .store(allow_injected, Ordering::Release);
    store_filter_path(device_path);
    sys::spawn()
}

pub fn stop() {
    sys::stop();
}

pub fn take_counts() -> i64 {
    take_motion().counts
}

pub fn take_motion() -> Motion {
    STATE.take_motion()
}

pub fn buttons() -> (bool, bool) {
    STATE.buttons()
}

pub fn take_buttons() -> (bool, bool) {
    STATE.take_buttons()
}

pub fn take_middle_click() -> bool {
    STATE.middle_clicked.swap(false, Ordering::Acquire)
}

pub fn force_release(button: MouseButton) -> bool {
    STATE.force_release(button)
}

pub fn clear() {
    STATE.clear();
}

pub fn now_ns() -> u64 {
    sys::now_ns()
}

pub fn counters() -> InputCounters {
    STATE.counters()
}

pub fn set_input_sink(enabled: bool) {
    let previous = STATE.input_sink.swap(enabled, Ordering::AcqRel);
    if previous != enabled {
        sys::request_register();
    }
}

pub fn set_device_filter(path: &str) {
    if store_filter_path(path) {
        sys::filter_changed();
    }
}

pub fn device_filter() -> DeviceFilter {
    STATE.filter()
}

pub fn refresh_filter() {
    sys::refresh_filter();
}

pub fn request_register() {
    sys::request_register();
}

pub fn registration_ok() -> bool {
    sys::registration_ok()
}

pub fn last_moved_device() -> Option<String> {
    let handle = STATE.last_device.load(Ordering::Relaxed);
    (handle != 0).then(|| sys::device_path(handle))?
}

pub fn list_mice() -> Vec<MouseInfo> {
    sys::list_mice()
}

fn store_filter_path(path: &str) -> bool {
    let mut current = STATE
        .filter_path
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if current.as_str() == path {
        return false;
    }
    path.clone_into(&mut current);
    true
}

fn hex_after(path: &str, key: &str, digits: usize) -> Option<u16> {
    let upper = path.to_ascii_uppercase();
    let start = upper.find(key)? + key.len();
    let run: String = upper[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    let tail = run.get(run.len().checked_sub(digits)?..)?;
    u16::from_str_radix(tail, 16).ok()
}

pub fn parse_vid_pid(path: &str) -> Option<(u16, u16)> {
    let vid = hex_after(path, "VID_", 4).or_else(|| hex_after(path, "VID&", 4))?;
    let pid = hex_after(path, "PID_", 4).or_else(|| hex_after(path, "PID&", 4))?;
    Some((vid, pid))
}

pub fn label_for(path: &str, product: Option<String>) -> String {
    let ids = parse_vid_pid(path).map(|(v, p)| format!("{v:04X}:{p:04X}"));
    match (product.filter(|s| !s.trim().is_empty()), ids) {
        (Some(name), Some(ids)) => format!("{} ({ids})", name.trim()),
        (Some(name), None) => name.trim().to_string(),
        (None, Some(ids)) => format!("USB {ids}"),
        (None, None) => path.rsplit('#').nth(1).unwrap_or(path).to_string(),
    }
}

pub fn disambiguate(labels: Vec<String>) -> Vec<String> {
    let mut seen: Vec<(String, usize)> = Vec::new();
    labels
        .iter()
        .map(|label| {
            let total = labels.iter().filter(|l| *l == label).count();
            if total == 1 {
                return label.clone();
            }
            let n = match seen.iter_mut().find(|(l, _)| l == label) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.1
                }
                None => {
                    seen.push((label.clone(), 1));
                    1
                }
            };
            format!("{label} #{n}")
        })
        .collect()
}
