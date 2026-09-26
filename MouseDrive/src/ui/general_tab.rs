#![deny(unsafe_code)]

use eframe::egui::{ComboBox, DragValue, OpenUrl, RichText, Ui};
use mousedrive::config::{Config, MOUSE_DPI_RANGE};
use mousedrive::control::{Command, Shared, Snapshot};
use mousedrive::input::{self, DeviceFilter, MouseInfo};
use mousedrive::status::Cue;

use super::calibration::Calibration;
use super::keybind::{KeyBinder, KeyField, key_row};
#[cfg(feature = "updater")]
use super::update_ui::UpdaterUi;
use super::widgets::{
    Cx, Fmt, Param, check_row, combo_row, grid, param_row, param_row_i32, section,
};
use crate::lang::{Lang, fill};

const GAME_GUIDE_URL: &str = "https://github.com/MouseDrive/MouseDrive#in-game-settings";
pub const LOOP_INTERVALS_MS: [i32; 6] = [1, 2, 4, 5, 8, 10];
const MOUSE_COMBO_WIDTH: f32 = 220.0;
const SHOW_VJOY_DEVICE: bool = cfg!(windows);

pub struct Env<'a> {
    pub snap: &'a Snapshot,
    pub shared: &'a Shared,
    pub mice: &'a mut Vec<MouseInfo>,
    pub keybind: &'a mut KeyBinder,
    pub calibration: &'a mut Calibration,
    #[cfg(feature = "updater")]
    pub updater: &'a mut UpdaterUi,
}

pub fn show(ui: &mut Ui, cx: &Cx, cfg: &mut Config, env: Env) {
    output(ui, cx, cfg, env.snap);
    mouse(ui, cx, cfg, env.mice, env.calibration);
    if env.snap.device_filter == DeviceFilter::SelectedMissing {
        ui.label(RichText::new(cx.s.filter_missing).color(ui.visuals().warn_fg_color));
    }
    keys(ui, cx, cfg, env.keybind);
    feedback(ui, cx, cfg, env.shared);
    view(ui, cx, cfg);
    #[cfg(feature = "updater")]
    env.updater.section(ui, cx, cfg);
    game(ui, cx);
}

pub fn loop_label(ms: i32) -> String {
    format!("{} Hz ({ms} ms)", 1000 / ms.max(1))
}

fn output(ui: &mut Ui, cx: &Cx, cfg: &mut Config, snap: &Snapshot) {
    let (s, d) = (cx.s, Config::default());
    section(ui, s.sec_output);
    grid(ui, "gen_output", |ui| {
        let label = ui.label(s.loop_rate).on_hover_text(s.tip_loop_rate);
        ComboBox::from_id_salt("loop_rate")
            .selected_text(loop_label(cfg.thread_interval_ms))
            .show_ui(ui, |ui| {
                for ms in LOOP_INTERVALS_MS {
                    ui.selectable_value(&mut cfg.thread_interval_ms, ms, loop_label(ms));
                }
            })
            .response
            .labelled_by(label.id)
            .on_hover_text(s.tip_loop_rate);
        ui.label("");
        ui.end_row();
        if SHOW_VJOY_DEVICE {
            let device = Param::new(
                s.vjoy_device,
                s.tip_vjoy_device,
                f64::from(d.vjoy_device_id),
                1.0..=16.0,
                Fmt::Num(0),
            );
            param_row_i32(ui, cx, &device, &mut cfg.vjoy_device_id);
        }
    });
    let summary = &snap.loop_summary;
    let measured = fill(
        s.loop_measured,
        &[
            ("hz", &cx.lang.num(summary.hz, 0)),
            ("p99", &cx.lang.num(summary.p99_ms, 2)),
        ],
    );
    ui.label(RichText::new(measured).weak());
}

fn mouse(
    ui: &mut Ui,
    cx: &Cx,
    cfg: &mut Config,
    mice: &mut Vec<MouseInfo>,
    calibration: &mut Calibration,
) {
    let (s, d) = (cx.s, Config::default());
    section(ui, s.sec_mouse);
    mouse_picker(ui, cx, &mut cfg.mouse_device, mice);
    ui.horizontal(|ui| {
        let label = ui.label(s.mouse_dpi).on_hover_text(s.tip_mouse_dpi);
        let unknown = s.dpi_unknown;
        let field = DragValue::new(&mut cfg.mouse_dpi)
            .range(0..=*MOUSE_DPI_RANGE.end())
            .speed(10.0)
            .update_while_editing(false)
            .custom_formatter(move |v, _| {
                if v < 1.0 {
                    unknown.to_string()
                } else {
                    format!("{v:.0}")
                }
            });
        let edited = ui
            .add(field)
            .labelled_by(label.id)
            .on_hover_text(s.tip_mouse_dpi)
            .changed();
        if edited {
            cfg.mouse_dpi = normalize_dpi(cfg.mouse_dpi);
        }
        if ui.button(s.btn_measure).clicked() {
            calibration.open();
        }
    });
    grid(ui, "gen_sink", |ui| {
        let label = (s.input_sink, s.tip_input_sink);
        check_row(
            ui,
            cx,
            label,
            &mut cfg.input_sink_enabled,
            d.input_sink_enabled,
        );
    });
    if !cfg.input_sink_enabled {
        ui.label(RichText::new(s.input_sink_off_warn).color(ui.visuals().error_fg_color));
    }
}

