#![deny(unsafe_code)]

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK_FILE: &str = "mousedrive.lock";
const RETRY_STEP: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug)]
pub enum LockError {
    Busy(PathBuf),
    NoDirectory,
    Io(PathBuf, io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Busy(path) => write!(f, "kilit başka süreçte: {}", path.display()),
            LockError::NoDirectory => f.write_str("kilit klasörü bulunamadı"),
            LockError::Io(path, e) => write!(f, "{}: {e}", path.display()),
        }
    }
}

pub fn acquire(wait: Duration) -> Result<InstanceLock, LockError> {
    let dir = lock_dir().ok_or(LockError::NoDirectory)?;
    acquire_at(&dir.join(LOCK_FILE), wait)
}

fn acquire_at(path: &Path, wait: Duration) -> Result<InstanceLock, LockError> {
    let file = open_lock_file(path).map_err(|e| LockError::Io(path.to_path_buf(), e))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => {
                return Ok(InstanceLock {
                    _file: file,
                    path: path.to_path_buf(),
                });
            }
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(RETRY_STEP);
            }
            Err(TryLockError::WouldBlock) => return Err(LockError::Busy(path.to_path_buf())),
            Err(TryLockError::Error(e)) => return Err(LockError::Io(path.to_path_buf(), e)),
        }
    }
}

#[cfg(target_os = "linux")]
fn open_lock_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(windows)]
fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(target_os = "linux")]
fn lock_dir() -> Option<PathBuf> {
    runtime_dir(std::env::var_os("XDG_RUNTIME_DIR")).or_else(crate::config::user_config_dir)
}

#[cfg(target_os = "linux")]
fn runtime_dir(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
}

#[cfg(windows)]
fn lock_dir() -> Option<PathBuf> {
    crate::config::user_config_dir()
}
