#![deny(unsafe_code)]

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mousedrive::{fsutil, log, platform};

use super::error::UpdateError;
use super::parse_version;

const EXE_SUFFIX: &str = std::env::consts::EXE_SUFFIX;
const BACKUP_SUFFIX: &str = ".old";
const STAGE_PREFIX: &str = ".mousedrive-update";
const PROBE_PREFIX: &str = ".mousedrive-write-test";
const LEFTOVER_AGE: Duration = Duration::from_secs(10 * 60);
const MANAGED_ROOTS: [&str; 2] = ["/usr", "/nix/store"];
const LOCAL_ROOT: &str = "/usr/local";
const WAIT_STEP: Duration = Duration::from_millis(50);
const SPAWN_ATTEMPTS: u32 = 5;
const SPAWN_RETRY: Duration = Duration::from_millis(50);
const MAX_OUTPUT_BYTES: u64 = 4096;
const MAX_REPORTED_CHARS: usize = 300;
#[cfg(windows)]
const RENAME_ATTEMPTS: u32 = 10;
#[cfg(windows)]
const RENAME_RETRY: Duration = Duration::from_millis(100);
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

static BACKUP_GENERATION: Mutex<u64> = Mutex::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Manual {
    PackageManager,
    AppImage,
    NoBuild,
}

impl Manual {
    pub(super) fn label(self) -> &'static str {
        match self {
            Manual::PackageManager => "paket yöneticisiyle kurulmuş",
            Manual::AppImage => "AppImage olarak çalışıyor",
            Manual::NoBuild => "bu sistem için hazır paket yok",
        }
    }
}

pub(super) fn local_block(
    exe: Option<&Path>,
    appimage: Option<&OsStr>,
    appdir: Option<&OsStr>,
) -> Option<Manual> {
    if inside_appimage(exe, appimage, appdir) {
        return Some(Manual::AppImage);
    }
    exe.filter(|exe| package_managed(exe))
        .map(|_| Manual::PackageManager)
}

fn inside_appimage(exe: Option<&Path>, appimage: Option<&OsStr>, appdir: Option<&OsStr>) -> bool {
    if !appimage.is_some_and(|value| !value.is_empty()) {
        return false;
    }
    match (exe, appdir.filter(|dir| !dir.is_empty())) {
        (Some(exe), Some(dir)) => exe.starts_with(dir),
        _ => true,
    }
}

fn package_managed(exe: &Path) -> bool {
    MANAGED_ROOTS.iter().any(|root| exe.starts_with(root)) && !exe.starts_with(LOCAL_ROOT)
}

pub(super) fn exe_dir(exe: &Path) -> Result<PathBuf, UpdateError> {
    exe.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            UpdateError::NoExe(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} için klasör yok", exe.display()),
            ))
        })
}

fn unique_name(prefix: &str, suffix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    format!("{prefix}-{}-{nanos}{suffix}", std::process::id())
}

pub(super) fn probe_writable(dir: &Path) -> Result<(), UpdateError> {
    let probe = dir.join(unique_name(PROBE_PREFIX, ""));
    let not_writable = |source| UpdateError::NotWritable {
        dir: dir.to_path_buf(),
        source,
    };
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(not_writable)?;
    fs::remove_file(&probe).map_err(not_writable)
}

pub(super) struct Staged {
    path: PathBuf,
}

impl Staged {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => log::warn(&format!(
                "geçici güncelleme dosyası silinemedi ({}): {e}",
                self.path.display()
            )),
        }
    }
}

pub(super) fn remove_leftovers(dir: &Path) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            log::warn(&format!(
                "program klasörü okunamadı ({}): {e}",
                dir.display()
            ));
            return;
        }
    };
    for entry in entries {
        match entry {
            Ok(entry) if is_leftover(&entry) => remove_leftover(&entry.path()),
            Ok(_) => {}
            Err(e) => log::warn(&format!(
                "program klasöründeki bir kayıt okunamadı ({}): {e}",
                dir.display()
            )),
        }
    }
}

