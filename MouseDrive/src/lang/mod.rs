#![deny(unsafe_code)]

mod en;
mod tr;

use mousedrive::input::{Class, Skip};
use mousedrive::priority::ThreadPriority;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Tr = 0,
    En = 1,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Tr, Lang::En];

    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Lang::En,
            _ => Lang::Tr,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::Tr => "Türkçe",
            Lang::En => "English",
        }
    }

    pub fn num(self, v: f64, decimals: usize) -> String {
        let s = format!("{v:.decimals$}");
        match self {
            Lang::Tr => s.replace('.', ","),
            Lang::En => s,
        }
    }

    pub fn pct(self, fraction: f64, decimals: usize) -> String {
        let n = self.num(fraction * 100.0, decimals);
        match self {
            Lang::Tr => format!("%{n}"),
            Lang::En => format!("{n}%"),
        }
    }
}

pub fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .trim_matches(|c: char| c == '%' || c.is_whitespace())
        .replace(',', ".");
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    args.iter().fold(template.to_string(), |acc, (key, value)| {
        acc.replace(&format!("{{{key}}}"), value)
    })
}

pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::Tr => &tr::TR,
        Lang::En => &en::EN,
    }
}

#[cfg_attr(
    any(not(feature = "updater"), not(target_os = "linux")),
    allow(dead_code)
)]
pub struct Strings {
    pub nav_home: &'static str,
    pub nav_monitor: &'static str,
    pub nav_settings: &'static str,
    pub nav_hint: &'static str,
    pub home_title: &'static str,
    pub home_desc: &'static str,
    pub getting_started: &'static str,
    pub step_setup: &'static str,
    pub step_setup_desc: &'static str,
    pub step_bind: &'static str,
    pub step_bind_desc: &'static str,
    pub step_tune: &'static str,
    pub step_tune_desc: &'static str,
    pub live_title: &'static str,
    pub live_desc: &'static str,
    pub profile_actions: &'static str,
    pub profile_saved: &'static str,
    pub profile_unsaved: &'static str,
    pub advanced_controls: &'static str,
    pub advanced_hint: &'static str,
    pub steering_intro: &'static str,
    pub throttle_intro: &'static str,
    pub brake_intro: &'static str,
    pub general_intro: &'static str,
    pub monitor_intro: &'static str,
    pub steering_help: &'static str,
    pub throttle_help: &'static str,
    pub brake_help: &'static str,
    pub brake_response: &'static str,
    pub brake_preview: &'static str,
    pub curves_title: &'static str,
    pub connection_tools: &'static str,
    pub feedback_title: &'static str,
    pub save_hint: &'static str,
    pub save_short: &'static str,
    pub live_shortcuts: &'static str,
    pub learn_title: &'static str,
    pub learn_desc: &'static str,
    pub system_details: &'static str,
    pub monitor_empty: &'static str,
    pub monitor_lost: &'static str,

