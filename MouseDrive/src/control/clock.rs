#[cfg(target_os = "linux")]
pub(super) use linux::LoopClock;
#[cfg(not(target_os = "linux"))]
pub(super) use portable::LoopClock;

#[cfg(target_os = "linux")]
mod linux {
    use std::time::Duration;

    use crate::platform;

    const NANOS_PER_MS: f64 = 1_000_000.0;

    pub(in crate::control) struct LoopClock {
        start_ns: u64,
        sleep_failed: bool,
    }

    impl LoopClock {
        pub(in crate::control) fn start() -> Self {
            Self {
                start_ns: platform::monotonic_ns(),
                sleep_failed: false,
            }
        }

        pub(in crate::control) fn now_ms(&self) -> f64 {
            platform::monotonic_ns().saturating_sub(self.start_ns) as f64 / NANOS_PER_MS
        }

        pub(in crate::control) fn sleep_until(&mut self, deadline_ms: f64, sleep_ms: f64) {
            let offset_ns = (deadline_ms * NANOS_PER_MS).round() as u64;
            let Err(e) = platform::sleep_until_ns(self.start_ns.saturating_add(offset_ns)) else {
                return;
            };
            if !self.sleep_failed {
                self.sleep_failed = true;
                crate::log::warn(&format!(
                    "mutlak zamanlı uyku başarısız, göreli uykuya geçildi: {e}"
                ));
            }
            std::thread::sleep(Duration::from_secs_f64(sleep_ms / 1000.0));
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod portable {
    use std::time::{Duration, Instant};

    pub(in crate::control) struct LoopClock {
        start: Instant,
    }

    impl LoopClock {
        pub(in crate::control) fn start() -> Self {
            Self {
                start: Instant::now(),
            }
        }

        pub(in crate::control) fn now_ms(&self) -> f64 {
            self.start.elapsed().as_secs_f64() * 1000.0
        }

        pub(in crate::control) fn sleep_until(&mut self, _deadline_ms: f64, sleep_ms: f64) {
            std::thread::sleep(Duration::from_secs_f64(sleep_ms / 1000.0));
        }
    }
}
