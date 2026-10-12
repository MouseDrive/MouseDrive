use std::collections::HashSet;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use evdev::raw_stream::RawDevice;
use evdev::{AttributeSetRef, KeyCode};

use super::super::{
    Class, DeviceReport, Found, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_UP, Report, STATE,
    identity_parts,
};
use super::classify::DeviceCaps;
use super::decode::{Decoder, Event, KeyChange, Packet, Step, button_flags, key_diff};
use super::now_ns;
use super::scan::device_report;
use crate::keys::linux as keys;

const MAX_READS_PER_WAKE: usize = 64;
const EVIOCSCLOCKID: libc::Ioctl = libc::_IOW::<libc::c_int>(b'E' as u32, 0xa0);

static NEXT_HANDLE: AtomicIsize = AtomicIsize::new(1);
static CLOCK_WARNED: AtomicBool = AtomicBool::new(false);

pub(super) fn open_node(node: &Path) -> io::Result<RawDevice> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(node)?;
    RawDevice::from_fd(OwnedFd::from(file))
}

pub(super) fn node_readable(node: &Path) -> bool {
    let Ok(path) = CString::new(node.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: path sıfır sonlu ve çağrı boyunca geçerli; faccessat yalnız izin sorgular.
    unsafe { libc::faccessat(libc::AT_FDCWD, path.as_ptr(), libc::R_OK, libc::AT_EACCESS) == 0 }
}

fn use_monotonic_clock(fd: RawFd) -> bool {
    let clock: libc::c_int = libc::CLOCK_MONOTONIC;
    // SAFETY: fd açık bir evdev düğümü; EVIOCSCLOCKID bir int işaretçisi bekler ve yerel değişkenin adresi verilir.
    if unsafe { libc::ioctl(fd, EVIOCSCLOCKID, &raw const clock) } == 0 {
        return true;
    }
    let error = io::Error::last_os_error();
    if !CLOCK_WARNED.swap(true, Ordering::Relaxed) {
        crate::log::warn(&format!(
            "evdev olay saati CLOCK_MONOTONIC yapılamadı, okuma anı kullanılacak: {error}"
        ));
    }
    false
}

pub(super) enum ReadOutcome {
    Drained,
    Pending,
    Dead,
}

pub(super) struct Open {
    pub(super) dev: RawDevice,
    pub(super) node: PathBuf,
    pub(super) handle: isize,
    pub(super) path: String,
    pub(super) name: String,
    pub(super) class: Class,
    pub(super) mouse: bool,
    pub(super) pointer: bool,
    pub(super) ids: (u16, u16),
    pub(super) virtual_source: bool,
    caps: DeviceCaps,
    moved_at: Option<u64>,
    kernel_clock: bool,
    clock_switched_at: Option<u64>,
    pressed: HashSet<u16>,
    decoder: Decoder,
    needs_sync: bool,
    overflows: u64,
}

impl Open {
    pub(super) fn new(dev: RawDevice, node: PathBuf, caps: DeviceCaps, class: Class) -> Self {
        let clock_switched_at = use_monotonic_clock(dev.as_raw_fd()).then(now_ns);
        let handle = handle_for(&caps);
        let path = identity(
            caps.vendor,
            caps.product,
            &caps.uniq,
            &caps.phys,
            &caps.name,
        );
        let mut open = Self {
            dev,
            node,
            handle,
            path,
            name: caps.name.trim().to_string(),
            class,
            mouse: class == Class::Mouse,
            pointer: class.is_pointer(),
            ids: (caps.vendor, caps.product),
            virtual_source: caps.is_virtual(),
            caps,
            moved_at: None,
            kernel_clock: clock_switched_at.is_some(),
            clock_switched_at,
            pressed: HashSet::new(),
            decoder: Decoder::default(),
            needs_sync: false,
            overflows: 0,
        };
        open.resync();
        open
    }

    pub(super) fn report(&self) -> DeviceReport {
        device_report(&self.node, Some(&self.caps), Some(self.class), None)
    }

    pub(super) fn take_moved_at(&mut self) -> Option<u64> {
        self.moved_at.take()
    }

    pub(super) fn needs_sync(&self) -> bool {
        self.needs_sync
    }

    pub(super) fn held_buttons(&self) -> u16 {
        if self.pointer {
            button_flags(&self.pressed, false)
        } else {
            0
        }
    }

    pub(super) fn read(&mut self, events: &mut Vec<Event>) -> ReadOutcome {
        for _ in 0..MAX_READS_PER_WAKE {
            events.clear();
            let read_at = (!self.kernel_clock).then(now_ns);
            match self.dev.fetch_events() {
                Ok(batch) => events
                    .extend(batch.map(|e| Event::from_raw(libc::input_event::from(e), read_at))),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return ReadOutcome::Drained,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    crate::log::info(&format!(
                        "girdi cihazı kapatıldı ({}): {e}",
                        self.node.display()
                    ));
                    return ReadOutcome::Dead;
                }
            }
            if events.is_empty() {
                return ReadOutcome::Drained;
            }
            for &event in events.iter() {
                self.handle(event);
            }
        }
        ReadOutcome::Pending
    }

    fn handle(&mut self, event: Event) {
        let switched_at = self.clock_switched_at.take();
        match self.decoder.push(event) {
            Step::Overflow if switched_at.is_some_and(|at| event.time_ns() <= at) => {
                self.needs_sync = true;
            }
            Step::Overflow => self.note_overflow(),
            Step::Packet(packet) => self.apply(&packet),
            Step::Skip => {}
        }
    }

    fn note_overflow(&mut self) {
        self.needs_sync = true;
        self.overflows += 1;
        if self.pointer && STATE.accepts(self.handle) {
            STATE.dropped_reports.fetch_add(1, Ordering::Relaxed);
        }
        if self.overflows.is_power_of_two() {
            crate::log::warn(&format!(
                "evdev olay tamponu taştı ({}, {}. kez); kaçan sayımlar kayıp, tuş durumu yeniden okunacak",
                self.node.display(),
                self.overflows
            ));
        }
    }

    fn apply(&mut self, packet: &Packet) {
        for key in &packet.keys {
            self.track_key(key.code, key.down);
        }
        if !self.pointer {
            return;
        }
        let Some(report) = packet.report() else {
            return;
        };
        let report = pointer_report(report, self.class);
        if report.dx != 0 || report.dy != 0 {
            self.moved_at = Some(report.time_ns);
        }
        emit(self.handle, report);
    }

    pub(super) fn resync(&mut self) -> Option<KeyChange> {
        if self.decoder.mid_packet() {
            return None;
        }
        match self.dev.get_key_state() {
            Ok(held) => {
                self.needs_sync = false;
                Some(self.sync_keys(&held))
            }
            Err(e) => {
                crate::log::warn(&format!(
                    "tuş durumu okunamadı ({}): {e}",
                    self.node.display()
                ));
                None
            }
        }
    }

    fn sync_keys(&mut self, held: &AttributeSetRef<KeyCode>) -> KeyChange {
        let held: HashSet<u16> = held
            .iter()
            .map(|k| k.0)
            .filter(|&code| keys::vk_for_code(code).is_some())
            .collect();
        let change = key_diff(&self.pressed, &held);
        for &code in &change.released {
            self.track_key(code, false);
        }
        for &code in &change.pressed {
            self.track_key(code, true);
        }
        change
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

    pub(super) fn close(mut self) {
        let pressed = std::mem::take(&mut self.pressed);
        let ups = button_flags(&pressed, false);
        for &code in &pressed {
            if let Some(vk) = keys::vk_for_code(code) {
                keys::release(vk);
            }
        }
        if self.pointer && ups != 0 {
            STATE.process(self.handle, buttons_report(ups));
        }
    }
}

pub(super) fn buttons_report(buttons: u16) -> Report {
    Report {
        buttons,
        time_ns: now_ns(),
        ..Report::default()
    }
}

pub(super) fn emit(handle: isize, report: Report) {
    let sink = STATE.input_sink.load(Ordering::Acquire);
    if !sink && !keys::app_is_foreground() {
        let ups = report.buttons & (RI_MOUSE_LEFT_BUTTON_UP | RI_MOUSE_RIGHT_BUTTON_UP);
        if ups != 0 {
            STATE.process(handle, buttons_report(ups));
        }
        return;
    }
    STATE.process(handle, report);
}

fn pointer_report(report: Report, class: Class) -> Report {
    Report {
        absolute: report.absolute && class != Class::Touchpad,
        ..report
    }
}

fn handle_for(caps: &DeviceCaps) -> isize {
    if caps.is_virtual() {
        0
    } else {
        NEXT_HANDLE.fetch_add(1, Ordering::Relaxed)
    }
}

pub(super) fn caps_of_device(dev: &RawDevice) -> DeviceCaps {
    let id = dev.input_id();
    DeviceCaps {
        name: dev.name().unwrap_or_default().trim().to_string(),
        bus: id.bus_type().0,
        vendor: id.vendor(),
        product: id.product(),
        keys: dev
            .supported_keys()
            .map(|set| set.iter().map(|k| k.0).collect())
            .unwrap_or_default(),
        rel: dev
            .supported_relative_axes()
            .map(|set| set.iter().map(|a| a.0).collect())
            .unwrap_or_default(),
        abs: dev
            .supported_absolute_axes()
            .map(|set| set.iter().map(|a| a.0).collect())
            .unwrap_or_default(),
        props: dev.properties().iter().map(|p| p.0).collect(),
        phys: dev.physical_path().unwrap_or_default().trim().to_string(),
        uniq: dev.unique_name().unwrap_or_default().trim().to_string(),
        synthetic: false,
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

#[derive(Clone, Copy, Debug)]
pub(super) struct Candidate<'a> {
    pub(super) handle: isize,
    pub(super) path: &'a str,
    pub(super) ids: (u16, u16),
    pub(super) name: &'a str,
    pub(super) virtual_source: bool,
}

impl<'a> Candidate<'a> {
    pub(super) fn of(open: &'a Open) -> Self {
        Self {
            handle: open.handle,
            path: &open.path,
            ids: open.ids,
            name: &open.name,
            virtual_source: open.virtual_source,
        }
    }

    fn found(&self) -> Found {
        Found {
            handle: self.handle,
            path: self.path.to_string(),
        }
    }

    fn same_model(&self, wanted: &str) -> bool {
        if self.virtual_source {
            return false;
        }
        identity_parts(wanted).is_some_and(|parts| {
            let (vendor, product) = self.ids;
            identity(vendor, product, "", parts.port, self.name).eq_ignore_ascii_case(wanted)
        })
    }
}

pub(super) fn find_selected(candidates: &[Candidate], wanted: &str) -> Option<Found> {
    if let Some(exact) = candidates
        .iter()
        .find(|c| c.path.eq_ignore_ascii_case(wanted))
    {
        return Some(exact.found());
    }
    let mut moved = candidates.iter().filter(|c| c.same_model(wanted));
    match (moved.next(), moved.next()) {
        (Some(only), None) => Some(only.found()),
        _ => None,
    }
}
