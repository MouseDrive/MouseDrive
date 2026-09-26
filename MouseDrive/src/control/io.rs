use crate::input::{self, DeviceFilter, InputCounters, MouseButton};
use crate::output::OutputBackend;
use crate::setup::SetupReport;
use crate::sound::SoundPlayer;
use crate::status::{AppStatus, Cue};
use crate::{keys, overlay};

#[cfg(target_os = "linux")]
use crate::uinput as backend;
#[cfg(windows)]
use crate::vjoy as backend;

pub(crate) trait ControlIo {
    fn take_counts(&mut self) -> i64;
    fn buttons(&self) -> (bool, bool);
    fn take_middle_click(&mut self) -> bool;
    fn force_release(&mut self, button: MouseButton) -> bool;
    fn clear_input(&mut self);
    fn input_counters(&self) -> InputCounters;
    fn device_filter(&self) -> DeviceFilter;
    fn registration_ok(&self) -> bool;
    fn request_register(&mut self);
    fn apply_input_settings(&mut self, input_sink: bool, device_path: &str);

    fn key_down(&self, vk: i32) -> bool;
    fn buttons_swapped(&self) -> bool;
    fn app_foreground(&self) -> bool;

    fn device_events(&self) -> (u64, u64);
    fn connect(
        &mut self,
        device_id: u32,
        buttons_required: i32,
    ) -> (SetupReport, Option<Box<dyn OutputBackend>>);

    fn play(&mut self, cue: Cue, volume_pct: u8);
    fn overlay_status(&mut self, status: AppStatus);
    fn overlay_config(&mut self, enabled: bool, corner: i32);
}

pub(crate) struct SystemIo {
    sound: Option<SoundPlayer>,
}

impl SystemIo {
    pub(crate) fn new() -> Self {
        let sound = SoundPlayer::start()
            .inspect_err(|e| crate::log::line(&format!("ses thread'i başlatılamadı: {e}")))
            .ok();
        Self { sound }
    }
}

impl ControlIo for SystemIo {
    fn take_counts(&mut self) -> i64 {
        input::take_counts()
    }

    fn buttons(&self) -> (bool, bool) {
        input::buttons()
    }

    fn take_middle_click(&mut self) -> bool {
        input::take_middle_click()
    }

    fn force_release(&mut self, button: MouseButton) -> bool {
        input::force_release(button)
    }

    fn clear_input(&mut self) {
        input::clear();
    }

    fn input_counters(&self) -> InputCounters {
        input::counters()
    }

    fn device_filter(&self) -> DeviceFilter {
        input::device_filter()
    }

    fn registration_ok(&self) -> bool {
        input::registration_ok()
    }

    fn request_register(&mut self) {
        input::request_register();
    }

    fn apply_input_settings(&mut self, input_sink: bool, device_path: &str) {
        input::set_input_sink(input_sink);
        input::set_device_filter(device_path);
    }

    fn key_down(&self, vk: i32) -> bool {
        keys::is_key_down(vk)
    }

    fn buttons_swapped(&self) -> bool {
        keys::buttons_swapped()
    }

    fn app_foreground(&self) -> bool {
        keys::app_is_foreground()
    }

    fn device_events(&self) -> (u64, u64) {
        backend::device_events()
    }

    fn connect(
        &mut self,
        device_id: u32,
        buttons_required: i32,
    ) -> (SetupReport, Option<Box<dyn OutputBackend>>) {
        let (report, device) = backend::probe(device_id, buttons_required);
        (
            report,
            device.map(|d| Box::new(d) as Box<dyn OutputBackend>),
        )
    }

    fn play(&mut self, cue: Cue, volume_pct: u8) {
        if let Some(sound) = &self.sound {
            sound.play(cue, volume_pct);
        }
    }

    fn overlay_status(&mut self, status: AppStatus) {
        overlay::set_status(status);
    }

    fn overlay_config(&mut self, enabled: bool, corner: i32) {
        overlay::configure(enabled, corner);
    }
}
