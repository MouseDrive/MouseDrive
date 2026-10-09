#![deny(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use mousedrive::config::Config;
use mousedrive::instance::{self, InstanceLock, LockError};
#[cfg(target_os = "linux")]
use mousedrive::platform::{DisplayEnv, MissingLibs, WindowSystem};
use mousedrive::{log, platform};

use crate::cli::Args;
#[cfg(target_os = "linux")]
use crate::cli::Backend;
#[cfg(target_os = "linux")]
use crate::lang::fill;
use crate::lang::{Lang, Strings};

const RELAUNCH_LOCK_WAIT: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    title: &'static str,
    body: String,
    log: Option<String>,
}

pub fn refuse(refusal: Refusal) -> ! {
    if let Some(text) = &refusal.log {
        log::error(text);
    }
    platform::alert(refusal.title, &refusal.body);
    if refusal.log.is_some() {
        log::flush();
    }
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
pub fn check_root(args: &Args, s: &Strings) -> Result<(), Refusal> {
    match root_refusal(platform::running_as_root(), args.allow_root, s) {
        Some(refusal) => Err(refusal),
        None => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn root_refusal(root: bool, allowed: bool, s: &Strings) -> Option<Refusal> {
    (root && !allowed).then(|| Refusal {
        title: s.root_refused_title,
        body: s.root_refused_body.to_owned(),
        log: None,
    })
}

pub fn configured_lang(config: Option<&Path>, fallback: Lang) -> Lang {
    config
        .and_then(|path| Config::load_from_file(path).ok())
        .map_or(fallback, |cfg| Lang::from_i32(cfg.language))
}

pub fn run(args: &Args, retried: bool, s: &Strings) -> Result<Option<InstanceLock>, Refusal> {
    #[cfg(target_os = "linux")]
    check_session(args, s)?;
    #[cfg(not(target_os = "linux"))]
    if args.allow_root {
        log::warn("--allow-root yalnız Linux'ta geçerli; yok sayıldı");
    }
    let relaunched = args.updated_from.is_some() || args.rolled_back;
    single_instance(lock_wait(relaunched || retried), s)
}

#[cfg(target_os = "linux")]
fn check_session(args: &Args, s: &Strings) -> Result<(), Refusal> {
    if args.allow_root && platform::running_as_root() {
        log::warn("root olarak çalışıyor (--allow-root)");
    }
    let display = platform::display_env();
    if !display.any() {
        return Err(no_display(s));
    }
    let system = window_system(display, args.backend);
    let missing = platform::missing_runtime_libs(system);
    if missing.is_empty() {
        return Ok(());
    }
    Err(missing_libs(s, system, &missing))
}

#[cfg(target_os = "linux")]
pub fn window_system(display: DisplayEnv, backend: Option<Backend>) -> WindowSystem {
    display.window_system(backend.map(|b| match b {
        Backend::X11 => WindowSystem::X11,
        Backend::Wayland => WindowSystem::Wayland,
    }))
}

#[cfg(target_os = "linux")]
fn no_display(s: &Strings) -> Refusal {
    Refusal {
        title: s.no_display_title,
        body: s.no_display_body.to_owned(),
        log: Some("ekran bulunamadı: WAYLAND_DISPLAY, WAYLAND_SOCKET ve DISPLAY tanımsız".into()),
    }
}

#[cfg(target_os = "linux")]
fn missing_libs(s: &Strings, system: WindowSystem, missing: &MissingLibs) -> Refusal {
    let libs = missing.sonames.join(", ");
    let body = fill(
        s.missing_libs_body,
        &[
            ("libs", &libs),
            ("debian", &missing.debian),
            ("fedora", &missing.fedora),
            ("arch", &missing.arch),
        ],
    );
    Refusal {
        title: s.missing_libs_title,
        body,
        log: Some(format!(
            "pencere için gereken kütüphaneler eksik ({system:?}): {libs}"
        )),
    }
}

fn lock_wait(relaunched: bool) -> Duration {
    if relaunched {
        RELAUNCH_LOCK_WAIT
    } else {
        Duration::ZERO
    }
}

fn single_instance(wait: Duration, s: &Strings) -> Result<Option<InstanceLock>, Refusal> {
    match instance::acquire(wait) {
        Ok(lock) => {
            log::info(&format!(
                "tek örnek kilidi alındı: {}",
                lock.path().display()
            ));
            Ok(Some(lock))
        }
        Err(LockError::Busy(path)) => Err(already_running(s, &path)),
        Err(e) => {
            log::warn(&format!(
                "tek örnek kilidi alınamadı, kilitsiz devam ediliyor: {e}"
            ));
            Ok(None)
        }
    }
}

fn already_running(s: &Strings, lock: &Path) -> Refusal {
    Refusal {
        title: s.instance_running_title,
        body: s.instance_running_body.to_owned(),
        log: Some(format!(
            "başka bir MouseDrive çalışıyor; bu örnek kapanıyor (kilit: {})",
            lock.display()
        )),
    }
}
