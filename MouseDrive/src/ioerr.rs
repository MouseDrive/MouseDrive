#![deny(unsafe_code)]

use std::io::{Error, ErrorKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    NotFound,
    Denied,
    Exists,
    ReadOnly,
    Full,
    Busy,
    Interrupted,
    InvalidData,
    TimedOut,
    Unsupported,
    IsDirectory,
    NotDirectory,
    DirectoryNotEmpty,
}

pub const REASON_COUNT: usize = 13;

pub const REASONS_TR: [&str; REASON_COUNT] = [
    "bulunamadı",
    "izin yok",
    "zaten var",
    "dosya sistemi salt okunur",
    "disk dolu",
    "kaynak meşgul",
    "işlem kesildi",
    "geçersiz veri",
    "zaman aşımı",
    "desteklenmiyor",
    "bir klasör",
    "klasör değil",
    "klasör boş değil",
];

pub const UNKNOWN_TR: &str = "bilinmeyen hata";
pub const CODE_TR: &str = "hata kodu {code}";

impl Reason {
    pub fn from_kind(kind: ErrorKind) -> Option<Self> {
        Some(match kind {
            ErrorKind::NotFound => Reason::NotFound,
            ErrorKind::PermissionDenied => Reason::Denied,
            ErrorKind::AlreadyExists => Reason::Exists,
            ErrorKind::ReadOnlyFilesystem => Reason::ReadOnly,
            ErrorKind::StorageFull => Reason::Full,
            ErrorKind::ResourceBusy => Reason::Busy,
            ErrorKind::Interrupted => Reason::Interrupted,
            ErrorKind::InvalidData => Reason::InvalidData,
            ErrorKind::TimedOut => Reason::TimedOut,
            ErrorKind::Unsupported => Reason::Unsupported,
            ErrorKind::IsADirectory => Reason::IsDirectory,
            ErrorKind::NotADirectory => Reason::NotDirectory,
            ErrorKind::DirectoryNotEmpty => Reason::DirectoryNotEmpty,
            _ => return None,
        })
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IoFailure {
    pub kind: ErrorKind,
    pub code: Option<i32>,
}

impl IoFailure {
    pub fn from_kind(kind: ErrorKind) -> Self {
        Self { kind, code: None }
    }

    pub fn from_os(code: i32) -> Self {
        Self::from(&Error::from_raw_os_error(code))
    }

    pub fn reason(self) -> Option<Reason> {
        Reason::from_kind(self.kind)
    }

    pub fn text_tr(self) -> String {
        match (self.reason(), self.code) {
            (Some(reason), _) => REASONS_TR[reason.index()].to_string(),
            (None, Some(code)) => CODE_TR.replace("{code}", &code.to_string()),
            (None, None) => UNKNOWN_TR.to_string(),
        }
    }
}

impl From<&Error> for IoFailure {
    fn from(e: &Error) -> Self {
        Self {
            kind: e.kind(),
            code: e.raw_os_error(),
        }
    }
}

impl From<Error> for IoFailure {
    fn from(e: Error) -> Self {
        Self::from(&e)
    }
}

pub fn kind_tr(kind: ErrorKind) -> String {
    IoFailure::from_kind(kind).text_tr()
}
