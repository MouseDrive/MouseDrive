#![deny(unsafe_code)]

use std::time::Duration;

use mousedrive::bind::{BindPhase, BindTarget};
use mousedrive::control::{AbSide, Snapshot};
use mousedrive::input::DeviceFilter;
use mousedrive::logic::BrakePhase;
use mousedrive::output::OutputFrame;
use mousedrive::status::{AppStatus, Connection, SessionState};

pub(super) const FAST: Duration = Duration::from_millis(16);
const IDLE: Duration = Duration::from_millis(150);
const BACKGROUND: Duration = Duration::from_millis(250);
const HOLD_MS: f64 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shown {
    status: AppStatus,
    connection: Connection,
    capture: bool,
    binding: Option<(BindTarget, BindPhase)>,
    ab_side: Option<AbSide>,
    pending_apply: bool,
    frame: OutputFrame,
    steering_raw: f64,
    throttle_target: f64,
    throttle_pressed: bool,
    brake_pressed: bool,
    brake_floor: f64,
    brake_phase: BrakePhase,
    counts_total: i64,
    device_filter: DeviceFilter,
    registration_ok: bool,
    session: SessionState,
    drops_since_center: u64,
}

impl Shown {
    pub(super) fn of(snap: &Snapshot) -> Self {
        Self {
            status: snap.status,
            connection: snap.connection,
            capture: snap.capture,
            binding: snap.binding,
            ab_side: snap.ab_side,
            pending_apply: snap.pending_apply,
            frame: snap.frame,
            steering_raw: snap.steering_raw,
            throttle_target: snap.throttle_target,
            throttle_pressed: snap.throttle_pressed,
            brake_pressed: snap.brake_pressed,
            brake_floor: snap.brake_floor,
            brake_phase: snap.brake_phase,
            counts_total: snap.counts_total,
            device_filter: snap.device_filter,
            registration_ok: snap.registration_ok,
            session: snap.session,
            drops_since_center: snap.drops_since_center,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Load {
    pub focused: bool,
    pub busy: bool,
    pub animating: bool,
}

fn repaint_after(load: Load) -> Duration {
    match (load.busy, load.focused, load.animating) {
        (true, _, _) | (false, true, true) => FAST,
        (false, true, false) => IDLE,
        (false, false, _) => BACKGROUND,
    }
}

#[derive(Debug, Default)]
pub(super) struct RepaintPlan {
    shown: Option<Shown>,
    changed_at_ms: Option<f64>,
}

impl RepaintPlan {
    pub(super) fn next(&mut self, shown: Shown, now_ms: f64, load: Load) -> Duration {
        if self.shown != Some(shown) {
            self.shown = Some(shown);
            self.changed_at_ms = Some(now_ms);
        }
        let recent = self
            .changed_at_ms
            .is_some_and(|at| (0.0..=HOLD_MS).contains(&(now_ms - at)));
        repaint_after(Load {
            animating: load.animating || recent,
            ..load
        })
    }
}
