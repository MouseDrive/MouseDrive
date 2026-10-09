#![windows_subsystem = "windows"]
#![deny(unsafe_code)]

mod cli;
mod curve_editor;
mod lang;
mod lifecycle;
mod preflight;
mod ui;
#[cfg(feature = "updater")]
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use eframe::egui::{IconData, ViewportBuilder};
use mousedrive::control::{self, Options, Shared};
use mousedrive::instance::InstanceLock;
use mousedrive::{config, crash, input, log, platform};

#[cfg(feature = "updater")]
use crate::cli::ROLLED_BACK;
use crate::cli::{Action, Backend, UPDATED_FROM};
#[cfg(feature = "updater")]
use crate::lang::fill;
use crate::lang::{Lang, Strings, strings};
use crate::lifecycle::ExitRequest;
use crate::ui::{App, Startup};

const TITLE: &str = concat!("MouseDrive v", env!("CARGO_PKG_VERSION"));
const WINDOW_SIZE: [f32; 2] = [1180.0, 680.0];
const WINDOW_MIN_SIZE: [f32; 2] = [760.0, 560.0];
const APP_ICON_PNG: &[u8] = include_bytes!("../image/icon.png");

struct Threads {
    control: Option<JoinHandle<()>>,
    input: Option<JoinHandle<()>>,
}

struct Prepared {
    exit_request: Arc<ExitRequest>,
    instance: Option<InstanceLock>,
}

struct Launch {
    errors: Vec<String>,
    updated_from: Option<String>,
    rolled_back: bool,
    previous_crash: Option<PathBuf>,
    backend: Option<Backend>,
    exit_request: Arc<ExitRequest>,
}

fn started(
    name: &str,
    result: std::io::Result<JoinHandle<()>>,
    errors: &mut Vec<String>,
) -> Option<JoinHandle<()>> {
    result
        .map_err(|e| {
            log::error(&format!("{name} thread'i başlatılamadı: {e}"));
            errors.push(format!("{name}: {e}"));
        })
        .ok()
}

fn main() -> eframe::Result<()> {
    let args = cli::parse(
        std::env::args_os()
            .skip(1)
            .map(|a| a.to_string_lossy().into_owned()),
    );
    let cli_lang = cli::env_lang();
    match args.action {
        Action::Help => return print_stdout(&cli::help_text(strings(cli_lang))),
        Action::Version => return print_stdout(&cli::version_line()),
        Action::ListDevices => {
            let devices = input::list_devices();
            return print_stdout(&cli::device_list(&devices, strings(cli_lang)));
        }
        Action::Run => {}
    }
    let Some(Prepared {
        exit_request,
        instance,
    }) = prepare(&args, cli_lang)
    else {
        return Ok(());
    };
    let previous_crash = take_previous_crash();
    let exe = std::env::current_exe();
    let _timer = platform::setup_process();

    let startup = Startup::load();
    let cfg = &startup.config;
    let mut errors = Vec::new();
    let input_thread = input::start(
        cfg.input_sink_enabled,
        &cfg.mouse_device,
        args.allow_injected,
    );
    let input_thread = started("input", input_thread, &mut errors);

    let (shared, commands) = Shared::new(startup.config.clone());
    let options = Options {
        test_counter: args.test_counter,
    };
    let control_thread = control::spawn(Arc::clone(&shared), commands, options);
    let threads = Threads {
        control: started("control", control_thread, &mut errors),
        input: input_thread,
    };

    let restart = Arc::new(AtomicBool::new(false));
    let launch = Launch {
        errors,
        updated_from: args.updated_from,
        rolled_back: args.rolled_back,
        previous_crash,
        backend: args.backend,
        exit_request: Arc::clone(&exit_request),
    };
    let result = run_window(startup, Arc::clone(&shared), Arc::clone(&restart), launch);
    shutdown(&shared, threads);
    drop(instance);
    if restart.load(Ordering::Acquire) {
        let lang = Lang::from_i32(shared.config().1.language);
        relaunch(exe, strings(lang));
    }
    #[cfg(target_os = "linux")]
    if result.is_err() {
        lifecycle::retry_on_x11(args.backend, exit_request.requested());
    }
    log::flush();
    result
}

