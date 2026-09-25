#![deny(unsafe_code)]

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    let result = written.and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = with_suffix(path, &format!(".{ts}.bak"));
    std::fs::copy(path, &backup)?;
    Ok(backup)
}
