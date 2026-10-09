#![deny(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::curve::Curve;
use crate::logic::filters::{MAX_SMOOTHING_MS, tau_from_legacy_alpha};

mod portable;

pub use self::portable::{Portable, exe_dir, log_portable_mode, portable_dir, portable_mode};

pub const CURRENT_CONFIG_VERSION: u32 = 3;

pub const CONFIG_FILE: &str = "config.toml";

pub const PORTABLE_MARKER: &str = "mousedrive.portable";

const VERSION_KEY: &str = "config_version";

const LEGACY_KEYS: [&str; 4] = [
    "steering_filter_alpha",
    "mouse_delta_cap",
    "overlay_enabled",
    "overlay_corner",
];

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
        Self::parse(content).map(|parsed| parsed.config)
    }

    fn parse(content: &str) -> Result<Parsed, ConfigError> {
        let raw: toml::Table = toml::from_str(content).map_err(parse_error)?;
        let mut cfg: Config = toml::from_str(content).map_err(parse_error)?;
        let version = raw
            .get(VERSION_KEY)
            .and_then(toml::Value::as_integer)
            .unwrap_or(1);
        if version < 2 {
            let interval = f64::from(cfg.thread_interval_ms.max(1));
            migrate_legacy_tuning(&mut cfg.tuning, &raw, interval);
        }
        if version < 3 {
            reset_legacy_deadzone(&mut cfg.tuning);
        }
        cfg.config_version = CURRENT_CONFIG_VERSION;
        Ok(Parsed {
            config: cfg,
            version,
            unknown: unknown_keys(&raw),
            foreign: is_foreign(&raw),
        })
    }

    pub fn load_from_file(path: &Path) -> Result<Self, ConfigError> {
        Self::read(path).map(|parsed| parsed.config)
    }

    fn read(path: &Path) -> Result<Parsed, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::parse(&content)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<(), ConfigError> {
        let content =
            toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        crate::fsutil::write_atomic(path, content.as_bytes()).map_err(ConfigError::Io)
    }
}

struct Parsed {
    config: Config,
    version: i64,
    unknown: Vec<String>,
    foreign: bool,
}

fn schema() -> Option<&'static toml::Table> {
    static SCHEMA: OnceLock<Option<toml::Table>> = OnceLock::new();
    SCHEMA
        .get_or_init(|| toml::Table::try_from(Config::default()).ok())
        .as_ref()
}

fn is_known_key(key: &str) -> bool {
    LEGACY_KEYS.contains(&key) || schema().is_some_and(|known| known.contains_key(key))
}

fn unknown_keys(raw: &toml::Table) -> Vec<String> {
    schema().map_or_else(Vec::new, |known| unknown_in(raw, known, ""))
}

fn unknown_in(raw: &toml::Table, known: &toml::Table, prefix: &str) -> Vec<String> {
    raw.iter()
        .flat_map(|(key, value)| {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match (known.get(key), value) {
                (Some(toml::Value::Table(inner)), toml::Value::Table(table)) => {
                    unknown_in(table, inner, &path)
                }
                (Some(_), _) => Vec::new(),
                (None, _) if prefix.is_empty() && LEGACY_KEYS.contains(&key.as_str()) => Vec::new(),
                (None, _) => vec![path],
            }
        })
        .collect()
}

