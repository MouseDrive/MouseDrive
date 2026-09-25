#![deny(unsafe_code)]

use eframe::egui::{Button, OpenUrl, RichText, Ui};
use mousedrive::config::Config;

use super::theme::text_on;
use super::widgets::Cx;
use crate::lang::Strings;
use crate::update::{UpdateChecker, UpdateStatus, unix_now};

pub const CHECK_INTERVAL_S: i64 = 86_400;

pub struct UpdaterUi {
    checker: UpdateChecker,
}

impl UpdaterUi {
    pub fn new(cfg: &mut Config) -> Self {
        let updater = Self {
            checker: UpdateChecker::new(),
        };
        let now = unix_now();
        if due(cfg.auto_check_updates, cfg.last_update_check, now) {
            cfg.last_update_check = now;
            updater.checker.spawn_check();
        }
        updater
    }

    pub fn ready_to_restart(&self) -> bool {
        self.checker.status() == UpdateStatus::ReadyToRestart
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.checker.status(),
            UpdateStatus::Checking | UpdateStatus::Updating
        )
    }

    pub fn button(&self, ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
        match self.checker.status() {
            UpdateStatus::Available(info) if info.version != cfg.skipped_version => {
                let bg = cx.pal.update;
                let text = RichText::new(format!("⬆ {} {}", cx.s.upd_update_btn, info.version))
                    .color(text_on(bg))
                    .strong();
                if ui.add(Button::new(text).fill(bg)).clicked() {
                    if info.auto_installable() {
                        self.checker.spawn_update(info.clone());
                    } else {
                        ui.ctx().open_url(OpenUrl::new_tab(info.html_url.clone()));
                    }
                }
                if ui.small_button(cx.s.upd_skip).clicked() {
                    cfg.skipped_version = info.version;
                }
            }
            UpdateStatus::Updating => {
                ui.spinner();
                ui.label(cx.s.upd_updating);
            }
            UpdateStatus::ReadyToRestart => {
                ui.label(cx.s.upd_restarting);
            }
            UpdateStatus::UpdateFailed(info) => {
                ui.colored_label(ui.visuals().error_fg_color, cx.s.upd_update_failed);
                ui.hyperlink_to(cx.s.upd_download, info.html_url);
            }
            _ => {}
        }
    }

    pub fn section(&self, ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
        ui.add_space(4.0);
        ui.checkbox(&mut cfg.auto_check_updates, cx.s.upd_auto_check);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy(), Button::new(cx.s.upd_check_now))
                .clicked()
            {
                cfg.last_update_check = unix_now();
                self.checker.spawn_check();
            }
            if let Some(text) = status_text(&self.checker.status(), cx.s) {
                ui.label(RichText::new(text).weak());
            }
        });
    }
}

pub fn due(enabled: bool, last: i64, now: i64) -> bool {
    enabled && (now < last || now - last >= CHECK_INTERVAL_S)
}

pub fn status_text(status: &UpdateStatus, s: &Strings) -> Option<String> {
    let text = match status {
        UpdateStatus::Idle => return None,
        UpdateStatus::Checking => s.upd_checking,
        UpdateStatus::UpToDate => s.upd_up_to_date,
        UpdateStatus::Failed => s.upd_failed,
        UpdateStatus::Available(info) => {
            return Some(format!("{} {}", s.upd_available, info.version));
        }
        UpdateStatus::Updating => s.upd_updating,
        UpdateStatus::ReadyToRestart => s.upd_restarting,
        UpdateStatus::UpdateFailed(_) => s.upd_update_failed,
    };
    Some(text.to_string())
}
