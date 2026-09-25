#![windows_subsystem = "windows"]
#![deny(unsafe_code)]

mod curve_editor;
mod lang;
mod ui;
#[cfg(feature = "updater")]
mod update;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use eframe::egui::{IconData, ViewportBuilder};
use mousedrive::control::{self, Options, Shared};
use mousedrive::{input, log, overlay, platform};

use crate::lang::Lang;
use crate::ui::{App, Startup, status_labels};

const TITLE: &str = concat!("MouseDrive v", env!("CARGO_PKG_VERSION"));
const WINDOW_SIZE: [f32; 2] = [1000.0, 640.0];
const WINDOW_MIN_SIZE: [f32; 2] = [760.0, 500.0];
const APP_ICON_PNG: &[u8] = include_bytes!("../image/icon.png");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Args {
    test_counter: bool,
    allow_injected: bool,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    args.into_iter()
        .fold(Args::default(), |a, arg| match arg.as_str() {
            "--test-counter" => Args {
                test_counter: true,
                ..a
            },
            "--allow-injected" => Args {
                allow_injected: true,
                ..a
            },
            _ => a,
        })
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
    let _timer = platform::setup_process();
    log::line(&format!("{TITLE} başlatıldı"));

    let startup = Startup::load();
    let cfg = &startup.config;
    let mut errors = Vec::new();
    let input_thread = input::start(
        cfg.input_sink_enabled,
        &cfg.mouse_device,
        args.allow_injected,
    );
    let input_thread = started("raw-input", input_thread, &mut errors);
    let overlay_thread = started("overlay", overlay::start(), &mut errors);
    overlay::configure(cfg.overlay_enabled, cfg.overlay_corner);
    overlay::set_labels(status_labels(Lang::from_i32(cfg.language)));

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
    let result = run_window(startup, Arc::clone(&shared), Arc::clone(&restart), errors);
    shutdown(&shared, threads);
    if restart.load(Ordering::Acquire) {
        relaunch();
    }
    result
}

fn run_window(
    startup: Startup,
    shared: Arc<Shared>,
    restart: Arc<AtomicBool>,
    errors: Vec<String>,
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
    join("raw-input", threads.input);
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

fn relaunch() {
    let result = std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn());
    if let Err(e) = result {
        log::line(&format!("yeniden başlatılamadı: {e}"));
    }
}
