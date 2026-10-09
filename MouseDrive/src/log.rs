#![deny(unsafe_code)]

mod bridge;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant, SystemTime};

pub use self::bridge::install_bridge;
pub use self::writer::timestamp;
use self::writer::{Record, Writer};

const MAX_LOG_BYTES: u64 = 1024 * 1024;
const QUEUE_CAPACITY: usize = 1024;
const BATCH_LIMIT: usize = 256;
const IDLE_SUMMARY: Duration = Duration::from_secs(10);
const FLUSH_TIMEOUT: Duration = Duration::from_millis(500);
const FLUSH_RETRY: Duration = Duration::from_millis(5);
pub(crate) const LOG_FILE: &str = "mousedrive.log";
const OLD_LOG_FILE: &str = "mousedrive.log.old";
const WRITER_THREAD: &str = "log";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn label(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }
}

enum Message {
    Record(Record),
    Flush(SyncSender<()>),
}

static QUEUE: OnceLock<Option<SyncSender<Message>>> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub fn line(text: &str) {
    info(text);
}

pub fn info(text: &str) {
    submit(Level::Info, text);
}

pub fn warn(text: &str) {
    submit(Level::Warn, text);
}

pub fn error(text: &str) {
    submit(Level::Error, text);
}

pub fn flush() {
    if std::thread::current().name() == Some(WRITER_THREAD) {
        return;
    }
    if let Some(Some(queue)) = QUEUE.get() {
        flush_queue(queue, FLUSH_TIMEOUT);
    }
}

pub fn log_dir() -> Option<PathBuf> {
    crate::config::portable_dir().or_else(platform_log_dir)
}

pub fn log_path() -> Option<PathBuf> {
    Some(log_dir()?.join(LOG_FILE))
}

pub fn move_legacy_log() {
    let (Some(from), Some(to)) = (crate::config::config_dir(), log_dir()) else {
        return;
    };
    if from == to {
        return;
    }
    let mut moved = Vec::new();
    for (name, result) in move_logs(&from, &to) {
        match result {
            Ok(()) => moved.push(name),
            Err(e) => warn(&format!(
                "eski günlük taşınamadı ({}): {e}",
                from.join(name).display()
            )),
        }
    }
    if !moved.is_empty() {
        info(&format!(
            "eski günlük {} klasöründen buraya taşındı: {}",
            from.display(),
            moved.join(", ")
        ));
    }
}

fn move_logs(from: &Path, to: &Path) -> Vec<(&'static str, std::io::Result<()>)> {
    [LOG_FILE, OLD_LOG_FILE]
        .into_iter()
        .filter(|name| from.join(name).is_file() && !to.join(name).exists())
        .map(|name| (name, std::fs::rename(from.join(name), to.join(name))))
        .collect()
}

#[cfg(target_os = "linux")]
fn platform_log_dir() -> Option<PathBuf> {
    let base = crate::config::xdg_base_dir(
        std::env::var_os("XDG_STATE_HOME"),
        std::env::var_os("HOME"),
        ".local/state",
    )?;
    let dir = base.join("mousedrive");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        report_stderr(&format!("günlük klasörü oluşturulamadı: {e}"));
    }
    Some(dir)
}

#[cfg(windows)]
fn platform_log_dir() -> Option<PathBuf> {
    crate::config::config_dir()
}

pub(crate) fn thread_label() -> String {
    let thread = std::thread::current();
    thread
        .name()
        .map_or_else(|| format!("{:?}", thread.id()), str::to_owned)
}

fn submit(level: Level, text: &str) {
    let record = Record::now(level, text);
    if record.thread == WRITER_THREAD {
        return write_direct(record);
    }
    match queue() {
        Some(queue) => submit_to(queue, &DROPPED, record),
        None => write_direct(record),
    }
}

fn submit_to(queue: &SyncSender<Message>, dropped: &AtomicU64, record: Record) {
    match queue.try_send(Message::Record(record)) {
        Ok(()) | Err(TrySendError::Disconnected(Message::Flush(_))) => {}
        Err(TrySendError::Full(_)) => {
            dropped.fetch_add(1, Ordering::Relaxed);
        }
        Err(TrySendError::Disconnected(Message::Record(record))) => write_direct(record),
    }
}

fn write_direct(record: Record) {
    let mut writer = Writer::new(log_path(), MAX_LOG_BYTES);
    writer.push(record);
    writer.write_out();
}

fn queue() -> Option<&'static SyncSender<Message>> {
    QUEUE.get_or_init(start_writer).as_ref()
}

fn start_writer() -> Option<SyncSender<Message>> {
    let (queue, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
    let spawned = std::thread::Builder::new()
        .name(WRITER_THREAD.into())
        .spawn(move || {
            let writer = Writer::new(log_path(), MAX_LOG_BYTES);
            run_writer(&receiver, writer, &DROPPED, IDLE_SUMMARY);
        });
    match spawned {
        Ok(_) => Some(queue),
        Err(e) => {
            report_stderr(&format!("günlük thread'i başlatılamadı: {e}"));
            None
        }
    }
}

fn run_writer(
    receiver: &Receiver<Message>,
    mut writer: Writer,
    dropped: &AtomicU64,
    idle: Duration,
) {
    loop {
        let next = if writer.has_repeats() {
            receiver.recv_timeout(idle)
        } else {
            receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let first = match next {
            Ok(message) => message,
            Err(RecvTimeoutError::Timeout) => {
                writer.finish_repeats();
                writer.write_out();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut acks = Vec::new();
        for message in std::iter::once(first).chain(receiver.try_iter().take(BATCH_LIMIT)) {
            match message {
                Message::Record(record) => writer.push(record),
                Message::Flush(ack) => acks.push(ack),
            }
        }
        writer.note_dropped(dropped.swap(0, Ordering::Relaxed), SystemTime::now());
        if !acks.is_empty() {
            writer.finish_repeats();
        }
        writer.write_out();
        for ack in acks {
            let _ = ack.try_send(());
        }
    }
    writer.finish_repeats();
    writer.write_out();
}

fn flush_queue(queue: &SyncSender<Message>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    let (ack, done) = mpsc::sync_channel(1);
    let mut message = Message::Flush(ack);
    loop {
        match queue.try_send(message) {
            Ok(()) => break,
            Err(TrySendError::Full(back)) if Instant::now() < deadline => {
                message = back;
                std::thread::sleep(FLUSH_RETRY);
            }
            Err(_) => return false,
        }
    }
    done.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .is_ok()
}

fn report_stderr(text: &str) {
    use std::io::Write as _;
    let _ = writeln!(std::io::stderr(), "{text}");
}
