#![deny(unsafe_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Connection {
    Connected,
    SetupRequired,
    Busy,
    Lost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppStatus {
    SetupRequired,
    DeviceBusy,
    Lost,
    Binding,
    Paused,
    NotReading,
    Degraded,
    Active,
}

impl AppStatus {
    pub const ALL: [AppStatus; 8] = [
        AppStatus::SetupRequired,
        AppStatus::DeviceBusy,
        AppStatus::Lost,
        AppStatus::Binding,
        AppStatus::Paused,
        AppStatus::NotReading,
        AppStatus::Degraded,
        AppStatus::Active,
    ];

    pub fn index(self) -> usize {
        match self {
            AppStatus::SetupRequired => 0,
            AppStatus::DeviceBusy => 1,
            AppStatus::Lost => 2,
            AppStatus::Binding => 3,
            AppStatus::Paused => 4,
            AppStatus::NotReading => 5,
            AppStatus::Degraded => 6,
            AppStatus::Active => 7,
        }
    }

    pub fn from_index(i: usize) -> Self {
        AppStatus::ALL
            .get(i)
            .copied()
            .unwrap_or(AppStatus::SetupRequired)
    }
}

pub fn status_rgb(s: AppStatus) -> [u8; 3] {
    match s {
        AppStatus::Active => [0x56, 0xB4, 0xE9],
        AppStatus::Paused => [0x9E, 0x9E, 0x9E],
        AppStatus::NotReading | AppStatus::Degraded | AppStatus::DeviceBusy => [0xE6, 0x9F, 0x00],
        AppStatus::Lost | AppStatus::SetupRequired => [0xD5, 0x5E, 0x00],
        AppStatus::Binding => [0xCC, 0x79, 0xA7],
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Health {
    pub write_failures: u64,
    pub loop_hz: f64,
    pub target_hz: f64,
    pub clipped_ticks: u64,
    pub version_mismatch: bool,
}

pub const SLOW_LOOP_RATIO: f64 = 0.9;

impl Health {
    pub fn slow_loop(&self) -> bool {
        self.loop_hz > 0.0 && self.loop_hz < self.target_hz * SLOW_LOOP_RATIO
    }

    pub fn degraded(&self) -> bool {
        self.write_failures > 0
            || self.slow_loop()
            || self.clipped_ticks > 0
            || self.version_mismatch
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatusInputs {
    pub connection: Connection,
    pub setup_failures: bool,
    pub capture_enabled: bool,
    pub binding: bool,
    pub input_sink_enabled: bool,
    pub app_foreground: bool,
    pub health: Health,
}

pub fn derive_status(i: &StatusInputs) -> AppStatus {
    match i.connection {
        Connection::SetupRequired => return AppStatus::SetupRequired,
        Connection::Busy => return AppStatus::DeviceBusy,
        Connection::Lost => return AppStatus::Lost,
        Connection::Connected => {}
    }
    if i.setup_failures {
        AppStatus::SetupRequired
    } else if i.binding {
        AppStatus::Binding
    } else if !i.capture_enabled {
        AppStatus::Paused
    } else if !i.input_sink_enabled && !i.app_foreground {
        AppStatus::NotReading
    } else if i.health.degraded() {
        AppStatus::Degraded
    } else {
        AppStatus::Active
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cue {
    CaptureOn,
    CaptureOff,
    Warning,
    Lost,
    Reconnected,
    ProfileChanged,
    Bind,
}

pub const LOST_REPEAT_MS: f64 = 3000.0;
pub const DEGRADED_MIN_INTERVAL_MS: f64 = 60_000.0;

fn is_running(s: AppStatus) -> bool {
    matches!(
        s,
        AppStatus::Active | AppStatus::NotReading | AppStatus::Degraded
    )
}

#[derive(Clone, Debug, Default)]
pub struct CuePlanner {
    last: Option<AppStatus>,
    last_lost_cue_ms: f64,
    last_degraded_cue_ms: Option<f64>,
}

impl CuePlanner {
    pub fn on_status(&mut self, status: AppStatus, now_ms: f64) -> Option<Cue> {
        let prev = self.last.replace(status)?;
        if status == AppStatus::Lost {
            let repeat_due = now_ms - self.last_lost_cue_ms >= LOST_REPEAT_MS;
            if prev != AppStatus::Lost || repeat_due {
                self.last_lost_cue_ms = now_ms;
                return Some(Cue::Lost);
            }
            return None;
        }
        if prev == status {
            return None;
        }
        self.transition_cue(prev, status, now_ms)
    }

    fn transition_cue(&mut self, prev: AppStatus, status: AppStatus, now_ms: f64) -> Option<Cue> {
        match (prev, status) {
            (AppStatus::Lost, s) if is_running(s) || s == AppStatus::Paused => {
                Some(Cue::Reconnected)
            }
            (AppStatus::Paused, s) if is_running(s) => Some(Cue::CaptureOn),
            (p, AppStatus::Paused) if is_running(p) => Some(Cue::CaptureOff),
            (_, AppStatus::DeviceBusy) => Some(Cue::Warning),
            (_, AppStatus::Degraded) => {
                let due = self
                    .last_degraded_cue_ms
                    .is_none_or(|t| now_ms - t >= DEGRADED_MIN_INTERVAL_MS);
                if !due {
                    return None;
                }
                self.last_degraded_cue_ms = Some(now_ms);
                Some(Cue::Warning)
            }
            _ => None,
        }
    }
}
