#![deny(unsafe_code)]

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceState {
    Free,
    Own,
    Busy,
    Missing,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Versions {
    pub dll: u16,
    pub driver: u16,
    pub matched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub pid: u32,
    pub name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AxisCaps {
    pub x: bool,
    pub y: bool,
    pub rz: bool,
}

impl AxisCaps {
    pub fn missing(&self) -> Vec<&'static str> {
        [(self.x, "X"), (self.y, "Y"), (self.rz, "Rz")]
            .iter()
            .filter(|(present, _)| !present)
            .map(|(_, name)| *name)
            .collect()
    }
}

pub const REQUIRED_BUTTONS: i32 = 2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetupReport {
    pub device_id: u32,
    pub buttons_required: i32,
    pub dll_path: Option<PathBuf>,
    pub dll_error: Option<String>,
    pub alternate_dll: Option<PathBuf>,
    pub driver_enabled: Option<bool>,
    pub driver_detail: Option<String>,
    pub versions: Option<Versions>,
    pub device_state: Option<DeviceState>,
    pub owner: Option<Owner>,
    pub axes: Option<AxisCaps>,
    pub buttons: Option<i32>,
    pub acquired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckId {
    Dll,
    Driver,
    Versions,
    Device,
    Axes,
    Buttons,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Pass,
    Warn,
    Fail,
    NotRun,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Check {
    pub id: CheckId,
    pub state: CheckState,
}

pub fn format_version(v: u16) -> String {
    format!("{}.{}.{}", (v >> 8) & 0xF, (v >> 4) & 0xF, v & 0xF)
}

fn pass_or(ok: bool, otherwise: CheckState) -> CheckState {
    if ok { CheckState::Pass } else { otherwise }
}

impl SetupReport {
    fn device_check(&self) -> CheckState {
        match self.device_state {
            None => CheckState::NotRun,
            Some(DeviceState::Free | DeviceState::Own) => pass_or(self.acquired, CheckState::Fail),
            Some(_) => CheckState::Fail,
        }
    }

    pub fn checks(&self) -> Vec<Check> {
        let dll = pass_or(
            self.dll_path.is_some() && self.dll_error.is_none(),
            CheckState::Fail,
        );
        let driver = self
            .driver_enabled
            .map_or(CheckState::NotRun, |on| pass_or(on, CheckState::Fail));
        let versions = self
            .versions
            .map_or(CheckState::NotRun, |v| pass_or(v.matched, CheckState::Warn));
        let axes = self.axes.map_or(CheckState::NotRun, |a| {
            pass_or(a.missing().is_empty(), CheckState::Fail)
        });
        let buttons = self.buttons.map_or(CheckState::NotRun, |n| {
            pass_or(n >= self.buttons_required, CheckState::Fail)
        });
        vec![
            Check {
                id: CheckId::Dll,
                state: dll,
            },
            Check {
                id: CheckId::Driver,
                state: driver,
            },
            Check {
                id: CheckId::Versions,
                state: versions,
            },
            Check {
                id: CheckId::Device,
                state: self.device_check(),
            },
            Check {
                id: CheckId::Axes,
                state: axes,
            },
            Check {
                id: CheckId::Buttons,
                state: buttons,
            },
        ]
    }

    pub fn has_failures(&self) -> bool {
        self.checks().iter().any(|c| c.state == CheckState::Fail)
    }

    pub fn has_warnings(&self) -> bool {
        self.checks().iter().any(|c| c.state == CheckState::Warn)
    }

    pub fn is_busy(&self) -> bool {
        self.device_state == Some(DeviceState::Busy)
    }
}