fn prepare(args: &cli::Args, cli_lang: Lang) -> Option<Prepared> {
    let cli_strings = strings(cli_lang);
    #[cfg(target_os = "linux")]
    if let Err(refusal) = preflight::check_root(args, cli_strings) {
        preflight::refuse(refusal);
    }
    let exit_request = ExitRequest::new();
    let signals = lifecycle::watch_signals(&exit_request);
    start_logging(&args.unknown, cli_strings);
    if let Err(e) = signals {
        log::warn(&format!("kapatma sinyalleri izlenemiyor: {e}"));
    }
    let lang = preflight::configured_lang(config::config_path().as_deref(), cli_lang);
    let instance = preflight::run(args, x11_retry(), strings(lang))
        .unwrap_or_else(|refusal| preflight::refuse(refusal));
    if exit_request.requested() {
        log::info("başlatma sırasında kapatma istendi");
        log::flush();
        return None;
    }
    Some(Prepared {
        exit_request,
        instance,
    })
}

#[cfg(target_os = "linux")]
fn x11_retry() -> bool {
    let retried = lifecycle::is_x11_retry();
    if retried {
        log::info("Wayland penceresi açılamadığı için X11 ile yeniden başlatıldı");
    }
    retried
}

#[cfg(not(target_os = "linux"))]
fn x11_retry() -> bool {
    false
}

fn start_logging(unknown: &[String], s: &Strings) {
    for arg in unknown {
        print_stderr(&cli::unknown_warning(s, arg));
    }
    log::move_legacy_log();
    crash::install_panic_hook();
    log::install_bridge();
    log::info(&format!(
        "{TITLE} başlatıldı ({})",
        platform::environment_summary()
    ));
    config::log_portable_mode();
    for arg in unknown {
        log::warn(&format!(
            "bilinmeyen komut satırı seçeneği yok sayıldı: {arg}"
        ));
    }
}

fn take_previous_crash() -> Option<PathBuf> {
    let previous_crash = crash::take_previous_crash();
    if let Some(report) = &previous_crash {
        log::warn(&format!(
            "önceki oturum beklenmedik şekilde kapanmış; rapor: {}",
            report.display()
        ));
    }
    previous_crash
}

fn print_stdout(text: &str) -> eframe::Result<()> {
    use std::io::Write as _;
    let _ = writeln!(std::io::stdout().lock(), "{text}");
    Ok(())
}

fn print_stderr(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr().lock(), "{text}");
}

fn run_window(
    startup: Startup,
    shared: Arc<Shared>,
    restart: Arc<AtomicBool>,
    launch: Launch,
) -> eframe::Result<()> {
    let wayland = wayland_window(launch.backend);
    let viewport = ViewportBuilder::default()
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(WINDOW_MIN_SIZE)
        .with_title(TITLE);
    let options = eframe::NativeOptions {
        viewport: match app_icon() {
            Some(icon) => viewport.with_icon(icon),
            None => viewport,
        },
        event_loop_builder: event_loop_hook(launch.backend),
        vsync: !wayland,
        ..Default::default()
    };
    eframe::run_native(
        "MouseDrive",
        options,
        Box::new(move |cc| {
            launch.exit_request.attach(&cc.egui_ctx);
            let mut app = App::new(startup, shared, restart, Arc::clone(&launch.exit_request));
            if wayland {
                app.limit_frame_rate();
            }
            launch
                .errors
                .iter()
                .for_each(|e| app.report_launch_error(e));
            if let Some(from) = &launch.updated_from {
                app.report_update(from);
            }
            if launch.rolled_back {
                log::warn("güncelleme geri alındıktan sonra önceki sürüm açıldı");
                app.report_rollback();
            }
            #[cfg(feature = "updater")]
            if !launch.rolled_back {
                update::forget_backup_later();
            }
            if let Some(report) = &launch.previous_crash {
                app.report_crash(report);
            }
            Ok(Box::new(app))
        }),
    )
    .inspect_err(|e| log::error(&format!("pencere açılamadı: {e}")))
}

