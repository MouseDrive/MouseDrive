#![deny(unsafe_code)]

use eframe::egui::{Button, RichText, Ui};
use mousedrive::config::Config;
use mousedrive::keys::UNBOUND;

use super::widgets::{Cx, key_text};

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
}

#[derive(Default)]
pub struct KeyBinder {
    state: Option<(KeyField, Phase)>,
}

impl KeyBinder {
    pub fn start(&mut self, field: KeyField) {
        self.state = Some((field, Phase::WaitRelease));
    }

    pub fn listening(&self) -> bool {
        self.state.is_some()
    }

    fn active(&self) -> Option<KeyField> {
        self.state.map(|(field, _)| field)
    }

    pub fn poll(&mut self, pressed: Option<i32>, escape: bool) -> Option<Outcome> {
        let (field, phase) = self.state?;
        if escape {
            self.state = None;
            return Some(Outcome::Cancelled);
        }
        match (phase, pressed) {
            (Phase::WaitRelease, None) => {
                self.state = Some((field, Phase::Listen));
                None
            }
            (Phase::Listen, Some(vk)) => {
                self.state = None;
                Some(Outcome::Bound(field, vk))
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
    if button.on_hover_text(cx.s.tip_key).clicked() && !listening {
        binder.start(field);
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
