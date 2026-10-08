#![deny(unsafe_code)]

use std::fmt::Display;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::Level;

const BURST: u32 = 5;
const WINDOW: Duration = Duration::from_secs(60);
const MAX_TARGETS: usize = 64;

struct Window {
    target: String,
    start: Instant,
    passed: u32,
    suppressed: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Pass { suppressed: u32 },
    Drop,
}

pub(super) struct Limiter {
    windows: Vec<Window>,
}

impl Limiter {
    pub(super) const fn new() -> Self {
        Self {
            windows: Vec::new(),
        }
    }

    pub(super) fn check(&mut self, target: &str, now: Instant) -> Verdict {
        let Some(window) = self.windows.iter_mut().find(|w| w.target == target) else {
            if self.windows.len() >= MAX_TARGETS {
                self.windows.clear();
            }
            self.windows.push(Window {
                target: target.to_owned(),
                start: now,
                passed: 1,
                suppressed: 0,
            });
            return Verdict::Pass { suppressed: 0 };
        };
        if now.saturating_duration_since(window.start) >= WINDOW {
            window.start = now;
            window.passed = 1;
            return Verdict::Pass {
                suppressed: std::mem::take(&mut window.suppressed),
            };
        }
        if window.passed < BURST {
            window.passed += 1;
            return Verdict::Pass { suppressed: 0 };
        }
        window.suppressed = window.suppressed.saturating_add(1);
        Verdict::Drop
    }
}

pub(super) fn forward(
    limiter: &mut Limiter,
    now: Instant,
    target: &str,
    level: ::log::Level,
    message: &dyn Display,
    mut emit: impl FnMut(Level, &str),
) {
    let Verdict::Pass { suppressed } = limiter.check(target, now) else {
        return;
    };
    if suppressed > 0 {
        emit(
            Level::Warn,
            &format!("{target}: hız sınırı nedeniyle {suppressed} mesaj atlandı"),
        );
    }
    let level = match level {
        ::log::Level::Error => Level::Error,
        _ => Level::Warn,
    };
    emit(level, &format!("{target}: {message}"));
}

pub(super) struct Bridge {
    limiter: Mutex<Limiter>,
}

impl Bridge {
    pub(super) const fn new() -> Self {
        Self {
            limiter: Mutex::new(Limiter::new()),
        }
    }
}

impl ::log::Log for Bridge {
    fn enabled(&self, metadata: &::log::Metadata<'_>) -> bool {
        metadata.level() <= ::log::Level::Warn
    }

    fn log(&self, record: &::log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let mut limiter = self.limiter.lock().unwrap_or_else(PoisonError::into_inner);
        forward(
            &mut limiter,
            Instant::now(),
            record.target(),
            record.level(),
            record.args(),
            super::submit,
        );
    }

    fn flush(&self) {
        super::flush();
    }
}

static BRIDGE: Bridge = Bridge::new();

pub fn install_bridge() {
    match ::log::set_logger(&BRIDGE) {
        Ok(()) => ::log::set_max_level(::log::LevelFilter::Warn),
        Err(e) => super::warn(&format!("log köprüsü kurulamadı: {e}")),
    }
}
