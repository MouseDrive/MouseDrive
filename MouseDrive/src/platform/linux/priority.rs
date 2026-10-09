use std::ffi::c_int;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, TryLockError};

use super::rtkit::RtKit;
use crate::priority::{
    PrioritySlot, RAISED_NICE, RtTime, ThreadPriority, allowed_nice, describe, log_outcome,
    realtime_priority, rttime_limits,
};

const GUARDED_THREADS: usize = 8;

static GUARDED: [Guarded; GUARDED_THREADS] = [const { Guarded::new() }; GUARDED_THREADS];
static RTTIME: Mutex<()> = Mutex::new(());
static INHERITED_RTTIME: OnceLock<libc::rlim_t> = OnceLock::new();

pub(super) trait Scheduler {
    fn realtime_limit(&self) -> u64;
    fn nice_limit(&self) -> u64;
    fn prepare_realtime(&mut self) -> io::Result<()>;
    fn set_realtime(&mut self, priority: i32) -> Result<(), i32>;
    fn set_nice(&mut self, nice: i32) -> Result<(), i32>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KitLimits {
    pub(super) max_realtime: i32,
    pub(super) min_nice: i32,
    pub(super) rttime_max_us: i64,
}

pub(super) trait RealtimeKit {
    fn limits(&self) -> Result<KitLimits, String>;
    fn make_realtime(&self, tid: u64, priority: u32) -> Result<(), String>;
    fn make_high_priority(&self, tid: u64, nice: i32) -> Result<(), String>;
}

pub fn raise_current_thread_priority(label: &'static str, slot: &'static PrioritySlot) {
    let tid = current_tid();
    let result = raise(&mut System { tid, slot });
    slot.set(result);
    let ThreadPriority::Failed(errno) = result else {
        log_outcome(label, result);
        return;
    };
    crate::log::info(&format!(
        "{label} thread'i önceliği doğrudan yükseltilemedi ({}); rtkit deneniyor",
        io::Error::from_raw_os_error(errno)
    ));
    request_rtkit(label, tid, slot, result);
}

pub(super) fn raise(sched: &mut impl Scheduler) -> ThreadPriority {
    if let Some(priority) = realtime_priority(sched.realtime_limit()) {
        match try_realtime(sched, priority) {
            Ok(()) => return ThreadPriority::Realtime(priority),
            Err(e) => crate::log::warn(&format!(
                "SCHED_RR {priority} ayarlanamadı, nice denenecek: {e}"
            )),
        }
    }
    lower_nice(sched)
}

fn try_realtime(sched: &mut impl Scheduler, priority: i32) -> io::Result<()> {
    sched.prepare_realtime()?;
    sched
        .set_realtime(priority)
        .map_err(io::Error::from_raw_os_error)
}

fn lower_nice(sched: &mut impl Scheduler) -> ThreadPriority {
    let errno = match sched.set_nice(RAISED_NICE) {
        Ok(()) => return ThreadPriority::Nice(RAISED_NICE),
        Err(errno) => errno,
    };
    match allowed_nice(sched.nice_limit()) {
        Some(nice) if nice > RAISED_NICE && sched.set_nice(nice).is_ok() => {
            ThreadPriority::Nice(nice)
        }
        _ => ThreadPriority::Failed(errno),
    }
}

pub(super) fn raise_with_kit(
    kit: &impl RealtimeKit,
    tid: u64,
    prepare: impl FnOnce(u64) -> io::Result<()>,
) -> Result<ThreadPriority, String> {
    let limits = kit.limits()?;
    let mut errors = Vec::new();
    let ceiling = u64::try_from(limits.rttime_max_us)
        .ok()
        .filter(|us| *us > 0);
    let realtime = u64::try_from(limits.max_realtime)
        .ok()
        .and_then(realtime_priority);
    if let (Some(priority), Some(ceiling)) = (realtime, ceiling) {
        let made = prepare(ceiling)
            .map_err(|e| format!("RLIMIT_RTTIME ayarlanamadı: {e}"))
            .and_then(|()| kit.make_realtime(tid, priority.unsigned_abs()));
        match made {
            Ok(()) => return Ok(ThreadPriority::Realtime(priority)),
            Err(e) => errors.push(e),
        }
    }
    if limits.min_nice < 0 {
        let nice = limits.min_nice.max(RAISED_NICE);
        match kit.make_high_priority(tid, nice) {
            Ok(()) => return Ok(ThreadPriority::Nice(nice)),
            Err(e) => errors.push(e),
        }
    }
    if errors.is_empty() {
        errors.push("rtkit öncelik yükseltmeye izin vermiyor".into());
    }
    Err(errors.join("; "))
}

fn request_rtkit(
    label: &'static str,
    tid: libc::pid_t,
    slot: &'static PrioritySlot,
    failed: ThreadPriority,
) {
    let Ok(thread) = u64::try_from(tid) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("rtkit".into())
        .spawn(move || {
            let outcome = RtKit::connect().and_then(|kit| {
                raise_with_kit(&kit, thread, |ceiling| {
                    prepare_for_rtkit(tid, slot, ceiling)
                })
            });
            match outcome {
                Ok(priority) if slot.replace(failed, priority) => crate::log::info(&format!(
                    "{label} thread'i önceliği rtkit ile: {}",
                    describe(priority)
                )),
                Ok(priority) => crate::log::warn(&format!(
                    "{label} thread'i rtkit ile yükseltildi ({}) ama durumu bu arada değişti: {}",
                    describe(priority),
                    describe(slot.get())
                )),
                Err(e) => crate::log::warn(&format!(
                    "{label} thread'i önceliği yükseltilemedi; rtkit: {e}"
                )),
            }
        });
    if let Err(e) = spawned {
        crate::log::warn(&format!(
            "{label} thread'i önceliği yükseltilemedi; rtkit isteği başlatılamadı: {e}"
        ));
    }
}

fn prepare_for_rtkit(
    tid: libc::pid_t,
    slot: &'static PrioritySlot,
    ceiling: u64,
) -> io::Result<()> {
    install_cpu_guard()?;
    prepare_rttime(Some(ceiling))?;
    guard_thread(tid, slot);
    Ok(())
}

struct System {
    tid: libc::pid_t,
    slot: &'static PrioritySlot,
}

impl Scheduler for System {
    fn realtime_limit(&self) -> u64 {
        soft_limit(libc::RLIMIT_RTPRIO)
    }

