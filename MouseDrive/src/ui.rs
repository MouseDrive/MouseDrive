use std::sync::atomic::Ordering;
use std::time::Duration;

use eframe::egui;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

use crate::MouseDriveApp;
use crate::config::{Config, get_config_path};
use crate::curve::{Curve, CurvePreset};
use crate::curve_editor::{CurveDisplay, curve_editor};
use crate::input::{LEFT_BUTTON, RAW_INPUT_THREAD_ID, RIGHT_BUTTON};
use crate::lang::{Lang, Strings, strings};
use crate::logic::{BrakeState, RampDir, STEERING_RANGE};
#[cfg(feature = "updater")]
use crate::update::UpdateStatus;
use crate::vjoy::VJoyStatus;

const TAB_STEERING: u8 = 0;
const TAB_THROTTLE: u8 = 1;
const TAB_BRAKE: u8 = 2;
const TAB_GENERAL: u8 = 3;

const WM_QUIT: u32 = 0x0012;

pub(crate) const ACCENT: egui::Color32 = egui::Color32::from_rgb(55, 138, 221);
const COL_THROTTLE: egui::Color32 = egui::Color32::from_rgb(99, 153, 34);
const COL_BRAKE: egui::Color32 = egui::Color32::from_rgb(226, 75, 74);
const COL_OK: egui::Color32 = egui::Color32::from_rgb(29, 158, 117);

