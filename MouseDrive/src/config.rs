#![deny(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::curve::Curve;
use crate::logic::filters::{MAX_SMOOTHING_MS, tau_from_legacy_alpha};

pub const CURRENT_CONFIG_VERSION: u32 = 3;

const LEGACY_DEFAULT_DEADZONE: f64 = 0.02;

pub const DEFAULT_PROFILE: &str = "Default";

pub const MOUSE_DPI_RANGE: std::ops::RangeInclusive<i32> = 100..=32000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    pub mouse_sens: f64,
    pub steering_mode: i32,
    pub steering_expo: f64,
    pub steering_smoothing_ms: f64,
    pub steering_adaptive_min_cutoff_hz: f64,
    pub steering_adaptive_beta: f64,
    pub steering_deadzone: f64,
    pub steering_saturation: f64,
    pub steering_spring_strength: f64,

    pub throttle_cut_enabled: bool,
    pub throttle_curve_exp: f64,
    pub throttle_min_cut_at_full: f64,
    pub throttle_cut_start: f64,
    pub throttle_cut_max: f64,
    pub throttle_ramp_ms: i32,
    pub throttle_drop_ms: i32,

    pub brake_max_output: f64,
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

impl Default for Tuning {
    fn default() -> Self {
        Self {
            mouse_sens: 3.0,
            steering_mode: 0,
            steering_expo: 1.5,
            steering_smoothing_ms: 13.9,
            steering_adaptive_min_cutoff_hz: 5.0,
            steering_adaptive_beta: 5.0,
            steering_deadzone: 0.0,
            steering_saturation: 1.0,
            steering_spring_strength: 0.15,

            throttle_cut_enabled: true,
            throttle_curve_exp: 2.0,
            throttle_min_cut_at_full: 0.70,
            throttle_cut_start: 0.19,
            throttle_cut_max: 0.8,
            throttle_ramp_ms: 75,
            throttle_drop_ms: 25,

            brake_max_output: 1.0,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub config_version: u32,

    pub thread_interval_ms: i32,
    pub input_sink_enabled: bool,
    pub exit_on_close: bool,
    pub capture_toggle_key: i32,
    pub language: i32,
    pub vjoy_device_id: i32,
    pub active_profile: String,

    pub gear_keys_enabled: bool,
    pub gear_up_key: i32,
    pub gear_down_key: i32,
    pub ab_toggle_key: i32,

    pub sounds_enabled: bool,
    pub sound_volume: i32,
    pub overlay_enabled: bool,
    pub overlay_corner: i32,
    pub colorblind_palette: bool,
    pub ui_zoom: f64,

    pub auto_check_updates: bool,
    pub last_update_check: i64,
    pub skipped_version: String,

    pub mouse_dpi_scale: f64,
    pub mouse_dpi: i32,
    pub mouse_device: String,
    pub steering_max_rate_pct_s: f64,

    #[serde(flatten)]
    pub tuning: Tuning,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: CURRENT_CONFIG_VERSION,

            thread_interval_ms: 4,
            input_sink_enabled: true,
            exit_on_close: true,
            capture_toggle_key: 0x77,
            language: 1,
            vjoy_device_id: 1,
            active_profile: DEFAULT_PROFILE.to_string(),

            gear_keys_enabled: true,
            gear_up_key: 0x57,
            gear_down_key: 0x53,
            ab_toggle_key: 0,

            sounds_enabled: true,
            sound_volume: 60,
            overlay_enabled: false,
            overlay_corner: 1,
            colorblind_palette: false,
            ui_zoom: 1.0,

            auto_check_updates: true,
            last_update_check: 0,
            skipped_version: String::new(),

            mouse_dpi_scale: 1.0,
            mouse_dpi: 0,
            mouse_device: String::new(),
            steering_max_rate_pct_s: 0.0,

            tuning: Tuning::default(),
        }
    }
}

macro_rules! clamp_i {
    ($s:ident, $out:ident, $f:ident, $min:expr, $max:expr) => {
        if $s.$f < $min || $s.$f > $max {
            $s.$f = $s.$f.clamp($min, $max);
            $out.push(stringify!($f));
        }
    };
}

macro_rules! clamp_f {
    ($s:ident, $d:ident, $out:ident, $f:ident, $min:expr, $max:expr) => {
        if !$s.$f.is_finite() {
            $s.$f = $d.$f;
            $out.push(stringify!($f));
        } else if $s.$f < $min || $s.$f > $max {
            $s.$f = $s.$f.clamp($min, $max);
            $out.push(stringify!($f));
        }
    };
}

macro_rules! check_curve {
    ($s:ident, $out:ident, $f:ident) => {
        if $s.$f.validate() > 0 {
            $out.push(stringify!($f));
        }
    };
}

impl Tuning {
    pub fn validate(&mut self) -> Vec<&'static str> {
        let d = Tuning::default();
        let mut out = Vec::new();