    fn nice_limit(&self) -> u64 {
        soft_limit(libc::RLIMIT_NICE)
    }

    fn prepare_realtime(&mut self) -> io::Result<()> {
        install_cpu_guard()?;
        prepare_rttime(None)
    }

    fn set_realtime(&mut self, priority: i32) -> Result<(), i32> {
        guard_thread(self.tid, self.slot);
        let param = sched_param(priority);
        let policy = libc::SCHED_RR | libc::SCHED_RESET_ON_FORK;
        // SAFETY: pid 0 yalnız çağıran thread'i seçer; param yerel ve geçerli.
        let rc = unsafe { libc::sched_setscheduler(0, policy, &param) };
        if rc == 0 { Ok(()) } else { Err(last_errno()) }
    }

    fn set_nice(&mut self, nice: i32) -> Result<(), i32> {
        let Ok(tid) = libc::id_t::try_from(self.tid) else {
            return Err(libc::ESRCH);
        };
        // SAFETY: Linux'ta nice thread başınadır; yalnız bu thread değişir.
        let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, tid, nice) };
        if rc == 0 { Ok(()) } else { Err(last_errno()) }
    }
}

fn current_tid() -> libc::pid_t {
    // SAFETY: gettid yalnız çağıran thread'in kimliğini döner.
    unsafe { libc::gettid() }
}

fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

fn sched_param(priority: c_int) -> libc::sched_param {
    // SAFETY: sched_param düz bir C yapısıdır; sıfırlanmış hali geçerlidir.
    let mut param: libc::sched_param = unsafe { std::mem::zeroed() };
    param.sched_priority = priority;
    param
}

fn limit_value(raw: libc::rlim_t) -> Option<u64> {
    (raw != libc::RLIM_INFINITY).then_some(raw)
}

fn raw_limit(value: Option<u64>) -> libc::rlim_t {
    value.unwrap_or(libc::RLIM_INFINITY)
}

fn read_limit(resource: libc::__rlimit_resource_t) -> io::Result<RtTime> {
    let mut raw = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: raw yerel ve yazılabilir bir rlimit yapısı.
    if unsafe { libc::getrlimit(resource, &mut raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(RtTime {
        soft: limit_value(raw.rlim_cur),
        hard: limit_value(raw.rlim_max),
    })
}

fn soft_limit(resource: libc::__rlimit_resource_t) -> u64 {
    read_limit(resource).map_or(0, |limit| limit.soft.unwrap_or(u64::MAX))
}

fn prepare_rttime(ceiling: Option<u64>) -> io::Result<()> {
    let _serial = RTTIME.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, wanted)) = apply_rttime(ceiling)? {
        crate::log::info(&format!(
            "RLIMIT_RTTIME ayarlandı: yumuşak {}, sert {}",
            describe_limit(wanted.soft),
            describe_limit(wanted.hard)
        ));
    }
    Ok(())
}

pub fn keep_cpu_budget() {
    static NOTED: AtomicBool = AtomicBool::new(false);
    let _serial = match RTTIME.try_lock() {
        Ok(serial) => serial,
        Err(TryLockError::Poisoned(serial)) => serial.into_inner(),
        Err(TryLockError::WouldBlock) => return,
    };
    let note = match apply_rttime(None) {
        Ok(None) => return,
        Ok(Some((before, after))) => format!(
            "RLIMIT_RTTIME başka bir bileşen tarafından değiştirilmiş (yumuşak {}, sert {}); yumuşak sınır yeniden {} yapıldı",
            describe_limit(before.soft),
            describe_limit(before.hard),
            describe_limit(after.soft)
        ),
        Err(e) => format!("RLIMIT_RTTIME yeniden ayarlanamadı: {e}"),
    };
    if !NOTED.swap(true, Ordering::Relaxed) {
        crate::log::warn(&note);
    }
}