impl eframe::App for MouseDriveApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.snapshot = self.shared.snapshot();
        self.vjoy_status = self.shared.vjoy_status();

        self.handle_update_restart(ctx);

        if ctx.input(|i| i.viewport().close_requested()) && !self.config.exit_on_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }

        let focused = ctx.input(|i| i.focused);
        let repaint_ms = if focused { 16 } else { 250 };
        ctx.request_repaint_after(Duration::from_millis(repaint_ms));

        let s = strings(Lang::from_i32(self.config.language));

        egui::SidePanel::left("settings_panel")
            .resizable(true)
            .default_width(340.0)
            .min_width(280.0)
            .show_animated(ctx, self.settings_panel_open, |ui| {
                ui.vertical_centered(|ui| {
                    ui.heading(s.settings);
                });
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
                    ui.selectable_value(&mut self.settings_tab, TAB_STEERING, s.tab_steering);
                    ui.selectable_value(&mut self.settings_tab, TAB_THROTTLE, s.tab_throttle);
                    ui.selectable_value(&mut self.settings_tab, TAB_BRAKE, s.tab_brake);
                    ui.selectable_value(&mut self.settings_tab, TAB_GENERAL, s.tab_general);
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.settings_tab {
                    TAB_STEERING => self.draw_steering_tab(ui, s),
                    TAB_THROTTLE => self.draw_throttle_tab(ui, s),
                    TAB_BRAKE => self.draw_brake_tab(ui, s),
                    TAB_GENERAL => self.draw_general_tab(ui, s),
                    _ => {}
                });

                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    if ui.button(s.btn_load).clicked()
                        && let Some(path) = get_config_path()
                        && let Some(mut cfg) = Config::load_from_file(&path)
                    {
                        cfg.validate();
                        self.config = cfg;
                        self.sync_globals_from_config();
                        self.shared.request_curves_reseed();
                    }
                    if ui.button(s.btn_save).clicked()
                        && let Some(path) = get_config_path()
                    {
                        let _ = self.config.save_to_file(&path);
                    }
                    if ui.button(s.btn_default).clicked() {
                        self.config = Config::default();
                        self.sync_globals_from_config();
                        self.shared.request_curves_reseed();
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);

            ui.horizontal(|ui| {
                if ui.button(s.settings).clicked() {
                    self.settings_panel_open = !self.settings_panel_open;
                }
                ui.heading(&self.title);
                self.draw_update_button(ui, s);
            });
            ui.separator();

            if let Some(n) = self.config_notice {
                egui::Frame::NONE
                    .inner_margin(egui::Margin::symmetric(6, 4))
                    .corner_radius(4.0)
                    .fill(ui.visuals().faint_bg_color)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                ui.visuals().warn_fg_color,
                                s.config_corrected.replace("{}", &n.to_string()),
                            );
                            if ui.small_button("\u{2715}").clicked() {
                                self.config_notice = None;
                            }
                        });
                    });
                ui.add_space(2.0);
            }

            ui.horizontal(|ui| {
                let neutral = ui.visuals().faint_bg_color;
                let weak = ui.visuals().weak_text_color();
                let connected = matches!(self.vjoy_status, VJoyStatus::Connected);
                chip(
                    ui,
                    vjoy_status_text(&self.vjoy_status, s),
                    connected.then_some(COL_OK),
                    neutral,
                );
                let cap = self.snapshot.capture_enabled;
                chip(
                    ui,
                    if cap {
                        s.capture_active
                    } else {
                        s.capture_paused
                    },
                    Some(if cap { ACCENT } else { weak }),
                    neutral,
                );
                chip(ui, s.capture_toggle_hint, None, neutral);
            });

            ui.add_space(4.0);

            {
                let norm =
                    (self.snapshot.steering_filtered / STEERING_RANGE).clamp(-1.0, 1.0) as f32;
                let pct = (norm.abs() * 100.0).round();
                let txt = if norm < -0.001 {
                    format!("\u{25C0} {} {pct:.0}%", s.steer_left)
                } else if norm > 0.001 {
                    format!("{} {pct:.0}% \u{25B6}", s.steer_right)
                } else {
                    format!("{pct:.0}%")
                };
                ui.horizontal(|ui| {
                    ui.label(s.gauge_steering);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.colored_label(ACCENT, txt);
                    });
                });
                steering_bar(ui, norm);
                ui.add_space(6.0);
            }

            gauge(
                ui,
                s.gauge_throttle,
                self.snapshot.throttle as f32,
                &format!("{:.0}%", self.snapshot.throttle * 100.0),
                COL_THROTTLE,
            );
            gauge(
                ui,
                s.gauge_brake,
                self.snapshot.brake as f32,
                &format!("{:.0}%", self.snapshot.brake * 100.0),
                COL_BRAKE,
            );

            ui.separator();

            let lmb = LEFT_BUTTON.load(Ordering::Acquire);
            let rmb = RIGHT_BUTTON.load(Ordering::Acquire);
            let w_on = self.snapshot.w_key_pressed;
            let s_on = self.snapshot.s_key_pressed;

            ui.horizontal(|ui| {
                let neutral = ui.visuals().faint_bg_color;
                let active_bg = COL_OK.linear_multiply(0.25);
                let pill = |ui: &mut egui::Ui, text: &str, on: bool| {
                    chip(
                        ui,
                        text,
                        on.then_some(COL_OK),
                        if on { active_bg } else { neutral },
                    );
                };
                pill(ui, s.left_click, lmb);
                pill(ui, s.right_click, rmb);
                pill(ui, "W", w_on);
                pill(ui, "S", s_on);
            });

            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button(s.btn_reset_steering).clicked() {
                    self.shared.request_reset_steering();
                }
                if ui.button(s.btn_reconnect_vjoy).clicked() {
                    self.shared.request_reconnect();
                }
            });
        });

        self.publish_config();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_control_thread();

        let thread_id = RAW_INPUT_THREAD_ID.load(Ordering::SeqCst);
        if thread_id != 0 {
            unsafe {
                let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

fn vjoy_status_text<'a>(status: &VJoyStatus, s: &'a Strings) -> &'a str {
    match status {
        VJoyStatus::Connected => s.vjoy_connected,
        VJoyStatus::DllNotFound => s.vjoy_dll_not_found,
        VJoyStatus::DriverDisabled => s.vjoy_driver_disabled,
        VJoyStatus::DeviceBusy => s.vjoy_device_busy,
        VJoyStatus::DeviceMissing => s.vjoy_device_missing,
        VJoyStatus::AcquireFailed => s.vjoy_acquire_failed,
        VJoyStatus::Unknown => s.vjoy_unknown,
    }
}

impl MouseDriveApp {
    #[cfg(feature = "updater")]
    fn handle_update_restart(&mut self, _ctx: &egui::Context) {
        if !self.restart_initiated && self.update_checker.status() == UpdateStatus::ReadyToRestart {
            self.restart_initiated = true;
            self.stop_control_thread();
            let thread_id = crate::input::RAW_INPUT_THREAD_ID.load(Ordering::SeqCst);
            if thread_id != 0 {
                unsafe {
                    let _ = PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).spawn();
            }
            std::process::exit(0);
        }
    }

    #[cfg(not(feature = "updater"))]
    fn handle_update_restart(&mut self, _ctx: &egui::Context) {}

