#![deny(unsafe_code)]

use std::fmt::Display;

use zbus::blocking::Connection;
use zbus::zvariant::OwnedValue;

use super::priority::{KitLimits, RealtimeKit};

const SERVICE: &str = "org.freedesktop.RealtimeKit1";
const PATH: &str = "/org/freedesktop/RealtimeKit1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

pub(super) struct RtKit {
    conn: Connection,
}

impl RtKit {
    pub(super) fn connect() -> Result<Self, String> {
        Connection::system()
            .map(|conn| Self { conn })
            .map_err(|e| format!("sistem D-Bus'ına bağlanılamadı: {e}"))
    }

    fn property<T>(&self, name: &str) -> Result<T, String>
    where
        T: TryFrom<OwnedValue>,
        T::Error: Display,
    {
        let reply = self
            .conn
            .call_method(
                Some(SERVICE),
                PATH,
                Some(PROPERTIES),
                "Get",
                &(SERVICE, name),
            )
            .map_err(|e| format!("{name} okunamadı: {e}"))?;
        let value: OwnedValue = reply
            .body()
            .deserialize()
            .map_err(|e| format!("{name} çözülemedi: {e}"))?;
        T::try_from(value).map_err(|e| format!("{name} beklenen türde değil: {e}"))
    }

    fn call<B>(&self, method: &str, body: &B) -> Result<(), String>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        self.conn
            .call_method(Some(SERVICE), PATH, Some(SERVICE), method, body)
            .map(drop)
            .map_err(|e| format!("{method} reddedildi: {e}"))
    }
}

impl RealtimeKit for RtKit {
    fn limits(&self) -> Result<KitLimits, String> {
        Ok(KitLimits {
            max_realtime: self.property("MaxRealtimePriority")?,
            min_nice: self.property("MinNiceLevel")?,
            rttime_max_us: self.property("RTTimeUSecMax")?,
        })
    }

    fn make_realtime(&self, tid: u64, priority: u32) -> Result<(), String> {
        self.call("MakeThreadRealtime", &(tid, priority))
    }

    fn make_high_priority(&self, tid: u64, nice: i32) -> Result<(), String> {
        self.call("MakeThreadHighPriority", &(tid, nice))
    }
}
