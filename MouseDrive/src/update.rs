#![deny(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RELEASES_API: &str = "https://api.github.com/repos/Toxpox/MouseDrive/releases/latest";
const HTTP_TIMEOUT_SECS: u64 = 5;
const DOWNLOAD_TIMEOUT_SECS: u64 = 120;
const MAX_ZIP_BYTES: u64 = 100 * 1024 * 1024;
const MAX_SUMS_BYTES: u64 = 64 * 1024;
const USER_AGENT: &str = concat!("MouseDrive/", env!("CARGO_PKG_VERSION"));

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
    UpdateFailed(ReleaseInfo),
}

pub struct UpdateChecker {
    status: Arc<Mutex<UpdateStatus>>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateChecker {
    pub fn new() -> Self {
        Self {
            status: Arc::new(Mutex::new(UpdateStatus::Idle)),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(UpdateStatus::Failed)
    }

    pub fn spawn_check(&self) {
        if let Ok(mut s) = self.status.lock() {
            if matches!(*s, UpdateStatus::Checking | UpdateStatus::Updating) {
                return;
            }
            *s = UpdateStatus::Checking;
        } else {
            return;
        }

        let status = Arc::clone(&self.status);
        std::thread::spawn(move || {
            let result = check_latest().unwrap_or(UpdateStatus::Failed);
            if let Ok(mut s) = status.lock() {
                *s = result;
            }
        });
    }

    pub fn spawn_update(&self, info: ReleaseInfo) {
        if let Ok(mut s) = self.status.lock() {
            if matches!(*s, UpdateStatus::Updating) {
                return;
            }
            *s = UpdateStatus::Updating;
        } else {
            return;
        }

        let status = Arc::clone(&self.status);
        std::thread::spawn(move || {
            let result = match run_update(&info) {
                Ok(()) => UpdateStatus::ReadyToRestart,
                Err(()) => UpdateStatus::UpdateFailed(info),
            };
            if let Ok(mut s) = status.lock() {
                *s = result;
            }
        });
    }
}

fn check_latest() -> Option<UpdateStatus> {
    let info = fetch_latest()?;
    let remote = parse_version(&info.version)?;
    let local = parse_version(env!("CARGO_PKG_VERSION"))?;
    if remote > local {
        Some(UpdateStatus::Available(info))
    } else {
        Some(UpdateStatus::UpToDate)
    }
}

fn fetch_latest() -> Option<ReleaseInfo> {
    let resp = ureq::get(RELEASES_API)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
        .call()
        .ok()?;
    let body = resp.into_string().ok()?;
    parse_release_json(&body)
}

fn parse_release_json(body: &str) -> Option<ReleaseInfo> {
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
            if name.starts_with("MouseDrive-") && name.ends_with("-windows-x64.zip") {
                zip_url = Some(url.to_string());
                zip_name = name.to_string();
            } else if name == "SHA256SUMS.txt" {
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

fn run_update(info: &ReleaseInfo) -> Result<(), ()> {
    let zip_url = info.zip_url.as_ref().ok_or(())?;
    let sums_url = info.sums_url.as_ref().ok_or(())?;

    let zip_bytes = download(zip_url, MAX_ZIP_BYTES)?;
    let sums_text = String::from_utf8(download(sums_url, MAX_SUMS_BYTES)?).map_err(|_| ())?;

    let expected = parse_sha256sums(&sums_text, &info.zip_name).ok_or(())?;
    if sha256_hex(&zip_bytes) != expected {
        return Err(());
    }

    let exe_bytes = extract_exe(&zip_bytes)?;

    let tmp = std::env::temp_dir().join("mousedrive-update.exe");
    std::fs::write(&tmp, exe_bytes).map_err(|_| ())?;
    let replaced = self_replace::self_replace(&tmp);
    let _ = std::fs::remove_file(&tmp);
    replaced.map_err(|_| ())
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, ()> {
    use std::io::Read;

    let resp = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .call()
        .map_err(|_| ())?;
    let mut buf = Vec::new();
    resp.into_reader()
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|_| ())?;
    if buf.len() as u64 > limit {
        return Err(());
    }
    Ok(buf)
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

fn extract_exe(zip_bytes: &[u8]) -> Result<Vec<u8>, ()> {
    use std::io::Read;

    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| ())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|_| ())?;
        let name = file.name().to_string();
        if name.contains("..") {
            continue;
        }
        let base = name.rsplit(['/', '\\']).next().unwrap_or("");
        if base.eq_ignore_ascii_case("mousedrive.exe") {
            let mut out = Vec::new();
            file.read_to_end(&mut out).map_err(|_| ())?;
            if out.is_empty() {
                return Err(());
            }
            return Ok(out);
        }
    }
    Err(())
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
