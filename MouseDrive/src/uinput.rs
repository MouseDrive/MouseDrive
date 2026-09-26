#![deny(unsafe_code)]

use std::io;
use std::path::{Path, PathBuf};

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    UinputAbsSetup,
};

use crate::output::{AxisRange, AxisRanges, OutputBackend, OutputFrame, WriteError};
use crate::setup::{AxisCaps, DeviceState, SetupReport};

pub const UINPUT_PATH: &str = "/dev/uinput";
pub const DEVICE_NAME: &str = "MouseDrive Virtual Wheel";
const BUTTON_COUNT: u16 = 8;
const FIRST_BUTTON: u16 = KeyCode::BTN_TRIGGER.0;
const AXES: [AbsoluteAxisCode; 3] = [
    AbsoluteAxisCode::ABS_X,
    AbsoluteAxisCode::ABS_Y,
    AbsoluteAxisCode::ABS_RZ,
];
const EVENT_COUNT: usize = AXES.len() + BUTTON_COUNT as usize;
const RANGES: AxisRanges = AxisRanges {
    steering: AxisRange::DEFAULT,
    throttle: AxisRange::DEFAULT,
    brake: AxisRange::DEFAULT,
};

pub fn device_events() -> (u64, u64) {
    (0, 0)
}

pub fn probe(device_id: u32, buttons_required: i32) -> (SetupReport, Option<UinputDevice>) {
    let mut report = SetupReport {
        device_id,
        buttons_required,
        ..SetupReport::default()
    };
    match UinputDevice::create() {
        Ok(device) => {
            report.dll_path = Some(PathBuf::from(UINPUT_PATH));
            report.driver_enabled = Some(true);
            report.device_state = Some(DeviceState::Free);
            report.axes = Some(AxisCaps {
                x: true,
                y: true,
                rz: true,
            });
            report.buttons = Some(i32::from(BUTTON_COUNT));
            report.acquired = true;
            (report, Some(device))
        }
        Err(e) => {
            apply_error(&mut report, &e);
            (report, None)
        }
    }
}

fn apply_error(report: &mut SetupReport, e: &io::Error) {
    if e.kind() == io::ErrorKind::NotFound && !Path::new(UINPUT_PATH).exists() {
        report.dll_error = Some(format!("{UINPUT_PATH}: {e}"));
        return;
    }
    report.dll_path = Some(PathBuf::from(UINPUT_PATH));
    if e.kind() == io::ErrorKind::PermissionDenied {
        report.driver_enabled = Some(false);
        report.driver_detail = Some(e.to_string());
        return;
    }
    crate::log::line(&format!("sanal direksiyon oluşturulamadı: {e}"));
    report.driver_enabled = Some(true);
    report.device_state = Some(DeviceState::Unknown);
}

pub struct UinputDevice {
    device: VirtualDevice,
    node: Option<PathBuf>,
}

impl UinputDevice {
    fn create() -> io::Result<Self> {
        let mut buttons = AttributeSet::<KeyCode>::new();
        for i in 0..BUTTON_COUNT {
            buttons.insert(KeyCode(FIRST_BUTTON + i));
        }
        let mut builder = VirtualDevice::builder()?
            .name(DEVICE_NAME)
            .input_id(InputId::new(BusType::BUS_VIRTUAL, 0, 0, 1))
            .with_keys(&buttons)?;
        let range = AxisRange::DEFAULT;
        for axis in AXES {
            let info = AbsInfo::new(range.center(), range.min, range.max, 0, 0, 0);
            builder = builder.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
        }
        let mut device = builder.build()?;
        let node = device
            .enumerate_dev_nodes_blocking()
            .ok()
            .and_then(|mut nodes| nodes.find_map(Result::ok));
        let mut out = Self { device, node };
        let _ = out.write(&OutputFrame::NEUTRAL);
        Ok(out)
    }
}

fn frame_events(frame: &OutputFrame) -> [InputEvent; EVENT_COUNT] {
    let abs = EventType::ABSOLUTE.0;
    let key = EventType::KEY.0;
    let values = [
        RANGES.steering.map_signed(frame.steering),
        RANGES.throttle.map_unit(frame.throttle),
        RANGES.brake.map_unit(frame.brake),
    ];
    std::array::from_fn(|i| match AXES.get(i) {
        Some(axis) => InputEvent::new(abs, axis.0, values[i]),
        None => {
            let bit = i - AXES.len();
            let down = frame.buttons >> bit & 1;
            InputEvent::new(key, FIRST_BUTTON + bit as u16, down as i32)
        }
    })
}

impl OutputBackend for UinputDevice {
    fn write(&mut self, frame: &OutputFrame) -> Result<(), WriteError> {
        self.device
            .emit(&frame_events(frame))
            .map_err(|_| WriteError)
    }

    fn describe(&self) -> String {
        match &self.node {
            Some(node) => format!("uinput: {DEVICE_NAME} ({})", node.display()),
            None => format!("uinput: {DEVICE_NAME}"),
        }
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        let _ = self.write(&OutputFrame::NEUTRAL);
    }
}
