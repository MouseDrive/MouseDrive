use std::collections::HashSet;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use evdev::{AbsoluteAxisCode, BusType, Device, KeyCode, RelativeAxisCode};

use super::{
    MouseInfo, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
    RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP, STATE, disambiguate, label_for,
};
use crate::keys::linux as keys;
use crate::platform::{self, Wake, pollfd};

const DEV_INPUT: &str = "/dev/input";
const HOTPLUG_SETTLE: Duration = Duration::from_millis(250);
const WATCH_RETRY: Duration = Duration::from_secs(2);
const OWN_DEVICE_PREFIX: &str = "MouseDrive";

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const KEY_REPEAT: i32 = 2;

const REQ_STOP: u8 = 1;
const REQ_RESCAN: u8 = 2;
const REQ_FILTER: u8 = 4;

static WAKE: Wake = Wake::new();
static NEXT_HANDLE: AtomicIsize = AtomicIsize::new(1);
static POINTER_OPEN: AtomicBool = AtomicBool::new(false);
static DEVICES: Mutex<Vec<Known>> = Mutex::new(Vec::new());
static DENIED: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct Known {
    handle: isize,
    path: String,
    name: String,
    mouse: bool,
}

pub(super) fn spawn() -> io::Result<JoinHandle<()>> {
    let wake = WAKE.fd()?;
    WAKE.clear(REQ_STOP);
    std::thread::Builder::new()
        .name("evdev-input".into())
        .spawn(move || thread_main(wake))
}

pub(super) fn stop() {
    WAKE.post(REQ_STOP);
}

pub(super) fn request_register() {
    WAKE.post(REQ_RESCAN);
}

pub(super) fn filter_changed() {
    WAKE.post(REQ_FILTER);
}

pub(super) fn registration_ok() -> bool {
    POINTER_OPEN.load(Ordering::Acquire)
}

pub(super) fn device_path(handle: isize) -> Option<String> {
    let devices = DEVICES.lock().ok()?;
    devices
        .iter()
        .find(|d| d.handle == handle)
        .map(|d| d.path.clone())
}

pub(super) fn list_mice() -> Vec<MouseInfo> {
    let mice: Vec<Known> = DEVICES
        .lock()
        .map(|d| {
            d.iter()
                .filter(|k| k.mouse && k.handle != 0)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let labels = mice
        .iter()
        .map(|k| label_for(&k.path, Some(k.name.clone())))
        .collect();
    mice.into_iter()
        .zip(disambiguate(labels))
        .map(|(k, label)| MouseInfo {
            path: k.path,
            label,
        })
        .collect()
}

pub fn denied_mice() -> Vec<String> {
    DENIED.lock().map(|d| d.clone()).unwrap_or_default()
}

fn thread_main(wake: RawFd) {
    let _ = crate::platform::raise_current_thread_priority();
    let mut notify = watch_dev_input();
    let mut reader = Reader::default();
    reader.rescan();
    let mut fds: Vec<libc::pollfd> = Vec::new();
    loop {
        if notify.is_none() {
            notify = watch_dev_input();
            if notify.is_none() && reader.rescan_at.is_none() {
                reader.rescan_at = Some(Instant::now() + WATCH_RETRY);
            }
        }
        fds.clear();
        fds.push(pollfd(wake));
        fds.push(pollfd(notify.as_ref().map_or(-1, AsRawFd::as_raw_fd)));
        fds.extend(reader.devices.iter().map(|d| pollfd(d.dev.as_raw_fd())));
        let timeout = reader.poll_timeout(Instant::now());
        if let Err(e) = platform::poll(&mut fds, timeout) {
            crate::log::line(&format!("evdev poll başarısız: {e}"));
            break;
        }
        if fds[0].revents != 0 {
            platform::drain(wake);
        }
        let requests = WAKE.take();
        if requests & REQ_STOP != 0 {
            break;
        }
        if fds[1].revents != 0 {
            if let Some(fd) = &notify {
                platform::drain(fd.as_raw_fd());
            }
            reader.rescan_at = Some(Instant::now() + HOTPLUG_SETTLE);
        }
        let ready: Vec<usize> = fds[2..]
            .iter()
            .enumerate()
            .filter(|(_, p)| p.revents != 0)
            .map(|(i, _)| i)
            .collect();
        reader.read_ready(&ready);

        let due = reader.rescan_at.is_some_and(|t| Instant::now() >= t);
        if requests & REQ_RESCAN != 0 || due {
            reader.rescan();
        } else if requests & REQ_FILTER != 0 {
            reader.resolve_filter();
        }
    }
    reader.close_all();
}

fn watch_dev_input() -> Option<OwnedFd> {
    // SAFETY: bayraklar geçerli; dönen fd hemen sahiplenilir.
    let raw = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
    if raw < 0 {
        return None;
    }
    // SAFETY: raw yeni açılmış ve başka sahibi olmayan bir fd.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mask = libc::IN_CREATE | libc::IN_DELETE | libc::IN_ATTRIB;
    // SAFETY: yol sıfır sonlu sabit bir C dizgisi.
    let watch = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c"/dev/input".as_ptr(), mask) };
    (watch >= 0).then_some(fd)
}

