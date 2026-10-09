#![deny(unsafe_code)]

use std::ffi::OsString;
use std::fs::{File, OpenOptions, Permissions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SYMLINK_HOPS: usize = 40;
const TMP_SUFFIX: &str = ".tmp";
const BACKUP_SUFFIX: &str = ".bak";
const MAX_BACKUPS_PER_SECOND: u32 = 100;

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s: OsString = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let target = resolve_symlinks(path)?;
    let original = existing_permissions(&target)?;
    let tmp = with_suffix(&target, TMP_SUFFIX);
    let result =
        write_tmp(&tmp, bytes, original.as_ref()).and_then(|()| std::fs::rename(&tmp, &target));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sync_dir(parent_dir(&target));
    Ok(())
}

fn resolve_symlinks(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_SYMLINK_HOPS {
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = std::fs::read_link(&current)?;
                current = parent_dir(&current).join(link);
            }
            Ok(_) => return Ok(current),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(current),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("too many symbolic links: {}", path.display()),
    ))
}

fn existing_permissions(path: &Path) -> io::Result<Option<Permissions>> {
    match std::fs::metadata(path) {
        Ok(meta) => Ok(Some(meta.permissions())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_tmp(tmp: &Path, bytes: &[u8], original: Option<&Permissions>) -> io::Result<()> {
    match std::fs::remove_file(tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    write_new(tmp, bytes, original)
}

pub fn write_new(path: &Path, bytes: &[u8], permissions: Option<&Permissions>) -> io::Result<()> {
    let mut file = create_new(path, permissions)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

#[cfg(unix)]
fn create_new(path: &Path, original: Option<&Permissions>) -> io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mode = original.map(|p| p.mode() & 0o7777);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    if let Some(mode) = mode {
        options.mode(mode);
    }
    let file = options.open(path)?;
    if let Some(mode) = mode
        && let Err(e) = file.set_permissions(Permissions::from_mode(mode))
    {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(e);
    }
    Ok(file)
}

#[cfg(not(unix))]
fn create_new(path: &Path, _original: Option<&Permissions>) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(unix)]
pub fn sync_dir(dir: &Path) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    let Err(e) = File::open(dir).and_then(|d| d.sync_all()) else {
        return;
    };
    if !WARNED.swap(true, Ordering::Relaxed) {
        crate::log::warn(&format!(
            "klasör diske eşitlenemedi ({}): {e}",
            dir.display()
        ));
    }
}

#[cfg(not(unix))]
pub fn sync_dir(_dir: &Path) {}

pub fn backup_file(path: &Path) -> io::Result<PathBuf> {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut source = File::open(path)?;
    let permissions = source.metadata()?.permissions();
    for n in 0..MAX_BACKUPS_PER_SECOND {
        let stamp = if n == 0 {
            ts.to_string()
        } else {
            format!("{ts}-{n}")
        };
        let backup = with_suffix(path, &format!(".{stamp}{BACKUP_SUFFIX}"));
        let mut target = match create_new(&backup, Some(&permissions)) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let copied = io::copy(&mut source, &mut target).and_then(|_| target.sync_all());
        if let Err(e) = copied {
            let _ = std::fs::remove_file(&backup);
            return Err(e);
        }
        return Ok(backup);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("too many backups of {}", path.display()),
    ))
}

pub fn backup_once(path: &Path) -> io::Result<PathBuf> {
    let content = std::fs::read(path)?;
    match find_backup(path, &content) {
        Some(existing) => Ok(existing),
        None => backup_file(path),
    }
}

fn find_backup(path: &Path, content: &[u8]) -> Option<PathBuf> {
    let prefix = format!("{}.", path.file_name()?.to_str()?);
    std::fs::read_dir(parent_dir(path))
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|candidate| is_backup_name(candidate, &prefix))
        .find(|candidate| std::fs::read(candidate).is_ok_and(|bytes| bytes == content))
}

fn is_backup_name(candidate: &Path, prefix: &str) -> bool {
    let Some(name) = candidate.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some(stamp) = name
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(BACKUP_SUFFIX))
    else {
        return false;
    };
    let (secs, n) = stamp.split_once('-').unwrap_or((stamp, "0"));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    digits(secs) && digits(n)
}
