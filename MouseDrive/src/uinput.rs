#![deny(unsafe_code)]

use std::io;
use std::path::PathBuf;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode,
    UinputAbsSetup,
};

use crate::output::{AxisRange, AxisRanges, OutputBackend, OutputFrame, WriteError};
use crate::setup::{AxisCaps, DeviceState, OpenError, OpenErrorKind, SetupReport};

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
const AXIS_RANGES: [AxisRange; 3] = [RANGES.steering, RANGES.throttle, RANGES.brake];

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

fn classify(e: &io::Error) -> OpenErrorKind {
    match e.raw_os_error() {
        Some(libc::ENOENT | libc::ENODEV | libc::ENXIO) => OpenErrorKind::ModuleMissing,
        Some(libc::EACCES | libc::EPERM) => OpenErrorKind::Permission,
        Some(libc::EROFS) => OpenErrorKind::ReadOnly,
        Some(_) => OpenErrorKind::Other,
        None => match e.kind() {
            io::ErrorKind::NotFound => OpenErrorKind::ModuleMissing,
            io::ErrorKind::PermissionDenied => OpenErrorKind::Permission,
            io::ErrorKind::ReadOnlyFilesystem => OpenErrorKind::ReadOnly,
            _ => OpenErrorKind::Other,
        },
    }
}

fn apply_error(report: &mut SetupReport, e: &io::Error) {
    let kind = classify(e);
    let message = e.to_string();
    match kind {
        OpenErrorKind::ModuleMissing => {
            report.dll_error = Some(format!("{UINPUT_PATH}: {message}"));
        }
        OpenErrorKind::Permission | OpenErrorKind::ReadOnly => {
            report.dll_path = Some(PathBuf::from(UINPUT_PATH));
            report.driver_enabled = Some(false);
            report.driver_detail = Some(message.clone());
        }
        OpenErrorKind::Other => {
            report.dll_path = Some(PathBuf::from(UINPUT_PATH));
            report.driver_enabled = Some(true);
            report.device_state = Some(DeviceState::Unknown);
        }
    }
    report.open_error = Some(OpenError { kind, message });
}

pub struct UinputDevice {
    device: VirtualDevice,
    node: Option<PathBuf>,
}

impl UinputDevice {
    fn create() -> io::Result<Self> {
        let mut device = build_device()?;
        let node = device
            .enumerate_dev_nodes_blocking()
            .ok()
            .and_then(|mut nodes| nodes.find_map(Result::ok));
        if let Err(e) = device.emit(&frame_events(&OutputFrame::NEUTRAL)) {
            crate::log::warn(&format!(
                "sanal direksiyonun ilk nötr değeri yazılamadı: {e}"
            ));
        }
        Ok(Self { device, node })
    }
}

fn build_device() -> io::Result<VirtualDevice> {
    let mut buttons = AttributeSet::<KeyCode>::new();
    for i in 0..BUTTON_COUNT {
        buttons.insert(KeyCode(FIRST_BUTTON + i));
    }
    let mut builder = VirtualDevice::builder()?
        .name(DEVICE_NAME)
        .input_id(InputId::new(BusType::BUS_VIRTUAL, 0, 0, 1))
        .with_keys(&buttons)?;
    for (info, axis) in initial_abs_info().into_iter().zip(AXES) {
        builder = builder.with_absolute_axis(&UinputAbsSetup::new(axis, info))?;
    }
    builder.build()
}

fn axis_values(frame: &OutputFrame) -> [i32; 3] {
    [
        RANGES.steering.map_signed(frame.steering),
        RANGES.throttle.map_unit(frame.throttle),
        RANGES.brake.map_unit(frame.brake),
    ]
}

fn initial_abs_info() -> [AbsInfo; 3] {
    let neutral = axis_values(&OutputFrame::NEUTRAL);
    std::array::from_fn(|i| {
        let range = AXIS_RANGES[i];
        AbsInfo::new(neutral[i], range.min, range.max, 0, 0, 0)
    })
}

fn frame_events(frame: &OutputFrame) -> [InputEvent; EVENT_COUNT] {
    let abs = EventType::ABSOLUTE.0;
    let key = EventType::KEY.0;
    let values = axis_values(frame);
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

    fn survives_reconnect(&self) -> bool {
        true
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        let _ = self.write(&OutputFrame::NEUTRAL);
    }
}