#[derive(Default)]
struct Reader {
    devices: Vec<Open>,
    rescan_at: Option<Instant>,
    events: Vec<(u16, u16, i32)>,
}

impl Reader {
    fn poll_timeout(&self, now: Instant) -> i32 {
        self.rescan_at.map_or(-1, |t| {
            let ms = t.saturating_duration_since(now).as_millis();
            i32::try_from(ms).unwrap_or(i32::MAX)
        })
    }

    fn read_ready(&mut self, ready: &[usize]) {
        let mut dead = Vec::new();
        for &i in ready {
            let Some(open) = self.devices.get_mut(i) else {
                continue;
            };
            if !open.read(&mut self.events) {
                dead.push(i);
            }
        }
        if dead.is_empty() {
            return;
        }
        for &i in dead.iter().rev() {
            self.devices.remove(i).close();
        }
        self.publish();
    }

    fn rescan(&mut self) {
        self.rescan_at = None;
        let nodes = event_nodes();
        let (kept, gone): (Vec<Open>, Vec<Open>) = std::mem::take(&mut self.devices)
            .into_iter()
            .partition(|d| nodes.contains(&d.node));
        gone.into_iter().for_each(Open::close);
        self.devices = kept;

        let mut denied = Vec::new();
        for node in nodes {
            if self.devices.iter().any(|d| d.node == node) {
                continue;
            }
            match Device::open(&node) {
                Ok(dev) => self.devices.extend(Open::new(dev, node)),
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    denied.extend(sysfs_mouse_name(&node));
                }
                Err(_) => {}
            }
        }
        self.publish();
        if let Ok(mut d) = DENIED.lock() {
            *d = denied;
        }
    }

    fn publish(&self) {
        let known = self.devices.iter().map(Open::known).collect();
        if let Ok(mut d) = DEVICES.lock() {
            *d = known;
        }
        let pointer = self.devices.iter().any(|d| d.pointer);
        POINTER_OPEN.store(pointer, Ordering::Release);
        self.resolve_filter();
    }

    fn resolve_filter(&self) {
        STATE.resolve_filter(|path| {
            self.devices
                .iter()
                .find(|d| d.mouse && d.handle != 0 && d.path.eq_ignore_ascii_case(path))
                .map(|d| d.handle)
        });
    }

    fn close_all(&mut self) {
        std::mem::take(&mut self.devices)
            .into_iter()
            .for_each(Open::close);
        self.publish();
    }
}

fn event_nodes() -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(DEV_INPUT) else {
        return Vec::new();
    };
    let mut nodes: Vec<PathBuf> = dir
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("event"))
        })
        .collect();
    nodes.sort();
    nodes
}

struct Open {
    dev: Device,
    node: PathBuf,
    handle: isize,
    path: String,
    name: String,
    mouse: bool,
    pointer: bool,
    pressed: HashSet<u16>,
    frame: Frame,
}

impl Open {
    fn new(dev: Device, node: PathBuf) -> Option<Self> {
        let name = dev.name().unwrap_or_default().trim().to_string();
        if name.starts_with(OWN_DEVICE_PREFIX) {
            return None;
        }
        let keys_set = dev.supported_keys();
        let has_key = |k: KeyCode| keys_set.is_some_and(|s| s.contains(k));
        let rel = dev.supported_relative_axes();
        let has_rel = |a: RelativeAxisCode| rel.is_some_and(|r| r.contains(a));
        let abs_x = dev
            .supported_absolute_axes()
            .is_some_and(|a| a.contains(AbsoluteAxisCode::ABS_X));
        let rel_xy = has_rel(RelativeAxisCode::REL_X) && has_rel(RelativeAxisCode::REL_Y);
        let pointer = has_key(KeyCode::BTN_LEFT) && (rel_xy || abs_x);
        let has_mapped_key =
            keys_set.is_some_and(|s| s.iter().any(|k| keys::vk_for_code(k.0).is_some()));
        if !pointer && !has_mapped_key {
            return None;
        }
        let _ = dev.set_nonblocking(true);
        let id = dev.input_id();
        let handle = if id.bus_type() == BusType::BUS_VIRTUAL {
            0
        } else {
            NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
        };
        let path = identity(
            id.vendor(),
            id.product(),
            dev.unique_name().unwrap_or_default(),
            dev.physical_path().unwrap_or_default(),
            &name,
        );
        let mut open = Self {
            dev,
            node,
            handle,
            path,
            name,
            mouse: pointer && rel_xy,
            pointer,
            pressed: HashSet::new(),
            frame: Frame::default(),
        };
        open.seed_held_keys();
        Some(open)
    }

