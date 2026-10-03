use std::ffi::c_void;

use windows::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, HIGH_PRIORITY_CLASS,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
    ProcessPowerThrottling, SetPriorityClass, SetProcessInformation, SetThreadPriority,
    THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MessageBoxW,
};
use windows::core::PCWSTR;

const TIMERR_NOERROR: u32 = 0;

pub struct TimerResolution {
    active: bool,
}

impl TimerResolution {
    pub fn request_1ms() -> Self {
        // SAFETY: basit çağrı; eşleşen timeEndPeriod Drop'ta yapılır.
        let active = unsafe { timeBeginPeriod(1) } == TIMERR_NOERROR;
        Self { active }
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        if self.active {
            // SAFETY: yalnız başarılı timeBeginPeriod(1) sonrası çağrılır.
            unsafe { timeEndPeriod(1) };
        }
    }
}

fn power_throttling_opt_out() -> PROCESS_POWER_THROTTLING_STATE {
    PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    }
}

pub fn opt_out_power_throttling() -> bool {
    let state = power_throttling_opt_out();
    // SAFETY: yapı çağrı boyunca yaşar ve boyutu doğru verilir.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&state as *const PROCESS_POWER_THROTTLING_STATE).cast::<c_void>(),
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
    }
    .is_ok()
}

pub fn raise_process_priority() -> bool {
    // SAFETY: yalnız bu sürecin öncelik sınıfı.
    unsafe { SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) }.is_ok()
}

pub fn raise_current_thread_priority() -> bool {
    // SAFETY: yalnız çağıran thread'in önceliği.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST) }.is_ok()
}

pub fn alert(title: &str, body: &str) {
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let (title, body) = (wide(title), wide(body));
    // SAFETY: iki metin de sıfır sonlu ve çağrı boyunca yaşar; sahip pencere yok.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

pub fn setup_process() -> TimerResolution {
    if !raise_process_priority() {
        crate::log::line("HIGH öncelik sınıfı ayarlanamadı");
    }
    if !opt_out_power_throttling() {
        crate::log::line("güç kısıtlaması opt-out başarısız (eski Windows?)");
    }
    TimerResolution::request_1ms()
}
