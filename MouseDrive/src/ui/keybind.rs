#![deny(unsafe_code)]

use eframe::egui::{Button, RichText, Ui};
use mousedrive::config::Config;
use mousedrive::keys::UNBOUND;

use super::widgets::{Cx, key_text};

const LISTEN_TIMEOUT_S: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyField {
    Capture,
    GearUp,
    GearDown,
    Ab,
}

impl KeyField {
    pub fn slot(self, cfg: &mut Config) -> &mut i32 {
        match self {
            KeyField::Capture => &mut cfg.capture_toggle_key,
            KeyField::GearUp => &mut cfg.gear_up_key,
            KeyField::GearDown => &mut cfg.gear_down_key,
            KeyField::Ab => &mut cfg.ab_toggle_key,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    WaitRelease,
    Listen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Bound(KeyField, i32),
    Cancelled,
    TimedOut,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct KeyInput {
    pub pressed: Option<i32>,
    pub escape: bool,
    pub now_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Listening {
    field: KeyField,
    phase: Phase,
    since_s: f64,
}

#[derive(Default)]
pub struct KeyBinder {
    state: Option<Listening>,
    timed_out: bool,
}

impl KeyBinder {
    fn start(&mut self, field: KeyField, now_s: f64) {
        self.timed_out = false;
        self.state = Some(Listening {
            field,
            phase: Phase::WaitRelease,
            since_s: now_s,
        });
    }

    pub fn click(&mut self, field: KeyField, now_s: f64) -> Option<Outcome> {
        if self.active() == Some(field) {
            self.state = None;
            return Some(Outcome::Cancelled);
        }
        self.start(field, now_s);
        None
    }

    pub fn listening(&self) -> bool {
        self.state.is_some()
    }

    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    fn active(&self) -> Option<KeyField> {
        self.state.map(|listening| listening.field)
    }

    pub fn poll(&mut self, input: KeyInput) -> Option<Outcome> {
        let listening = self.state?;
        if input.escape {
            self.state = None;
            return Some(Outcome::Cancelled);
        }
        if input.now_s - listening.since_s >= LISTEN_TIMEOUT_S {
            self.state = None;
            self.timed_out = true;
            return Some(Outcome::TimedOut);
        }
        match (listening.phase, input.pressed) {
            (Phase::WaitRelease, None) => {
                self.state = Some(Listening {
                    phase: Phase::Listen,
                    ..listening
                });
                None
            }
            (Phase::Listen, Some(vk)) => {
                self.state = None;
                Some(Outcome::Bound(listening.field, vk))
            }
            _ => None,
        }
    }
}

pub fn key_row(
    ui: &mut Ui,
    cx: &Cx,
    binder: &mut KeyBinder,
    (field, label): (KeyField, &str),
    value: &mut i32,
    clearable: bool,
) {
    let l = ui.label(label).on_hover_text(cx.s.tip_key);
    let listening = binder.active() == Some(field);
    let text = if listening {
        RichText::new(cx.s.key_press).italics()
    } else {
        RichText::new(key_text(cx.s, *value))
    };
    let button = ui
        .add(Button::new(text).selected(listening))
        .labelled_by(l.id);
    if button.on_hover_text(cx.s.tip_key).clicked() {
        binder.click(field, ui.input(|i| i.time));
    }
    if clearable && *value != UNBOUND {
        if ui.small_button(cx.s.key_clear).clicked() {
            *value = UNBOUND;
        }
    } else {
        ui.label("");
    }
    ui.end_row();
}