    #[cfg(feature = "updater")]
    fn draw_update_button(&mut self, ui: &mut egui::Ui, s: &Strings) {
        match self.update_checker.status() {
            UpdateStatus::Available(info) => {
                if info.version == self.config.skipped_version {
                    return;
                }
                let btn = egui::Button::new(
                    egui::RichText::new(format!("\u{2B06} {} {}", s.upd_update_btn, info.version))
                        .color(egui::Color32::WHITE)
                        .strong(),
                )
                .fill(egui::Color32::from_rgb(35, 134, 54));
                if ui.add(btn).clicked() {
                    if info.auto_installable() {
                        self.update_checker.spawn_update(info.clone());
                    } else {
                        ui.ctx()
                            .open_url(egui::OpenUrl::new_tab(info.html_url.clone()));
                    }
                }
                if ui.small_button(s.upd_skip).clicked() {
                    self.config.skipped_version = info.version.clone();
                    if let Some(path) = get_config_path() {
                        let _ = self.config.save_to_file(&path);
                    }
                }
            }
            UpdateStatus::Updating => {
                ui.spinner();
                ui.label(s.upd_updating);
            }
            UpdateStatus::ReadyToRestart => {
                ui.label(s.upd_restarting);
            }
            UpdateStatus::UpdateFailed(info) => {
                ui.colored_label(ui.visuals().error_fg_color, s.upd_update_failed);
                ui.hyperlink_to(s.upd_download, info.html_url.clone());
            }
            _ => {}
        }
    }

    #[cfg(not(feature = "updater"))]
    fn draw_update_button(&mut self, _ui: &mut egui::Ui, _s: &Strings) {}

    #[cfg(feature = "updater")]
    fn draw_update_section(&mut self, ui: &mut egui::Ui, s: &Strings) {
        ui.checkbox(&mut self.config.auto_check_updates, s.upd_auto_check);
        ui.horizontal(|ui| {
            if ui.button(s.upd_check_now).clicked() {
                self.update_checker.spawn_check();
            }
            match self.update_checker.status() {
                UpdateStatus::Checking => {
                    ui.label(s.upd_checking);
                }
                UpdateStatus::UpToDate => {
                    ui.label(s.upd_up_to_date);
                }
                UpdateStatus::Failed => {
                    ui.label(s.upd_failed);
                }
                UpdateStatus::Available(info) => {
                    ui.label(format!("{} {}", s.upd_available, info.version));
                }
                UpdateStatus::Updating => {
                    ui.label(s.upd_updating);
                }
                UpdateStatus::ReadyToRestart => {
                    ui.label(s.upd_restarting);
                }
                UpdateStatus::UpdateFailed(_) => {
                    ui.label(s.upd_update_failed);
                }
                UpdateStatus::Idle => {}
            }
        });
        ui.separator();
    }

    #[cfg(not(feature = "updater"))]
    fn draw_update_section(&mut self, _ui: &mut egui::Ui, _s: &Strings) {}

    fn draw_steering_tab(&mut self, ui: &mut egui::Ui, s: &Strings) {
        slider_f64(ui, s.sensitivity, &mut self.config.mouse_sens, 0.5..=10.0);

        if slider_f64(ui, s.dpi_scale, &mut self.config.mouse_dpi_scale, 0.5..=2.0) {
            self.sync_globals_from_config();
        }
        if slider_i32(ui, s.delta_cap, &mut self.config.mouse_delta_cap, 50..=800) {
            self.sync_globals_from_config();
        }

        ui.horizontal(|ui| {
            ui.label(s.mode);
            egui::ComboBox::from_id_salt("steering_mode")
                .selected_text(match self.config.steering_mode {
                    0 => s.mode_linear,
                    1 => s.mode_expo,
                    2 => s.mode_filtered,
                    3 => s.mode_self_center,
                    _ => s.mode_linear,
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.config.steering_mode, 0, s.mode_linear);
                    ui.selectable_value(&mut self.config.steering_mode, 1, s.mode_expo);
                    ui.selectable_value(&mut self.config.steering_mode, 2, s.mode_filtered);
                    ui.selectable_value(&mut self.config.steering_mode, 3, s.mode_self_center);
                });
        });

