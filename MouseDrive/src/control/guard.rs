pub const CHECK_INTERVAL_MS: f64 = 100.0;
pub const MISSES_TO_RELEASE: u8 = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ButtonGuard {
    confirmed: bool,
    misses: u8,
}

impl ButtonGuard {
    pub fn check(&mut self, raw_down: bool, async_down: bool) -> bool {
        if !raw_down {
            *self = Self::default();
            return false;
        }
        if async_down {
            self.confirmed = true;
            self.misses = 0;
            return false;
        }
        if !self.confirmed {
            return false;
        }
        self.misses = self.misses.saturating_add(1);
        if self.misses >= MISSES_TO_RELEASE {
            *self = Self::default();
            return true;
        }
        false
    }
}

pub fn async_pressed(own_vk_down: bool, other_vk_down: bool, swapped: bool) -> bool {
    own_vk_down || (swapped && other_vk_down)
}