        clamp_f!(self, d, out, mouse_sens, 0.5, 10.0);
        clamp_i!(self, out, steering_mode, 0, 4);
        clamp_f!(self, d, out, steering_expo, 0.5, 3.0);
        clamp_f!(self, d, out, steering_smoothing_ms, 0.0, MAX_SMOOTHING_MS);
        clamp_f!(self, d, out, steering_adaptive_min_cutoff_hz, 0.5, 30.0);
        clamp_f!(self, d, out, steering_adaptive_beta, 0.0, 50.0);
        clamp_f!(self, d, out, steering_deadzone, 0.0, 0.5);
        clamp_f!(self, d, out, steering_saturation, 0.5, 1.0);
        clamp_f!(self, d, out, steering_spring_strength, 0.0, 1.0);

        clamp_f!(self, d, out, throttle_curve_exp, 0.5, 4.0);
        clamp_f!(self, d, out, throttle_min_cut_at_full, 0.3, 0.95);
        clamp_f!(self, d, out, throttle_cut_start, 0.0, 0.5);
        clamp_f!(self, d, out, throttle_cut_max, 0.3, 1.0);
        clamp_i!(self, out, throttle_ramp_ms, 10, 1000);
        clamp_i!(self, out, throttle_drop_ms, 5, 200);

        clamp_f!(self, d, out, brake_max_output, 0.1, 1.0);
        clamp_i!(self, out, brake_fast_apply_ms, 1, 200);
        clamp_i!(self, out, brake_hold_ms, 100, 3000);
        clamp_i!(self, out, brake_release_total_ms, 200, 5000);
        clamp_f!(self, d, out, brake_release_accel_exp, 0.5, 4.0);
        clamp_i!(self, out, brake_fast_release_ms, 10, 500);
        clamp_i!(self, out, brake_tap_ms, 10, 500);
        clamp_f!(self, d, out, brake_min_ratio_base, 0.0, 1.0);
        clamp_f!(self, d, out, brake_min_ratio_max, 0.0, 1.0);
        clamp_f!(self, d, out, brake_curve_exp, 0.5, 4.0);
        clamp_f!(self, d, out, brake_after_release_hold_ratio, 0.0, 0.5);
        clamp_i!(self, out, brake_after_release_hold_ms, 0, 3000);

        check_curve!(self, out, throttle_rise_curve);
        check_curve!(self, out, throttle_fall_curve);
        check_curve!(self, out, brake_apply_curve);
        check_curve!(self, out, brake_posthold_curve);

        out
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))
    }

    pub fn from_toml_str(content: &str, interval_ms: f64) -> Result<Self, ConfigError> {
        let raw: toml::Table = toml::from_str(content).map_err(parse_error)?;
        let mut tuning: Tuning = toml::from_str(content).map_err(parse_error)?;
        migrate_legacy_tuning(&mut tuning, &raw, interval_ms);
        Ok(tuning)
    }
}

impl Config {
    pub fn validate(&mut self) -> Vec<&'static str> {
        let d = Config::default();
        let mut out = Vec::new();

        clamp_i!(self, out, thread_interval_ms, 1, 20);
        clamp_i!(self, out, capture_toggle_key, 1, 255);
        clamp_i!(self, out, language, 0, 1);
        clamp_i!(self, out, vjoy_device_id, 1, 16);
        if crate::profiles::validate_name(&self.active_profile).is_err() {
            self.active_profile = DEFAULT_PROFILE.to_string();
            out.push("active_profile");
        }

        clamp_i!(self, out, gear_up_key, 1, 255);
        clamp_i!(self, out, gear_down_key, 1, 255);
        clamp_i!(self, out, ab_toggle_key, 0, 255);

        clamp_i!(self, out, sound_volume, 0, 100);
        clamp_i!(self, out, overlay_corner, 0, 3);
        clamp_f!(self, d, out, ui_zoom, 0.75, 2.0);

        if self.last_update_check < 0 {
            self.last_update_check = 0;
            out.push("last_update_check");
        }
        if self.skipped_version.len() > 32 {
            self.skipped_version.clear();
            out.push("skipped_version");
        }

        clamp_f!(self, d, out, mouse_dpi_scale, 0.5, 2.0);
        if self.mouse_dpi != 0 && !MOUSE_DPI_RANGE.contains(&self.mouse_dpi) {
            self.mouse_dpi = self
                .mouse_dpi
                .clamp(*MOUSE_DPI_RANGE.start(), *MOUSE_DPI_RANGE.end());
            out.push("mouse_dpi");
        }
        if self.mouse_device.len() > 512 {
            self.mouse_device.clear();
            out.push("mouse_device");
        }
        clamp_f!(self, d, out, steering_max_rate_pct_s, 0.0, 10000.0);

