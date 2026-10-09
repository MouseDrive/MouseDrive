mod classify;
mod decode;
mod device;
mod scan;

use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use self::classify::DeviceCaps;
use self::decode::{Event, KeyChange, clock_ns};
use self::device::{Candidate, Open, ReadOutcome, buttons_report, emit, find_selected};
use self::scan::{NodeCache, Opened, Verdict, device_report, evaluate, event_nodes};
use super::{
    Class, DeviceReport, InputState, MouseButton, MouseInfo, RI_MOUSE_LEFT_BUTTON_UP,
    RI_MOUSE_RIGHT_BUTTON_UP, STATE, Skip, disambiguate, label_for,
};
use crate::platform::{self, Wake, pollfd};

const HOTPLUG_SETTLE: Duration = Duration::from_millis(250);
const WATCH_RETRY: Duration = Duration::from_secs(2);
const RECONCILE_EVERY: Duration = Duration::from_millis(100);
const POLL_BACKOFF_MIN: Duration = Duration::from_millis(100);
const POLL_BACKOFF_MAX: Duration = Duration::from_secs(2);

const REQ_STOP: u8 = 1;
const REQ_RESCAN: u8 = 2;
const REQ_FILTER: u8 = 4;

static WAKE: Wake = Wake::new();
static POINTER_OPEN: AtomicBool = AtomicBool::new(false);
static DEVICES: Mutex<Vec<Known>> = Mutex::new(Vec::new());
static REPORTS: Mutex<Vec<DeviceReport>> = Mutex::new(Vec::new());

#[derive(Clone, PartialEq, Eq)]
struct Known {
    handle: isize,
    path: String,
    name: String,
    mouse: bool,
}

impl Known {
    fn of(open: &Open) -> Self {
        Self {
            handle: open.handle,
            path: open.path.clone(),
            name: open.name.clone(),
            mouse: open.mouse,
        }
    }
}

pub(super) fn spawn() -> io::Result<JoinHandle<()>> {
    let started = WAKE.fd().and_then(|wake| {
        WAKE.clear(REQ_STOP);
        STATE.thread_stopped.store(false, Ordering::Release);
        std::thread::Builder::new()
            .name("evdev-input".into())
            .spawn(move || thread_main(wake))
    });
    if started.is_err() {
        STATE.thread_stopped.store(true, Ordering::Release);
    }
    started
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

pub(super) fn refresh_filter() {
    WAKE.post(REQ_RESCAN);
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

pub fn devices() -> Vec<DeviceReport> {
    REPORTS.lock().map(|r| r.clone()).unwrap_or_default()
}

pub fn denied_mice() -> Vec<String> {
    denied(Class::is_pointer)
}

pub fn denied_keyboards() -> Vec<String> {
    denied(|class| class == Class::Keyboard)
}

fn denied(wanted: impl Fn(Class) -> bool + Copy) -> Vec<String> {
    devices()
        .iter()
        .filter(|r| r.denied_as(wanted))
        .map(DeviceReport::label)
        .collect()
}

pub(super) fn list_devices() -> Vec<DeviceReport> {
    scan::list_devices()
}

pub(super) fn now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts yazılabilir yerel bir timespec; saat kimliği sabit ve geçerli.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0;
    }
    let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
    clock_ns(secs, nanos)
}

struct ExitWatch {
    state: &'static InputState,
    requested: bool,
}

impl ExitWatch {
    fn new(state: &'static InputState) -> Self {
        Self {
            state,
            requested: false,
        }
    }
}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        if !self.requested {
            self.state.thread_stopped.store(true, Ordering::Release);
            crate::log::error("evdev girdi thread'i beklenmedik biçimde durdu");
        }
    }
}

#[derive(Default)]
struct Backoff {
    wait: Option<Duration>,
}

impl Backoff {
    fn fail(&mut self) -> (Duration, bool) {
        let first = self.wait.is_none();
        let wait = self
            .wait
            .map_or(POLL_BACKOFF_MIN, |w| (w * 2).min(POLL_BACKOFF_MAX));
        self.wait = Some(wait);
        (wait, first)
    }

