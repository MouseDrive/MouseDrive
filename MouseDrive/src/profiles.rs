#![deny(unsafe_code)]

use std::fmt;
use std::path::{Path, PathBuf};

use crate::config::{ConfigError, DEFAULT_PROFILE, Tuning};

pub const MAX_NAME_LEN: usize = 64;
const EXTENSION: &str = "toml";
const INVALID_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    InvalidChar(char),
    TrailingDotOrSpace,
    Reserved,
    Duplicate,
}

pub fn validate_name(name: &str) -> Result<(), NameError> {
    if name.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(NameError::TooLong);
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || INVALID_CHARS.contains(c))
    {
        return Err(NameError::InvalidChar(c));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(NameError::TrailingDotOrSpace);
    }
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        return Err(NameError::Reserved);
    }
    Ok(())
}

#[derive(Debug)]
pub enum ProfileError {
    Name(NameError),
    NotFound(String),
    LastProfile,
    Io(std::io::Error),
    Config(ConfigError),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProfileError::Name(e) => write!(f, "invalid name: {e:?}"),
            ProfileError::NotFound(n) => write!(f, "profile not found: {n}"),
            ProfileError::LastProfile => f.write_str("the last profile cannot be deleted"),
            ProfileError::Io(e) => write!(f, "{e}"),
            ProfileError::Config(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProfileError {}

impl From<std::io::Error> for ProfileError {
    fn from(e: std::io::Error) -> Self {
        ProfileError::Io(e)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedProfile {
    pub tuning: Tuning,
    pub corrected: Vec<&'static str>,
}

#[derive(Debug, Default)]
pub struct DeadzoneReset {
    pub changed: Vec<String>,
    pub failed: Vec<(String, ProfileError)>,
}

pub struct ProfileStore {
    dir: PathBuf,
}

fn write_tuning(path: &Path, tuning: &Tuning) -> Result<(), ProfileError> {
    let text = tuning.to_toml().map_err(ProfileError::Config)?;
    crate::fsutil::write_atomic(path, text.as_bytes())?;
    Ok(())
}

impl ProfileStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.join("profiles"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_of(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.{EXTENSION}"))
    }

    pub fn list(&self) -> Result<Vec<String>, ProfileError> {
        let mut names = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(names),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let path = entry?.path();
            let is_profile = path.extension().is_some_and(|e| e == EXTENSION);
            if let (true, Some(stem)) = (is_profile, path.file_stem().and_then(|s| s.to_str())) {
                names.push(stem.to_string());
            }
        }
        names.sort_by_key(|n| n.to_lowercase());
        Ok(names)
    }

    pub fn find(&self, name: &str) -> Option<String> {
        let wanted = name.to_lowercase();
        self.list()
            .ok()?
            .into_iter()
            .find(|n| n.to_lowercase() == wanted)
    }

    pub fn load(&self, name: &str, interval_ms: f64) -> Result<LoadedProfile, ProfileError> {
        let stored = self
            .find(name)
            .ok_or_else(|| ProfileError::NotFound(name.to_string()))?;
        let text = std::fs::read_to_string(self.path_of(&stored))?;
        let mut tuning = Tuning::from_toml_str(&text, interval_ms).map_err(ProfileError::Config)?;
        let corrected = tuning.validate();
        Ok(LoadedProfile { tuning, corrected })
    }

    pub fn save(&self, name: &str, tuning: &Tuning) -> Result<(), ProfileError> {
        validate_name(name).map_err(ProfileError::Name)?;
        std::fs::create_dir_all(&self.dir)?;
        let target = self.find(name).unwrap_or_else(|| name.to_string());
        write_tuning(&self.path_of(&target), tuning)
    }

    pub fn save_new(&self, name: &str, tuning: &Tuning) -> Result<(), ProfileError> {
        validate_name(name).map_err(ProfileError::Name)?;
        if self.find(name).is_some() {
            return Err(ProfileError::Name(NameError::Duplicate));
        }
        self.save(name, tuning)
    }

    pub fn duplicate(&self, from: &str, to: &str) -> Result<(), ProfileError> {
        let stored = self
            .find(from)
            .ok_or_else(|| ProfileError::NotFound(from.to_string()))?;
        validate_name(to).map_err(ProfileError::Name)?;
        if self.find(to).is_some() {
            return Err(ProfileError::Name(NameError::Duplicate));
        }
        std::fs::copy(self.path_of(&stored), self.path_of(to))?;
        Ok(())
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), ProfileError> {
        let stored = self
            .find(from)
            .ok_or_else(|| ProfileError::NotFound(from.to_string()))?;
        validate_name(to).map_err(ProfileError::Name)?;
        let same_profile = stored.to_lowercase() == to.to_lowercase();
        if !same_profile && self.find(to).is_some() {
            return Err(ProfileError::Name(NameError::Duplicate));
        }
        std::fs::rename(self.path_of(&stored), self.path_of(to))?;
        Ok(())
    }

    pub fn delete(&self, name: &str) -> Result<(), ProfileError> {
        let stored = self
            .find(name)
            .ok_or_else(|| ProfileError::NotFound(name.to_string()))?;
        if self.list()?.len() <= 1 {
            return Err(ProfileError::LastProfile);
        }
        std::fs::remove_file(self.path_of(&stored))?;
        Ok(())
    }

    pub fn ensure_initialized(&self, current: &Tuning) -> Result<bool, ProfileError> {
        if !self.list()?.is_empty() {
            return Ok(false);
        }
        self.save(DEFAULT_PROFILE, current)?;
        Ok(true)
    }

    pub fn reset_legacy_deadzone(&self, interval_ms: f64) -> Result<DeadzoneReset, ProfileError> {
        let mut report = DeadzoneReset::default();
        for name in self.list()? {
            let path = self.path_of(&name);
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(mut tuning) = Tuning::from_toml_str(&text, interval_ms) else {
                continue;
            };
            if !crate::config::reset_legacy_deadzone(&mut tuning) {
                continue;
            }
            match write_tuning(&path, &tuning) {
                Ok(()) => report.changed.push(name),
                Err(e) => report.failed.push((name, e)),
            }
        }
        Ok(report)
    }

    pub fn resolve_active(&self, wanted: &str) -> Option<String> {
        self.find(wanted)
            .or_else(|| self.find(DEFAULT_PROFILE))
            .or_else(|| self.list().ok()?.into_iter().next())
    }
}
