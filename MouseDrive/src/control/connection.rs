use crate::output::{OutputBackend, OutputFrame, WriteHealth};
use crate::setup::SetupReport;
use crate::status::Connection;

use super::io::ControlIo;

pub(crate) const SETUP_RETRY_MS: f64 = 2000.0;
pub(crate) const LOST_RETRY_MS: f64 = 1000.0;
pub(crate) const LOST_GIVE_UP_MS: f64 = 15_000.0;

pub(crate) struct ConnectionManager {
    backend: Option<Box<dyn OutputBackend>>,
    connection: Connection,
    report: SetupReport,
    report_seq: u64,
    setup_failures: bool,
    backend_desc: Option<String>,
    writes: WriteHealth,
    reconnects: u32,
    connected_once: bool,
    next_retry_ms: f64,
    lost_since_ms: Option<f64>,
    events_seen: Option<(u64, u64)>,
    device_id: u32,
    buttons_required: i32,
}

impl ConnectionManager {
    pub(crate) fn new(device_id: u32, buttons_required: i32) -> Self {
        Self {
            backend: None,
            connection: Connection::SetupRequired,
            report: SetupReport::default(),
            report_seq: 0,
            setup_failures: false,
            backend_desc: None,
            writes: WriteHealth::default(),
            reconnects: 0,
            connected_once: false,
            next_retry_ms: 0.0,
            lost_since_ms: None,
            events_seen: None,
            device_id,
            buttons_required,
        }
    }

    pub(crate) fn connection(&self) -> Connection {
        self.connection
    }

    pub(crate) fn report(&self) -> &SetupReport {
        &self.report
    }

    pub(crate) fn report_seq(&self) -> u64 {
        self.report_seq
    }

    pub(crate) fn setup_failures(&self) -> bool {
        self.connection == Connection::Connected && self.setup_failures
    }

    pub(crate) fn backend_desc(&self) -> Option<String> {
        self.backend_desc.clone()
    }

    pub(crate) fn writes(&self) -> WriteHealth {
        self.writes
    }

    pub(crate) fn reconnects(&self) -> u32 {
        self.reconnects
    }

    pub(crate) fn retry_in_ms(&self, now_ms: f64) -> Option<f64> {
        self.backend
            .is_none()
            .then(|| (self.next_retry_ms - now_ms).max(0.0))
    }

    pub(crate) fn set_target(&mut self, device_id: u32, buttons_required: i32, now_ms: f64) {
        if device_id != self.device_id {
            self.device_id = device_id;
            self.reconnect_now(now_ms);
        }
        if buttons_required != self.buttons_required {
            self.buttons_required = buttons_required;
            let report = SetupReport {
                buttons_required,
                ..self.report.clone()
            };
            self.set_report(report);
        }
    }

    pub(crate) fn reconnect_now(&mut self, now_ms: f64) {
        self.drop_backend();
        self.connection = Connection::SetupRequired;
        self.lost_since_ms = None;
        self.next_retry_ms = now_ms;
    }

    pub(crate) fn poll(&mut self, now_ms: f64, io: &mut impl ControlIo) {
        let events = io.device_events();
        let prev = self.events_seen.replace(events).unwrap_or(events);
        if events.0 != prev.0 && self.backend.is_some() {
            self.mark_lost(now_ms);
        }
        if events.1 != prev.1 && self.backend.is_none() {
            self.next_retry_ms = now_ms;
        }
        if self.backend.is_none() && now_ms >= self.next_retry_ms {
            self.attempt(now_ms, io);
        }
    }

    pub(crate) fn write(&mut self, frame: &OutputFrame, now_ms: f64) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        self.writes.record(backend.write(frame));
        if self.writes.is_lost() {
            self.mark_lost(now_ms);
        }
    }

    fn attempt(&mut self, now_ms: f64, io: &mut impl ControlIo) {
        let (report, backend) = io.connect(self.device_id, self.buttons_required);
        self.set_report(report);
        match backend {
            Some(backend) => self.on_connected(backend),
            None => self.on_failed(now_ms),
        }
    }

    fn on_connected(&mut self, backend: Box<dyn OutputBackend>) {
        if self.connected_once {
            self.reconnects += 1;
        }
        self.connected_once = true;
        self.backend_desc = Some(backend.describe());
        self.backend = Some(backend);
        self.connection = Connection::Connected;
        self.lost_since_ms = None;
        self.writes.consecutive_failures = 0;
    }

    fn on_failed(&mut self, now_ms: f64) {
        let still_lost = self
            .lost_since_ms
            .is_some_and(|since| now_ms - since < LOST_GIVE_UP_MS);
        self.connection = if self.report.is_busy() {
            Connection::Busy
        } else if still_lost {
            Connection::Lost
        } else {
            Connection::SetupRequired
        };
        let delay = if still_lost {
            LOST_RETRY_MS
        } else {
            SETUP_RETRY_MS
        };
        self.next_retry_ms = now_ms + delay;
    }

    fn mark_lost(&mut self, now_ms: f64) {
        crate::log::line("çıkış bağlantısı koptu; yeniden denenecek");
        self.drop_backend();
        self.connection = Connection::Lost;
        self.lost_since_ms = Some(now_ms);
        self.next_retry_ms = now_ms + LOST_RETRY_MS;
        self.writes.consecutive_failures = 0;
    }

    fn drop_backend(&mut self) {
        self.backend = None;
        self.backend_desc = None;
    }

    fn set_report(&mut self, report: SetupReport) {
        if report != self.report {
            self.setup_failures = report.has_failures();
            self.report = report;
            self.report_seq += 1;
        }
    }
}
