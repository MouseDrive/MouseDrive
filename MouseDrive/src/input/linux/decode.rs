use std::collections::HashSet;

use super::super::{
    RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
    RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP, Report,
};

const NANOS_PER_SEC: u64 = 1_000_000_000;
const NANOS_PER_MICRO: u64 = 1_000;

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;
const KEY_REPEAT: i32 = 2;

pub(super) fn clock_ns(secs: u64, nanos: u64) -> u64 {
    secs.saturating_mul(NANOS_PER_SEC).saturating_add(nanos)
}

fn timeval_ns(tv: libc::timeval) -> u64 {
    let secs = u64::try_from(tv.tv_sec).unwrap_or(0);
    let micros = u64::try_from(tv.tv_usec).unwrap_or(0);
    clock_ns(secs, micros.saturating_mul(NANOS_PER_MICRO))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Event {
    kind: u16,
    code: u16,
    value: i32,
    time_ns: u64,
}

impl Event {
    pub(super) fn from_raw(raw: libc::input_event, read_at_ns: Option<u64>) -> Self {
        Self {
            kind: raw.type_,
            code: raw.code,
            value: raw.value,
            time_ns: read_at_ns.unwrap_or_else(|| timeval_ns(raw.time)),
        }
    }

    pub(super) fn time_ns(self) -> u64 {
        self.time_ns
    }

    fn is(self, kind: u16, code: u16) -> bool {
        self.kind == kind && self.code == code
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyEdge {
    pub(super) code: u16,
    pub(super) down: bool,
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
    fn add(&mut self, event: Event) {
        match (event.kind, event.code) {
            (EV_REL, REL_X) => self.dx = self.dx.saturating_add(event.value),
            (EV_REL, REL_Y) => self.dy = self.dy.saturating_add(event.value),
            (EV_ABS, ABS_X | ABS_Y) => self.absolute = true,
            (EV_KEY, code) if event.value != KEY_REPEAT => {
                let flag = button_flag(code, event.value != 0);
                if flag == 0 {
                    return;
                }
                self.buttons |= flag;
            }
            _ => return,
        }
        self.dirty = true;
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Packet {
    frame: Frame,
    time_ns: u64,
    pub(super) keys: Vec<KeyEdge>,
}

impl Packet {
    pub(super) fn report(&self) -> Option<Report> {
        let f = self.frame;
        f.dirty.then_some(Report {
            dx: f.dx,
            dy: f.dy,
            buttons: f.buttons,
            absolute: f.absolute,
            time_ns: self.time_ns,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    Skip,
    Overflow,
    Packet(Packet),
}

#[derive(Default)]
pub(super) struct Decoder {
    frame: Frame,
    keys: Vec<KeyEdge>,
    dropping: bool,
}

impl Decoder {
    pub(super) fn mid_packet(&self) -> bool {
        self.dropping || self.frame.dirty || !self.keys.is_empty()
    }

    pub(super) fn push(&mut self, event: Event) -> Step {
        if event.is(EV_SYN, SYN_DROPPED) {
            return self.overflow();
        }
        let report = event.is(EV_SYN, SYN_REPORT);
        if self.dropping {
            self.dropping = !report;
            return Step::Skip;
        }
        if report {
            return self.finish(event.time_ns);
        }
        if event.kind == EV_KEY && event.value != KEY_REPEAT {
            self.keys.push(KeyEdge {
                code: event.code,
                down: event.value != 0,
            });
        }
        self.frame.add(event);
        Step::Skip
    }

    fn overflow(&mut self) -> Step {
        self.frame = Frame::default();
        self.keys.clear();
        self.dropping = true;
        Step::Overflow
    }

    fn finish(&mut self, time_ns: u64) -> Step {
        let frame = std::mem::take(&mut self.frame);
        let keys = std::mem::take(&mut self.keys);
        if !frame.dirty && keys.is_empty() {
            return Step::Skip;
        }
        Step::Packet(Packet {
            frame,
            time_ns,
            keys,
        })
    }
}

pub(super) fn button_flag(code: u16, down: bool) -> u16 {
    match (code, down) {
        (BTN_LEFT, true) => RI_MOUSE_LEFT_BUTTON_DOWN,
        (BTN_LEFT, false) => RI_MOUSE_LEFT_BUTTON_UP,
        (BTN_RIGHT, true) => RI_MOUSE_RIGHT_BUTTON_DOWN,
        (BTN_RIGHT, false) => RI_MOUSE_RIGHT_BUTTON_UP,
        (BTN_MIDDLE, true) => RI_MOUSE_MIDDLE_BUTTON_DOWN,
        _ => 0,
    }
}

pub(super) fn button_flags<'a>(codes: impl IntoIterator<Item = &'a u16>, down: bool) -> u16 {
    codes
        .into_iter()
        .fold(0, |flags, &code| flags | button_flag(code, down))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct KeyChange {
    pub(super) pressed: Vec<u16>,
    pub(super) released: Vec<u16>,
}

impl KeyChange {
    pub(super) fn button_flags(&self, held_elsewhere: u16) -> u16 {
        let ups = button_flags(&self.released, false) & !held_elsewhere;
        button_flags(&self.pressed, true) | ups
    }
}

pub(super) fn key_diff(tracked: &HashSet<u16>, held: &HashSet<u16>) -> KeyChange {
    let mut pressed: Vec<u16> = held.difference(tracked).copied().collect();
    let mut released: Vec<u16> = tracked.difference(held).copied().collect();
    pressed.sort_unstable();
    released.sort_unstable();
    KeyChange { pressed, released }
}
