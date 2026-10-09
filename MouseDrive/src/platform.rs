#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

use std::io;
use std::process::{Child, Command};

#[cfg(target_os = "linux")]
pub use linux::{
    DisplayEnv, MissingLibs, ProcessGuard, SignalEvent, WindowSystem, alert, child_command,
    display_env, environment_summary, keep_cpu_budget, missing_runtime_libs, monotonic_ns,
    raise_current_thread_priority, running_as_root, setup_process, signal_name, sleep_until_ns,
    tighten_timer_slack, watch_termination_signals,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{Wake, drain, poll, pollfd};
#[cfg(windows)]
pub use win::{
    TimerResolution as ProcessGuard, alert, child_command, environment_summary,
    raise_current_thread_priority, setup_process,
};

pub fn spawn_reaped(command: &mut Command) -> io::Result<u32> {
    let child = command.spawn()?;
    let id = child.id();
    reap_in_background(child);
    Ok(id)
}

pub(crate) fn reap_in_background(mut child: Child) {
    let id = child.id();
    let spawned = std::thread::Builder::new()
        .name("reaper".into())
        .spawn(move || {
            if let Err(e) = child.wait() {
                crate::log::warn(&format!("alt süreç {id} beklenemedi: {e}"));
            }
        });
    if let Err(e) = spawned {
        crate::log::warn(&format!(
            "alt süreç {id} için bekleme thread'i başlatılamadı: {e}"
        ));
    }
}
