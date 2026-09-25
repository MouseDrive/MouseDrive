use serde::{Deserialize, Serialize};
use windows::Win32::UI::Shell::{CSIDL_APPDATA, SHGFP_TYPE_CURRENT, SHGetFolderPathW};

use crate::curve::Curve;

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub config_version: u32,

    pub thread_interval_ms: i32,
    pub input_sink_enabled: bool,
    pub exit_on_close: bool,
    pub capture_toggle_key: i32,
    pub language: i32,
    pub vjoy_device_id: i32,

    pub auto_check_updates: bool,
    pub last_update_check: i64,
    pub skipped_version: String,

    pub mouse_sens: f64,
    pub mouse_dpi_scale: f64,
    pub mouse_delta_cap: i32,
    pub steering_mode: i32,
    pub steering_expo: f64,
    pub steering_filter_alpha: f64,
    pub steering_deadzone: f64,
    pub steering_saturation: f64,
    pub steering_spring_strength: f64,

    pub throttle_curve_exp: f64,
    pub throttle_min_cut_at_full: f64,
    pub throttle_cut_start: f64,
    pub throttle_cut_max: f64,
    pub throttle_ramp_ms: i32,
    pub throttle_drop_ms: i32,

    pub brake_fast_apply_ms: i32,
    pub brake_hold_ms: i32,
    pub brake_release_total_ms: i32,
    pub brake_release_accel_exp: f64,
    pub brake_fast_release_ms: i32,
    pub brake_tap_ms: i32,
    pub brake_min_ratio_base: f64,
    pub brake_min_ratio_max: f64,
    pub brake_curve_exp: f64,
    pub brake_trail_enabled: bool,
    pub brake_after_release_hold_ratio: f64,
    pub brake_after_release_hold_ms: i32,

    pub throttle_rise_curve: Curve,
    pub throttle_fall_curve: Curve,
    pub brake_apply_curve: Curve,
    pub brake_posthold_curve: Curve,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: 1,

            thread_interval_ms: 4,
            input_sink_enabled: true,
            exit_on_close: true,
            capture_toggle_key: 0x77,
            language: 1,
            vjoy_device_id: 1,

            auto_check_updates: true,
            last_update_check: 0,
            skipped_version: String::new(),

            mouse_sens: 3.0,
            mouse_dpi_scale: 1.0,
            mouse_delta_cap: 180,
            steering_mode: 0,
            steering_expo: 1.5,
            steering_filter_alpha: 0.25,
            steering_deadzone: 0.02,
            steering_saturation: 1.0,
            steering_spring_strength: 0.15,

            throttle_curve_exp: 2.0,
            throttle_min_cut_at_full: 0.70,
            throttle_cut_start: 0.19,
            throttle_cut_max: 0.8,
            throttle_ramp_ms: 75,
            throttle_drop_ms: 25,

            brake_fast_apply_ms: 10,
            brake_hold_ms: 1750,
            brake_release_total_ms: 2500,
            brake_release_accel_exp: 1.7,
            brake_fast_release_ms: 65,
            brake_tap_ms: 120,
            brake_min_ratio_base: 0.40,
            brake_min_ratio_max: 0.55,
            brake_curve_exp: 2.0,
            brake_trail_enabled: false,
            brake_after_release_hold_ratio: 0.06,
            brake_after_release_hold_ms: 500,

            throttle_rise_curve: Curve::default(),
            throttle_fall_curve: Curve::default(),
            brake_apply_curve: Curve::default(),
            brake_posthold_curve: Curve::default(),
        }
    }
}

pub const CURRENT_CONFIG_VERSION: u32 = 1;

