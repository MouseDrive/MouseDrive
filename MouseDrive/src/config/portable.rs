#![deny(unsafe_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use super::{CONFIG_FILE, PORTABLE_MARKER, VERSION_KEY, is_foreign, is_known_key, schema};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Portable {
    #[default]
    Off,
    Marker,
    VersionKey,
    IgnoredForeign {
        marked: bool,
    },
    IgnoredUnmarked,
}

impl Portable {
    pub fn is_portable(self) -> bool {
        matches!(self, Portable::Marker | Portable::VersionKey)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConfigKind {
    Missing,
    Empty,
    Versioned,
    Unversioned,
    Foreign,
}

fn detect_portable(dir: &Path) -> Portable {
    let marked = dir.join(PORTABLE_MARKER).exists();
    match (config_kind(&dir.join(CONFIG_FILE)), marked) {
        (ConfigKind::Foreign, marked) => Portable::IgnoredForeign { marked },
        (_, true) => Portable::Marker,
        (ConfigKind::Missing, false) => Portable::Off,
        (ConfigKind::Versioned, false) => Portable::VersionKey,
        (ConfigKind::Empty | ConfigKind::Unversioned, false) => Portable::IgnoredUnmarked,
    }
}

fn config_kind(path: &Path) -> ConfigKind {
    match std::fs::read(path) {
        Ok(bytes) => classify_config_text(&String::from_utf8_lossy(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ConfigKind::Missing,
        Err(_) => ConfigKind::Unversioned,
    }
}

fn classify_config_text(text: &str) -> ConfigKind {
    match toml::from_str::<toml::Table>(text) {
        Ok(raw) if raw.is_empty() => ConfigKind::Empty,
        Ok(raw) if raw.contains_key(VERSION_KEY) => ConfigKind::Versioned,
        Ok(raw) if is_foreign(&raw) => ConfigKind::Foreign,
        Ok(_) => ConfigKind::Unversioned,
        Err(_) => classify_broken_text(text),
    }
}

fn classify_broken_text(text: &str) -> ConfigKind {
    let keys: Vec<&str> = top_level_keys(text).collect();
    if keys.contains(&VERSION_KEY) {
        ConfigKind::Versioned
    } else if keys.iter().any(|key| is_known_key(key)) {
        ConfigKind::Unversioned
    } else {
        ConfigKind::Foreign
    }
}

fn top_level_keys(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim_start)
        .take_while(|line| !line.starts_with('['))
        .filter_map(|line| line.split_once('='))
        .map(|(key, _)| key.trim())
}

struct PortableState {
    dir: Option<PathBuf>,
    mode: Portable,
}

fn portable_state() -> &'static PortableState {
    static STATE: OnceLock<PortableState> = OnceLock::new();
    STATE.get_or_init(|| {
        let dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let mode = dir.as_deref().map_or(Portable::Off, detect_portable);
        PortableState { dir, mode }
    })
}

pub fn portable_mode() -> Portable {
    portable_state().mode
}

pub fn exe_dir() -> Option<PathBuf> {
    portable_state().dir.clone()
}

pub fn log_portable_mode() {
    if schema().is_none() {
        crate::log::error("config anahtar listesi çıkarılamadı; yabancı config denetimi kapalı");
    }
    let dir = exe_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    match portable_mode() {
        Portable::Off => {}
        Portable::Marker | Portable::VersionKey => {
            crate::log::info(&format!("taşınabilir mod: ayarlar {dir} klasöründe"));
        }
        Portable::IgnoredForeign { marked } => {
            let marker = if marked {
                format!(" ({PORTABLE_MARKER} olmasına rağmen)")
            } else {
                String::new()
            };
            crate::log::warn(&format!(
                "{dir} içindeki {CONFIG_FILE} mousedrive'a ait görünmüyor; taşınabilir mod kapalı{marker}, dosyaya dokunulmadı"
            ));
        }
        Portable::IgnoredUnmarked => crate::log::warn(&format!(
            "{dir} içindeki {CONFIG_FILE} için {PORTABLE_MARKER} işareti ya da {VERSION_KEY} anahtarı yok; taşınabilir mod kapalı"
        )),
    }
}

pub fn portable_dir() -> Option<PathBuf> {
    let state = portable_state();
    state
        .mode
        .is_portable()
        .then(|| state.dir.clone())
        .flatten()
}
