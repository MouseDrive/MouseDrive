#![deny(unsafe_code)]

use eframe::egui::{Button, Context, ProgressBar, RichText, Window};
use mousedrive::bind::{BindPhase, BindTarget};
use mousedrive::control::{Command, Shared, Snapshot};

use super::widgets::Cx;
use crate::lang::{Strings, fill};

fn target_index(t: BindTarget) -> usize {
    match t {
        BindTarget::Steering => 0,
        BindTarget::Throttle => 1,
        BindTarget::Brake => 2,
        BindTarget::GearUp => 3,
        BindTarget::GearDown => 4,
    }
}

pub fn phase_text(target: BindTarget, phase: BindPhase, s: &Strings) -> (String, Option<f32>) {
    let name = s.bind_targets[target_index(target)];
    match phase {
        BindPhase::Countdown(ms) => {
            let secs = (ms / 1000.0).ceil().max(0.0);
            (
                fill(s.bind_countdown, &[("secs", &format!("{secs:.0}"))]),
                None,
            )
        }
        BindPhase::Sweep(p) => (fill(s.bind_sweep, &[("target", name)]), Some(p as f32)),
        BindPhase::Done => (name.to_string(), Some(1.0)),
    }
}

pub fn show(ctx: &Context, cx: &Cx, open: &mut bool, snap: &Snapshot, shared: &Shared) {
    Window::new(cx.s.bind_title)
        .open(open)
        .collapsible(false)
        .default_width(420.0)
        .show(ctx, |ui| {
            ui.label(cx.s.bind_intro);
            ui.add_space(6.0);
            let busy = snap.binding.is_some();
            ui.horizontal_wrapped(|ui| {
                for target in BindTarget::ALL {
                    let button = Button::new(cx.s.bind_targets[target_index(target)]);
                    if ui.add_enabled(!busy, button).clicked() {
                        shared.send(Command::StartBind(target));
                    }
                }
            });
            let Some((target, phase)) = snap.binding else {
                return;
            };
            ui.add_space(6.0);
            let (text, progress) = phase_text(target, phase, cx.s);
            ui.label(RichText::new(text).strong());
            if let Some(p) = progress {
                ui.add(ProgressBar::new(p));
            }
            if ui.button(cx.s.btn_cancel).clicked() {
                shared.send(Command::CancelBind);
            }
        });
}
