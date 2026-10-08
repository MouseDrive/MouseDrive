#![deny(unsafe_code)]

use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub(super) struct FramePacer {
    interval: Option<Duration>,
    last: Option<Instant>,
}

impl FramePacer {
    pub(super) fn new(interval: Duration) -> Self {
        Self {
            interval: Some(interval),
            last: None,
        }
    }

    pub(super) fn pace(&mut self) {
        let Some(interval) = self.interval else {
            return;
        };
        if let Some(last) = self.last
            && let Some(wait) = remaining(interval, last.elapsed())
        {
            std::thread::sleep(wait);
        }
        self.last = Some(Instant::now());
    }
}

fn remaining(interval: Duration, elapsed: Duration) -> Option<Duration> {
    interval.checked_sub(elapsed).filter(|wait| !wait.is_zero())
}
