#![deny(unsafe_code)]

use eframe::egui::{RichText, Ui};

use super::widgets::Cx;
use super::{App, Tab, dashboard, theme};

const STEP_COLUMNS_WIDTH: f32 = 720.0;

impl App {
    pub(super) fn overview(&mut self, ui: &mut Ui, cx: &Cx) {
        if let Some(action) = dashboard::show(ui, cx, &self.snap, self.session.config()) {
            self.run_dashboard_action(action);
        }
        ui.add_space(12.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(cx.s.getting_started).size(18.0).strong());
            let steps = [
                (cx.s.step_setup, cx.s.step_setup_desc),
                (cx.s.step_bind, cx.s.step_bind_desc),
                (cx.s.step_tune, cx.s.step_tune_desc),
            ];
            let mut step = |ui: &mut Ui, i: usize| {
                let (title, detail) = steps[i];
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:02}", i + 1))
                            .color(theme::ACCENT)
                            .strong(),
                    );
                    if ui.button(title).clicked() {
                        match i {
                            0 => self.setup.open = true,
                            1 => self.bind_helper_open = true,
                            _ => self.tab = Tab::Steering,
                        }
                    }
                });
                theme::hint(ui, detail);
            };
            if ui.available_width() >= STEP_COLUMNS_WIDTH {
                ui.columns(3, |columns| {
                    for (i, column) in columns.iter_mut().enumerate() {
                        step(column, i);
                    }
                });
            } else {
                for i in 0..3 {
                    step(ui, i);
                }
            }
        });
        ui.add_space(12.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(cx.s.learn_title).strong());
            theme::hint(ui, cx.s.learn_desc);
            theme::hint(ui, cx.s.live_shortcuts);
        });
    }
}