    pub status_labels: [&'static str; 8],
    pub det_setup_required: &'static str,
    pub det_device_busy: &'static str,
    pub det_lost: &'static str,
    pub det_binding: &'static str,
    pub det_paused: &'static str,
    pub det_not_reading: &'static str,
    pub det_mouse_missing: &'static str,
    pub det_degraded: &'static str,
    pub det_active: &'static str,
    pub det_no_pointer: &'static str,
    pub det_loop_stalled: &'static str,
    pub det_input_lost: &'static str,
    pub det_mouse_moved: &'static str,
    pub det_mouse_grabbed: &'static str,
    pub det_session_locked: &'static str,
    pub det_session_inactive: &'static str,
    pub reason_write_failures: &'static str,
    pub reason_slow_loop: &'static str,
    pub reason_clipped: &'static str,
    pub reason_version: &'static str,
    pub act_open_setup: &'static str,
    pub act_reconnect: &'static str,
    pub act_start: &'static str,
    pub act_pause: &'static str,
    pub act_enable_sink: &'static str,
    pub act_use_all_mice: &'static str,
    pub act_details: &'static str,
    pub act_cancel_bind: &'static str,
    pub chip_vjoy: &'static str,
    pub chip_mouse_hz: &'static str,
    pub pending_apply: &'static str,

    pub setup_title: &'static str,
    pub setup_intro: &'static str,
    pub chk_dll: &'static str,
    pub chk_driver: &'static str,
    pub chk_versions: &'static str,
    pub chk_device: &'static str,
    pub chk_axes: &'static str,
    pub chk_buttons: &'static str,
    pub chk_not_run: &'static str,
    pub fix_dll: &'static str,
    pub fix_driver: &'static str,
    pub fix_versions: &'static str,
    pub fix_alternate_dll: &'static str,
    pub fix_device_busy: &'static str,
    pub fix_device_missing: &'static str,
    pub fix_device_acquire: &'static str,
    pub fix_read_only: &'static str,
    pub fix_axes: &'static str,
    pub fix_buttons: &'static str,
    pub dev_free: &'static str,
    pub dev_busy: &'static str,
    pub dev_busy_unknown: &'static str,
    pub dev_missing: &'static str,
    pub dev_not_acquired: &'static str,
    pub dev_create_failed: &'static str,
    pub versions_match: &'static str,
    pub versions_mismatch: &'static str,
    pub axes_missing: &'static str,
    pub buttons_count: &'static str,
    pub setup_live_test: &'static str,
    pub setup_next_bind: &'static str,
    pub setup_next_game: &'static str,
    pub btn_recheck: &'static str,
    pub btn_setup_tool: &'static str,
    pub btn_setup_link: &'static str,
    pub device_number: &'static str,
    pub err_launch: &'static str,
    #[cfg(target_os = "linux")]
    pub chk_mouse_access: &'static str,
    #[cfg(target_os = "linux")]
    pub mouse_access_denied: &'static str,
    #[cfg(target_os = "linux")]
    pub mouse_access_none: &'static str,
    #[cfg(target_os = "linux")]
    pub fix_mouse_access: &'static str,
    #[cfg(target_os = "linux")]
    pub setup_devices: &'static str,
    #[cfg(target_os = "linux")]
    pub chk_wheel_access: &'static str,
    #[cfg(target_os = "linux")]
    pub wheel_ready: &'static str,
    #[cfg(target_os = "linux")]
    pub wheel_pending: &'static str,
    #[cfg(target_os = "linux")]
    pub wheel_not_joystick: &'static str,
    #[cfg(target_os = "linux")]
    pub wheel_no_seat: &'static str,
    #[cfg(target_os = "linux")]
    pub wheel_no_tag: &'static str,
    #[cfg(target_os = "linux")]
    pub fix_wheel_access: &'static str,
    #[cfg(target_os = "linux")]
    pub mouse_access_touchpad_only: &'static str,
    #[cfg(target_os = "linux")]
    pub mouse_access_touchpad_skipped: &'static str,
    #[cfg(target_os = "linux")]
    pub fix_touchpad_only: &'static str,
    #[cfg(target_os = "linux")]
    pub chk_keyboard_access: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_access_denied: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_access_none: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_not_needed: &'static str,
    #[cfg(target_os = "linux")]
    pub fix_keyboard_access: &'static str,
    #[cfg(target_os = "linux")]
    pub chk_setup_rule: &'static str,
    #[cfg(target_os = "linux")]
    pub setup_rule_broad: &'static str,
    #[cfg(target_os = "linux")]
    pub fix_setup_rule_broad: &'static str,
    #[cfg(target_os = "linux")]
    pub uinput_via_rules: &'static str,
    #[cfg(target_os = "linux")]
    pub setup_commands: &'static str,
    #[cfg(target_os = "linux")]
    pub setup_commands_intro: &'static str,
    #[cfg(target_os = "linux")]
    pub step_uinput: &'static str,
    #[cfg(target_os = "linux")]
    pub step_rule: &'static str,
    #[cfg(target_os = "linux")]
    pub step_apply: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_tier: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_tier_warning: &'static str,
    #[cfg(target_os = "linux")]
    pub keyboard_tier_why: &'static str,
    #[cfg(target_os = "linux")]
    pub step_keyboard_rule: &'static str,
    #[cfg(target_os = "linux")]
    pub step_keyboard_apply: &'static str,
    #[cfg(target_os = "linux")]
    pub btn_copy_keyboard: &'static str,
    #[cfg(target_os = "linux")]
    pub nixos_title: &'static str,
    #[cfg(target_os = "linux")]
    pub nixos_note: &'static str,
    #[cfg(target_os = "linux")]
    pub setup_proton_axes: &'static str,

    pub g_steering: &'static str,
    pub g_throttle: &'static str,
    pub g_brake: &'static str,
    pub steer_left: &'static str,
    pub steer_right: &'static str,
    pub steer_bar_tip: &'static str,
    pub throttle_gauge_tip: &'static str,
    pub brake_gauge_tip: &'static str,
    pub pill_lmb: &'static str,
    pub pill_rmb: &'static str,
    pub pill_gear_up: &'static str,
    pub pill_gear_down: &'static str,
    pub phase_prefix: &'static str,
    pub phase_names: [&'static str; 6],
    pub btn_reset_steering: &'static str,
    pub tip_reset_steering: &'static str,
    pub btn_bind_helper: &'static str,
    pub btn_setup: &'static str,
    pub test_counter_on: &'static str,

    pub diag_line: &'static str,
    pub diag_mouse: &'static str,
    pub footer_line: &'static str,
    pub footer_write_errors: &'static str,
    pub footer_reconnects: &'static str,
    pub diag_absolute: &'static str,
    pub diag_stuck: &'static str,
    pub diag_registration: &'static str,
    pub diag_dropped: &'static str,
    pub diag_reconciled: &'static str,
    pub diag_input_stopped: &'static str,
    pub diag_demoted: &'static str,
    pub diag_priority: &'static str,
    pub diag_latency: &'static str,
    pub prio_normal: &'static str,
    pub prio_nice: &'static str,
    pub prio_realtime: &'static str,
    pub prio_high: &'static str,
    pub prio_demoted: &'static str,
    pub prio_failed: &'static str,
    pub btn_copy_diag: &'static str,
    pub copied: &'static str,
    pub dev_classes: [&'static str; 5],
    pub dev_in_use: &'static str,
    pub dev_skip_own: &'static str,
    pub dev_skip_not_input: &'static str,
    pub dev_skip_gamepad: &'static str,
    pub dev_skip_gamepad_touchpad: &'static str,
    pub dev_skip_denied: &'static str,
    pub dev_skip_open_failed: &'static str,
    pub dev_list_columns: [&'static str; 6],
    pub dev_list_empty: &'static str,

    pub profile: &'static str,
    pub btn_save: &'static str,
    pub btn_save_as: &'static str,
    pub btn_duplicate: &'static str,
    pub btn_rename: &'static str,
    pub btn_delete: &'static str,
    pub btn_revert: &'static str,
    pub btn_undo: &'static str,
    pub btn_redo: &'static str,
    pub btn_open_folder: &'static str,
    pub tip_unsaved: &'static str,
    pub tip_revert: &'static str,
    pub dlg_new_name: &'static str,
    pub dlg_rename: &'static str,
    pub dlg_duplicate: &'static str,
    pub btn_ok: &'static str,
    pub btn_cancel: &'static str,
    pub switch_unsaved: &'static str,
    pub btn_dont_save: &'static str,
    pub delete_confirm: &'static str,
    pub name_empty: &'static str,
    pub name_too_long: &'static str,
    pub name_invalid_char: &'static str,
    pub name_trailing: &'static str,
    pub name_reserved: &'static str,
    pub name_duplicate: &'static str,
    pub err_profile_save: &'static str,
    pub err_profile_load: &'static str,
    pub err_profile_delete: &'static str,
    pub err_profile_list: &'static str,
    pub err_profile_backup: &'static str,
    pub profile_ambiguous: &'static str,
    pub profile_corrected: &'static str,
    pub ab_start: &'static str,
    pub ab_end: &'static str,
    pub tip_ab_start: &'static str,
    pub tip_ab_toggle: &'static str,
    pub tip_ab_end: &'static str,

    pub tab_steering: &'static str,
    pub tab_throttle: &'static str,
    pub tab_brake: &'static str,
    pub advanced: &'static str,
    pub tip_reset_default: &'static str,
    pub tip_changed: &'static str,
    pub tip_range: &'static str,
    pub btn_tab_defaults: &'static str,

    pub sens: &'static str,
    pub tip_sens: &'static str,
    pub cm_per_lock: &'static str,
    pub counts_per_lock: &'static str,
    pub cm_input: &'static str,
    pub tip_cm_input: &'static str,
    pub mode: &'static str,
    pub mode_names: [&'static str; 5],
    pub mode_desc: [&'static str; 5],
    pub expo: &'static str,
    pub tip_expo: &'static str,
    pub smoothing: &'static str,
    pub tip_smoothing: &'static str,
    pub spring: &'static str,
    pub tip_spring: &'static str,
    pub adaptive_cutoff: &'static str,
    pub tip_adaptive_cutoff: &'static str,
    pub adaptive_beta: &'static str,
    pub tip_adaptive_beta: &'static str,
    pub deadzone: &'static str,
    pub tip_deadzone: &'static str,
    pub saturation: &'static str,
    pub tip_saturation: &'static str,
    pub max_rate: &'static str,
    pub tip_max_rate: &'static str,
    pub dpi_scale: &'static str,
    pub tip_dpi_scale: &'static str,
    pub shared_note: &'static str,

    pub cut_enabled: &'static str,
    pub tip_cut_enabled: &'static str,
    pub cut_summary: &'static str,
    pub cut_off: &'static str,
    pub axis_steering_pct: &'static str,
    pub axis_max_throttle: &'static str,
    pub now_marker: &'static str,
    pub ramp: &'static str,
    pub tip_ramp: &'static str,
    pub drop: &'static str,
    pub tip_drop: &'static str,
    pub cut_start: &'static str,
    pub tip_cut_start: &'static str,
    pub cut_end: &'static str,
    pub tip_cut_end: &'static str,
    pub cut_floor: &'static str,
    pub tip_cut_floor: &'static str,
    pub cut_exp: &'static str,
    pub tip_cut_exp: &'static str,
    pub curve_rise: &'static str,
    pub curve_fall: &'static str,

    pub brake_ceiling: &'static str,
    pub tip_brake_ceiling: &'static str,
    pub timeline: &'static str,
    pub tip_timeline: &'static str,
    pub axis_ms: &'static str,
    pub axis_brake_pct: &'static str,
    pub release_marker: &'static str,
    pub fill_ms: &'static str,
    pub tip_fill_ms: &'static str,
    pub curve_apply: &'static str,
    pub hold_ms: &'static str,
    pub tip_hold_ms: &'static str,
    pub decay_ms: &'static str,
    pub tip_decay_ms: &'static str,
    pub decay_exp: &'static str,
    pub tip_decay_exp: &'static str,
    pub curve_decay: &'static str,
    pub floor_base: &'static str,
    pub tip_floor_base: &'static str,
    pub trail: &'static str,
    pub tip_trail: &'static str,
    pub floor_max: &'static str,
    pub tip_floor_max: &'static str,
    pub floor_exp: &'static str,
    pub tip_floor_exp: &'static str,
    pub post_hold_ratio: &'static str,
    pub tip_post_hold_ratio: &'static str,
    pub post_hold_ms: &'static str,
    pub tip_post_hold_ms: &'static str,
    pub tap_ms: &'static str,
    pub tip_tap_ms: &'static str,
    pub release_ms: &'static str,
    pub tip_release_ms: &'static str,
    pub phase_heads: [&'static str; 5],

    pub curve_mode: &'static str,
    pub curve_linear: &'static str,
    pub curve_smooth: &'static str,
    pub curve_preset: &'static str,
    pub preset_names: [&'static str; 4],
    pub curve_reset: &'static str,
    pub curve_hint: &'static str,
    pub curve_point: &'static str,
    pub curve_time: &'static str,
    pub curve_value: &'static str,

    pub monitor: &'static str,
    pub tip_monitor: &'static str,
    pub freeze: &'static str,
    pub resume: &'static str,
    pub window_secs: &'static str,
    pub tr_steer_out: &'static str,
    pub tr_steer_raw: &'static str,
    pub tr_throttle: &'static str,
    pub tr_brake: &'static str,
    pub tr_lmb: &'static str,
    pub tr_rmb: &'static str,
    pub mk_capture_on: &'static str,
    pub mk_capture_off: &'static str,
    pub mk_profile: &'static str,
    pub axis_seconds: &'static str,

    pub sec_output: &'static str,
    pub loop_rate: &'static str,
    pub tip_loop_rate: &'static str,
    pub loop_measured: &'static str,
    pub vjoy_device: &'static str,
    pub tip_vjoy_device: &'static str,
    pub sec_mouse: &'static str,
    pub mouse_device: &'static str,
    pub all_mice: &'static str,
    pub btn_use_last_mouse: &'static str,
    pub btn_refresh: &'static str,
    pub mouse_dpi: &'static str,
    pub tip_mouse_dpi: &'static str,
    pub dpi_unknown: &'static str,
    pub btn_measure: &'static str,
    pub input_sink: &'static str,
    pub tip_input_sink: &'static str,
    pub input_sink_off_warn: &'static str,
    pub sec_keys: &'static str,
    pub key_capture: &'static str,
    pub gear_keys: &'static str,
    pub tip_gear_keys: &'static str,
    pub key_gear_up: &'static str,
    pub key_gear_down: &'static str,
    pub key_ab: &'static str,
    pub key_none: &'static str,
    pub key_press: &'static str,
    pub key_clear: &'static str,
    pub tip_key: &'static str,
    pub mouse_button: &'static str,
    pub sec_feedback: &'static str,
    pub sounds: &'static str,
    pub volume: &'static str,
    pub btn_test_sound: &'static str,
    pub sec_view: &'static str,
    pub language: &'static str,
    pub colorblind: &'static str,
    pub tip_colorblind: &'static str,
    pub ui_zoom: &'static str,
    pub exit_on_close: &'static str,
    pub tip_exit_on_close: &'static str,
    pub sec_game: &'static str,
    pub game_checklist: [&'static str; 5],
    pub btn_game_guide: &'static str,

    pub calib_title: &'static str,
    pub calib_steps: &'static str,
    pub calib_distance: &'static str,
    pub btn_start: &'static str,
    pub btn_done: &'static str,
    pub btn_apply: &'static str,
    pub calib_counted: &'static str,
    pub calib_result: &'static str,
    pub calib_no_motion: &'static str,

    pub bind_title: &'static str,
    pub bind_intro: &'static str,
    pub bind_targets: [&'static str; 5],
    pub bind_countdown: &'static str,
    pub bind_sweep: &'static str,

    pub notice_corrected: &'static str,
    pub notice_unreadable: &'static str,
    pub notice_backup: &'static str,
    pub notice_backup_copy: &'static str,
    pub notice_backup_failed: &'static str,
    pub notice_newer: &'static str,
    pub notice_unknown_keys: &'static str,
    pub notice_foreign: &'static str,
    pub notice_portable_unmarked: &'static str,
    pub notice_portable_foreign: &'static str,
    pub notice_updated: &'static str,
    pub notice_update_rolled_back: &'static str,
    pub link_changelog: &'static str,
    pub notice_crashed: &'static str,
    pub notice_deadzone_reset: &'static str,
    pub err_deadzone_reset: &'static str,
    pub err_config_save: &'static str,
    pub err_no_config_dir: &'static str,
    pub btn_dismiss: &'static str,
    pub close_title: &'static str,
    pub close_body: &'static str,
    pub btn_save_quit: &'static str,
    pub btn_quit_nosave: &'static str,
    pub restart_title: &'static str,
    pub restart_body: &'static str,
    pub btn_save_restart: &'static str,
    pub btn_restart_nosave: &'static str,
    pub btn_later: &'static str,
    pub install_wait_title: &'static str,
    pub install_wait_body: &'static str,
    pub btn_keep_open: &'static str,
    pub btn_quit_now: &'static str,
    pub relaunch_failed_title: &'static str,
    pub relaunch_failed_body: &'static str,
    pub relaunch_restored_body: &'static str,
    pub relaunch_broken_body: &'static str,
    pub cli_help: &'static str,
    pub cli_unknown_arg: &'static str,
    pub instance_running_title: &'static str,
    pub instance_running_body: &'static str,
    pub root_refused_title: &'static str,
    pub root_refused_body: &'static str,
    pub no_display_title: &'static str,
    pub no_display_body: &'static str,
    pub missing_libs_title: &'static str,
    pub missing_libs_body: &'static str,

    pub upd_auto_check: &'static str,
    pub upd_check_now: &'static str,
    pub upd_checking: &'static str,
    pub upd_up_to_date: &'static str,
    pub upd_failed: &'static str,
    pub upd_available: &'static str,
    pub upd_skip: &'static str,
    pub upd_download: &'static str,
    pub upd_update_btn: &'static str,
    pub upd_update_tip: &'static str,
    pub upd_updating: &'static str,
    pub upd_restarting: &'static str,
    pub upd_installed: &'static str,
    pub upd_restart_now: &'static str,
    pub upd_update_failed: &'static str,
    pub upd_manual_package: &'static str,
    pub upd_manual_appimage: &'static str,
    pub upd_manual_no_build: &'static str,
    pub upd_manual_tip: &'static str,
    pub upd_fail_network: &'static str,
    pub upd_fail_verify: &'static str,
    pub upd_fail_not_writable: &'static str,
    pub upd_fail_self_test: &'static str,
    pub upd_fail_replace: &'static str,
    pub upd_fail_restore: &'static str,
    pub upd_fail_other: &'static str,
    pub upd_fail_tip: &'static str,
    pub upd_fail_restore_tip: &'static str,

    pub saving: &'static str,
    pub key_timed_out: &'static str,
    pub close_again_hint: &'static str,
    pub btn_save_csv: &'static str,
    pub tip_save_csv: &'static str,
    pub csv_saved: &'static str,
    pub csv_failed: &'static str,
    pub btn_open_folder_short: &'static str,
}

impl Strings {
    pub fn device_class(&self, class: Class) -> &'static str {
        self.dev_classes[class.index()]
    }

    pub fn thread_priority(&self, priority: ThreadPriority) -> String {
        match priority {
            ThreadPriority::Normal => self.prio_normal.into(),
            ThreadPriority::Nice(n) => fill(self.prio_nice, &[("n", &n.to_string())]),
            ThreadPriority::Realtime(p) => fill(self.prio_realtime, &[("n", &p.to_string())]),
            ThreadPriority::High => self.prio_high.into(),
            ThreadPriority::Demoted => self.prio_demoted.into(),
            ThreadPriority::Failed(code) => {
                let error = std::io::Error::from_raw_os_error(code).to_string();
                fill(self.prio_failed, &[("error", &error)])
            }
        }
    }

    pub fn device_skip(&self, skip: Skip) -> String {
        match skip {
            Skip::OwnDevice => self.dev_skip_own.into(),
            Skip::NotPointerNoKeys => self.dev_skip_not_input.into(),
            Skip::Gamepad => self.dev_skip_gamepad.into(),
            Skip::GamepadTouchpad => self.dev_skip_gamepad_touchpad.into(),
            Skip::PermissionDenied => self.dev_skip_denied.into(),
            Skip::OpenFailed(kind) => {
                fill(self.dev_skip_open_failed, &[("error", &kind.to_string())])
            }
        }
    }
}
