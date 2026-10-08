#![deny(unsafe_code)]

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Level;

const REPEAT_REPORT_INTERVAL: Duration = Duration::from_secs(60);
const SECS_PER_DAY: u64 = 86_400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Record {
    pub(super) at: SystemTime,
    pub(super) level: Level,
    pub(super) thread: String,
    pub(super) text: String,
}

impl Record {
    pub(super) fn now(level: Level, text: &str) -> Self {
        Self {
            at: SystemTime::now(),
            level,
            thread: super::thread_label(),
            text: text.to_owned(),
        }
    }

    fn same_message(&self, other: &Record) -> bool {
        self.level == other.level && self.thread == other.thread && self.text == other.text
    }
}

struct Repeats {
    count: u32,
    since: SystemTime,
    last_at: SystemTime,
}

pub(super) struct Writer {
    path: Option<PathBuf>,
    limit: u64,
    file: Option<File>,
    size: u64,
    pending: String,
    previous: Option<Record>,
    repeats: Option<Repeats>,
    failure_reported: bool,
}

impl Writer {
    pub(super) fn new(path: Option<PathBuf>, limit: u64) -> Self {
        Self {
            path,
            limit,
            file: None,
            size: 0,
            pending: String::new(),
            previous: None,
            repeats: None,
            failure_reported: false,
        }
    }

    pub(super) fn push(&mut self, record: Record) {
        if self
            .previous
            .as_ref()
            .is_some_and(|p| p.same_message(&record))
        {
            self.count_repeat(record.at);
            return;
        }
        self.finish_repeats();
        format_record(&mut self.pending, &record);
        self.previous = Some(record);
    }

    fn count_repeat(&mut self, at: SystemTime) {
        let repeats = self.repeats.get_or_insert(Repeats {
            count: 0,
            since: at,
            last_at: at,
        });
        repeats.count = repeats.count.saturating_add(1);
        repeats.last_at = at;
        let running = at.duration_since(repeats.since).unwrap_or_default();
        if running >= REPEAT_REPORT_INTERVAL {
            self.finish_repeats();
        }
    }

    pub(super) fn has_repeats(&self) -> bool {
        self.repeats.is_some()
    }

    pub(super) fn finish_repeats(&mut self) {
        let (Some(repeats), Some(previous)) = (self.repeats.take(), &self.previous) else {
            return;
        };
        let summary = Record {
            at: repeats.last_at,
            level: previous.level,
            thread: previous.thread.clone(),
            text: format!("{} (×{} tekrar)", previous.text, repeats.count),
        };
        format_record(&mut self.pending, &summary);
    }

    pub(super) fn note_dropped(&mut self, dropped: u64, at: SystemTime) {
        if dropped == 0 {
            return;
        }
        self.finish_repeats();
        let record = Record {
            at,
            level: Level::Warn,
            thread: super::WRITER_THREAD.to_owned(),
            text: format!("günlük kuyruğu doldu; {dropped} satır atlandı"),
        };
        format_record(&mut self.pending, &record);
        self.previous = Some(record);
    }

    pub(super) fn write_out(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.pending);
        if let Err(e) = self.append(text.as_bytes()) {
            self.file = None;
            self.report_failure(&e.to_string());
        }
    }

    fn append(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.open()?;
        if self.size > self.limit {
            self.rotate();
        }
        let file = self.open()?;
        file.write_all(bytes)?;
        self.size = self.size.saturating_add(bytes.len() as u64);
        Ok(())
    }

    fn open(&mut self) -> std::io::Result<&mut File> {
        if self.file.is_none() {
            let Some(path) = &self.path else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "günlük klasörü yok",
                ));
            };
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            self.size = file.metadata().map_or(0, |m| m.len());
            self.file = Some(file);
        }
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("günlük dosyası açık değil"))
    }

    fn rotate(&mut self) {
        let Some(path) = &self.path else { return };
        self.file = None;
        self.size = 0;
        if let Err(e) = std::fs::rename(path, path.with_extension("log.old")) {
            let text = format!("günlük döndürülemedi: {e}");
            self.report_failure(&text);
        }
    }

    fn report_failure(&mut self, text: &str) {
        if std::mem::replace(&mut self.failure_reported, true) {
            return;
        }
        super::report_stderr(&format!("mousedrive günlüğü yazılamadı: {text}"));
    }
}

pub(super) fn format_record(out: &mut String, record: &Record) {
    let mut lines = record.text.lines();
    let first = lines.next().unwrap_or_default();
    let _ = writeln!(
        out,
        "{} {:<5} [{}] {first}",
        timestamp(record.at),
        record.level.label(),
        record.thread
    );
    for line in lines {
        let _ = writeln!(out, "    {line}");
    }
}

pub(crate) fn timestamp(at: SystemTime) -> String {
    let since_epoch = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since_epoch.as_secs();
    let days = i64::try_from(secs / SECS_PER_DAY).unwrap_or(0);
    let of_day = secs % SECS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
        since_epoch.subsec_millis()
    )
}

pub(super) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}