pub fn normalize_dpi(dpi: i32) -> i32 {
    if dpi <= 0 {
        0
    } else {
        dpi.clamp(*MOUSE_DPI_RANGE.start(), *MOUSE_DPI_RANGE.end())
    }
}

fn mouse_picker(ui: &mut Ui, cx: &Cx, device: &mut String, mice: &mut Vec<MouseInfo>) {
    let s = cx.s;
    let selected = mouse_label(device, mice, s.all_mice);
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(s.mouse_device);
        ComboBox::from_id_salt("mouse_device")
            .selected_text(selected)
            .width(MOUSE_COMBO_WIDTH)
            .show_ui(ui, |ui| {
                ui.selectable_value(device, String::new(), s.all_mice);
                for m in mice.iter() {
                    ui.selectable_value(device, m.path.clone(), &m.label);
                }
            })
            .response
            .labelled_by(label.id);
        if ui.button(s.btn_refresh).clicked() {
            *mice = input::list_mice();
        }
        if ui.button(s.btn_use_last_mouse).clicked()
            && let Some(path) = input::last_moved_device()
        {
            *device = path;
            *mice = input::list_mice();
        }
    });
}

pub fn mouse_label(device: &str, mice: &[MouseInfo], all: &str) -> String {
    if device.is_empty() {
        return all.to_string();
    }
    mice.iter()
        .find(|m| m.path.eq_ignore_ascii_case(device))
        .map_or_else(|| device.to_string(), |m| m.label.clone())
}

fn keys(ui: &mut Ui, cx: &Cx, cfg: &mut Config, binder: &mut KeyBinder) {
    let (s, d) = (cx.s, Config::default());
    section(ui, s.sec_keys);
    grid(ui, "gen_keys", |ui| {
        key_row(
            ui,
            cx,
            binder,
            (KeyField::Capture, s.key_capture),
            &mut cfg.capture_toggle_key,
            false,
        );
        check_row(
            ui,
            cx,
            (s.gear_keys, s.tip_gear_keys),
            &mut cfg.gear_keys_enabled,
            d.gear_keys_enabled,
        );
        if cfg.gear_keys_enabled {
            key_row(
                ui,
                cx,
                binder,
                (KeyField::GearUp, s.key_gear_up),
                &mut cfg.gear_up_key,
                false,
            );
            key_row(
                ui,
                cx,
                binder,
                (KeyField::GearDown, s.key_gear_down),
                &mut cfg.gear_down_key,
                false,
            );
        }
        key_row(
            ui,
            cx,
            binder,
            (KeyField::Ab, s.key_ab),
            &mut cfg.ab_toggle_key,
            true,
        );
    });
}

fn feedback(ui: &mut Ui, cx: &Cx, cfg: &mut Config, shared: &Shared) {
    let (s, d) = (cx.s, Config::default());
    section(ui, s.sec_feedback);
    grid(ui, "gen_feedback", |ui| {
        check_row(
            ui,
            cx,
            (s.sounds, s.sounds),
            &mut cfg.sounds_enabled,
            d.sounds_enabled,
        );
        if cfg.sounds_enabled {
            let volume = Param::new(
                s.volume,
                s.sounds,
                f64::from(d.sound_volume),
                0.0..=100.0,
                Fmt::Num(0),
            );
            param_row_i32(ui, cx, &volume, &mut cfg.sound_volume);
        }
        check_row(
            ui,
            cx,
            (s.overlay, s.tip_overlay),
            &mut cfg.overlay_enabled,
            d.overlay_enabled,
        );
        if cfg.overlay_enabled {
            combo_row(
                ui,
                cx,
                (s.corner, s.tip_overlay),
                &mut cfg.overlay_corner,
                &s.corners,
                d.overlay_corner,
            );
        }
    });
    if cfg.sounds_enabled && ui.button(s.btn_test_sound).clicked() {
        shared.send(Command::PlayCue(Cue::CaptureOn));
    }
}

fn view(ui: &mut Ui, cx: &Cx, cfg: &mut Config) {
    let (s, d) = (cx.s, Config::default());
    section(ui, s.sec_view);
    let languages: Vec<&str> = Lang::ALL.iter().map(|l| l.label()).collect();
    grid(ui, "gen_view", |ui| {
        combo_row(
            ui,
            cx,
            (s.language, s.language),
            &mut cfg.language,
            &languages,
            d.language,
        );
        let colorblind = (s.colorblind, s.tip_colorblind);
        check_row(
            ui,
            cx,
            colorblind,
            &mut cfg.colorblind_palette,
            d.colorblind_palette,
        );
        let zoom = Param::new(s.ui_zoom, s.ui_zoom, d.ui_zoom, 0.75..=2.0, Fmt::PCT0);
        param_row(ui, cx, &zoom, &mut cfg.ui_zoom);
        let exit = (s.exit_on_close, s.tip_exit_on_close);
        check_row(ui, cx, exit, &mut cfg.exit_on_close, d.exit_on_close);
    });
}

fn game(ui: &mut Ui, cx: &Cx) {
    let s = cx.s;
    section(ui, s.sec_game);
    for item in s.game_checklist {
        ui.label(format!("• {item}"));
    }
    if ui.button(s.btn_game_guide).clicked() {
        ui.ctx().open_url(OpenUrl::new_tab(GAME_GUIDE_URL));
    }
}
