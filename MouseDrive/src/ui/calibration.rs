#![deny(unsafe_code)]

use eframe::egui::{Button, Context, DragValue, RichText, Ui, Window};
use mousedrive::config::MOUSE_DPI_RANGE;
use mousedrive::ergonomics::dpi_from_measurement;

use super::widgets::Cx;
use crate::lang::{fill, parse_num};

pub const MIN_COUNTS: i64 = 100;
const DEFAULT_DISTANCE_CM: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    Measuring { start: i64 },
    Done { counts: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Calibration {
    open: bool,
    distance_cm: f64,
    state: State,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            open: false,
            distance_cm: DEFAULT_DISTANCE_CM,
            state: State::Idle,
        }
    }
}

impl Calibration {
    pub fn open(&mut self) {
        self.open = true;
        self.state = State::Idle;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn start(&mut self, counts_total: i64) {
        self.state = State::Measuring {
            start: counts_total,
        };
    }

    pub fn finish(&mut self, counts_total: i64) {
        if let State::Measuring { start } = self.state {
            self.state = State::Done {
                counts: counts_total - start,
            };
        }
    }

    pub fn counted(&self, counts_total: i64) -> Option<i64> {
        match self.state {
            State::Idle => None,
            State::Measuring { start } => Some(counts_total - start),
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

    pub fn show(&mut self, ctx: &Context, cx: &Cx, counts_total: i64, dpi: &mut i32) {
        let mut open = self.open;
        let mut apply = false;
        Window::new(cx.s.calib_title)
            .open(&mut open)
            .collapsible(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                let steps = fill(
                    cx.s.calib_steps,
                    &[("cm", &cx.lang.num(self.distance_cm, 0))],
                );
                ui.label(steps);
                self.distance_field(ui, cx);
                apply = self.controls(ui, cx, counts_total);
            });
        if let Some(measured) = self.result().filter(|_| apply) {
            *dpi = measured;
            open = false;
        }
        self.open = open;
    }

    fn distance_field(&mut self, ui: &mut Ui, cx: &Cx) {
        let measuring = matches!(self.state, State::Measuring { .. });
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

    fn controls(&mut self, ui: &mut Ui, cx: &Cx, counts_total: i64) -> bool {
        let measuring = matches!(self.state, State::Measuring { .. });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!measuring, Button::new(cx.s.btn_start))
                .clicked()
            {
                self.start(counts_total);
            }
            if ui
                .add_enabled(measuring, Button::new(cx.s.btn_done))
                .clicked()
            {
                self.finish(counts_total);
            }
        });
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
