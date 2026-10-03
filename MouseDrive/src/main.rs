#![windows_subsystem = "windows"]
#![deny(unsafe_code)]

mod curve_editor;
mod lang;
mod ui;
#[cfg(feature = "updater")]
mod update;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use eframe::egui::{IconData, ViewportBuilder};
use mousedrive::control::{self, Options, Shared};
use mousedrive::{input, log, overlay, platform};

use crate::lang::{Lang, Strings, strings};
use crate::ui::{App, Startup, status_labels};

const TITLE: &str = concat!("MouseDrive v", env!("CARGO_PKG_VERSION"));
const WINDOW_SIZE: [f32; 2] = [1180.0, 680.0];
const WINDOW_MIN_SIZE: [f32; 2] = [760.0, 560.0];
const APP_ICON_PNG: &[u8] = include_bytes!("../image/icon.png");
const UPDATED_FROM: &str = "--updated-from";
const MAX_VERSION_LEN: usize = 32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Args {
    test_counter: bool,
    allow_injected: bool,
    updated_from: Option<String>,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    let mut args = args.into_iter().peekable();
    let mut parsed = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--test-counter" => parsed.test_counter = true,
            "--allow-injected" => parsed.allow_injected = true,
            UPDATED_FROM => parsed.updated_from = args.next_if(|v| is_version(v)),
            _ => {}
        }
    }
    parsed
}

fn is_version(text: &str) -> bool {
    text.len() <= MAX_VERSION_LEN
        && text.starts_with(|c: char| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

struct Threads {
    control: Option<JoinHandle<()>>,
    input: Option<JoinHandle<()>>,
    overlay: Option<JoinHandle<()>>,
}

fn started(
    name: &str,
    result: std::io::Result<JoinHandle<()>>,
    errors: &mut Vec<String>,
) -> Option<JoinHandle<()>> {
    result
        .map_err(|e| {
            log::line(&format!("{name} thread'i başlatılamadı: {e}"));
            errors.push(format!("{name}: {e}"));
        })
        .ok()
}

fn main() -> eframe::Result<()> {
    let args = parse_args(std::env::args().skip(1));
    let exe = std::env::current_exe();
    let _timer = platform::setup_process();
    log::line(&format!("{TITLE} başlatıldı"));

    let startup = Startup::load();
    let cfg = &startup.config;
    let lang = Lang::from_i32(cfg.language);
    let mut errors = Vec::new();
    let input_thread = input::start(
        cfg.input_sink_enabled,
        &cfg.mouse_device,
        args.allow_injected,
    );
    let input_thread = started("input", input_thread, &mut errors);
    let overlay_thread = started("overlay", overlay::start(), &mut errors);
    overlay::configure(cfg.overlay_enabled, cfg.overlay_corner);
    overlay::set_labels(status_labels(lang));

    let (shared, commands) = Shared::new(startup.config.clone());
    let options = Options {
        test_counter: args.test_counter,
    };
    let control_thread = control::spawn(Arc::clone(&shared), commands, options);
    let threads = Threads {
        control: started("control", control_thread, &mut errors),
        input: input_thread,
        overlay: overlay_thread,
    };

    let restart = Arc::new(AtomicBool::new(false));
    let result = run_window(
        startup,
        Arc::clone(&shared),
        Arc::clone(&restart),
        errors,
        args.updated_from,
    );
    shutdown(&shared, threads);
    if restart.load(Ordering::Acquire) {
        let lang = Lang::from_i32(shared.config().1.language);
        relaunch(exe, strings(lang));
    }
    result
}

fn run_window(
    startup: Startup,
    shared: Arc<Shared>,
    restart: Arc<AtomicBool>,
    errors: Vec<String>,
    updated_from: Option<String>,
) -> eframe::Result<()> {
    let viewport = ViewportBuilder::default()
        .with_inner_size(WINDOW_SIZE)
        .with_min_inner_size(WINDOW_MIN_SIZE)
        .with_title(TITLE);
    let options = eframe::NativeOptions {
        viewport: match app_icon() {
            Some(icon) => viewport.with_icon(icon),
            None => viewport,
        },
        ..Default::default()
    };
    eframe::run_native(
        "MouseDrive",
        options,
        Box::new(move |_cc| {
            let mut app = App::new(startup, shared, restart);
            errors.iter().for_each(|e| app.report_launch_error(e));
            if let Some(from) = &updated_from {
                app.report_update(from);
            }
            Ok(Box::new(app))
        }),
    )
}

fn app_icon() -> Option<IconData> {
    eframe::icon_data::from_png_bytes(APP_ICON_PNG)
        .map_err(|e| log::line(&format!("uygulama ikonu okunamadı: {e}")))
        .ok()
}

fn shutdown(shared: &Shared, threads: Threads) {
    shared.stop();
    join("control", threads.control);
    input::stop();
    join("input", threads.input);
    overlay::stop();
    join("overlay", threads.overlay);
    log::line("kapatıldı");
}

fn join(name: &str, handle: Option<JoinHandle<()>>) {
    if let Some(handle) = handle
        && handle.join().is_err()
    {
        log::line(&format!("{name} thread'i panikle bitti"));
    }
}

fn relaunch(exe: std::io::Result<PathBuf>, s: &Strings) {
    let result = exe.and_then(|exe| {
        std::process::Command::new(exe)
            .args([UPDATED_FROM, env!("CARGO_PKG_VERSION")])
            .spawn()
    });
    if let Err(e) = result {
        log::line(&format!("yeniden başlatılamadı: {e}"));
        platform::alert(s.relaunch_failed_title, s.relaunch_failed_body);
    }
}
