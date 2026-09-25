#![deny(unsafe_code)]

use std::collections::VecDeque;

use crate::config::{Config, Tuning};
use crate::control::{Shared, StaleConfig};

pub const UNDO_LIMIT: usize = 100;
pub const DISK_DEBOUNCE_MS: f64 = 500.0;
pub const DISK_RETRY_MS: f64 = 5000.0;

pub trait ConfigLink {
    fn epoch(&self) -> u64;
    fn read(&self) -> (u64, Config);
    fn publish(&self, config: &Config, base_epoch: u64) -> Result<u64, StaleConfig>;
}

impl ConfigLink for Shared {
    fn epoch(&self) -> u64 {
        self.config_epoch()
    }

    fn read(&self) -> (u64, Config) {
        self.config()
    }

    fn publish(&self, config: &Config, base_epoch: u64) -> Result<u64, StaleConfig> {
        self.publish_config(config, base_epoch)
    }
}

#[derive(Clone, Debug)]
pub struct History<T> {
    past: VecDeque<T>,
    future: Vec<T>,
    committed: T,
    limit: usize,
}

impl<T: Clone + PartialEq> History<T> {
    pub fn new(initial: T, limit: usize) -> Self {
        Self {
            past: VecDeque::new(),
            future: Vec::new(),
            committed: initial,
            limit: limit.max(1),
        }
    }

    pub fn settle(&mut self, current: &T, interacting: bool) {
        if interacting || *current == self.committed {
            return;
        }
        let before = std::mem::replace(&mut self.committed, current.clone());
        self.past.push_back(before);
        if self.past.len() > self.limit {
            self.past.pop_front();
        }
        self.future.clear();
    }

    pub fn undo(&mut self, current: &T) -> Option<T> {
        self.settle(current, false);
        let prev = self.past.pop_back()?;
        let now = std::mem::replace(&mut self.committed, prev.clone());
        self.future.push(now);
        Some(prev)
    }

    pub fn redo(&mut self, current: &T) -> Option<T> {
        self.settle(current, false);
        let next = self.future.pop()?;
        let now = std::mem::replace(&mut self.committed, next.clone());
        self.past.push_back(now);
        Some(next)
    }

    pub fn can_undo(&self, current: &T) -> bool {
        !self.past.is_empty() || *current != self.committed
    }

    pub fn can_redo(&self, current: &T) -> bool {
        !self.future.is_empty() && *current == self.committed
    }

    pub fn reset(&mut self, current: &T) {
        self.past.clear();
        self.future.clear();
        self.committed = current.clone();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Adopted {
    Profile,
    Other,
}

pub struct Session {
    config: Config,
    epoch: u64,
    published: Config,
    saved_tuning: Tuning,
    history: History<Tuning>,
    disk: Option<Config>,
    disk_pending: Option<(Config, f64)>,
}

impl Session {
    pub fn new(config: Config, epoch: u64, disk: Option<Config>) -> Self {
        Self {
            saved_tuning: config.tuning.clone(),
            history: History::new(config.tuning.clone(), UNDO_LIMIT),
            published: config.clone(),
            config,
            epoch,
            disk,
            disk_pending: None,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn saved_tuning(&self) -> &Tuning {
        &self.saved_tuning
    }

    pub fn is_dirty(&self) -> bool {
        self.config.tuning != self.saved_tuning
    }

    pub fn sync_in(&mut self, link: &impl ConfigLink) -> Option<Adopted> {
        if link.epoch() == self.epoch {
            return None;
        }
        let (epoch, config) = link.read();
        Some(self.adopt(epoch, config))
    }

    pub fn sync_out(&mut self, link: &impl ConfigLink, interacting: bool) -> Option<Adopted> {
        self.history.settle(&self.config.tuning, interacting);
        if self.config == self.published {
            return None;
        }
        match link.publish(&self.config, self.epoch) {
            Ok(epoch) => {
                self.epoch = epoch;
                self.published = self.config.clone();
                None
            }
            Err(stale) => Some(self.adopt(stale.epoch, *stale.config)),
        }
    }

    fn adopt(&mut self, epoch: u64, config: Config) -> Adopted {
        let kind = if config.active_profile != self.published.active_profile {
            Adopted::Profile
        } else {
            Adopted::Other
        };
        if kind == Adopted::Profile {
            self.saved_tuning = config.tuning.clone();
        }
        self.history.reset(&config.tuning);
        self.published = config.clone();
        self.config = config;
        self.epoch = epoch;
        kind
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo(&self.config.tuning)
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo(&self.config.tuning)
    }

    pub fn undo(&mut self) -> bool {
        self.history
            .undo(&self.config.tuning)
            .map(|tuning| self.config.tuning = tuning)
            .is_some()
    }

    pub fn redo(&mut self) -> bool {
        self.history
            .redo(&self.config.tuning)
            .map(|tuning| self.config.tuning = tuning)
            .is_some()
    }

    pub fn revert_to_saved(&mut self) {
        self.config.tuning = self.saved_tuning.clone();
    }

    pub fn mark_saved(&mut self) {
        self.saved_tuning = self.config.tuning.clone();
    }

    pub fn disk_content(&self) -> Config {
        self.config.with_tuning(self.saved_tuning.clone())
    }

    pub fn disk_write_due(&mut self, now_ms: f64, interacting: bool) -> Option<Config> {
        let wanted = self.disk_content();
        if self.disk.as_ref() == Some(&wanted) {
            self.disk_pending = None;
            return None;
        }
        let since = match &self.disk_pending {
            Some((pending, since)) if *pending == wanted => *since,
            _ => {
                self.disk_pending = Some((wanted, now_ms));
                return None;
            }
        };
        (!interacting && now_ms - since >= DISK_DEBOUNCE_MS).then_some(wanted)
    }

    pub fn disk_write_needed(&self) -> Option<Config> {
        let wanted = self.disk_content();
        (self.disk.as_ref() != Some(&wanted)).then_some(wanted)
    }

    pub fn disk_written(&mut self, written: Config) {
        self.disk = Some(written);
        self.disk_pending = None;
    }

    pub fn disk_write_failed(&mut self, now_ms: f64) {
        if let Some((_, since)) = self.disk_pending.as_mut() {
            *since = now_ms + DISK_RETRY_MS;
        }
    }
}
