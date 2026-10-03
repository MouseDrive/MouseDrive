#![deny(unsafe_code)]

use std::sync::atomic::Ordering;

use eframe::egui::{Context, Id, Modal, RichText, ViewportCommand};

use super::App;
use super::widgets::Cx;
use crate::lang::{Strings, fill};

const CLOSE_DIALOG_WIDTH: f32 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseDecision {
    Quit,
    Minimize,
    Confirm,
    Wait,
}

fn close_decision(exit_on_close: bool, dirty: bool, waiting: bool) -> CloseDecision {
    match (waiting, exit_on_close, dirty) {
        (true, _, _) => CloseDecision::Wait,
        (false, false, _) => CloseDecision::Minimize,
        (false, true, true) => CloseDecision::Confirm,
        (false, true, false) => CloseDecision::Quit,
    }
}

#[cfg(feature = "updater")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestartDecision {
    Wait,
    Restart,
    Confirm,
}

#[cfg(feature = "updater")]
fn restart_decision(due: bool, closing: bool, dirty: bool) -> RestartDecision {
    match (due && !closing, dirty) {
        (false, _) => RestartDecision::Wait,
        (true, true) => RestartDecision::Confirm,
        (true, false) => RestartDecision::Restart,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseChoice {
    SaveAndQuit,
    Quit,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseReason {
    Quit,
    Restart,
}

fn close_texts(s: &Strings, reason: CloseReason) -> [&'static str; 5] {
    match reason {
        CloseReason::Quit => [
            s.close_title,
            s.close_body,
            s.btn_save_quit,
            s.btn_quit_nosave,
            s.btn_cancel,
        ],
        CloseReason::Restart => [
            s.restart_title,
            s.restart_body,
            s.btn_save_restart,
            s.btn_restart_nosave,
            s.btn_later,
        ],
    }
}

#[derive(Default)]
pub(super) struct CloseState {
    allowed: bool,
    confirm: Option<CloseReason>,
    after_install: Option<CloseReason>,
}

impl App {
    pub(super) fn handle_close(&mut self, ctx: &Context, cx: &Cx) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        if requested && !self.close.allowed {
            let cfg = self.session.config();
            let waiting = self.close.after_install.is_some();
            match close_decision(cfg.exit_on_close, self.session.is_dirty(), waiting) {
                CloseDecision::Quit => self.quit(ctx, CloseReason::Quit),
                CloseDecision::Minimize => {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                CloseDecision::Confirm => {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    self.close.confirm = Some(CloseReason::Quit);
                }
                CloseDecision::Wait => ctx.send_viewport_cmd(ViewportCommand::CancelClose),
            }
        }
        if let Some(reason) = self.close.after_install
            && !self.installing()
        {
            self.quit(ctx, reason);
        }
        if let Some(reason) = self.close.confirm {
            self.close_dialog(ctx, cx, reason);
        } else if let Some(reason) = self.close.after_install {
            self.install_wait_dialog(ctx, cx, reason);
        }
    }

    fn close_dialog(&mut self, ctx: &Context, cx: &Cx, reason: CloseReason) {
        let s = cx.s;
        let [title, body, save, discard, cancel] = close_texts(s, reason);
        let name = self.session.config().active_profile.clone();
        let mut choice = None;
        let response = Modal::new(Id::new("close_dialog")).show(ctx, |ui| {
            ui.set_width(CLOSE_DIALOG_WIDTH);
            ui.label(RichText::new(title).strong());
            ui.label(fill(body, &[("name", &name)]));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button(save).clicked() {
                    choice = Some(CloseChoice::SaveAndQuit);
                }
                if ui.button(discard).clicked() {
                    choice = Some(CloseChoice::Quit);
                }
                if ui.button(cancel).clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        match choice.or(response.should_close().then_some(CloseChoice::Cancel)) {
            None => {}
            Some(CloseChoice::SaveAndQuit) => {
                if self
                    .profiles
                    .save_active(s, &mut self.session, &mut self.notices)
                {
                    self.quit(ctx, reason);
                } else {
                    self.cancel_close();
                }
            }
            Some(CloseChoice::Quit) => self.quit(ctx, reason),
            Some(CloseChoice::Cancel) => self.cancel_close(),
        }
    }

    fn install_wait_dialog(&mut self, ctx: &Context, cx: &Cx, reason: CloseReason) {
        let s = cx.s;
        let response = Modal::new(Id::new("install_wait")).show(ctx, |ui| {
            ui.set_width(CLOSE_DIALOG_WIDTH);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(s.install_wait_title).strong());
            });
            ui.label(s.install_wait_body);
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                let quit = ui.button(s.btn_quit_now).clicked();
                (quit, ui.button(s.btn_keep_open).clicked())
            })
            .inner
        });
        let (quit, keep_open) = response.inner;
        if quit {
            self.close_now(ctx, reason);
        } else if keep_open || response.should_close() {
            self.cancel_close();
        }
    }

    fn cancel_close(&mut self) {
        self.close = CloseState::default();
        #[cfg(feature = "updater")]
        if self.updater.restart_due() {
            self.updater.hold_restart();
        }
    }

    fn quit(&mut self, ctx: &Context, reason: CloseReason) {
        if self.installing() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.close = CloseState {
                after_install: Some(reason),
                ..CloseState::default()
            };
            return;
        }
        self.close_now(ctx, reason);
    }

    fn close_now(&mut self, ctx: &Context, reason: CloseReason) {
        if reason == CloseReason::Restart {
            self.restart.store(true, Ordering::Release);
        }
        self.close = CloseState {
            allowed: true,
            ..CloseState::default()
        };
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    #[cfg(feature = "updater")]
    pub(super) fn handle_restart(&mut self, ctx: &Context) {
        let closing = self.close.allowed
            || self.close.confirm.is_some()
            || self.close.after_install.is_some()
            || ctx.input(|i| i.viewport().close_requested());
        let dirty = self.session.is_dirty();
        match restart_decision(self.updater.restart_due(), closing, dirty) {
            RestartDecision::Wait => {}
            RestartDecision::Restart => self.quit(ctx, CloseReason::Restart),
            RestartDecision::Confirm => {
                self.updater.hold_restart();
                self.close.confirm = Some(CloseReason::Restart);
            }
        }
    }

    #[cfg(feature = "updater")]
    fn installing(&self) -> bool {
        self.updater.installing()
    }

    #[cfg(not(feature = "updater"))]
    fn installing(&self) -> bool {
        false
    }
}
