#![deny(unsafe_code)]

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

fn log_path() -> Option<String> {
    let cfg = crate::config::get_config_path()?;
    let dir = std::path::Path::new(&cfg).parent()?;
    dir.join("mousedrive.log").to_str().map(|s| s.to_string())
}

pub fn line(msg: &str) {
    let Some(path) = log_path() else { return };
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