        slider_f64(
            ui,
            s.deadzone,
            &mut self.config.steering_deadzone,
            0.0..=0.5,
        );
        slider_f64(
            ui,
            s.saturation,
            &mut self.config.steering_saturation,
            0.5..=1.0,
        );
        slider_f64(ui, s.expo_power, &mut self.config.steering_expo, 0.5..=3.0);
        slider_f64(
            ui,
            s.filter_alpha,
            &mut self.config.steering_filter_alpha,
            0.0..=1.0,
        );
        slider_f64(
            ui,
            s.self_center_strength,
            &mut self.config.steering_spring_strength,
            0.0..=1.0,
        );
    }

    fn draw_throttle_tab(&mut self, ui: &mut egui::Ui, s: &Strings) {
        slider_f64(
            ui,
            s.cut_start,
            &mut self.config.throttle_cut_start,
            0.0..=0.5,
        );
        slider_f64(ui, s.cut_max, &mut self.config.throttle_cut_max, 0.3..=1.0);
        slider_f64(
            ui,
            s.min_at_full_lock,
            &mut self.config.throttle_min_cut_at_full,
            0.3..=0.95,
        );
        slider_i32(ui, s.ramp_ms, &mut self.config.throttle_ramp_ms, 10..=1000);
        slider_i32(ui, s.drop_ms, &mut self.config.throttle_drop_ms, 5..=200);
        slider_f64(
            ui,
            s.curve_power,
            &mut self.config.throttle_curve_exp,
            0.5..=4.0,
        );

        ui.separator();
        let rise_phase =
            (self.snapshot.throttle_dir == RampDir::Rising).then_some(self.snapshot.throttle_phase);
        let fall_phase = (self.snapshot.throttle_dir == RampDir::Falling)
            .then_some(self.snapshot.throttle_phase);
        let mut changed = false;
        changed |= curve_section(
            ui,
            s,
            s.curve_rise,
            "throttle_rise_curve",
            &mut self.config.throttle_rise_curve,
            CurveDisplay::Normal,
            rise_phase,
        );
        changed |= curve_section(
            ui,
            s,
            s.curve_fall,
            "throttle_fall_curve",
            &mut self.config.throttle_fall_curve,
            CurveDisplay::MirrorX,
            fall_phase,
        );
        if changed {
            self.shared.request_curves_reseed();
        }
    }

    fn draw_brake_tab(&mut self, ui: &mut egui::Ui, s: &Strings) {
        slider_f64(
            ui,
            s.min_ratio_base,
            &mut self.config.brake_min_ratio_base,
            0.0..=1.0,
        );
        slider_f64(
            ui,
            s.min_ratio_max,
            &mut self.config.brake_min_ratio_max,
            0.0..=1.0,
        );
        slider_f64(
            ui,
            s.brake_curve_power,
            &mut self.config.brake_curve_exp,
            0.5..=4.0,
        );
        ui.checkbox(&mut self.config.brake_trail_enabled, s.dynamic_minimum);
        slider_i32(ui, s.hold_ms, &mut self.config.brake_hold_ms, 100..=3000);
        slider_i32(
            ui,
            s.release_total_ms,
            &mut self.config.brake_release_total_ms,
            200..=5000,
        );
        slider_f64(
            ui,
            s.release_accel_power,
            &mut self.config.brake_release_accel_exp,
            0.5..=4.0,
        );
        slider_i32(
            ui,
            s.fast_apply_ms,
            &mut self.config.brake_fast_apply_ms,
            1..=200,
        );
        slider_i32(
            ui,
            s.fast_release_ms,
            &mut self.config.brake_fast_release_ms,
            10..=500,
        );
        slider_f64(
            ui,
            s.post_hold_ratio,
            &mut self.config.brake_after_release_hold_ratio,
            0.0..=0.5,
        );
        slider_i32(
            ui,
            s.post_hold_ms,
            &mut self.config.brake_after_release_hold_ms,
            0..=3000,
        );

        ui.separator();
        let apply_phase = (self.snapshot.brake_state == BrakeState::Press)
            .then_some(self.snapshot.brake_apply_phase);
        let posthold_phase = (self.snapshot.brake_state == BrakeState::PostHold)
            .then_some(self.snapshot.brake_posthold_phase);
        let mut changed = false;
        changed |= curve_section(
            ui,
            s,
            s.curve_apply,
            "brake_apply_curve",
            &mut self.config.brake_apply_curve,
            CurveDisplay::Normal,
            apply_phase,
        );
        changed |= curve_section(
            ui,
            s,
            s.curve_posthold,
            "brake_posthold_curve",
            &mut self.config.brake_posthold_curve,
            CurveDisplay::MirrorY,
            posthold_phase,
        );
        if changed {
            self.shared.request_curves_reseed();
        }
    }

    fn draw_general_tab(&mut self, ui: &mut egui::Ui, s: &Strings) {
        slider_i32(
            ui,
            s.update_interval_ms,
            &mut self.config.thread_interval_ms,
            1..=20,
        );

        ui.horizontal(|ui| {
            ui.label(s.vjoy_device);
            egui::ComboBox::from_id_salt("vjoy_device_id")
                .selected_text(self.config.vjoy_device_id.to_string())
                .show_ui(ui, |ui| {
                    for id in 1..=16 {
                        ui.selectable_value(&mut self.config.vjoy_device_id, id, id.to_string());
                    }
                });
        });

        if ui
            .checkbox(&mut self.config.input_sink_enabled, s.background_capture)
            .changed()
        {
            self.sync_globals_from_config();
        }
        ui.checkbox(&mut self.config.exit_on_close, s.exit_on_close);

        ui.separator();
        self.draw_update_section(ui, s);

        ui.horizontal(|ui| {
            ui.label(s.language);
            egui::ComboBox::from_id_salt("language_select")
                .selected_text(Lang::from_i32(self.config.language).label())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.config.language, 0, Lang::Tr.label());
                    ui.selectable_value(&mut self.config.language, 1, Lang::En.label());
                });
        });
    }
}

