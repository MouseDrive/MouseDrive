#![deny(unsafe_code)]

use std::backtrace::Backtrace;
use std::io::ErrorKind;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CRASH_FILE: &str = "last_crash.txt";
const SEEN_FILE: &str = "last_crash.old.txt";

pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        record_panic(info);
        previous(info);
    }));
}

pub fn take_previous_crash() -> Option<PathBuf> {
    take_marker(&crate::log::log_dir()?)
}

fn record_panic(info: &PanicHookInfo<'_>) {
    let location = info.location().map_or_else(
        || "?".to_owned(),
        |l| format!("{}:{}:{}", l.file(), l.line(), l.column()),
    );
    let report = crash_report(
        &crate::log::timestamp(SystemTime::now()),
        &crate::log::thread_label(),
        &location,
        info.payload_as_str().unwrap_or("?"),
        &Backtrace::force_capture().to_string(),
    );
    crate::log::error(&report);
    if let Some(dir) = crate::log::log_dir()
        && let Err(e) = std::fs::write(dir.join(CRASH_FILE), &report)
    {
        crate::log::error(&format!("çökme raporu yazılamadı: {e}"));
    }
    crate::log::flush();
}

fn crash_report(
    time: &str,
    thread: &str,
    location: &str,
    message: &str,
    backtrace: &str,
) -> String {
    format!(
        "MouseDrive v{} panik ({time})\nthread: '{thread}'\nkonum: {location}\nmesaj: {message}\nbacktrace:\n{backtrace}",
        env!("CARGO_PKG_VERSION")
    )
}

fn take_marker(dir: &Path) -> Option<PathBuf> {
    let marker = dir.join(CRASH_FILE);
    let kept = dir.join(SEEN_FILE);
    match std::fs::rename(&marker, &kept) {
        Ok(()) => Some(kept),
        Err(e) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => {
            crate::log::warn(&format!("çökme işareti taşınamadı: {e}"));
            match std::fs::remove_file(&marker) {
                Ok(()) => Some(dir.join(crate::log::LOG_FILE)),
                Err(e) => {
                    crate::log::warn(&format!("çökme işareti silinemedi: {e}"));
                    Some(marker)
                }
            }
        }
    }
}
