mod alert;
mod desktop;
mod priority;
mod rtkit;
mod signals;

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

const TIMER_SLACK_NS: libc::c_ulong = 1_000;
const NANOS_PER_SEC: u64 = 1_000_000_000;
const OS_RELEASE_FILES: [&str; 2] = ["/etc/os-release", "/usr/lib/os-release"];

pub use alert::alert;
pub use desktop::{
    DisplayEnv, MissingLibs, WindowSystem, display_env, missing_runtime_libs, running_as_root,
};
pub use priority::{keep_cpu_budget, raise_current_thread_priority};
pub use signals::{SignalEvent, child_command, signal_name, watch_termination_signals};

pub(crate) struct Wake {
    fd: OnceLock<OwnedFd>,
    pending: AtomicU8,
}

impl Wake {
    pub(crate) const fn new() -> Self {
        Self {
            fd: OnceLock::new(),
            pending: AtomicU8::new(0),
        }
    }

    pub(crate) fn fd(&self) -> io::Result<RawFd> {
        if let Some(fd) = self.fd.get() {
            return Ok(fd.as_raw_fd());
        }
        // SAFETY: bayraklar geçerli; dönen fd hemen sahiplenilir.
        let raw = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: raw yeni açılmış ve başka sahibi olmayan bir fd.
        let owned = unsafe { OwnedFd::from_raw_fd(raw) };
        Ok(self.fd.get_or_init(|| owned).as_raw_fd())
    }

    pub(crate) fn post(&self, bits: u8) {
        self.pending.fetch_or(bits, Ordering::AcqRel);
        if let Ok(fd) = self.fd() {
            let one = 1u64;
            // SAFETY: fd açık bir eventfd; 8 baytlık yerel değer yazılır.
            let _ = unsafe { libc::write(fd, (&raw const one).cast(), 8) };
        }
    }

    pub(crate) fn take(&self) -> u8 {
        self.pending.swap(0, Ordering::AcqRel)
    }

    pub(crate) fn clear(&self, bits: u8) {
        self.pending.fetch_and(!bits, Ordering::AcqRel);
    }
}

pub(crate) fn pollfd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

pub(crate) fn poll(fds: &mut [libc::pollfd], timeout_ms: i32) -> io::Result<()> {
    // SAFETY: fds geçerli bir pollfd dizisi; uzunluğu dizinin kendisinden.
    let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
    if n >= 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    if err.kind() != io::ErrorKind::Interrupted {
        return Err(err);
    }
    fds.iter_mut().for_each(|p| p.revents = 0);
    Ok(())
}

pub(crate) fn drain(fd: RawFd) {
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: buf yazılabilir ve uzunluğu verilir.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
    }
}

pub struct ProcessGuard;

pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: ts yazılabilir yerel bir timespec; saat kimliği sabit ve geçerli.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0;
    }
    let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
    let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
    secs.saturating_mul(NANOS_PER_SEC).saturating_add(nanos)
}

pub fn sleep_until_ns(deadline_ns: u64) -> io::Result<()> {
    let deadline = libc::timespec {
        tv_sec: libc::time_t::try_from(deadline_ns / NANOS_PER_SEC).unwrap_or(libc::time_t::MAX),
        tv_nsec: libc::c_long::try_from(deadline_ns % NANOS_PER_SEC).unwrap_or(0),
    };
    loop {
        // SAFETY: deadline geçerli bir mutlak CLOCK_MONOTONIC zamanı; kalan süre istenmez.
        let rc = unsafe {
            libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                libc::TIMER_ABSTIME,
                &deadline,
                std::ptr::null_mut(),
            )
        };
        match rc {
            0 => return Ok(()),
            libc::EINTR => {}
            e => return Err(io::Error::from_raw_os_error(e)),
        }
    }
}

pub fn tighten_timer_slack() -> bool {
    // SAFETY: PR_SET_TIMERSLACK yalnız çağıran thread'in gevşekliğini değiştirir.
    unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, TIMER_SLACK_NS) == 0 }
}

pub fn setup_process() -> ProcessGuard {
    ProcessGuard
}

pub fn environment_summary() -> String {
    let distro = OS_RELEASE_FILES
        .iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| os_release_name(&text));
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").ok();
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    describe_environment(
        distro.as_deref(),
        kernel.as_deref().map(str::trim),
        session.as_deref(),
        desktop.as_deref(),
        env_set("WAYLAND_DISPLAY") || env_set("WAYLAND_SOCKET"),
        env_set("DISPLAY"),
    )
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty())
}

fn os_release_name(text: &str) -> Option<String> {
    let field = |key: &str| {
        text.lines()
            .find_map(|line| line.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|value| value.trim().trim_matches(['"', '\'']).trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    field("PRETTY_NAME").or_else(|| field("NAME"))
}

fn describe_environment(
    distro: Option<&str>,
    kernel: Option<&str>,
    session: Option<&str>,
    desktop: Option<&str>,
    wayland: bool,
    x11: bool,
) -> String {
    let known = |v: Option<&str>| v.filter(|s| !s.is_empty()).unwrap_or("?").to_owned();
    let present = |set: bool| if set { "var" } else { "yok" };
    format!(
        "linux {}, dağıtım {}, çekirdek {}, oturum {}, masaüstü {}, wayland {}, x11 {}",
        std::env::consts::ARCH,
        known(distro),
        known(kernel),
        known(session),
        known(desktop),
        present(wayland),
        present(x11)
    )
}