#[cfg(target_os = "linux")]
fn wayland_window(backend: Option<Backend>) -> bool {
    let wayland = preflight::window_system(platform::display_env(), backend)
        == platform::WindowSystem::Wayland;
    if wayland {
        log::info("Wayland penceresi: dikey eşitleme kapalı, kare hızı uygulamada sınırlanıyor");
    }
    wayland
}

#[cfg(not(target_os = "linux"))]
fn wayland_window(_backend: Option<Backend>) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn event_loop_hook(backend: Option<Backend>) -> Option<eframe::EventLoopBuilderHook> {
    use winit::platform::wayland::EventLoopBuilderExtWayland as _;
    use winit::platform::x11::EventLoopBuilderExtX11 as _;
    let backend = backend?;
    log::info(&format!("pencere arka ucu zorlandı: {}", backend.label()));
    Some(Box::new(move |builder| match backend {
        Backend::X11 => {
            builder.with_x11();
        }
        Backend::Wayland => {
            builder.with_wayland();
        }
    }))
}

#[cfg(not(target_os = "linux"))]
fn event_loop_hook(backend: Option<Backend>) -> Option<eframe::EventLoopBuilderHook> {
    if let Some(backend) = backend {
        log::warn(&format!(
            "--{} yalnız Linux'ta geçerli; yok sayıldı",
            backend.label()
        ));
    }
    None
}

fn app_icon() -> Option<IconData> {
    eframe::icon_data::from_png_bytes(APP_ICON_PNG)
        .map_err(|e| log::warn(&format!("uygulama ikonu okunamadı: {e}")))
        .ok()
}

fn shutdown(shared: &Shared, threads: Threads) {
    shared.stop();
    join("control", threads.control);
    input::stop();
    join("input", threads.input);
    log::info("kapatıldı");
    log::flush();
}

fn join(name: &str, handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle
        && handle.join().is_err()
    {
        log::error(&format!("{name} thread'i panikle bitti"));
    }
}

#[cfg(feature = "updater")]
fn relaunch(exe: std::io::Result<PathBuf>, s: &Strings) {
    let exe = match exe {
        Ok(exe) => exe,
        Err(e) => {
            log::error(&format!("yeniden başlatılamadı: {e}"));
            platform::alert(s.relaunch_failed_title, s.relaunch_failed_body);
            return;
        }
    };
    let new_args = [UPDATED_FROM, env!("CARGO_PKG_VERSION")];
    match update::relaunch(&exe, &new_args, &[ROLLED_BACK]) {
        update::Relaunch::Started | update::Relaunch::RolledBack => {}
        update::Relaunch::RestoredOnly => {
            platform::alert(s.relaunch_failed_title, s.relaunch_restored_body);
        }
        update::Relaunch::Broken => {
            let backup = update::backup_path(&exe).display().to_string();
            let exe = exe.display().to_string();
            let body = fill(
                s.relaunch_broken_body,
                &[("backup", &backup), ("exe", &exe)],
            );
            platform::alert(s.relaunch_failed_title, &body);
        }
    }
}

#[cfg(not(feature = "updater"))]
fn relaunch(exe: std::io::Result<PathBuf>, s: &Strings) {
    let result = exe.and_then(|exe| {
        platform::spawn_reaped(
            platform::child_command(exe).args([UPDATED_FROM, env!("CARGO_PKG_VERSION")]),
        )
    });
    if let Err(e) = result {
        log::error(&format!("yeniden başlatılamadı: {e}"));
        platform::alert(s.relaunch_failed_title, s.relaunch_failed_body);
    }
}