fn is_leftover(entry: &fs::DirEntry) -> bool {
    let name = entry.file_name();
    let ours = name.to_str().is_some_and(|name| {
        [STAGE_PREFIX, PROBE_PREFIX].iter().any(|prefix| {
            name.strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('-'))
        })
    });
    ours && entry
        .metadata()
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age >= LEFTOVER_AGE)
}

fn remove_leftover(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => log::info(&format!(
            "yarım kalmış bir güncellemenin dosyası silindi: {}",
            path.display()
        )),
        Err(e) => log::warn(&format!(
            "yarım kalmış güncelleme dosyası silinemedi ({}): {e}",
            path.display()
        )),
    }
}

pub(super) fn stage(dir: &Path, exe: &Path, bytes: &[u8]) -> Result<Staged, UpdateError> {
    let permissions = fs::metadata(exe).map_err(UpdateError::NoExe)?.permissions();
    let path = dir.join(unique_name(STAGE_PREFIX, EXE_SUFFIX));
    fsutil::write_new(&path, bytes, Some(&permissions)).map_err(|source| UpdateError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(Staged { path })
}

pub(super) fn spawn(command: &mut Command) -> io::Result<Child> {
    let mut attempt = 1;
    loop {
        match command.spawn() {
            Err(e) if busy(&e) && attempt < SPAWN_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(SPAWN_RETRY);
            }
            other => return other,
        }
    }
}

fn busy(e: &io::Error) -> bool {
    #[cfg(windows)]
    if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) {
        return true;
    }
    e.kind() == io::ErrorKind::ExecutableFileBusy
}

pub(super) fn wait_for(child: &mut Child, limit: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        std::thread::sleep(WAIT_STEP.min(deadline - now));
    }
}

pub(super) fn self_test(path: &Path, version: &str, limit: Duration) -> Result<(), UpdateError> {
    let mut command = platform::child_command(path);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child =
        spawn(&mut command).map_err(|e| UpdateError::SelfTest(format!("başlatılamadı: {e}")))?;
    let status = match wait_for(&mut child, limit) {
        Ok(Some(status)) => status,
        Ok(None) => {
            stop(&mut child);
            let detail = format!("{} s içinde bitmedi", limit.as_secs_f32());
            return Err(UpdateError::SelfTest(detail));
        }
        Err(e) => {
            stop(&mut child);
            return Err(UpdateError::SelfTest(format!("beklenemedi: {e}")));
        }
    };
    let stdout = read_pipe(child.stdout.take());
    let stderr = read_pipe(child.stderr.take());
    judge(
        status.success(),
        &status.to_string(),
        &stdout,
        &stderr,
        version,
    )
    .map_err(UpdateError::SelfTest)?;
    log::info(&format!("yeni sürüm denendi: {}", stdout.trim()));
    Ok(())
}

fn stop(child: &mut Child) {
    if let Err(e) = child.kill() {
        log::warn(&format!("deneme süreci durdurulamadı: {e}"));
    }
    if let Err(e) = child.wait() {
        log::warn(&format!("deneme süreci beklenemedi: {e}"));
    }
}

