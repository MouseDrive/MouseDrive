use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::thread::JoinHandle;

use crate::status::AppStatus;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
use linux as sys;
#[cfg(windows)]
use win as sys;

const BASE_WIDTH: i32 = 190;
const BASE_HEIGHT: i32 = 28;
const BASE_MARGIN: i32 = 12;
const ALPHA: u8 = 230;
const TOPMOST_REFRESH_MS: u32 = 2000;

const BACKGROUND: [u8; 3] = [28, 28, 32];
const TEXT: [u8; 3] = [240, 240, 240];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Config,
    Redraw,
    Stop,
}

struct OverlayState {
    enabled: AtomicBool,
    corner: AtomicI32,
    status: AtomicUsize,
    labels: Mutex<[String; AppStatus::ALL.len()]>,
}

static STATE: OverlayState = OverlayState {
    enabled: AtomicBool::new(false),
    corner: AtomicI32::new(1),
    status: AtomicUsize::new(usize::MAX),
    labels: Mutex::new([const { String::new() }; AppStatus::ALL.len()]),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub fn place(work: Area, width: i32, height: i32, corner: i32, margin: i32) -> (i32, i32) {
    let left = work.left + margin;
    let right = work.right - margin - width;
    let top = work.top + margin;
    let bottom = work.bottom - margin - height;
    match corner {
        0 => (left, top),
        2 => (left, bottom),
        3 => (right, bottom),
        _ => (right, top),
    }
}

fn scaled(v: i32, dpi: i32) -> i32 {
    (i64::from(v) * i64::from(dpi.max(96)) / 96) as i32
}

fn current_status() -> AppStatus {
    AppStatus::from_index(STATE.status.load(Ordering::Acquire))
}

fn current_label(status: AppStatus) -> String {
    let text = STATE
        .labels
        .lock()
        .ok()
        .and_then(|l| l.get(status.index()).cloned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{status:?}"));
    format!("MouseDrive · {text}")
}

pub fn start() -> std::io::Result<JoinHandle<()>> {
    sys::start()
}

pub fn configure(enabled: bool, corner: i32) {
    let was_enabled = STATE.enabled.swap(enabled, Ordering::AcqRel);
    let old_corner = STATE.corner.swap(corner, Ordering::AcqRel);
    if was_enabled != enabled || old_corner != corner {
        sys::notify(Notice::Config);
    }
}

pub fn set_status(status: AppStatus) {
    let previous = STATE.status.swap(status.index(), Ordering::AcqRel);
    if previous != status.index() && STATE.enabled.load(Ordering::Acquire) {
        sys::notify(Notice::Redraw);
    }
}

pub fn set_labels(labels: [String; AppStatus::ALL.len()]) {
    if let Ok(mut current) = STATE.labels.lock() {
        *current = labels;
    }
    sys::notify(Notice::Redraw);
}

pub fn stop() {
    sys::notify(Notice::Stop);
}
