#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use eframe::egui::{Context, DragValue, Grid, OpenUrl, RichText, Ui, Window};
use mousedrive::control::{Command, SetupInfo, Shared, Snapshot};
use mousedrive::setup::{
    Check, CheckId, CheckState, DeviceState, Owner, SetupReport, format_version,
};

use super::notices::Notices;
use super::widgets::{Cx, bar, steering_bar};
use crate::lang::{Strings, fill};

const VJOY_DOWNLOAD_URL: &str = "https://github.com/jshafer817/vJoy/releases";
const VJOY_CONF_EXE: &str = "vJoyConf.exe";
const VJOY_DEFAULT_DIR: &str = r"C:\Program Files\vJoy\x64";

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub state: CheckState,
    pub title: String,
    pub detail: String,
    pub fix: Option<String>,
}

pub fn owner_label(o: &Owner) -> String {
    o.name.clone().unwrap_or_else(|| format!("PID {}", o.pid))
}

pub fn check_rows(report: &SetupReport, s: &Strings) -> Vec<Row> {
    report
        .checks()
        .into_iter()
        .map(|c| row(report, c, s))
        .collect()
}

type Parts = (String, String, Option<String>);

fn row(r: &SetupReport, c: Check, s: &Strings) -> Row {
    let failed = c.state == CheckState::Fail;
    let (title, detail, fix) = match c.id {
        CheckId::Dll => dll_parts(r, failed, s),
        CheckId::Driver => (
            s.chk_driver.into(),
            String::new(),
            failed.then(|| s.fix_driver.into()),
        ),
        CheckId::Versions => versions_parts(r, s),
        CheckId::Device => device_parts(r, failed, s),
        CheckId::Axes => axes_parts(r, failed, s),
        CheckId::Buttons => buttons_parts(r, failed, s),
    };
    if c.state == CheckState::NotRun {
        return Row {
            state: c.state,
            title,
            detail: s.chk_not_run.into(),
            fix: None,
        };
    }
    Row {
        state: c.state,
        title,
        detail,
        fix,
    }
}

fn dll_parts(r: &SetupReport, failed: bool, s: &Strings) -> Parts {
    let detail = r
        .dll_error
        .clone()
        .or_else(|| r.dll_path.as_ref().map(|p| p.display().to_string()))
        .unwrap_or_default();
    let fix = if failed {
        Some(s.fix_dll.to_string())
    } else {
        r.alternate_dll
            .as_ref()
            .map(|p| fill(s.fix_alternate_dll, &[("path", &p.display().to_string())]))
    };
    (s.chk_dll.into(), detail, fix)
}

fn versions_parts(r: &SetupReport, s: &Strings) -> Parts {
    let (detail, fix) = match r.versions {
        Some(v) if v.matched => (
            fill(s.versions_match, &[("dll", &format_version(v.dll))]),
            None,
        ),
        Some(v) => {
            let args = [
                ("dll", format_version(v.dll)),
                ("driver", format_version(v.driver)),
            ];
            let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
            (
                fill(s.versions_mismatch, &args),
                Some(s.fix_versions.to_string()),
            )
        }
        None => (String::new(), None),
    };
    (s.chk_versions.into(), detail, fix)
}

fn device_parts(r: &SetupReport, failed: bool, s: &Strings) -> Parts {
    let id = r.device_id.to_string();
    let detail = match r.device_state {
        Some(DeviceState::Free | DeviceState::Own) if r.acquired => s.dev_free.to_string(),
        Some(DeviceState::Busy) => r.owner.as_ref().map_or_else(
            || s.dev_busy_unknown.to_string(),
            |o| fill(s.dev_busy, &[("owner", &owner_label(o))]),
        ),
        Some(DeviceState::Missing) => s.dev_missing.to_string(),
        Some(_) => s.dev_not_acquired.to_string(),
        None => String::new(),
    };
    let fix = match r.device_state {
        Some(DeviceState::Busy) => Some(s.fix_device_busy.to_string()),
        Some(DeviceState::Missing) => Some(fill(s.fix_device_missing, &[("id", &id)])),
        _ if failed => Some(s.fix_device_acquire.to_string()),
        _ => None,
    };
    (fill(s.chk_device, &[("id", &id)]), detail, fix)
}

fn axes_parts(r: &SetupReport, failed: bool, s: &Strings) -> Parts {
    let missing = r.axes.map(|a| a.missing()).unwrap_or_default();
    let detail = if missing.is_empty() {
        String::new()
    } else {
        fill(s.axes_missing, &[("axes", &missing.join(", "))])
    };
    let fix = failed.then(|| fill(s.fix_axes, &[("id", &r.device_id.to_string())]));
    (s.chk_axes.into(), detail, fix)
}

fn buttons_parts(r: &SetupReport, failed: bool, s: &Strings) -> Parts {
    let detail = r.buttons.map_or_else(String::new, |n| {
        let (n, need) = (n.to_string(), r.buttons_required.to_string());
        fill(s.buttons_count, &[("n", &n), ("need", &need)])
    });
    (
        s.chk_buttons.into(),
        detail,
        failed.then(|| s.fix_buttons.into()),
    )
}