fn read_pipe(pipe: Option<impl Read>) -> String {
    let mut bytes = Vec::new();
    if let Some(pipe) = pipe
        && let Err(e) = pipe.take(MAX_OUTPUT_BYTES).read_to_end(&mut bytes)
    {
        log::warn(&format!("deneme sürecinin çıktısı okunamadı: {e}"));
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn judge(
    success: bool,
    status: &str,
    stdout: &str,
    stderr: &str,
    version: &str,
) -> Result<(), String> {
    if !success {
        return Err(format!("{status}: {}", short(stderr)));
    }
    let wanted = parse_version(version).ok_or_else(|| format!("geçersiz sürüm: {version}"))?;
    let reported = stdout.lines().next().is_some_and(|line| {
        line.split_whitespace()
            .any(|token| parse_version(token) == Some(wanted))
    });
    if reported {
        Ok(())
    } else {
        Err(format!(
            "{version} bekleniyordu, program şunu bildirdi: {}",
            short(stdout)
        ))
    }
}

fn short(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return "(çıktı yok)".to_string();
    }
    text.chars().take(MAX_REPORTED_CHARS).collect()
}

pub fn backup_path(exe: &Path) -> PathBuf {
    let mut name = exe.as_os_str().to_owned();
    name.push(BACKUP_SUFFIX);
    PathBuf::from(name)
}

fn remove_stale(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

pub(super) fn backup_generation() -> u64 {
    *BACKUP_GENERATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

pub(super) fn swap_in(exe: &Path, staged: Staged) -> Result<(), UpdateError> {
    let mut generation = BACKUP_GENERATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    *generation += 1;
    let backup = backup_path(exe);
    remove_stale(&backup).map_err(|e| {
        UpdateError::Replace(format!("eski yedek silinemedi ({}): {e}", backup.display()))
    })?;
    replace(exe, staged.path(), &backup)?;
    if let Some(dir) = exe.parent() {
        fsutil::sync_dir(dir);
    }
    log::info(&format!(
        "yeni sürüm yerleştirildi; önceki sürüm {} olarak saklandı",
        backup.display()
    ));
    Ok(())
}

#[cfg(unix)]
fn replace(exe: &Path, staged: &Path, backup: &Path) -> Result<(), UpdateError> {
    keep_backup(exe, backup)?;
    fs::rename(staged, exe).map_err(|e| {
        discard_backup(backup);
        UpdateError::Replace(format!("{}: {e}", exe.display()))
    })
}

#[cfg(unix)]
fn keep_backup(exe: &Path, backup: &Path) -> Result<(), UpdateError> {
    if fs::hard_link(exe, backup).is_ok() {
        return Ok(());
    }
    fs::copy(exe, backup).map(drop).map_err(|e| {
        discard_backup(backup);
        UpdateError::Replace(format!("yedek alınamadı ({}): {e}", backup.display()))
    })
}

#[cfg(unix)]
fn discard_backup(backup: &Path) {
    if let Err(e) = remove_stale(backup) {
        log::warn(&format!(
            "kullanılmayan yedek silinemedi ({}): {e}",
            backup.display()
        ));
    }
}

#[cfg(windows)]
fn replace(exe: &Path, staged: &Path, backup: &Path) -> Result<(), UpdateError> {
    rename_with_retry(exe, backup).map_err(|e| {
        UpdateError::Replace(format!("yedek alınamadı ({}): {e}", backup.display()))
    })?;
    rename_with_retry(staged, exe).map_err(|e| match rename_with_retry(backup, exe) {
        Ok(()) => UpdateError::Replace(format!("{}: {e}", exe.display())),
        Err(undo) => UpdateError::Restore(format!(
            "{}: {e}; önceki sürüm {} olarak kaldı: {undo}",
            exe.display(),
            backup.display()
        )),
    })
}

#[cfg(windows)]
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 1;
    loop {
        match fs::rename(from, to) {
            Err(e) if locked(&e) && attempt < RENAME_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(RENAME_RETRY);
            }
            other => return other,
        }
    }
}

#[cfg(windows)]
fn locked(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

#[cfg(unix)]
pub fn restore_backup(exe: &Path) -> io::Result<()> {
    fs::rename(backup_path(exe), exe)?;
    if let Some(dir) = exe.parent() {
        fsutil::sync_dir(dir);
    }
    Ok(())
}

#[cfg(windows)]
pub fn restore_backup(exe: &Path) -> io::Result<()> {
    rename_with_retry(&backup_path(exe), exe)
}

pub(super) fn forget_backup(exe: &Path, generation: u64) {
    let current = BACKUP_GENERATION
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if *current == generation {
        remove_backup(exe);
    } else {
        log::info("önceki sürümün yedeği bu oturumda yenilendi; silinmedi");
    }
}

fn remove_backup(exe: &Path) {
    let backup = backup_path(exe);
    match fs::remove_file(&backup) {
        Ok(()) => log::info(&format!(
            "önceki sürümün yedeği silindi: {}",
            backup.display()
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => log::warn(&format!(
            "önceki sürümün yedeği silinemedi ({}): {e}",
            backup.display()
        )),
    }
}
