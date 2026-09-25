#![deny(unsafe_code)]

use eframe::egui::{RichText, Ui};

use super::widgets::Cx;

const MAX_NOTICES: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub level: Level,
    pub text: String,
}

impl Notice {
    pub fn new(level: Level, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

#[derive(Default)]
pub struct Notices {
    items: Vec<Notice>,
}

impl Notices {
    pub fn push(&mut self, notice: Notice) {
        if self.items.contains(&notice) {
            return;
        }
        self.items.push(notice);
        if self.items.len() > MAX_NOTICES {
            self.items.remove(0);
        }
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Notice::new(Level::Error, text));
    }

    pub fn show(&mut self, ui: &mut Ui, cx: &Cx) {
        let mut dismissed = None;
        for (i, notice) in self.items.iter().enumerate() {
            let visuals = ui.visuals();
            let (icon, color) = match notice.level {
                Level::Info => ("ℹ", visuals.text_color()),
                Level::Warn => ("⚠", visuals.warn_fg_color),
                Level::Error => ("⛔", visuals.error_fg_color),
            };
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(icon).color(color));
                ui.label(&notice.text);
                if ui.small_button(cx.s.btn_dismiss).clicked() {
                    dismissed = Some(i);
                }
            });
        }
        if let Some(i) = dismissed {
            self.items.remove(i);
        }
    }
}
