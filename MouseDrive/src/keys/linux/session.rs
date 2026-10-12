use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, connection::Builder};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::status::SessionState;

const RUN_DIR: &str = "/run/systemd";
const CGROUP_FILE: &str = "/proc/self/cgroup";
const SESSION_ENV: &str = "XDG_SESSION_ID";
const MAX_ID_LEN: usize = 64;
const REFRESH: Duration = Duration::from_secs(1);
const BUS_RETRY: Duration = Duration::from_secs(5);
const METHOD_TIMEOUT: Duration = Duration::from_secs(2);
const LOCK_HOLD: Duration = Duration::from_secs(30);

const LOGIN1: &str = "org.freedesktop.login1";
const LOGIN1_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

static CURRENT: AtomicU8 = AtomicU8::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

pub(super) fn current() -> SessionState {
    decode(CURRENT.load(Ordering::Acquire))
}

pub(super) fn watch() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("session".into())
        .spawn(run);
    if let Err(e) = spawned {
        STARTED.store(false, Ordering::Release);
        crate::log::warn(&format!(
            "oturum izleme thread'i başlatılamadı; kilit ekranı algılanmayacak: {e}"
        ));
    }
}

fn run() {
    let run_dir = Path::new(RUN_DIR);
    let Some(id) = find_session(run_dir) else {
        crate::log::info(
            "logind oturumu bulunamadı; kilit ekranı ve oturum değişimi algılanmayacak",
        );
        return;
    };
    crate::log::info(&format!("logind oturumu izleniyor: {id}"));
    let mut watcher = Watcher::new(run_dir, id, BusLock::default(), LOCK_HOLD);
    loop {
        CURRENT.store(encode(watcher.refresh()), Ordering::Release);
        std::thread::sleep(REFRESH);
    }
}

fn encode(state: SessionState) -> u8 {
    match state {
        SessionState::Active => 0,
        SessionState::Locked => 1,
        SessionState::Inactive => 2,
    }
}

fn decode(raw: u8) -> SessionState {
    match raw {
        1 => SessionState::Locked,
        2 => SessionState::Inactive,
        _ => SessionState::Active,
    }
}

fn find_session(run_dir: &Path) -> Option<String> {
    let env = std::env::var(SESSION_ENV).ok();
    let cgroup = std::fs::read_to_string(CGROUP_FILE).ok();
    // SAFETY: getuid yan etkisizdir ve her zaman başarılı olur.
    let uid = unsafe { libc::getuid() };
    let user = std::fs::read_to_string(run_dir.join("users").join(uid.to_string())).ok();
    let sessions = run_dir.join("sessions");
    pick_session(env.as_deref(), cgroup.as_deref(), user.as_deref(), |id| {
        sessions.join(id).is_file()
    })
}

fn pick_session(
    env: Option<&str>,
    cgroup: Option<&str>,
    user: Option<&str>,
    known: impl Fn(&str) -> bool,
) -> Option<String> {
    let env = env.map(str::trim).filter(|id| valid_id(id));
    [
        env,
        cgroup.and_then(cgroup_session),
        user.and_then(display_session),
    ]
    .into_iter()
    .flatten()
    .find(|id| known(id))
    .map(str::to_owned)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn cgroup_session(text: &str) -> Option<&str> {
    text.lines()
        .filter_map(|line| line.splitn(3, ':').nth(2))
        .flat_map(|path| path.split('/'))
        .filter_map(|part| part.strip_prefix("session-")?.strip_suffix(".scope"))
        .find(|id| valid_id(id))
}

fn display_session(text: &str) -> Option<&str> {
    value(text, "DISPLAY").filter(|id| valid_id(id))
}

fn value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .map(str::trim)
}

fn parse_active(text: &str) -> Option<bool> {
    match value(text, "ACTIVE") {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => value(text, "STATE").map(|state| state == "active"),
    }
}

fn combine(active: Option<bool>, locked: Option<bool>) -> SessionState {
    if active == Some(false) {
        SessionState::Inactive
    } else if locked == Some(true) {
        SessionState::Locked
    } else {
        SessionState::Active
    }
}

trait LockHint {
    fn locked(&mut self, session: &str) -> Option<bool>;
}

struct Watcher<L> {
    file: PathBuf,
    id: String,
    lock: L,
    unreadable: bool,
    hold: Duration,
    locked_at: Option<Instant>,
    holding: bool,
}

impl<L: LockHint> Watcher<L> {
    fn new(run_dir: &Path, id: String, lock: L, hold: Duration) -> Self {
        Self {
            file: run_dir.join("sessions").join(&id),
            id,
            lock,
            unreadable: false,
            hold,
            locked_at: None,
            holding: false,
        }
    }

