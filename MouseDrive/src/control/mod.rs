mod connection;
mod engine;
pub mod guard;
mod io;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::bind::{BindPhase, BindTarget};
use crate::config::{Config, Tuning};
use crate::diagnostics::LoopSummary;
use crate::input::DeviceFilter;
use crate::logic::BrakePhase;
use crate::output::{OutputFrame, WriteHealth};
use crate::setup::SetupReport;
use crate::status::{AppStatus, Connection, Cue, Health};
use crate::telemetry::{DEFAULT_CAPACITY, Sample, TelemetryRing};

pub use engine::AbSide;
use engine::Engine;

pub const KEYBOARD_TEXT: u8 = 1;
pub const KEYBOARD_BINDING: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Reconnect,
    ResetSteering,
    SetCapture(bool),
    ToggleCapture,
    StartBind(BindTarget),
    CancelBind,
    ApplyProfile { name: String, tuning: Box<Tuning> },
    StartAb,
    ToggleAb,
    EndAb,
    PlayCue(Cue),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub test_counter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapshot {
    pub status: AppStatus,
    pub connection: Connection,
    pub capture: bool,
    pub binding: Option<(BindTarget, BindPhase)>,
    pub ab_side: Option<AbSide>,
    pub pending_apply: bool,
    pub frame: OutputFrame,
    pub steering_raw: f64,
    pub throttle_target: f64,
    pub throttle_pressed: bool,
    pub brake_pressed: bool,
    pub brake_floor: f64,
    pub brake_phase: BrakePhase,
    pub loop_summary: LoopSummary,
    pub target_hz: f64,
    pub writes: WriteHealth,
    pub reconnects: u32,
    pub retry_in_ms: Option<f64>,
    pub clipped_ticks: u64,
    pub mouse_hz: f64,
    pub absolute_events: u64,
    pub ignored_events: u64,
    pub device_filter: DeviceFilter,
    pub registration_ok: bool,
    pub stuck_releases: u64,
    pub counts_total: i64,
    pub health: Health,
    pub test_counter: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            status: AppStatus::SetupRequired,
            connection: Connection::SetupRequired,
            capture: false,
            binding: None,
            ab_side: None,
            pending_apply: false,
            frame: OutputFrame::NEUTRAL,
            steering_raw: 0.0,
            throttle_target: 0.0,
            throttle_pressed: false,
            brake_pressed: false,
            brake_floor: 0.0,
            brake_phase: BrakePhase::Idle,
            loop_summary: LoopSummary::default(),
            target_hz: 0.0,
            writes: WriteHealth::default(),
            reconnects: 0,
            retry_in_ms: None,
            clipped_ticks: 0,
            mouse_hz: 0.0,
            absolute_events: 0,
            ignored_events: 0,
            device_filter: DeviceFilter::AllMice,
            registration_ok: false,
            stuck_releases: 0,
            counts_total: 0,
            health: Health::default(),
            test_counter: false,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetupInfo {
    pub seq: u64,
    pub report: SetupReport,
    pub backend: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaleConfig {
    pub epoch: u64,
    pub config: Box<Config>,
}

struct ConfigSlot {
    epoch: u64,
    config: Config,
}

pub struct Shared {
    running: AtomicBool,
    config: Mutex<ConfigSlot>,
    config_epoch: AtomicU64,
    commands: Sender<Command>,
    snapshot: Mutex<Snapshot>,
    setup: Mutex<SetupInfo>,
    telemetry: Mutex<TelemetryRing>,
    gui_keyboard: AtomicU8,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    pub fn new(config: Config) -> (Arc<Self>, Receiver<Command>) {
        let (tx, rx) = mpsc::channel();
        let shared = Self {
            running: AtomicBool::new(true),
            config: Mutex::new(ConfigSlot { epoch: 0, config }),
            config_epoch: AtomicU64::new(0),
            commands: tx,
            snapshot: Mutex::new(Snapshot::default()),
            setup: Mutex::new(SetupInfo::default()),
            telemetry: Mutex::new(TelemetryRing::with_capacity(DEFAULT_CAPACITY)),
            gui_keyboard: AtomicU8::new(0),
        };
        (Arc::new(shared), rx)
    }

    pub fn config_epoch(&self) -> u64 {
        self.config_epoch.load(Ordering::Acquire)
    }

    pub fn config(&self) -> (u64, Config) {
        let slot = lock(&self.config);
        (slot.epoch, slot.config.clone())
    }

    pub fn publish_config(&self, config: &Config, base_epoch: u64) -> Result<u64, StaleConfig> {
        self.replace_config(config, base_epoch).ok_or_else(|| {
            let (epoch, config) = self.config();
            StaleConfig {
                epoch,
                config: Box::new(config),
            }
        })
    }

    fn replace_config(&self, config: &Config, base_epoch: u64) -> Option<u64> {
        let mut slot = lock(&self.config);
        if slot.epoch != base_epoch {
            return None;
        }
        slot.epoch += 1;
        slot.config = config.clone();
        self.config_epoch.store(slot.epoch, Ordering::Release);
        Some(slot.epoch)
    }

    pub fn send(&self, command: Command) {
        if self.commands.send(command).is_err() {
            crate::log::line("kontrol thread'i kapalı; komut atlandı");
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        *lock(&self.snapshot)
    }

    fn store_snapshot(&self, snapshot: Snapshot) {
        *lock(&self.snapshot) = snapshot;
    }

    pub fn setup_if_newer(&self, seen: u64) -> Option<SetupInfo> {
        let info = lock(&self.setup);
        (info.seq != seen).then(|| info.clone())
    }

    fn store_setup(&self, report: SetupReport, backend: Option<String>) {
        let mut info = lock(&self.setup);
        let seq = info.seq + 1;
        *info = SetupInfo {
            seq,
            report,
            backend,
        };
    }

    pub fn copy_telemetry(&self, after: Option<u64>, out: &mut Vec<Sample>) {
        lock(&self.telemetry).copy_since(after, out);
    }

    fn push_telemetry(&self, sample: Sample) {
        lock(&self.telemetry).push(sample);
    }

    pub fn set_gui_keyboard(&self, flags: u8) {
        self.gui_keyboard.store(flags, Ordering::Release);
    }

    fn gui_keyboard(&self) -> u8 {
        self.gui_keyboard.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }

    fn running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }
}

pub fn spawn(
    shared: Arc<Shared>,
    commands: Receiver<Command>,
    options: Options,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            if !crate::platform::raise_current_thread_priority() {
                crate::log::line("kontrol thread'i önceliği yükseltilemedi");
            }
            let mut io = io::SystemIo::new();
            let (epoch, config) = shared.config();
            let mut engine = Engine::new(config, epoch, options);
            run(&shared, &commands, &mut engine, &mut io);
        })
}

fn run(shared: &Shared, commands: &Receiver<Command>, engine: &mut Engine, io: &mut io::SystemIo) {
    let start = Instant::now();
    let mut next_ms = 0.0;
    while shared.running() {
        let now_ms = start.elapsed().as_secs_f64() * 1000.0;
        engine.tick(now_ms, io, shared, commands);
        let after_ms = start.elapsed().as_secs_f64() * 1000.0;
        let (next, sleep_ms) = pace(next_ms, engine.interval_ms(), after_ms);
        next_ms = next;
        if sleep_ms > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(sleep_ms / 1000.0));
        }
    }
}

fn pace(prev_deadline_ms: f64, interval_ms: f64, now_ms: f64) -> (f64, f64) {
    let next = prev_deadline_ms + interval_ms;
    if next <= now_ms {
        (now_ms, 0.0)
    } else {
        (next, next - now_ms)
    }
}
