#![deny(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use eframe::egui::{Context, ViewportCommand, ViewportId};
#[cfg(target_os = "linux")]
use mousedrive::log;
#[cfg(target_os = "linux")]
use mousedrive::platform::{self, DisplayEnv, SignalEvent};

#[cfg(target_os = "linux")]
use crate::cli::Backend;

#[cfg(target_os = "linux")]
const SIGNAL_GRACE: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(target_os = "linux")]
const SIGNAL_EXIT_BASE: i32 = 128;
#[cfg(target_os = "linux")]
const X11_RETRY_ENV: &str = "MOUSEDRIVE_X11_RETRY";

#[derive(Default)]
pub struct ExitRequest {
    requested: AtomicBool,
    ctx: OnceLock<Context>,
}

impl ExitRequest {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub fn attach(&self, ctx: &Context) {
        if self.ctx.set(ctx.clone()).is_ok() && self.requested() {
            close_window(ctx);
        }
    }

    #[cfg(target_os = "linux")]
    fn request(&self) {
        self.requested.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.ctx.get() {
            close_window(ctx);
        }
    }
}

fn close_window(ctx: &Context) {
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
}

#[cfg(target_os = "linux")]
pub fn watch_signals(request: &Arc<ExitRequest>) -> std::io::Result<()> {
    let request = Arc::clone(request);
    platform::watch_termination_signals(SIGNAL_GRACE, move |event| on_signal(&request, event))
}

#[cfg(not(target_os = "linux"))]
pub fn watch_signals(_request: &Arc<ExitRequest>) -> std::io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq, Eq)]
enum SignalAction {
    Close(String),
    ExitNow { code: i32, reason: String },
}

#[cfg(target_os = "linux")]
fn signal_action(event: SignalEvent) -> SignalAction {
    match event {
        SignalEvent::Received(signal) => SignalAction::Close(format!(
            "{} alındı; MouseDrive düzgünce kapatılıyor",
            platform::signal_name(signal)
        )),
        SignalEvent::Repeated(signal) => SignalAction::ExitNow {
            code: SIGNAL_EXIT_BASE + signal,
            reason: format!(
                "ikinci kez {} alındı; hemen çıkılıyor",
                platform::signal_name(signal)
            ),
        },
        SignalEvent::GraceExpired(signal) => SignalAction::ExitNow {
            code: SIGNAL_EXIT_BASE + signal,
            reason: format!(
                "{} sonrası kapanış {} s içinde bitmedi; zorla çıkılıyor",
                platform::signal_name(signal),
                SIGNAL_GRACE.as_secs()
            ),
        },
    }
}

#[cfg(target_os = "linux")]
fn on_signal(request: &ExitRequest, event: SignalEvent) {
    match signal_action(event) {
        SignalAction::Close(text) => {
            log::info(&text);
            request.request();
        }
        SignalAction::ExitNow { code, reason } => {
            log::warn(&reason);
            log::flush();
            std::process::exit(code);
        }
    }
}

#[cfg(target_os = "linux")]
pub fn is_x11_retry() -> bool {
    std::env::var_os(X11_RETRY_ENV).is_some()
}

#[cfg(target_os = "linux")]
fn x11_retry_wanted(
    display: DisplayEnv,
    retried: bool,
    forced: Option<Backend>,
    exit_requested: bool,
) -> bool {
    display.wayland && display.x11 && !retried && forced.is_none() && !exit_requested
}

#[cfg(target_os = "linux")]
pub fn retry_on_x11(forced: Option<Backend>, exit_requested: bool) {
    use std::os::unix::process::CommandExt as _;
    if !x11_retry_wanted(
        platform::display_env(),
        is_x11_retry(),
        forced,
        exit_requested,
    ) {
        return;
    }
    let error = match std::env::current_exe() {
        Ok(exe) => {
            log::warn(
                "pencere Wayland ile açılamadı; X11 (XWayland) ile bir kez yeniden deneniyor",
            );
            log::flush();
            platform::child_command(exe)
                .args(std::env::args_os().skip(1))
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("WAYLAND_SOCKET")
                .env(X11_RETRY_ENV, "1")
                .exec()
        }
        Err(e) => e,
    };
    log::error(&format!("X11 ile yeniden başlatılamadı: {error}"));
}
