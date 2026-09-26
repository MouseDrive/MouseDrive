use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU64, Ordering};
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

pub struct InputState {
    counts_x: AtomicI64,
    left: AtomicBool,
    right: AtomicBool,
    middle_clicked: AtomicBool,
    move_events: AtomicU64,
    absolute_events: AtomicU64,
    ignored_events: AtomicU64,
    last_device: AtomicIsize,
    accepted_device: AtomicIsize,
    filter_missing: AtomicBool,
    allow_injected: AtomicBool,
    input_sink: AtomicBool,
    filter_path: Mutex<String>,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            counts_x: AtomicI64::new(0),
            left: AtomicBool::new(false),
            right: AtomicBool::new(false),
            middle_clicked: AtomicBool::new(false),
            move_events: AtomicU64::new(0),
            absolute_events: AtomicU64::new(0),
            ignored_events: AtomicU64::new(0),
            last_device: AtomicIsize::new(0),
            accepted_device: AtomicIsize::new(0),
            filter_missing: AtomicBool::new(false),
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

    fn process(&self, device: isize, absolute: bool, dx: i32, dy: i32, button_flags: u16) {
        if !self.accepts(device) {
            self.ignored_events.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if absolute {
            self.absolute_events.fetch_add(1, Ordering::Relaxed);
        } else if dx != 0 || dy != 0 {
            let _ = self
                .counts_x
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |acc| {
                    Some(accumulate_counts(acc, dx))
                });
            self.move_events.fetch_add(1, Ordering::Relaxed);
            self.last_device.store(device, Ordering::Relaxed);
        }
        self.apply_buttons(button_flags);
    }

    fn apply_buttons(&self, bf: u16) {
        let updates = [
            (RI_MOUSE_LEFT_BUTTON_DOWN, &self.left, true),
            (RI_MOUSE_LEFT_BUTTON_UP, &self.left, false),
            (RI_MOUSE_RIGHT_BUTTON_DOWN, &self.right, true),
            (RI_MOUSE_RIGHT_BUTTON_UP, &self.right, false),
        ];
        for (mask, flag, value) in updates {
            if bf & mask != 0 {
                flag.store(value, Ordering::Release);
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
            .map(|p| p.clone())
            .unwrap_or_default();
        let (accepted, missing) = if path.is_empty() {
            (0, false)
        } else {
            match find(&path) {
                Some(h) => (h, false),
                None => (0, true),
            }
        };
        let previous = self.accepted_device.swap(accepted, Ordering::AcqRel);
        self.filter_missing.store(missing, Ordering::Release);
        if previous != accepted {
            self.left.store(false, Ordering::Release);
            self.right.store(false, Ordering::Release);
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
    STATE
        .counts_x
        .swap(0, Ordering::Relaxed)
        .clamp(-COUNT_ACCUM_LIMIT, COUNT_ACCUM_LIMIT)
}

pub fn buttons() -> (bool, bool) {
    (
        STATE.left.load(Ordering::Acquire),
        STATE.right.load(Ordering::Acquire),
    )
}

pub fn take_middle_click() -> bool {
    STATE.middle_clicked.swap(false, Ordering::Acquire)
}

pub fn force_release(button: MouseButton) -> bool {
    let flag = match button {
        MouseButton::Left => &STATE.left,
        MouseButton::Right => &STATE.right,
    };
    flag.compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub fn clear() {
    STATE.counts_x.store(0, Ordering::Relaxed);
    STATE.left.store(false, Ordering::Release);
    STATE.right.store(false, Ordering::Release);
    STATE.middle_clicked.store(false, Ordering::Release);
}

pub fn counters() -> InputCounters {
    InputCounters {
        move_events: STATE.move_events.load(Ordering::Relaxed),
        absolute_events: STATE.absolute_events.load(Ordering::Relaxed),
        ignored_events: STATE.ignored_events.load(Ordering::Relaxed),
    }
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
    if STATE.filter_missing.load(Ordering::Acquire) {
        DeviceFilter::SelectedMissing
    } else if STATE.accepted_device.load(Ordering::Acquire) != 0 {
        DeviceFilter::Selected
    } else {
        DeviceFilter::AllMice
    }
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
    let Ok(mut current) = STATE.filter_path.lock() else {
        return false;
    };
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
