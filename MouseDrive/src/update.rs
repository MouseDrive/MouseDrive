#![deny(unsafe_code)]

mod error;
mod install;
mod net;
mod relaunch;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mousedrive::log;

pub use self::error::{FailReason, UpdateError};
pub use self::install::{Manual, backup_path};
pub use self::relaunch::{Relaunch, forget_backup_later, relaunch};

const RELEASES_API: &str = "https://api.github.com/repos/MouseDrive/MouseDrive/releases/latest";
const MAX_ZIP_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SUMS_BYTES: u64 = 64 * 1024;
const SUMS_NAME: &str = "SHA256SUMS.txt";
const ZIP_PREFIX: &str = "MouseDrive-";
const SELF_TEST_TIMEOUT: Duration = Duration::from_secs(15);
const EXE_SUFFIX: &str = std::env::consts::EXE_SUFFIX;

fn arch_label(arch: &str) -> &str {
    match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

fn zip_suffix(os: &str, arch: &str) -> String {
    format!("-{os}-{}.zip", arch_label(arch))
}

fn platform_zip_suffix() -> String {
    zip_suffix(std::env::consts::OS, std::env::consts::ARCH)
}

#[derive(Clone, PartialEq, Debug)]
pub struct ReleaseInfo {
    pub version: String,
    pub html_url: String,
    pub zip_url: Option<String>,
    pub sums_url: Option<String>,
    pub zip_name: String,
}

impl ReleaseInfo {
    pub fn auto_installable(&self) -> bool {
        self.zip_url.is_some() && self.sums_url.is_some()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum UpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Failed,
    Available(ReleaseInfo),
    Updating,
    ReadyToRestart,
    UpdateFailed(ReleaseInfo, FailReason),
}

pub struct UpdateChecker {
    status: Arc<Mutex<UpdateStatus>>,
    gate: ReplaceGate,
    exe: Option<PathBuf>,
    local_block: Option<Manual>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for UpdateChecker {
    fn drop(&mut self) {
        self.gate.close();
    }
}

impl UpdateChecker {
    pub fn new() -> Self {
        let exe = std::env::current_exe()
            .map_err(|e| log::warn(&format!("güncelleyici programın yolunu bulamadı: {e}")))
            .ok();
        let appimage = std::env::var_os("APPIMAGE");
        let appdir = std::env::var_os("APPDIR");
        let local_block =
            install::local_block(exe.as_deref(), appimage.as_deref(), appdir.as_deref());
        if let Some(block) = local_block {
            log::info(&format!(
                "otomatik güncelleme kapalı ({}); yalnız sürüm sayfası gösterilecek",
                block.label()
            ));
        }
        Self {
            status: Arc::new(Mutex::new(UpdateStatus::Idle)),
            gate: ReplaceGate::default(),
            exe,
            local_block,
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(UpdateStatus::Failed)
    }

    pub fn manual(&self, info: &ReleaseInfo) -> Option<Manual> {
        manual_reason(self.local_block, info)
    }

    pub fn spawn_check(&self) {
        if let Ok(mut s) = self.status.lock() {
            if matches!(
                *s,
                UpdateStatus::Checking | UpdateStatus::Updating | UpdateStatus::ReadyToRestart
            ) {
                return;
            }
            *s = UpdateStatus::Checking;
        } else {
            return;
        }

        spawn_worker("update-check", &self.status, UpdateStatus::Failed, || {
            check_latest().unwrap_or_else(|e| {
                log::warn(&format!("güncelleme denetlenemedi: {e}"));
                UpdateStatus::Failed
            })
        });
    }

    pub fn spawn_update(&self, info: ReleaseInfo) {
        if let Some(manual) = self.manual(&info) {
            log::warn(&format!(
                "otomatik kurulum yapılmadı; bu kurulum elle güncellenmeli ({})",
                manual.label()
            ));
            return;
        }
        if let Ok(mut s) = self.status.lock() {
            if matches!(*s, UpdateStatus::Updating) {
                return;
            }
            *s = UpdateStatus::Updating;
        } else {
            return;
        }

        let gate = self.gate.clone();
        let exe = self.exe.clone();
        let failed = UpdateStatus::UpdateFailed(info.clone(), FailReason::Other);
        spawn_worker("update-install", &self.status, failed, move || {
            log::info(&format!("güncelleme kuruluyor: {}", info.version));
            match run_update(&info, exe.as_deref(), &gate) {
                Ok(()) => {
                    log::info(&format!("güncelleme kuruldu: {}", info.version));
                    UpdateStatus::ReadyToRestart
                }
                Err(e) => {
                    log::error(&format!("güncelleme kurulamadı ({}): {e}", info.version));
                    let reason = e.reason();
                    UpdateStatus::UpdateFailed(info, reason)
                }
            }
        });
    }
}

fn spawn_worker(
    name: &str,
    status: &Arc<Mutex<UpdateStatus>>,
    on_spawn_error: UpdateStatus,
    job: impl FnOnce() -> UpdateStatus + Send + 'static,
) {
    let shared = Arc::clone(status);
    let spawned = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || set_status(&shared, job()));
    if let Err(e) = spawned {
        log::error(&format!("{name} thread'i başlatılamadı: {e}"));
        set_status(status, on_spawn_error);
    }
}

fn set_status(status: &Mutex<UpdateStatus>, value: UpdateStatus) {
    *status.lock().unwrap_or_else(PoisonError::into_inner) = value;
}

fn manual_reason(local: Option<Manual>, info: &ReleaseInfo) -> Option<Manual> {
    local.or_else(|| (!info.auto_installable()).then_some(Manual::NoBuild))
}

#[derive(Clone, Default)]
struct ReplaceGate(Arc<Mutex<bool>>);

impl ReplaceGate {
    fn run(&self, replace: impl FnOnce() -> Result<(), UpdateError>) -> Result<(), UpdateError> {
        let closed = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if *closed {
            return Err(UpdateError::Closed);
        }
        replace()
    }

    fn close(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
    }
}

fn check_latest() -> Result<UpdateStatus, UpdateError> {
    let body = net::get_json(RELEASES_API)?;
    let suffix = platform_zip_suffix();
    let info = parse_release_json(&body, &suffix)
        .ok_or_else(|| UpdateError::Malformed("sürüm bilgisi okunamadı".to_string()))?;
    let remote = parse_version(&info.version).ok_or_else(|| {
        UpdateError::Malformed(format!("geçersiz sürüm etiketi: {}", info.version))
    })?;
    let local = parse_version(env!("CARGO_PKG_VERSION"))
        .ok_or_else(|| UpdateError::Malformed("yerel sürüm okunamadı".to_string()))?;
    if remote <= local {
        return Ok(UpdateStatus::UpToDate);
    }
    log::info(&format!("yeni sürüm bulundu: {}", info.version));
    if info.zip_url.is_none() {
        log::info(&format!(
            "bu sistem için hazır paket yok (*{suffix}); yalnız sürüm sayfası gösterilecek"
        ));
    }
    Ok(UpdateStatus::Available(info))
}

fn parse_release_json(body: &str, zip_suffix: &str) -> Option<ReleaseInfo> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let version = v.get("tag_name")?.as_str()?.to_string();
    let html_url = v.get("html_url")?.as_str()?.to_string();

    let mut zip_url = None;
    let mut sums_url = None;
    let mut zip_name = String::new();
    if let Some(assets) = v.get("assets").and_then(|a| a.as_array()) {
        for asset in assets {
            let name = asset.get("name").and_then(|x| x.as_str());
            let url = asset.get("browser_download_url").and_then(|x| x.as_str());
            let (Some(name), Some(url)) = (name, url) else {
                continue;
            };
            if name.starts_with(ZIP_PREFIX) && name.ends_with(zip_suffix) {
                zip_url = Some(url.to_string());
                zip_name = name.to_string();
            } else if name == SUMS_NAME {
                sums_url = Some(url.to_string());
            }
        }
    }

    Some(ReleaseInfo {
        version,
        html_url,
        zip_url,
        sums_url,
        zip_name,
    })
}

fn run_update(
    info: &ReleaseInfo,
    exe: Option<&Path>,
    gate: &ReplaceGate,
) -> Result<(), UpdateError> {
    let (Some(zip_url), Some(sums_url)) = (&info.zip_url, &info.sums_url) else {
        return Err(UpdateError::NoAsset);
    };
    let exe = exe.ok_or_else(|| {
        UpdateError::NoExe(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "program yolu bilinmiyor",
        ))
    })?;
    let dir = install::exe_dir(exe)?;
    install::probe_writable(&dir)?;
    install::remove_leftovers(&dir);

    let sums = net::download(sums_url, MAX_SUMS_BYTES)?;
    let zip_bytes = net::download(zip_url, MAX_ZIP_BYTES)?;
    verify_checksum(&sums, &info.zip_name, &zip_bytes)?;
    let exe_bytes = extract_exe(&zip_bytes)?;

    let staged = install::stage(&dir, exe, &exe_bytes)?;
    install::self_test(staged.path(), &info.version, SELF_TEST_TIMEOUT)?;
    gate.run(|| install::swap_in(exe, staged))
}

fn verify_checksum(sums: &[u8], zip_name: &str, zip_bytes: &[u8]) -> Result<(), UpdateError> {
    let sums = std::str::from_utf8(sums)
        .map_err(|_| UpdateError::Checksum(format!("{SUMS_NAME} metin değil")))?;
    let expected = parse_sha256sums(sums, zip_name)
        .ok_or_else(|| UpdateError::Checksum(format!("{zip_name} {SUMS_NAME} içinde yok")))?;
    let actual = sha256_hex(zip_bytes);
    if actual == expected {
        Ok(())
    } else {
        Err(UpdateError::Checksum(format!(
            "{zip_name}: beklenen {expected}, bulunan {actual}"
        )))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn parse_sha256sums(text: &str, file_name: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.trim_start_matches('*') == file_name {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

fn extract_exe(zip_bytes: &[u8]) -> Result<Vec<u8>, UpdateError> {
    use std::io::Read;

    let exe_name = format!("mousedrive{EXE_SUFFIX}");
    let archive_error = |e: &dyn std::fmt::Display| UpdateError::Archive(e.to_string());
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| archive_error(&e))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| archive_error(&e))?;
        let name = file.name().map_err(|e| archive_error(&e))?.into_owned();
        if name.contains("..") {
            continue;
        }
        let base = name.rsplit(['/', '\\']).next().unwrap_or("");
        if base.eq_ignore_ascii_case(&exe_name) {
            let mut out = Vec::new();
            file.read_to_end(&mut out).map_err(|e| archive_error(&e))?;
            if out.is_empty() {
                return Err(UpdateError::Archive(format!("{name} boş")));
            }
            return Ok(out);
        }
    }
    Err(UpdateError::Archive(format!("arşivde {exe_name} yok")))
}

pub fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
