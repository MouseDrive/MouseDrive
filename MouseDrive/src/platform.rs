#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
pub use linux::{ProcessGuard, raise_current_thread_priority, setup_process};
#[cfg(target_os = "linux")]
pub(crate) use linux::{Wake, drain, poll, pollfd};
#[cfg(windows)]
pub use win::{TimerResolution as ProcessGuard, raise_current_thread_priority, setup_process};