    fn recover(&mut self) -> bool {
        self.wait.take().is_some()
    }
}

fn thread_main(wake: RawFd) {
    let mut exit = ExitWatch::new(&STATE);
    let _ = crate::platform::raise_current_thread_priority();
    if !crate::platform::tighten_timer_slack() {
        crate::log::warn("girdi thread'i zamanlayıcı gevşekliği ayarlanamadı");
    }
    let mut notify = watch_dev_input();
    let mut reader = Reader::default();
    reader.rescan();
    let mut fds: Vec<libc::pollfd> = Vec::new();
    let mut backoff = Backoff::default();
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
        poll_or_back_off(&mut fds, timeout, wake, &mut backoff);
        if fds[0].revents != 0 {
            platform::drain(wake);
        }
        let requests = WAKE.take();
        if requests & REQ_STOP != 0 {
            exit.requested = true;
            break;
        }
        if fds[1].revents != 0 {
            if let Some(fd) = &notify {
                platform::drain(fd.as_raw_fd());
            }
            reader.rescan_at = Some(Instant::now() + HOTPLUG_SETTLE);
        }
        reader.read_ready(&ready_devices(&fds[2..]));
        reader.reconcile_if_due(Instant::now());
        let due = reader.rescan_at.is_some_and(|t| Instant::now() >= t);
        if requests & REQ_RESCAN != 0 || due {
            reader.rescan();
        } else if requests & REQ_FILTER != 0 {
            reader.resolve_filter();
        }
    }
    reader.close_all();
}

fn poll_or_back_off(fds: &mut [libc::pollfd], timeout: i32, wake: RawFd, backoff: &mut Backoff) {
    match platform::poll(fds, timeout) {
        Ok(()) => {
            if backoff.recover() {
                crate::log::info("evdev poll yeniden çalışıyor");
            }
        }
        Err(e) => {
            fds.iter_mut().for_each(|p| p.revents = 0);
            let (wait, first) = backoff.fail();
            if first {
                crate::log::error(&format!("evdev poll başarısız, yeniden denenecek: {e}"));
            }
            wait_for_wake(wake, wait);
        }
    }
}

fn wait_for_wake(wake: RawFd, wait: Duration) {
    let ms = i32::try_from(wait.as_millis()).unwrap_or(i32::MAX);
    let mut fds = [pollfd(wake)];
    match platform::poll(&mut fds, ms) {
        Ok(()) if fds[0].revents != 0 => platform::drain(wake),
        Ok(()) => {}
        Err(_) => std::thread::sleep(wait),
    }
}

