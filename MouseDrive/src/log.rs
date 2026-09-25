#![deny(unsafe_code)]

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn log_path() -> Option<PathBuf> {
    Some(crate::config::config_dir()?.join("mousedrive.log"))
}

fn rotate_if_large(path: &PathBuf) {
    let too_large = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
    if too_large {
        let _ = std::fs::rename(path, path.with_extension("log.old"));
    }
}

pub fn line(msg: &str) {
    let Some(path) = log_path() else { return };
    rotate_if_large(&path);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "[{ts}] {msg}");
    }
}