        out.extend(self.tuning.validate());
        out
    }

    pub fn with_tuning(&self, tuning: Tuning) -> Config {
        Config {
            tuning,
            ..self.clone()
        }
    }

    pub fn from_toml_str(content: &str) -> Result<Self, ConfigError> {
        Self::parse(content).map(|(cfg, _)| cfg)
    }

    fn parse(content: &str) -> Result<(Self, i64), ConfigError> {
        let raw: toml::Table = toml::from_str(content).map_err(parse_error)?;
        let mut cfg: Config = toml::from_str(content).map_err(parse_error)?;
        let file_version = raw
            .get("config_version")
            .and_then(toml::Value::as_integer)
            .unwrap_or(1);
        if file_version < 2 {
            let interval = f64::from(cfg.thread_interval_ms.max(1));
            migrate_legacy_tuning(&mut cfg.tuning, &raw, interval);
        }
        if file_version < 3 {
            reset_legacy_deadzone(&mut cfg.tuning);
        }
        cfg.config_version = CURRENT_CONFIG_VERSION;
        Ok((cfg, file_version))
    }

    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        Self::read(path).map(|(cfg, _)| cfg)
    }

    fn read(path: &Path) -> Result<(Self, i64), ConfigError> {
        let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::parse(&content)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        crate::fsutil::write_atomic(path, content.as_bytes()).map_err(ConfigError::Io)
    }
}

fn migrate_legacy_tuning(tuning: &mut Tuning, raw: &toml::Table, interval_ms: f64) {
    if raw.contains_key("steering_smoothing_ms") {
        return;
    }
    if let Some(alpha) = raw.get("steering_filter_alpha").and_then(toml_number) {
        tuning.steering_smoothing_ms = tau_from_legacy_alpha(alpha, interval_ms);
    }
}

pub fn reset_legacy_deadzone(tuning: &mut Tuning) -> bool {
    let is_legacy = (tuning.steering_deadzone - LEGACY_DEFAULT_DEADZONE).abs() < f64::EPSILON;
    if is_legacy {
        tuning.steering_deadzone = 0.0;
    }
    is_legacy
}

fn toml_number(v: &toml::Value) -> Option<f64> {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64))
}

fn parse_error(e: toml::de::Error) -> ConfigError {
    ConfigError::Parse(e.to_string())
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(String),
    Serialize(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "{e}"),
            ConfigError::Parse(e) | ConfigError::Serialize(e) => f.write_str(e.trim_end()),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, PartialEq)]
pub enum LoadNotice {
    Corrected(Vec<&'static str>),
    Unreadable {
        error: String,
        backup: Option<PathBuf>,
    },
}

pub struct Loaded {
    pub config: Config,
    pub notice: Option<LoadNotice>,
    pub outdated: bool,
}

pub fn load_startup(path: Option<&Path>) -> Loaded {
    let Some(path) = path else {
        return Loaded {
            config: Config::default(),
            notice: None,
            outdated: false,
        };
    };
    match Config::read(path) {
        Ok((mut config, file_version)) => {
            let fixed = config.validate();
            Loaded {
                config,
                notice: (!fixed.is_empty()).then_some(LoadNotice::Corrected(fixed)),
                outdated: file_version < i64::from(CURRENT_CONFIG_VERSION),
            }
        }
        Err(ConfigError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Loaded {
            config: Config::default(),
            notice: None,
            outdated: false,
        },
        Err(e) => Loaded {
            config: Config::default(),
            notice: Some(LoadNotice::Unreadable {
                error: e.to_string(),
                backup: crate::fsutil::backup_file(path).ok(),
            }),
            outdated: false,
        },
    }
}

fn portable_config_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("config.toml"))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn shell_appdata_dir() -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{CSIDL_APPDATA, SHGFP_TYPE_CURRENT, SHGetFolderPathW};
    let mut buf = [0u16; 260];
    // SAFETY: buf, API'nin istediği MAX_PATH boyutunda yazılabilir bir tampon;
    // pencere ve token parametreleri isteğe bağlı (None).
    let ok = unsafe {
        SHGetFolderPathW(
            None,
            CSIDL_APPDATA as i32,
            None,
            SHGFP_TYPE_CURRENT.0 as u32,
            &mut buf,
        )
    }
    .is_ok();
    if !ok {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

#[cfg(windows)]
fn user_config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(shell_appdata_dir)?;
    let dir = base.join("MouseDrive");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

#[cfg(target_os = "linux")]
fn user_config_dir() -> Option<PathBuf> {
    let base = xdg_config_home(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )?;
    let dir = base.join("mousedrive");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

#[cfg(target_os = "linux")]
fn xdg_config_home(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".config"))
        })
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(portable) = portable_config_path()
        && portable.exists()
    {
        return Some(portable);
    }
    user_config_dir().map(|d| d.join("config.toml"))
}

pub fn config_dir() -> Option<PathBuf> {
    config_path()?.parent().map(Path::to_path_buf)
}
