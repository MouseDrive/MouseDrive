#![deny(unsafe_code)]

use eframe::egui::{Button, Context, DragValue, Key, RichText, Ui, Window};
use mousedrive::config::MOUSE_DPI_RANGE;
use mousedrive::ergonomics::dpi_from_measurement;

use super::widgets::Cx;
use crate::lang::{fill, parse_num};

pub const MIN_COUNTS: i64 = 100;
pub const AUTO_FINISH_MS: f64 = 600.0;
const DEFAULT_DISTANCE_CM: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Measuring {
        start: i64,
        last: i64,
        moved_at_ms: f64,
    },
    Done {
        counts: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    open: bool,
    distance_cm: f64,
    state: State,
    focus_done: bool,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            open: false,
            distance_cm: DEFAULT_DISTANCE_CM,
            state: State::Idle,
            focus_done: false,
        }
    }
}

impl Calibration {
    pub fn open(&mut self) {
        self.open = true;
        self.state = State::Idle;
        self.focus_done = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn is_measuring(&self) -> bool {
        matches!(self.state, State::Measuring { .. })
    }

    pub fn start(&mut self, counts_total: i64, now_ms: f64) {
        self.state = State::Measuring {
            start: counts_total,
            last: counts_total,
            moved_at_ms: now_ms,
        };
        self.focus_done = true;
    }

    pub fn track(&mut self, counts_total: i64, now_ms: f64) {
        let State::Measuring {
            start,
            last,
            moved_at_ms,
        } = self.state
        else {
            return;
        };
        if counts_total != last {
            self.state = State::Measuring {
                start,
                last: counts_total,
                moved_at_ms: now_ms,
            };
            return;
        }
        let has_moved = (counts_total - start).abs() >= MIN_COUNTS;
        if has_moved && now_ms - moved_at_ms >= AUTO_FINISH_MS {
            self.finish(counts_total);
        }
    }

    pub fn finish(&mut self, counts_total: i64) {
        if let State::Measuring { start, .. } = self.state {
            self.state = State::Done {
                counts: counts_total - start,
            };
        }
    }

    fn take_done_focus(&mut self) -> bool {
        std::mem::take(&mut self.focus_done)
    }

    pub fn counted(&self, counts_total: i64) -> Option<i64> {
        match self.state {
            State::Idle => None,
            State::Measuring { start, .. } => Some(counts_total - start),
            State::Done { counts } => Some(counts),
        }
    }

    pub fn result(&self) -> Option<i32> {
        let State::Done { counts } = self.state else {
            return None;
        };
        (counts.abs() >= MIN_COUNTS)
            .then(|| dpi_from_measurement(counts, self.distance_cm))
            .flatten()
            .map(|dpi| dpi.clamp(*MOUSE_DPI_RANGE.start(), *MOUSE_DPI_RANGE.end()))
    }

    pub fn show(&mut self, ctx: &Context, cx: &Cx, counts_total: i64, now_ms: f64, dpi: &mut i32) {
        self.track(counts_total, now_ms);
        let mut open = self.open;
        let mut apply = false;
        Window::new(cx.s.calib_title)
            .open(&mut open)
            .collapsible(false)
            .vscroll(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                let steps = fill(
                    cx.s.calib_steps,
                    &[("cm", &cx.lang.num(self.distance_cm, 0))],
                );
                ui.label(steps);
                self.distance_field(ui, cx);
                apply = self.controls(ui, cx, counts_total, now_ms);
            });
        if let Some(measured) = self.result().filter(|_| apply) {
            *dpi = measured;
            open = false;
        }
        if !open {
            self.close();
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.state = State::Idle;
        self.focus_done = false;
    }

    fn distance_field(&mut self, ui: &mut Ui, cx: &Cx) {
        let measuring = self.is_measuring();
        let lang = cx.lang;
        ui.horizontal(|ui| {
            let label = ui.label(cx.s.calib_distance);
            let field = DragValue::new(&mut self.distance_cm)
                .range(1.0..=50.0)
                .speed(0.1)
                .custom_formatter(move |v, _| lang.num(v, 1))
                .custom_parser(parse_num);
            ui.add_enabled(!measuring, field).labelled_by(label.id);
        });
    }

    fn controls(&mut self, ui: &mut Ui, cx: &Cx, counts_total: i64, now_ms: f64) -> bool {
        let measuring = self.is_measuring();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!measuring, Button::new(cx.s.btn_start))
                .clicked()
            {
                self.start(counts_total, now_ms);
            }
            let done = ui.add_enabled(measuring, Button::new(cx.s.btn_done));
            if measuring && self.take_done_focus() {
                done.request_focus();
            }
            if done.clicked() {
                self.finish(counts_total);
            }
        });
        if measuring && finish_key_pressed(ui) {
            self.finish(counts_total);
        }
        if let Some(n) = self.counted(counts_total) {
            ui.label(fill(cx.s.calib_counted, &[("n", &n.abs().to_string())]));
        }
        if !matches!(self.state, State::Done { .. }) {
            return false;
        }
        match self.result() {
            Some(dpi) => {
                let text = fill(cx.s.calib_result, &[("dpi", &dpi.to_string())]);
                ui.label(RichText::new(text).strong());
                ui.button(cx.s.btn_apply).clicked()
            }
            None => {
                let warn = ui.visuals().warn_fg_color;
                ui.label(RichText::new(cx.s.calib_no_motion).color(warn));
                false
            }
        }
    }
}

fn finish_key_pressed(ui: &Ui) -> bool {
    !ui.ctx().wants_keyboard_input()
        && ui.input(|i| i.key_pressed(Key::Space) || i.key_pressed(Key::Enter))
}