fn is_foreign(raw: &toml::Table) -> bool {
    schema().is_some() && !raw.is_empty() && !raw.keys().any(|k| is_known_key(k))
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

pub type Backup = Result<PathBuf, String>;

#[derive(Clone, Debug, PartialEq)]
pub enum LoadNotice {
    Corrected(Vec<&'static str>),
    Unreadable { error: String, backup: Backup },
    Newer { version: i64, backup: Backup },
    UnknownKeys { keys: Vec<String>, backup: Backup },
    Foreign { backup: Backup },
}

impl LoadNotice {
    pub fn rewrites_file(&self) -> bool {
        matches!(
            self,
            LoadNotice::Corrected(_) | LoadNotice::Unreadable { .. }
        )
    }

    pub fn backup(&self) -> Option<&Backup> {
        match self {
            LoadNotice::Corrected(_) => None,
            LoadNotice::Unreadable { backup, .. }
            | LoadNotice::Newer { backup, .. }
            | LoadNotice::UnknownKeys { backup, .. }
            | LoadNotice::Foreign { backup } => Some(backup),
        }
    }
}

pub struct Loaded {
    pub config: Config,
    pub notices: Vec<LoadNotice>,
    pub outdated: bool,
    pub writable: bool,
}

impl Loaded {
    fn new(config: Config, notices: Vec<LoadNotice>, outdated: bool) -> Self {
        let writable = !notices
            .iter()
            .filter_map(LoadNotice::backup)
            .any(Result::is_err);
        Self {
            config,
            notices,
            outdated,
            writable,
        }
    }
}

pub fn load_startup(path: Option<&Path>) -> Loaded {
    let Some(path) = path else {
        return Loaded::new(Config::default(), Vec::new(), false);
    };
    let loaded = match Config::read(path) {
        Ok(parsed) => loaded_from(path, parsed),
        Err(ConfigError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            Loaded::new(Config::default(), Vec::new(), false)
        }
        Err(e) => {
            let notice = LoadNotice::Unreadable {
                error: e.to_string(),
                backup: backup_config(path),
            };
            Loaded::new(Config::default(), vec![notice], false)
        }
    };
    log_load(path, &loaded);
    loaded
}

fn loaded_from(path: &Path, parsed: Parsed) -> Loaded {
    let Parsed {
        mut config,
        version,
        unknown,
        foreign,
    } = parsed;
    if foreign {
        let notice = LoadNotice::Foreign {
            backup: backup_config(path),
        };
        return Loaded::new(Config::default(), vec![notice], false);
    }
    let current = i64::from(CURRENT_CONFIG_VERSION);
    let mut notices = Vec::new();
    if version > current {
        notices.push(LoadNotice::Newer {
            version,
            backup: backup_config(path),
        });
    } else if !unknown.is_empty() {
        notices.push(LoadNotice::UnknownKeys {
            keys: unknown,
            backup: backup_config(path),
        });
    }
    let fixed = config.validate();
    if !fixed.is_empty() {
        notices.push(LoadNotice::Corrected(fixed));
    }
    Loaded::new(config, notices, version < current)
}

fn backup_config(path: &Path) -> Backup {
    crate::fsutil::backup_once(path).map_err(|e| e.to_string())
}

fn log_load(path: &Path, loaded: &Loaded) {
    for notice in &loaded.notices {
        let what = load_summary(notice);
        match notice.backup() {
            None => crate::log::warn(&what),
            Some(Ok(backup)) => crate::log::warn(&format!("{what}; yedek: {}", backup.display())),
            Some(Err(e)) => crate::log::error(&format!(
                "{what}; yedek alınamadı ({e}), bu oturumda {} yazılmayacak",
                path.display()
            )),
        }
    }
}

fn load_summary(notice: &LoadNotice) -> String {
    match notice {
        LoadNotice::Corrected(fields) => format!("config düzeltildi: {}", fields.join(", ")),
        LoadNotice::Unreadable { error, .. } => format!("config okunamadı ({error})"),
        LoadNotice::Newer { version, .. } => {
            format!("config daha yeni bir sürümle yazılmış (config_version {version})")
        }
        LoadNotice::UnknownKeys { keys, .. } => {
            format!("config'te bilinmeyen anahtarlar var: {}", keys.join(", "))
        }
        LoadNotice::Foreign { .. } => "config mousedrive'a ait görünmüyor".to_string(),
    }
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
pub(crate) fn user_config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(shell_appdata_dir)?;
    let dir = base.join("MouseDrive");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

#[cfg(target_os = "linux")]
pub(crate) fn user_config_dir() -> Option<PathBuf> {
    let base = xdg_base_dir(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
        ".config",
    )?;
    let dir = base.join("mousedrive");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

#[cfg(target_os = "linux")]
pub(crate) fn xdg_base_dir(
    xdg: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    fallback: &str,
) -> Option<PathBuf> {
    xdg.map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(fallback))
        })
}

pub fn config_path() -> Option<PathBuf> {
    if let Some(dir) = portable_dir() {
        return Some(dir.join(CONFIG_FILE));
    }
    user_config_dir().map(|d| d.join(CONFIG_FILE))
}

pub fn config_dir() -> Option<PathBuf> {
    config_path()?.parent().map(Path::to_path_buf)
}
