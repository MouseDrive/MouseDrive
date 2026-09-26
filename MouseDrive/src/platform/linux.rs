use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

const TIMER_SLACK_NS: libc::c_ulong = 1_000;
const RAISED_NICE: libc::c_int = -10;

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

pub fn raise_current_thread_priority() -> bool {
    // SAFETY: gettid yalnız çağıran thread'in kimliğini döner.
    let tid = unsafe { libc::gettid() };
    let Ok(tid) = libc::id_t::try_from(tid) else {
        return false;
    };
    // SAFETY: Linux'ta nice thread başınadır; yalnız çağıran thread değişir.
    unsafe { libc::setpriority(libc::PRIO_PROCESS, tid, RAISED_NICE) == 0 }
}

pub fn setup_process() -> ProcessGuard {
    // SAFETY: yalnız çağıran thread'in (ve ondan açılacakların) ayarı değişir.
    let ok = unsafe { libc::prctl(libc::PR_SET_TIMERSLACK, TIMER_SLACK_NS) } == 0;
    if !ok {
        crate::log::line("zamanlayıcı gevşekliği ayarlanamadı");
    }
    ProcessGuard
}
