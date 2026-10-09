#![deny(unsafe_code)]

use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailReason {
    Network,
    Verify,
    NotWritable,
    SelfTest,
    Replace,
    Restore,
    Other,
}

#[derive(Debug)]
pub enum UpdateError {
    Network {
        url: String,
        detail: String,
    },
    TooLarge {
        url: String,
        limit: u64,
    },
    Malformed(String),
    NoAsset,
    Checksum(String),
    Signature(String),
    Archive(String),
    NoExe(io::Error),
    NotWritable {
        dir: PathBuf,
        source: io::Error,
    },
    Write {
        path: PathBuf,
        source: io::Error,
    },
    SelfTest(String),
    Replace(String),
    #[cfg_attr(not(windows), allow(dead_code))]
    Restore(String),
    Closed,
}

impl UpdateError {
    pub fn reason(&self) -> FailReason {
        match self {
            UpdateError::Network { .. } | UpdateError::TooLarge { .. } => FailReason::Network,
            UpdateError::Checksum(_) | UpdateError::Signature(_) | UpdateError::Archive(_) => {
                FailReason::Verify
            }
            UpdateError::NotWritable { .. } => FailReason::NotWritable,
            UpdateError::SelfTest(_) => FailReason::SelfTest,
            UpdateError::Write { .. } | UpdateError::Replace(_) => FailReason::Replace,
            UpdateError::Restore(_) => FailReason::Restore,
            UpdateError::Malformed(_)
            | UpdateError::NoAsset
            | UpdateError::NoExe(_)
            | UpdateError::Closed => FailReason::Other,
        }
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpdateError::Network { url, detail } => write!(f, "ağ hatası ({url}): {detail}"),
            UpdateError::TooLarge { url, limit } => {
                write!(f, "dosya {limit} bayt sınırını aşıyor: {url}")
            }
            UpdateError::Malformed(detail) => write!(f, "beklenmeyen yanıt: {detail}"),
            UpdateError::NoAsset => f.write_str("bu sistem için indirilebilir paket yok"),
            UpdateError::Checksum(detail) => write!(f, "sağlama toplamı tutmuyor: {detail}"),
            UpdateError::Signature(detail) => write!(f, "imza doğrulanamadı: {detail}"),
            UpdateError::Archive(detail) => write!(f, "arşiv okunamadı: {detail}"),
            UpdateError::NoExe(e) => write!(f, "çalışan programın yolu bulunamadı: {e}"),
            UpdateError::NotWritable { dir, source } => {
                write!(
                    f,
                    "program klasörüne yazılamıyor ({}): {source}",
                    dir.display()
                )
            }
            UpdateError::Write { path, source } => {
                write!(f, "yeni sürüm yazılamadı ({}): {source}", path.display())
            }
            UpdateError::SelfTest(detail) => write!(f, "yeni sürüm denemesi başarısız: {detail}"),
            UpdateError::Replace(detail) => write!(f, "program dosyası değiştirilemedi: {detail}"),
            UpdateError::Restore(detail) => write!(
                f,
                "program dosyası değiştirilemedi ve önceki sürüm geri yüklenemedi: {detail}"
            ),
            UpdateError::Closed => f.write_str("uygulama kapanırken güncelleme durduruldu"),
        }
    }
}

impl std::error::Error for UpdateError {}
