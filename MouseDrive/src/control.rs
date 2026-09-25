#![deny(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::input::{LEFT_BUTTON, MIDDLE_BUTTON_CLICKED, MOUSE_DELTA_X, RIGHT_BUTTON, is_key_down};
use crate::logic::{BrakeState, MouseDriveState, RampDir, STEERING_RANGE};
use crate::vjoy::{
    AXIS_CENTER, AXIS_MAX, AXIS_MIN, HID_USAGE_RZ, HID_USAGE_X, HID_USAGE_Y, VJoyApi, VJoyStatus,
    VjdStat,
};

#[derive(Clone)]
pub struct Snapshot {
    pub steering_filtered: f64,
    pub throttle: f64,
    pub brake: f64,
    pub throttle_dir: RampDir,
    pub throttle_phase: f64,
    pub brake_state: BrakeState,
    pub brake_apply_phase: f64,
    pub brake_posthold_phase: f64,
    pub w_key_pressed: bool,
    pub s_key_pressed: bool,
    pub capture_enabled: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            steering_filtered: 0.0,
            throttle: 0.0,
            brake: 0.0,
            throttle_dir: RampDir::Hold,
            throttle_phase: 0.0,
            brake_state: BrakeState::Idle,
            brake_apply_phase: 0.0,
            brake_posthold_phase: 0.0,
            w_key_pressed: false,
            s_key_pressed: false,
            capture_enabled: true,
        }
    }
}

pub struct Shared {
    config: Mutex<Config>,
    config_dirty: AtomicBool,
    snapshot: Mutex<Snapshot>,
    vjoy_status: Mutex<VJoyStatus>,
    running: AtomicBool,
    reconnect_req: AtomicBool,
    reset_steering_req: AtomicBool,
    curves_edited_req: AtomicBool,
}

impl Shared {
    pub fn new(config: Config) -> Arc<Self> {
        Arc::new(Self {
            config: Mutex::new(config),
            config_dirty: AtomicBool::new(false),
            snapshot: Mutex::new(Snapshot::default()),
            vjoy_status: Mutex::new(VJoyStatus::Unknown),
            running: AtomicBool::new(true),
            reconnect_req: AtomicBool::new(false),
            reset_steering_req: AtomicBool::new(false),
            curves_edited_req: AtomicBool::new(false),
        })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn vjoy_status(&self) -> VJoyStatus {
        self.vjoy_status
            .lock()
            .map(|s| s.clone())
            .unwrap_or(VJoyStatus::Unknown)
    }

    pub fn publish_config(&self, cfg: &Config) {
        if let Ok(mut c) = self.config.lock() {
            c.clone_from(cfg);
        }
        self.config_dirty.store(true, Ordering::Release);
    }

    pub fn request_reconnect(&self) {
        self.reconnect_req.store(true, Ordering::Release);
    }
    pub fn request_reset_steering(&self) {
        self.reset_steering_req.store(true, Ordering::Release);
    }
    pub fn request_curves_reseed(&self) {
        self.curves_edited_req.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Release);
    }
}

fn connect_vjoy(device_id: u32) -> (Option<VJoyApi>, VJoyStatus) {
    match VJoyApi::load() {
        Some(api) => {
            if !api.is_enabled() {
                crate::log::line("vJoy surucusu etkin degil");
                return (None, VJoyStatus::DriverDisabled);
            }
            match api.get_status(device_id) {
                VjdStat::Free | VjdStat::Own => {
                    if api.acquire(device_id) {
                        api.reset(device_id);
                        crate::log::line(&format!("vJoy baglandi (cihaz {device_id})"));
                        (Some(api), VJoyStatus::Connected)
                    } else {
                        crate::log::line(&format!("vJoy cihaz {device_id} alinamadi"));
                        (None, VJoyStatus::AcquireFailed)
                    }
                }
                VjdStat::Busy => {
                    crate::log::line(&format!("vJoy cihaz {device_id} mesgul"));
                    (None, VJoyStatus::DeviceBusy)
                }
                VjdStat::Miss => {
                    crate::log::line(&format!("vJoy cihaz {device_id} yok"));
                    (None, VJoyStatus::DeviceMissing)
                }
                _ => (None, VJoyStatus::Unknown),
            }
        }
        None => {
            crate::log::line("vJoyInterface.dll bulunamadi veya sembol eksik");
            (None, VJoyStatus::DllNotFound)
        }
    }
}

struct ControlLoop {
    config: Config,
    state: MouseDriveState,
    vjoy: Option<VJoyApi>,
    device_id: u32,
}

impl ControlLoop {
    fn new(config: Config) -> (Self, VJoyStatus) {
        let device_id = config.vjoy_device_id.max(1) as u32;
        let (vjoy, status) = connect_vjoy(device_id);
        (
            Self {
                config,
                state: MouseDriveState::new(),
                vjoy,
                device_id,
            },
            status,
        )
    }