    fn seed_held_keys(&mut self) {
        let Ok(held) = self.dev.get_key_state() else {
            return;
        };
        for key in held.iter() {
            self.track_key(key.0, true);
        }
    }

    fn known(&self) -> Known {
        Known {
            handle: self.handle,
            path: self.path.clone(),
            name: self.name.clone(),
            mouse: self.mouse,
        }
    }

    fn read(&mut self, events: &mut Vec<(u16, u16, i32)>) -> bool {
        events.clear();
        match self.dev.fetch_events() {
            Ok(it) => events.extend(it.map(|e| (e.event_type().0, e.code(), e.value()))),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
        for &(kind, code, value) in events.iter() {
            self.handle(kind, code, value);
        }
        true
    }

    fn handle(&mut self, kind: u16, code: u16, value: i32) {
        if kind == EV_KEY && value != KEY_REPEAT {
            self.track_key(code, value != 0);
        }
        if !self.pointer {
            return;
        }
        if let Some(frame) = self.frame.push(kind, code, value) {
            emit(self.handle, frame);
        }
    }

    fn track_key(&mut self, code: u16, down: bool) {
        let Some(vk) = keys::vk_for_code(code) else {
            return;
        };
        if down {
            if self.pressed.insert(code) {
                keys::press(vk);
            }
        } else if self.pressed.remove(&code) {
            keys::release(vk);
        }
    }

    fn close(mut self) {
        let mut ups = 0;
        for code in std::mem::take(&mut self.pressed) {
            ups |= button_flag(code, false);
            if let Some(vk) = keys::vk_for_code(code) {
                keys::release(vk);
            }
        }
        if self.pointer && ups != 0 {
            STATE.process(self.handle, false, 0, 0, ups);
        }
    }
}

fn emit(handle: isize, frame: Frame) {
    let sink = STATE.input_sink.load(Ordering::Acquire);
    if !sink && !keys::app_is_foreground() {
        let ups = frame.buttons & (RI_MOUSE_LEFT_BUTTON_UP | RI_MOUSE_RIGHT_BUTTON_UP);
        if ups != 0 {
            STATE.process(handle, false, 0, 0, ups);
        }
        return;
    }
    STATE.process(handle, frame.absolute, frame.dx, frame.dy, frame.buttons);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Frame {
    dx: i32,
    dy: i32,
    buttons: u16,
    absolute: bool,
    dirty: bool,
}

impl Frame {
    fn push(&mut self, kind: u16, code: u16, value: i32) -> Option<Frame> {
        match (kind, code) {
            (EV_REL, REL_X) => self.dx = self.dx.saturating_add(value),
            (EV_REL, REL_Y) => self.dy = self.dy.saturating_add(value),
            (EV_ABS, ABS_X | ABS_Y) => self.absolute = true,
            (EV_KEY, _) if value != KEY_REPEAT => {
                let flag = button_flag(code, value != 0);
                if flag == 0 {
                    return None;
                }
                self.buttons |= flag;
            }
            (EV_SYN, SYN_REPORT) => {
                let done = std::mem::take(self);
                return done.dirty.then_some(done);
            }
            _ => return None,
        }
        self.dirty = true;
        None
    }
}

fn button_flag(code: u16, down: bool) -> u16 {
    match (code, down) {
        (BTN_LEFT, true) => RI_MOUSE_LEFT_BUTTON_DOWN,
        (BTN_LEFT, false) => RI_MOUSE_LEFT_BUTTON_UP,
        (BTN_RIGHT, true) => RI_MOUSE_RIGHT_BUTTON_DOWN,
        (BTN_RIGHT, false) => RI_MOUSE_RIGHT_BUTTON_UP,
        (BTN_MIDDLE, true) => RI_MOUSE_MIDDLE_BUTTON_DOWN,
        _ => 0,
    }
}

fn identity(vendor: u16, product: u16, uniq: &str, phys: &str, name: &str) -> String {
    let place = if uniq.trim().is_empty() { phys } else { uniq };
    format!(
        "evdev#VID_{vendor:04X}&PID_{product:04X}#{}#{}",
        place.trim(),
        name.trim()
    )
}

fn sysfs_mouse_name(node: &Path) -> Option<String> {
    let event = node.file_name()?.to_str()?;
    let base = Path::new("/sys/class/input").join(event).join("device");
    let rel = std::fs::read_to_string(base.join("capabilities/rel")).ok()?;
    if !rel_caps_have_xy(&rel) {
        return None;
    }
    let name = std::fs::read_to_string(base.join("name")).unwrap_or_default();
    Some(format!("{} ({})", name.trim(), node.display()))
}

fn rel_caps_have_xy(caps: &str) -> bool {
    caps.split_whitespace()
        .next_back()
        .and_then(|w| u64::from_str_radix(w, 16).ok())
        .is_some_and(|bits| bits & 0b11 == 0b11)
}
