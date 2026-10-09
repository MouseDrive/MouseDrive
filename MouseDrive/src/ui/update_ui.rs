#![deny(unsafe_code)]

use eframe::egui::{Button, OpenUrl, Response, RichText, Ui};
use mousedrive::config::Config;

use super::theme::text_on;
use super::widgets::Cx;
use crate::lang::Strings;
use crate::update::{FailReason, Manual, ReleaseInfo, UpdateChecker, UpdateStatus, unix_now};

pub const CHECK_INTERVAL_S: i64 = 86_400;

pub struct UpdaterUi {
    checker: UpdateChecker,
    restart_held: bool,
    restart_asked: bool,
}

impl UpdaterUi {
    pub fn new(cfg: &mut Config) -> Self {
        let updater = Self {
            checker: UpdateChecker::new(),
            restart_held: false,
            restart_asked: false,
        };
        let now = unix_now();
        if due(cfg.auto_check_updates, cfg.last_update_check, now) {
            cfg.last_update_check = now;
            updater.checker.spawn_check();
        }
        updater
    }

    pub fn restart_due(&self) -> bool {
        !self.restart_held && self.checker.status() == UpdateStatus::ReadyToRestart
    }

    pub fn hold_restart(&mut self) {
        self.restart_held = true;
    }

    pub fn take_restart_request(&mut self) -> bool {
        std::mem::take(&mut self.restart_asked)
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.checker.status(),
            UpdateStatus::Checking | UpdateStatus::Updating
        )
    }

    pub fn installing(&self) -> bool {
        self.checker.status() == UpdateStatus::Updating
    }

    pub fn button(&mut self, ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
        match self.checker.status() {
            UpdateStatus::Available(info) if info.version != cfg.skipped_version => {
                self.offer(ui, cx, &info);
                if ui.small_button(cx.s.upd_skip).clicked() {
                    cfg.skipped_version = info.version;
                }
            }
            UpdateStatus::Updating => {
                ui.spinner();
                ui.label(cx.s.upd_updating);
            }
            UpdateStatus::ReadyToRestart if self.restart_held => {
                let restart = accent_button(ui, cx, cx.s.upd_restart_now);
                if restart.on_hover_text(cx.s.upd_installed).clicked() {
                    self.restart_held = false;
                    self.restart_asked = true;
                }
            }
            UpdateStatus::ReadyToRestart => {
                ui.label(cx.s.upd_restarting);
            }
            UpdateStatus::UpdateFailed(info, reason) => {
                let text = format!("{}: {}", cx.s.upd_update_failed, fail_text(reason, cx.s));
                ui.colored_label(ui.visuals().error_fg_color, text)
                    .on_hover_text(fail_tip(reason, cx.s));
                ui.hyperlink_to(cx.s.upd_download, info.html_url);
            }
            _ => {}
        }
    }

    fn offer(&self, ui: &mut Ui, cx: &Cx, info: &ReleaseInfo) {
        match self.checker.manual(info) {
            None => {
                let label = format!("⬆ {} {}", cx.s.upd_update_btn, info.version);
                let button = accent_button(ui, cx, &label).on_hover_text(cx.s.upd_update_tip);
                if button.clicked() {
                    self.checker.spawn_update(info.clone());
                }
            }
            Some(manual) => {
                let label = format!("⬆ {} {}", cx.s.upd_available, info.version);
                let tip = format!("{}. {}", manual_text(manual, cx.s), cx.s.upd_manual_tip);
                if accent_button(ui, cx, &label).on_hover_text(tip).clicked() {
                    ui.ctx().open_url(OpenUrl::new_tab(info.html_url.clone()));
                }
            }
        }
    }

    pub fn section(&self, ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
        ui.add_space(4.0);
        ui.checkbox(&mut cfg.auto_check_updates, cx.s.upd_auto_check);
        let status = self.checker.status();
        let can_check = !self.busy() && status != UpdateStatus::ReadyToRestart;
        ui.horizontal(|ui| {
            if ui
                .add_enabled(can_check, Button::new(cx.s.upd_check_now))
                .clicked()
            {
                cfg.last_update_check = unix_now();
                self.checker.spawn_check();
            }
            let manual = match &status {
                UpdateStatus::Available(info) => self.checker.manual(info),
                _ => None,
            };
            if let Some(text) = status_text(&status, manual, cx.s) {
                ui.label(RichText::new(text).weak());
            }
        });
    }
}

fn accent_button(ui: &mut Ui, cx: &Cx, label: &str) -> Response {
    let bg = cx.pal.update;
    let text = RichText::new(label).color(text_on(bg)).strong();
    ui.add(Button::new(text).fill(bg))
}

pub fn due(enabled: bool, last: i64, now: i64) -> bool {
    enabled && (now < last || now - last >= CHECK_INTERVAL_S)
}

pub fn status_text(status: &UpdateStatus, manual: Option<Manual>, s: &Strings) -> Option<String> {
    let text = match status {
        UpdateStatus::Idle => return None,
        UpdateStatus::Checking => s.upd_checking,
        UpdateStatus::UpToDate => s.upd_up_to_date,
        UpdateStatus::Failed => s.upd_failed,
        UpdateStatus::Available(info) => {
            let available = format!("{} {}", s.upd_available, info.version);
            return Some(match manual {
                Some(manual) => format!("{available} ({})", manual_text(manual, s)),
                None => available,
            });
        }
        UpdateStatus::Updating => s.upd_updating,
        UpdateStatus::ReadyToRestart => s.upd_installed,
        UpdateStatus::UpdateFailed(_, reason) => {
            return Some(format!(
                "{}: {}",
                s.upd_update_failed,
                fail_text(*reason, s)
            ));
        }
    };
    Some(text.to_string())
}

pub fn manual_text(manual: Manual, s: &Strings) -> &'static str {
    match manual {
        Manual::PackageManager => s.upd_manual_package,
        Manual::AppImage => s.upd_manual_appimage,
        Manual::NoBuild => s.upd_manual_no_build,
    }
}

pub fn fail_text(reason: FailReason, s: &Strings) -> &'static str {
    match reason {
        FailReason::Network => s.upd_fail_network,
        FailReason::Verify => s.upd_fail_verify,
        FailReason::NotWritable => s.upd_fail_not_writable,
        FailReason::SelfTest => s.upd_fail_self_test,
        FailReason::Replace => s.upd_fail_replace,
        FailReason::Restore => s.upd_fail_restore,
        FailReason::Other => s.upd_fail_other,
    }
}

pub fn fail_tip(reason: FailReason, s: &Strings) -> &'static str {
    match reason {
        FailReason::Restore => s.upd_fail_restore_tip,
        _ => s.upd_fail_tip,
    }
}