impl Config {
    pub fn load_from_file(path: &str) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut cfg: Config = toml::from_str(&content).ok()?;
        cfg.migrate();
        Some(cfg)
    }

    fn migrate(&mut self) {
        self.config_version = CURRENT_CONFIG_VERSION;
    }

    pub fn save_to_file(&self, path: &str) -> std::io::Result<()> {
        let content = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, content)
    }

    pub fn validate(&mut self) -> u32 {
        let mut corrected = 0u32;
        let d = Config::default();

        macro_rules! v_i {
            ($f:ident, $min:expr, $max:expr) => {
                if self.$f < $min || self.$f > $max {
                    self.$f = self.$f.clamp($min, $max);
                    corrected += 1;
                }
            };
        }

        macro_rules! v_f {
            ($f:ident, $min:expr, $max:expr) => {
                if self.$f.is_nan() || self.$f.is_infinite() {
                    self.$f = d.$f;
                    corrected += 1;
                } else if self.$f < $min || self.$f > $max {
                    self.$f = self.$f.clamp($min, $max);
                    corrected += 1;
                }
            };
        }

        v_i!(thread_interval_ms, 1, 20);
        v_i!(capture_toggle_key, 1, 255);
        v_i!(language, 0, 1);
        v_i!(vjoy_device_id, 1, 16);

        if self.last_update_check < 0 {
            self.last_update_check = 0;
            corrected += 1;
        }
        if self.skipped_version.len() > 32 {
            self.skipped_version.clear();
            corrected += 1;
        }

        v_f!(mouse_sens, 0.5, 10.0);
        v_f!(mouse_dpi_scale, 0.5, 2.0);
        v_i!(mouse_delta_cap, 50, 800);
        v_i!(steering_mode, 0, 3);
        v_f!(steering_expo, 0.5, 3.0);
        v_f!(steering_filter_alpha, 0.0, 1.0);
        v_f!(steering_deadzone, 0.0, 0.5);
        v_f!(steering_saturation, 0.5, 1.0);
        v_f!(steering_spring_strength, 0.0, 1.0);

        v_f!(throttle_curve_exp, 0.5, 4.0);
        v_f!(throttle_min_cut_at_full, 0.3, 0.95);
        v_f!(throttle_cut_start, 0.0, 0.5);
        v_f!(throttle_cut_max, 0.3, 1.0);
        v_i!(throttle_ramp_ms, 10, 1000);
        v_i!(throttle_drop_ms, 5, 200);

        v_i!(brake_fast_apply_ms, 1, 200);
        v_i!(brake_hold_ms, 100, 3000);
        v_i!(brake_release_total_ms, 200, 5000);
        v_f!(brake_release_accel_exp, 0.5, 4.0);
        v_i!(brake_fast_release_ms, 10, 500);
        v_i!(brake_tap_ms, 10, 500);
        v_f!(brake_min_ratio_base, 0.0, 1.0);
        v_f!(brake_min_ratio_max, 0.0, 1.0);
        v_f!(brake_curve_exp, 0.5, 4.0);
        v_f!(brake_after_release_hold_ratio, 0.0, 0.5);
        v_i!(brake_after_release_hold_ms, 0, 3000);

        corrected += self.throttle_rise_curve.validate();
        corrected += self.throttle_fall_curve.validate();
        corrected += self.brake_apply_curve.validate();
        corrected += self.brake_posthold_curve.validate();

        corrected
    }
}

pub fn get_config_path() -> Option<String> {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        let portable = exe_dir.join("config.toml");
        if portable.exists() {
            return portable.to_str().map(|s| s.to_string());
        }
    }

    unsafe {
        let mut buf = [0u16; 260];
        if SHGetFolderPathW(
            None,
            CSIDL_APPDATA as i32,
            None,
            SHGFP_TYPE_CURRENT.0 as u32,
            &mut buf,
        )
        .is_ok()
        {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let path = String::from_utf16_lossy(&buf[..len]);
            let config_dir = format!("{}\\MouseDrive", path);
            let _ = std::fs::create_dir_all(&config_dir);
            return Some(format!("{}\\config.toml", config_dir));
        }
    }
    None
}