fn vjoy_conf_path(dll: Option<&Path>) -> PathBuf {
    dll.and_then(Path::parent)
        .map(|dir| dir.join(VJOY_CONF_EXE))
        .filter(|p| p.exists())
        .unwrap_or_else(|| Path::new(VJOY_DEFAULT_DIR).join(VJOY_CONF_EXE))
}

pub struct Env<'a> {
    pub shared: &'a Shared,
    pub snap: &'a Snapshot,
    pub device_id: &'a mut i32,
    pub notices: &'a mut Notices,
    pub open_bind_helper: &'a mut bool,
}

#[derive(Default)]
pub struct SetupUi {
    pub open: bool,
    pub info: SetupInfo,
    seen_seq: u64,
    auto_opened: bool,
}

impl SetupUi {
    pub fn pull(&mut self, shared: &Shared) {
        let Some(info) = shared.setup_if_newer(self.seen_seq) else {
            return;
        };
        self.seen_seq = info.seq;
        if !self.auto_opened && info.report.has_failures() {
            self.open = true;
            self.auto_opened = true;
        }
        self.info = info;
    }

    pub fn show(&mut self, ctx: &Context, cx: &Cx, env: Env) {
        let Env {
            shared,
            snap,
            device_id,
            notices,
            open_bind_helper,
        } = env;
        let report = &self.info.report;
        let mut open = self.open;
        Window::new(cx.s.setup_title)
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .show(ctx, |ui| {
                ui.label(cx.s.setup_intro);
                ui.add_space(6.0);
                rows_grid(ui, cx, &check_rows(report, cx.s));
                ui.add_space(6.0);
                actions(ui, cx, report, shared, notices);
                device_number(ui, cx, device_id);
                ui.separator();
                live_test(ui, cx, snap);
                next_steps(ui, cx, open_bind_helper);
            });
        self.open = open;
    }
}

fn state_icon(ui: &Ui, cx: &Cx, state: CheckState) -> RichText {
    let v = ui.visuals();
    match state {
        CheckState::Pass => RichText::new("✔").color(cx.pal.ok),
        CheckState::Warn => RichText::new("⚠").color(v.warn_fg_color),
        CheckState::Fail => RichText::new("✖").color(v.error_fg_color),
        CheckState::NotRun => RichText::new("○").color(v.weak_text_color()),
    }
    .strong()
}

fn rows_grid(ui: &mut Ui, cx: &Cx, rows: &[Row]) {
    Grid::new("setup_rows")
        .num_columns(2)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            for row in rows {
                let icon = state_icon(ui, cx, row.state);
                ui.label(icon);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&row.title).strong());
                    if !row.detail.is_empty() {
                        ui.label(RichText::new(&row.detail).weak());
                    }
                    if let Some(fix) = &row.fix {
                        ui.label(RichText::new(fix).color(ui.visuals().warn_fg_color));
                    }
                });
                ui.end_row();
            }
        });
}

fn actions(ui: &mut Ui, cx: &Cx, report: &SetupReport, shared: &Shared, notices: &mut Notices) {
    ui.horizontal_wrapped(|ui| {
        if ui.button(cx.s.btn_recheck).clicked() {
            shared.send(Command::Reconnect);
        }
        if ui.button(cx.s.btn_open_vjoy_conf).clicked() {
            let exe = vjoy_conf_path(report.dll_path.as_deref());
            if let Err(e) = std::process::Command::new(&exe).spawn() {
                notices.error(fill(
                    cx.s.err_launch,
                    &[("error", &format!("{}: {e}", exe.display()))],
                ));
            }
        }
        if ui.button(cx.s.btn_download_vjoy).clicked() {
            ui.ctx().open_url(OpenUrl::new_tab(VJOY_DOWNLOAD_URL));
        }
    });
}

fn device_number(ui: &mut Ui, cx: &Cx, device_id: &mut i32) {
    ui.horizontal(|ui| {
        let label = ui
            .label(cx.s.device_number)
            .on_hover_text(cx.s.tip_vjoy_device);
        ui.add(DragValue::new(device_id).range(1..=16).speed(0.05))
            .labelled_by(label.id);
    });
}

fn live_test(ui: &mut Ui, cx: &Cx, snap: &Snapshot) {
    ui.label(cx.s.setup_live_test);
    let f = &snap.frame;
    steering_bar(
        ui,
        cx.s.g_steering,
        (f.steering, snap.steering_raw),
        0.0,
        cx.pal.accent,
    );
    bar(ui, cx.s.g_throttle, f.throttle, None, cx.pal.throttle);
    bar(ui, cx.s.g_brake, f.brake, None, cx.pal.brake);
}

fn next_steps(ui: &mut Ui, cx: &Cx, open_bind_helper: &mut bool) {
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(cx.s.setup_next_bind);
        if ui.button(cx.s.btn_bind_helper).clicked() {
            *open_bind_helper = true;
        }
    });
    ui.label(RichText::new(cx.s.setup_next_game).weak());
}