    fn reconnect(&mut self) -> VJoyStatus {
        if let Some(ref vjoy) = self.vjoy {
            vjoy.relinquish(self.device_id);
        }
        self.device_id = self.config.vjoy_device_id.max(1) as u32;
        let (vjoy, status) = connect_vjoy(self.device_id);
        self.vjoy = vjoy;
        status
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let delta_ms = now.duration_since(self.state.last_update).as_secs_f64() * 1000.0;
        self.state.last_update = now;

        let key_pressed = is_key_down(self.config.capture_toggle_key);
        if key_pressed && !self.state.capture_key_prev {
            self.state.capture_enabled = !self.state.capture_enabled;
            self.reset_state();
        }
        self.state.capture_key_prev = key_pressed;

        if MIDDLE_BUTTON_CLICKED.swap(false, Ordering::Acquire) {
            self.state.steering = 0.0;
            self.state.steering_filtered = 0.0;
        }

        self.state.w_key_pressed = is_key_down(0x57);
        self.state.s_key_pressed = is_key_down(0x53);

        let safe_interval = self.config.thread_interval_ms.max(1) as f64;
        let time_scale = (delta_ms / safe_interval).clamp(0.5, 2.0);

        if self.state.capture_enabled {
            self.state.update_steering(&self.config, delta_ms);
            self.state.update_throttle(&self.config, time_scale);
            self.state.update_brake(&self.config, now, time_scale);
        } else {
            self.state.steering_filtered = 0.0;
            self.state.throttle = 0.0;
            self.state.brake = 0.0;
        }

        self.send_to_vjoy();
    }

    fn reset_state(&mut self) {
        LEFT_BUTTON.store(false, Ordering::Release);
        RIGHT_BUTTON.store(false, Ordering::Release);
        MOUSE_DELTA_X.store(0, Ordering::Relaxed);
        self.state.steering = 0.0;
        self.state.steering_filtered = 0.0;
        self.state.throttle = 0.0;
        self.state.throttle_target = 0.0;
        self.state.throttle_phase = 0.0;
        self.state.throttle_dir = RampDir::Hold;
        self.state.brake = 0.0;
        self.state.brake_state = BrakeState::Idle;
        self.state.brake_apply_phase = 0.0;
        self.state.brake_posthold_phase = 0.0;
        if let Some(ref vjoy) = self.vjoy {
            vjoy.reset(self.device_id);
        }
    }

    fn send_to_vjoy(&self) {
        let Some(ref vjoy) = self.vjoy else { return };

        let safe_steering = self
            .state
            .steering_filtered
            .clamp(-STEERING_RANGE, STEERING_RANGE);
        let steer_axis = (AXIS_CENTER + safe_steering.round() as i32).clamp(AXIS_MIN, AXIS_MAX);
        let throttle_axis = (self.state.throttle * AXIS_MAX as f64).round() as i32;
        let brake_axis = (self.state.brake * AXIS_MAX as f64).round() as i32;

        vjoy.set_axis(steer_axis, self.device_id, HID_USAGE_X);
        vjoy.set_axis(throttle_axis, self.device_id, HID_USAGE_Y);
        vjoy.set_axis(brake_axis, self.device_id, HID_USAGE_RZ);
        vjoy.set_btn(self.state.w_key_pressed, self.device_id, 1);
        vjoy.set_btn(self.state.s_key_pressed, self.device_id, 2);
    }

    fn on_curves_edited(&mut self) {
        self.state.throttle_dir = RampDir::Hold;
        self.state.brake_apply_phase = self.config.brake_apply_curve.inverse_eval(self.state.brake);
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            steering_filtered: self.state.steering_filtered,
            throttle: self.state.throttle,
            brake: self.state.brake,
            throttle_dir: self.state.throttle_dir,
            throttle_phase: self.state.throttle_phase,
            brake_state: self.state.brake_state,
            brake_apply_phase: self.state.brake_apply_phase,
            brake_posthold_phase: self.state.brake_posthold_phase,
            w_key_pressed: self.state.w_key_pressed,
            s_key_pressed: self.state.s_key_pressed,
            capture_enabled: self.state.capture_enabled,
        }
    }
}

pub fn spawn(shared: Arc<Shared>) -> JoinHandle<()> {
    std::thread::spawn(move || run_loop(shared))
}

fn run_loop(shared: Arc<Shared>) {
    set_high_priority();

    let initial_cfg = shared.config.lock().map(|c| c.clone()).unwrap_or_default();
    let (mut ctl, status) = ControlLoop::new(initial_cfg);
    if let Ok(mut s) = shared.vjoy_status.lock() {
        *s = status;
    }

    while shared.running.load(Ordering::Acquire) {
        let tick_start = Instant::now();

        if shared.config_dirty.swap(false, Ordering::Acquire)
            && let Ok(c) = shared.config.lock()
        {
            ctl.config.clone_from(&c);
            drop(c);
            if ctl.config.vjoy_device_id.max(1) as u32 != ctl.device_id {
                shared.reconnect_req.store(true, Ordering::Release);
            }
        }

        if shared.curves_edited_req.swap(false, Ordering::Acquire) {
            ctl.on_curves_edited();
        }
        if shared.reconnect_req.swap(false, Ordering::Acquire) {
            let status = ctl.reconnect();
            if let Ok(mut s) = shared.vjoy_status.lock() {
                *s = status;
            }
        }
        if shared.reset_steering_req.swap(false, Ordering::Acquire) {
            ctl.state.steering = 0.0;
            ctl.state.steering_filtered = 0.0;
        }

        ctl.tick();

        if let Ok(mut snap) = shared.snapshot.lock() {
            *snap = ctl.snapshot();
        }

        let interval = Duration::from_millis(ctl.config.thread_interval_ms.max(1) as u64);
        let elapsed = tick_start.elapsed();
        if elapsed < interval {
            std::thread::sleep(interval - elapsed);
        }
    }

    if let Some(ref vjoy) = ctl.vjoy {
        vjoy.reset(ctl.device_id);
        vjoy.relinquish(ctl.device_id);
    }
    crate::log::line("kontrol thread'i kapandi");
}

fn set_high_priority() {
    #[allow(unsafe_code)]
    unsafe {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY,
        };
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(2));
    }
}
