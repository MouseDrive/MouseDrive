use std::ffi::{OsStr, c_int};
use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::Command;
use std::time::{Duration, Instant};

const TERMINATION_SIGNALS: [c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalEvent {
    Received(c_int),
    Repeated(c_int),
    GraceExpired(c_int),
}

pub fn signal_name(signal: c_int) -> &'static str {
    match signal {
        libc::SIGTERM => "SIGTERM",
        libc::SIGINT => "SIGINT",
        libc::SIGHUP => "SIGHUP",
        _ => "sinyal",
    }
}

pub fn watch_termination_signals(
    grace: Duration,
    on_event: impl FnMut(SignalEvent) + Send + 'static,
) -> io::Result<()> {
    let set = signal_set(&TERMINATION_SIGNALS);
    set_mask(libc::SIG_BLOCK, &set)?;
    let spawned = std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || wait_loop(&set, grace, on_event));
    match spawned {
        Ok(_) => Ok(()),
        Err(e) => {
            if let Err(undo) = set_mask(libc::SIG_UNBLOCK, &set) {
                return Err(io::Error::other(format!(
                    "{e}; maske geri alınamadı: {undo}"
                )));
            }
            Err(e)
        }
    }
}

pub fn child_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    let empty = signal_set(&[]);
    // SAFETY: kapanış fork ile exec arasında yalnız doğrudan sistem çağrısı olan
    // getrlimit/setrlimit ve async-signal-safe sigprocmask'i çağırır; küme önceden
    // hazırlanmıştır ve bellek ayrılmaz.
    unsafe {
        command.pre_exec(move || {
            super::priority::restore_inherited_rttime();
            if libc::sigprocmask(libc::SIG_SETMASK, &empty, std::ptr::null_mut()) == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        });
    }
    command
}

fn signal_set(signals: &[c_int]) -> libc::sigset_t {
    // SAFETY: sigset_t düz bir C yapısıdır; sigemptyset onu hemen başlatır.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: set yerel ve yazılabilir; numaralar geçerli sinyal sabitleri.
    unsafe {
        libc::sigemptyset(&mut set);
        for &signal in signals {
            libc::sigaddset(&mut set, signal);
        }
    }
    set
}

fn set_mask(how: c_int, set: &libc::sigset_t) -> io::Result<()> {
    // SAFETY: set başlatılmış bir küme; yalnız çağıran thread'in maskesi değişir.
    let rc = unsafe { libc::pthread_sigmask(how, set, std::ptr::null_mut()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

fn wait_loop(set: &libc::sigset_t, grace: Duration, mut on_event: impl FnMut(SignalEvent)) {
    let first = match wait_signal(set) {
        Ok(signal) => signal,
        Err(e) => {
            crate::log::error(&format!("sinyal beklenemiyor: {e}"));
            return;
        }
    };
    on_event(SignalEvent::Received(first));
    let next = match wait_signal_for(set, grace) {
        Ok(Some(signal)) => SignalEvent::Repeated(signal),
        Ok(None) => SignalEvent::GraceExpired(first),
        Err(e) => {
            crate::log::error(&format!("ikinci sinyal beklenemiyor: {e}"));
            SignalEvent::GraceExpired(first)
        }
    };
    on_event(next);
}

fn wait_signal(set: &libc::sigset_t) -> io::Result<c_int> {
    let mut signal: c_int = 0;
    // SAFETY: set başlatılmış bir küme; signal yerel bir değişken.
    let rc = unsafe { libc::sigwait(set, &mut signal) };
    if rc == 0 {
        Ok(signal)
    } else {
        Err(io::Error::from_raw_os_error(rc))
    }
}

fn wait_signal_for(set: &libc::sigset_t, timeout: Duration) -> io::Result<Option<c_int>> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let wait = libc::timespec {
            tv_sec: libc::time_t::try_from(left.as_secs()).unwrap_or(libc::time_t::MAX),
            tv_nsec: left.subsec_nanos() as libc::c_long,
        };
        // SAFETY: set başlatılmış bir küme; bilgi yapısı istenmez; wait yerel.
        let signal = unsafe { libc::sigtimedwait(set, std::ptr::null_mut(), &wait) };
        if signal > 0 {
            return Ok(Some(signal));
        }
        let err = io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EAGAIN) => return Ok(None),
            Some(libc::EINTR) => continue,
            _ => return Err(err),
        }
    }
}
