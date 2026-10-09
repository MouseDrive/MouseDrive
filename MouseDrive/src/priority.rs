#![deny(unsafe_code)]

use std::sync::atomic::{AtomicU64, Ordering};

pub const RAISED_NICE: i32 = -10;
pub const REALTIME_CAP: i32 = 10;
pub const RT_TIME_SOFT_US: u64 = 200_000;
pub const RT_TIME_MIN_US: u64 = 20_000;

const NICE_BASE: i32 = 20;
const NICE_RANGE: u64 = 40;

const TAG_NORMAL: u64 = 0;
const TAG_NICE: u64 = 1;
const TAG_REALTIME: u64 = 2;
const TAG_HIGH: u64 = 3;
const TAG_DEMOTED: u64 = 4;
const TAG_FAILED: u64 = 5;

const HRESULT_FACILITY_MASK: u32 = 0xFFFF_0000;
const HRESULT_FROM_WIN32: u32 = 0x8007_0000;
const WIN32_CODE_MASK: u32 = 0x0000_FFFF;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThreadPriority {
    #[default]
    Normal,
    Nice(i32),
    Realtime(i32),
    High,
    Demoted,
    Failed(i32),
}

impl ThreadPriority {
    fn to_bits(self) -> u64 {
        let (tag, value) = match self {
            ThreadPriority::Normal => (TAG_NORMAL, 0),
            ThreadPriority::Nice(n) => (TAG_NICE, n),
            ThreadPriority::Realtime(p) => (TAG_REALTIME, p),
            ThreadPriority::High => (TAG_HIGH, 0),
            ThreadPriority::Demoted => (TAG_DEMOTED, 0),
            ThreadPriority::Failed(code) => (TAG_FAILED, code),
        };
        (tag << 32) | u64::from(value.cast_unsigned())
    }

    fn from_bits(bits: u64) -> Self {
        let value = (bits as u32).cast_signed();
        match bits >> 32 {
            TAG_NICE => ThreadPriority::Nice(value),
            TAG_REALTIME => ThreadPriority::Realtime(value),
            TAG_HIGH => ThreadPriority::High,
            TAG_DEMOTED => ThreadPriority::Demoted,
            TAG_FAILED => ThreadPriority::Failed(value),
            _ => ThreadPriority::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Priorities {
    pub control: ThreadPriority,
    pub input: ThreadPriority,
}

impl Priorities {
    pub fn any_realtime(&self) -> bool {
        [self.control, self.input]
            .iter()
            .any(|priority| matches!(priority, ThreadPriority::Realtime(_)))
    }
}

pub struct PrioritySlot(AtomicU64);

impl PrioritySlot {
    pub const fn new() -> Self {
        Self(AtomicU64::new(TAG_NORMAL << 32))
    }

    pub fn get(&self) -> ThreadPriority {
        ThreadPriority::from_bits(self.0.load(Ordering::Acquire))
    }

    pub fn set(&self, priority: ThreadPriority) {
        self.0.store(priority.to_bits(), Ordering::Release);
    }

    pub fn replace(&self, expected: ThreadPriority, priority: ThreadPriority) -> bool {
        self.0
            .compare_exchange(
                expected.to_bits(),
                priority.to_bits(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

impl Default for PrioritySlot {
    fn default() -> Self {
        Self::new()
    }
}

pub fn realtime_priority(limit: u64) -> Option<i32> {
    (limit > 0).then(|| i32::try_from(limit).unwrap_or(i32::MAX).min(REALTIME_CAP))
}

pub fn allowed_nice(nice_limit: u64) -> Option<i32> {
    let floor = NICE_BASE - i32::try_from(nice_limit.min(NICE_RANGE)).unwrap_or(NICE_BASE);
    (floor < 0).then_some(floor.max(RAISED_NICE))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtTime {
    pub soft: Option<u64>,
    pub hard: Option<u64>,
}

pub fn rttime_limits(current: RtTime, ceiling: Option<u64>) -> Option<RtTime> {
    let hard = match (current.hard, ceiling) {
        (Some(hard), Some(ceiling)) => Some(hard.min(ceiling)),
        (hard, None) => hard,
        (None, ceiling) => ceiling,
    };
    let budget = hard.map_or(RT_TIME_SOFT_US, |hard| RT_TIME_SOFT_US.min(hard / 2));
    let soft = current.soft.map_or(budget, |soft| soft.min(budget));
    (soft >= RT_TIME_MIN_US).then_some(RtTime {
        soft: Some(soft),
        hard,
    })
}

pub fn win32_error_code(hresult: i32) -> i32 {
    let bits = hresult.cast_unsigned();
    if bits & HRESULT_FACILITY_MASK == HRESULT_FROM_WIN32 {
        (bits & WIN32_CODE_MASK).cast_signed()
    } else {
        hresult
    }
}

pub fn describe(priority: ThreadPriority) -> String {
    match priority {
        ThreadPriority::Normal => "normal".into(),
        ThreadPriority::Nice(n) => format!("nice {n}"),
        ThreadPriority::Realtime(p) => format!("gerçek zamanlı (SCHED_RR {p})"),
        ThreadPriority::High => "yüksek".into(),
        ThreadPriority::Demoted => "gerçek zamanlıdan normale düşürüldü".into(),
        ThreadPriority::Failed(code) => {
            format!(
                "yükseltilemedi: {}",
                std::io::Error::from_raw_os_error(code)
            )
        }
    }
}

pub fn log_outcome(label: &str, priority: ThreadPriority) {
    let text = format!("{label} thread'i önceliği: {}", describe(priority));
    match priority {
        ThreadPriority::Failed(_) | ThreadPriority::Demoted => crate::log::warn(&text),
        _ => crate::log::info(&text),
    }
}