fn curve_section(
    ui: &mut egui::Ui,
    s: &Strings,
    title: &str,
    id_salt: &str,
    curve: &mut Curve,
    display: CurveDisplay,
    live_phase: Option<f64>,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(title)
        .id_salt(id_salt)
        .default_open(false)
        .show(ui, |ui| {
            changed |= curve_editor(ui, curve, display, live_phase);

            ui.horizontal(|ui| {
                ui.label(s.curve_mode);
                egui::ComboBox::from_id_salt(format!("{id_salt}_mode"))
                    .selected_text(if curve.mode == 1 {
                        s.curve_mode_smooth
                    } else {
                        s.curve_mode_linear
                    })
                    .show_ui(ui, |ui| {
                        changed |= ui
                            .selectable_value(&mut curve.mode, 0, s.curve_mode_linear)
                            .changed();
                        changed |= ui
                            .selectable_value(&mut curve.mode, 1, s.curve_mode_smooth)
                            .changed();
                    });

                egui::ComboBox::from_id_salt(format!("{id_salt}_preset"))
                    .selected_text(s.curve_preset)
                    .show_ui(ui, |ui| {
                        let presets = [
                            (CurvePreset::Linear, s.preset_linear),
                            (CurvePreset::SCurve, s.preset_s_curve),
                            (CurvePreset::Aggressive, s.preset_aggressive),
                            (CurvePreset::Progressive, s.preset_progressive),
                        ];
                        for (preset, label) in presets {
                            if ui.selectable_label(false, label).clicked() {
                                *curve = Curve::preset(preset);
                                changed = true;
                            }
                        }
                    });

                if ui
                    .add_enabled(!curve.is_identity(), egui::Button::new(s.curve_reset))
                    .clicked()
                {
                    *curve = Curve::default();
                    changed = true;
                }
            });

            ui.label(egui::RichText::new(s.curve_hint).small().weak());
        });
    changed
}

fn slider_f64(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(val, range)).changed()
    })
    .inner
}

fn slider_i32(
    ui: &mut egui::Ui,
    label: &str,
    val: &mut i32,
    range: std::ops::RangeInclusive<i32>,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::Slider::new(val, range)).changed()
    })
    .inner
}

fn gauge(ui: &mut egui::Ui, label: &str, value: f32, pct: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.colored_label(color, pct);
        });
    });
    ui.add(
        egui::ProgressBar::new(value.clamp(0.0, 1.0))
            .fill(color)
            .desired_height(14.0),
    );
    ui.add_space(6.0);
}

fn steering_bar(ui: &mut egui::Ui, norm: f32) {
    let width = ui.available_width();
    let (resp, p) = ui.allocate_painter(egui::vec2(width, 14.0), egui::Sense::hover());
    let r = resp.rect;
    p.rect_filled(r, 4.0, ui.visuals().extreme_bg_color);
    let cx = r.center().x;
    p.line_segment(
        [egui::pos2(cx, r.top()), egui::pos2(cx, r.bottom())],
        egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
    );
    let half = r.width() * 0.5;
    let w = norm.abs().clamp(0.0, 1.0) * half;
    let fr = if norm >= 0.0 {
        egui::Rect::from_min_max(
            egui::pos2(cx, r.top() + 2.0),
            egui::pos2(cx + w, r.bottom() - 2.0),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(cx - w, r.top() + 2.0),
            egui::pos2(cx, r.bottom() - 2.0),
        )
    };
    p.rect_filled(fr, 3.0, ACCENT);
}

fn chip(ui: &mut egui::Ui, text: &str, dot: Option<egui::Color32>, bg: egui::Color32) {
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(c) = dot {
                    let (r, p) = ui.allocate_painter(egui::vec2(9.0, 9.0), egui::Sense::hover());
                    p.circle_filled(r.rect.center(), 3.5, c);
                }
                ui.label(text);
            });
        });
}