fn ready_devices(fds: &[libc::pollfd]) -> Vec<usize> {
    fds.iter()
        .enumerate()
        .filter(|(_, p)| p.revents != 0)
        .map(|(i, _)| i)
        .collect()
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

type Verdicts = HashMap<PathBuf, (Option<Class>, Option<Skip>)>;

#[derive(Default)]
struct Reader {
    devices: Vec<Open>,
    nodes: NodeCache,
    logged: Verdicts,
    selected_ids: Option<(u16, u16)>,
    rescan_at: Option<Instant>,
    reconcile_at: Option<Instant>,
    events: Vec<Event>,
}

impl Reader {
    fn poll_timeout(&self, now: Instant) -> i32 {
        let next = [self.rescan_at, self.reconcile_at]
            .into_iter()
            .flatten()
            .min();
        next.map_or(-1, |t| {
            let ms = t.saturating_duration_since(now).as_millis();
            i32::try_from(ms).unwrap_or(i32::MAX)
        })
    }

    fn read_ready(&mut self, ready: &[usize]) {
        let mut dead = Vec::new();
        let mut resync = Vec::new();
        for &i in ready {
            let Some(open) = self.devices.get_mut(i) else {
                continue;
            };
            match open.read(&mut self.events) {
                ReadOutcome::Dead => dead.push(i),
                ReadOutcome::Drained if open.needs_sync() => resync.push(i),
                ReadOutcome::Drained | ReadOutcome::Pending => {}
            }
            note_motion(open, self.selected_ids);
        }
        if !resync.is_empty() {
            self.reconcile(&resync);
        }
        if dead.is_empty() {
            return;
        }
        for &i in dead.iter().rev() {
            self.devices.remove(i).close();
        }
        self.publish();
    }

    fn reconcile_if_due(&mut self, now: Instant) {
        let (left, right) = STATE.buttons();
        if !left && !right {
            self.reconcile_at = None;
            return;
        }
        let due = *self.reconcile_at.get_or_insert(now + RECONCILE_EVERY);
        if now < due {
            return;
        }
        self.reconcile_at = Some(now + RECONCILE_EVERY);
        let targets: Vec<usize> = self
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| d.pointer && STATE.accepts(d.handle))
            .map(|(i, _)| i)
            .collect();
        self.reconcile(&targets);
    }

    fn reconcile(&mut self, targets: &[usize]) {
        let before = STATE.buttons();
        let changes: Vec<(usize, KeyChange)> = targets
            .iter()
            .filter_map(|&i| Some((i, self.devices.get_mut(i)?.resync()?)))
            .collect();
        let held = self.held_buttons();
        for (i, change) in changes {
            let open = &self.devices[i];
            if !open.pointer || !STATE.accepts(open.handle) {
                continue;
            }
            let flags = change.button_flags(held);
            if flags != 0 {
                emit(open.handle, buttons_report(flags));
            }
        }
        release_unheld(held);
        note_reconciled(before, STATE.buttons());
    }

    fn held_buttons(&self) -> u16 {
        self.devices
            .iter()
            .filter(|d| STATE.accepts(d.handle))
            .fold(0, |flags, d| flags | d.held_buttons())
    }

    fn rescan(&mut self) {
        self.rescan_at = None;
        let nodes = event_nodes();
        self.nodes.refresh(&nodes);
        let all = self.nodes.caps();
        self.close_invalid(&nodes, &all);
        let reports = nodes
            .iter()
            .filter_map(|node| self.scan_node(node, &all))
            .collect();
        self.publish();
        self.publish_reports(reports);
    }

    fn close_invalid(&mut self, nodes: &[PathBuf], all: &[DeviceCaps]) {
        let (kept, gone): (Vec<Open>, Vec<Open>) = std::mem::take(&mut self.devices)
            .into_iter()
            .partition(|d| {
                nodes.contains(&d.node)
                    && self
                        .nodes
                        .verdict(&d.node, all)
                        .is_none_or(|verdict| verdict.is_ok())
            });
        gone.into_iter().for_each(Open::close);
        self.devices = kept;
    }

    fn scan_node(&mut self, node: &Path, all: &[DeviceCaps]) -> Option<DeviceReport> {
        if let Some(open) = self.devices.iter().find(|d| d.node == node) {
            return Some(open.report());
        }
        let entry = self.nodes.get_mut(node)?;
        match evaluate(node, entry, all) {
            Verdict::Open(opened) => {
                let Opened { dev, caps, class } = *opened;
                let open = Open::new(dev, node.to_path_buf(), caps, class);
                let report = open.report();
                self.devices.push(open);
                Some(report)
            }
            Verdict::Skip(class, skip) => {
                Some(device_report(node, entry.caps.as_ref(), class, Some(skip)))
            }
        }
    }

    fn publish(&mut self) {
        let known: Vec<Known> = self.devices.iter().map(Known::of).collect();
        let changed = DEVICES.lock().is_ok_and(|mut d| {
            let changed = *d != known;
            *d = known;
            changed
        });
        let pointer = self.devices.iter().any(|d| d.pointer);
        POINTER_OPEN.store(pointer, Ordering::Release);
        self.resolve_filter();
        if changed {
            STATE.bump_generation();
        }
    }

    fn publish_reports(&mut self, reports: Vec<DeviceReport>) {
        self.logged = log_changes(&self.logged, &reports);
        let changed = REPORTS.lock().is_ok_and(|mut r| {
            let changed = *r != reports;
            *r = reports;
            changed
        });
        if changed {
            STATE.bump_generation();
        }
    }

    fn resolve_filter(&mut self) {
        let candidates: Vec<Candidate> = self
            .devices
            .iter()
            .filter(|d| d.mouse && d.handle != 0)
            .map(Candidate::of)
            .collect();
        STATE.resolve_filter(|wanted| find_selected(&candidates, wanted));
        let accepted = STATE.accepted_device.load(Ordering::Acquire);
        self.selected_ids = self
            .devices
            .iter()
            .find(|d| d.handle != 0 && d.handle == accepted)
            .map(|d| d.ids);
    }

    fn close_all(&mut self) {
        std::mem::take(&mut self.devices)
            .into_iter()
            .for_each(Open::close);
        self.publish();
    }
}