fn apply_rttime(ceiling: Option<u64>) -> io::Result<Option<(RtTime, RtTime)>> {
    let current = read_limit(libc::RLIMIT_RTTIME)?;
    INHERITED_RTTIME.get_or_init(|| raw_limit(current.soft));
    let Some(wanted) = rttime_limits(current, ceiling) else {
        return Err(io::Error::other(format!(
            "RLIMIT_RTTIME gerçek zamanlı çalışma için çok düşük (yumuşak {}, sert {})",
            describe_limit(current.soft),
            describe_limit(current.hard)
        )));
    };
    if wanted == current {
        return Ok(None);
    }
    let raw = libc::rlimit {
        rlim_cur: raw_limit(wanted.soft),
        rlim_max: raw_limit(wanted.hard),
    };
    // SAFETY: raw yerel ve geçerli bir rlimit yapısı; yalnız bu süreç etkilenir.
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &raw) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Some((current, wanted)))
}

fn describe_limit(limit: Option<u64>) -> String {
    limit.map_or_else(|| "sınırsız".to_owned(), |us| format!("{us} µs"))
}

pub(super) fn restore_inherited_rttime() {
    let Some(&soft) = INHERITED_RTTIME.get() else {
        return;
    };
    let mut raw = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: raw yerel ve yazılabilir; getrlimit ve setrlimit doğrudan sistem çağrısıdır,
    // bellek ayırmaz ve fork ile exec arasında da güvenlidir. Sert sınır değişmez.
    unsafe {
        if libc::getrlimit(libc::RLIMIT_RTTIME, &mut raw) == 0 {
            raw.rlim_cur = soft.min(raw.rlim_max);
            libc::setrlimit(libc::RLIMIT_RTTIME, &raw);
        }
    }
}

struct Guarded {
    tid: AtomicI32,
    slot: AtomicPtr<PrioritySlot>,
}

impl Guarded {
    const fn new() -> Self {
        Self {
            tid: AtomicI32::new(0),
            slot: AtomicPtr::new(std::ptr::null_mut()),
        }
    }
}

fn guard_thread(tid: libc::pid_t, slot: &'static PrioritySlot) {
    let wanted = std::ptr::from_ref(slot).cast_mut();
    let entry = GUARDED.iter().find(|entry| {
        match entry.slot.compare_exchange(
            std::ptr::null_mut(),
            wanted,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => true,
            Err(current) => current == wanted,
        }
    });
    match entry {
        Some(entry) => entry.tid.store(tid, Ordering::Release),
        None => {
            crate::log::warn("gerçek zamanlı thread tablosu dolu; düşürme durumu gösterilmeyecek")
        }
    }
}

fn install_cpu_guard() -> io::Result<()> {
    static INSTALLED: OnceLock<Result<(), i32>> = OnceLock::new();
    (*INSTALLED.get_or_init(install_cpu_handler)).map_err(io::Error::from_raw_os_error)
}

fn install_cpu_handler() -> Result<(), i32> {
    // SAFETY: sigaction düz bir C yapısıdır; sıfırlanmış hali geçerli bir başlangıçtır.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_cpu_limit as extern "C" fn(c_int) as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: action yerel ve yazılabilir; işleyici yalnız sinyal-güvenli çağrılar yapar.
    let rc = unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut())
    };
    if rc == 0 { Ok(()) } else { Err(last_errno()) }
}

extern "C" fn on_cpu_limit(_signal: c_int) {
    // SAFETY: errno konumu çağıran thread'e aittir; işleyici çıkmadan eski değer geri yazılır.
    let errno = unsafe { libc::__errno_location() };
    // SAFETY: errno işaretçisi thread ömrü boyunca geçerlidir.
    let saved = unsafe { *errno };
    let param = sched_param(0);
    let policy = libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK;
    // SAFETY: pid 0 yalnız sınırı aşan (çağıran) thread'i normal zamanlamaya döndürür;
    // SCHED_RESET_ON_FORK yetkisiz kaldırılamadığı için korunur.
    if unsafe { libc::sched_setscheduler(0, policy, &param) } == 0 {
        mark_demoted(current_tid());
    }
    // SAFETY: aynı thread'in errno konumuna önceki değer yazılır.
    unsafe { *errno = saved };
}

fn mark_demoted(tid: libc::pid_t) {
    for entry in &GUARDED {
        if entry.tid.load(Ordering::Acquire) != tid {
            continue;
        }
        let slot = entry.slot.load(Ordering::Acquire);
        // SAFETY: tabloya yalnız &'static PrioritySlot işaretçileri yazılır.
        if let Some(slot) = unsafe { slot.as_ref() } {
            slot.set(ThreadPriority::Demoted);
        }
    }
}