    fn refresh(&mut self) -> SessionState {
        let active = self.read_active();
        let locked = match active {
            Some(false) => {
                self.locked_at = None;
                self.holding = false;
                None
            }
            _ => self.lock_state(),
        };
        combine(active, locked)
    }

    fn lock_state(&mut self) -> Option<bool> {
        let now = Instant::now();
        if let Some(locked) = self.lock.locked(&self.id) {
            self.locked_at = locked.then_some(now);
            self.holding = false;
            return Some(locked);
        }
        let held = self
            .locked_at
            .is_some_and(|at| now.saturating_duration_since(at) < self.hold);
        self.note_hold(held);
        held.then_some(true)
    }

    fn note_hold(&mut self, held: bool) {
        if held == self.holding {
            return;
        }
        self.holding = held;
        if held {
            crate::log::warn(
                "ekran kilidi durumu okunamadı; son bilinen durum (kilitli) korunuyor",
            );
        } else {
            crate::log::warn("ekran kilidi durumu uzun süredir okunamıyor; oturum etkin sayılıyor");
        }
    }

    fn read_active(&mut self) -> Option<bool> {
        let read = std::fs::read_to_string(&self.file)
            .map_err(|e| e.to_string())
            .and_then(|text| parse_active(&text).ok_or_else(|| "ACTIVE satırı yok".into()));
        match read {
            Ok(active) => {
                if std::mem::take(&mut self.unreadable) {
                    crate::log::info("logind oturum durumu yeniden okunabiliyor");
                }
                Some(active)
            }
            Err(e) => {
                if !std::mem::replace(&mut self.unreadable, true) {
                    crate::log::warn(&format!(
                        "logind oturum durumu okunamadı ({}): {e}; oturum etkin sayılıyor",
                        self.file.display()
                    ));
                }
                None
            }
        }
    }
}

#[derive(Default)]
struct BusLock {
    link: Option<(Connection, OwnedObjectPath)>,
    retry_at: Option<Instant>,
    failing: bool,
}

impl BusLock {
    fn query(&mut self, session: &str) -> Result<bool, String> {
        let link = match self.link.take() {
            Some(link) => link,
            None => connect(session)?,
        };
        let locked = locked_hint(&link)?;
        self.link = Some(link);
        Ok(locked)
    }
}

impl LockHint for BusLock {
    fn locked(&mut self, session: &str) -> Option<bool> {
        if self.retry_at.is_some_and(|at| Instant::now() < at) {
            return None;
        }
        match self.query(session) {
            Ok(locked) => {
                self.retry_at = None;
                if std::mem::take(&mut self.failing) {
                    crate::log::info("ekran kilidi durumu logind'den yeniden okunabiliyor");
                }
                Some(locked)
            }
            Err(e) => {
                self.retry_at = Some(Instant::now() + BUS_RETRY);
                if !std::mem::replace(&mut self.failing, true) {
                    crate::log::warn(&format!(
                        "ekran kilidi durumu logind'den okunamıyor ({e}); yalnız oturum etkinliği izlenecek"
                    ));
                }
                None
            }
        }
    }
}

fn open(builder: Builder<'_>) -> zbus::Result<Connection> {
    builder.method_timeout(METHOD_TIMEOUT).build()
}

fn connect(session: &str) -> Result<(Connection, OwnedObjectPath), String> {
    let conn = Builder::system()
        .and_then(open)
        .map_err(|e| format!("sistem D-Bus'ına bağlanılamadı: {e}"))?;
    let reply = conn
        .call_method(
            Some(LOGIN1),
            LOGIN1_PATH,
            Some(MANAGER),
            "GetSession",
            &(session,),
        )
        .map_err(|e| format!("GetSession({session}) reddedildi: {e}"))?;
    let path: OwnedObjectPath = reply
        .body()
        .deserialize()
        .map_err(|e| format!("GetSession yanıtı çözülemedi: {e}"))?;
    Ok((conn, path))
}

fn locked_hint((conn, path): &(Connection, OwnedObjectPath)) -> Result<bool, String> {
    let reply = conn
        .call_method(
            Some(LOGIN1),
            path.as_str(),
            Some(PROPERTIES),
            "Get",
            &(SESSION, "LockedHint"),
        )
        .map_err(|e| format!("LockedHint okunamadı: {e}"))?;
    let value: OwnedValue = reply
        .body()
        .deserialize()
        .map_err(|e| format!("LockedHint çözülemedi: {e}"))?;
    bool::try_from(value).map_err(|e| format!("LockedHint beklenen türde değil: {e}"))
}