fn release_unheld(held: u16) {
    let buttons = [
        (MouseButton::Left, RI_MOUSE_LEFT_BUTTON_UP),
        (MouseButton::Right, RI_MOUSE_RIGHT_BUTTON_UP),
    ];
    for (button, up) in buttons {
        if held & up == 0 {
            STATE.force_release(button);
        }
    }
}

fn note_motion(open: &mut Open, selected_ids: Option<(u16, u16)>) {
    let Some(at) = open.take_moved_at() else {
        return;
    };
    let selected = open.handle != 0 && open.handle == STATE.accepted_device.load(Ordering::Relaxed);
    if selected {
        STATE.note_selected_motion(open.handle, at);
    } else if open.virtual_source && selected_ids == Some(open.ids) {
        STATE.note_twin_motion(at);
    }
}

fn log_changes(logged: &Verdicts, reports: &[DeviceReport]) -> Verdicts {
    reports
        .iter()
        .map(|r| {
            let node = PathBuf::from(&r.node);
            let verdict = (r.class, r.skip);
            if logged.get(&node) != Some(&verdict) {
                crate::log::info(&describe(r));
            }
            (node, verdict)
        })
        .collect()
}

fn describe(r: &DeviceReport) -> String {
    let device = format!(
        "{} \"{}\" ({:04x}:{:04x})",
        r.node, r.name, r.vendor, r.product
    );
    match (r.class, r.skip) {
        (Some(class), None) => format!("girdi cihazı açıldı: {device}, {}", class_name(class)),
        (class, Some(skip)) => {
            let class = class.map_or(String::new(), |c| format!(", {}", class_name(c)));
            format!(
                "girdi cihazı kullanılmıyor: {device}{class}: {}",
                skip_reason(skip)
            )
        }
        (None, None) => format!("girdi cihazı: {device}"),
    }
}

fn class_name(class: Class) -> &'static str {
    match class {
        Class::Mouse => "fare",
        Class::AbsolutePointer => "mutlak konumlu işaretçi",
        Class::Touchpad => "touchpad",
        Class::Keyboard => "klavye",
        Class::OtherKeys => "tuşlu cihaz",
    }
}

fn skip_reason(skip: Skip) -> String {
    match skip {
        Skip::OwnDevice => "MouseDrive'ın kendi cihazı".into(),
        Skip::NotPointerNoKeys => "fare ya da klavye değil".into(),
        Skip::Gamepad => "oyun kolu".into(),
        Skip::GamepadTouchpad => "oyun kolunun touchpad'i".into(),
        Skip::PermissionDenied => "okuma izni yok".into(),
        Skip::OpenFailed(kind) => format!("açılamadı ({kind})"),
    }
}

fn released_buttons(before: (bool, bool), after: (bool, bool)) -> u64 {
    u64::from(before.0 && !after.0) + u64::from(before.1 && !after.1)
}

fn note_reconciled(before: (bool, bool), after: (bool, bool)) {
    let released = released_buttons(before, after);
    if released == 0 {
        return;
    }
    STATE
        .reconcile_releases
        .fetch_add(released, Ordering::Relaxed);
    crate::log::warn(&format!(
        "tuş durumu uzlaştırması bırakma olayı kaçan {released} butonu bıraktı"
    ));
}
