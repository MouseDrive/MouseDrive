use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use libloading::Library;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::PWSTR;

use crate::output::{AxisRange, AxisRanges, OutputBackend, OutputFrame, WriteError};
use crate::setup::{AxisCaps, DeviceState, Owner, SetupReport, Versions};

pub const HID_USAGE_X: u32 = 0x30;
pub const HID_USAGE_Y: u32 = 0x31;
pub const HID_USAGE_Z: u32 = 0x32;
pub const HID_USAGE_RZ: u32 = 0x35;

const DLL_NAME: &str = "vJoyInterface.dll";

type Bool = i32;
type FnVoidBool = unsafe extern "C" fn() -> Bool;
type FnDriverMatch = unsafe extern "C" fn(*mut u16, *mut u16) -> Bool;
type FnIdInt = unsafe extern "C" fn(u32) -> i32;
type FnIdBool = unsafe extern "C" fn(u32) -> Bool;
type FnIdVoid = unsafe extern "C" fn(u32);
type FnUpdate = unsafe extern "C" fn(u32, *mut c_void) -> Bool;
type FnAxisExist = unsafe extern "C" fn(u32, u32) -> Bool;
type FnAxisLimit = unsafe extern "C" fn(u32, u32, *mut i32) -> Bool;
type RemovalCallback = unsafe extern "system" fn(Bool, Bool, *mut c_void);
type FnRegisterRemoval = unsafe extern "C" fn(RemovalCallback, *mut c_void);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct JoystickPositionV3 {
    b_device: u8,
    w_throttle: i32,
    w_rudder: i32,
    w_aileron: i32,
    w_axis_x: i32,
    w_axis_y: i32,
    w_axis_z: i32,
    w_axis_x_rot: i32,
    w_axis_y_rot: i32,
    w_axis_z_rot: i32,
    w_slider: i32,
    w_dial: i32,
    w_wheel: i32,
    w_accelerator: i32,
    w_brake: i32,
    w_clutch: i32,
    w_steering: i32,
    w_axis_vx: i32,
    w_axis_vy: i32,
    l_buttons: i32,
    b_hats: u32,
    b_hats_ex1: u32,
    b_hats_ex2: u32,
    b_hats_ex3: u32,
    l_buttons_ex1: i32,
    l_buttons_ex2: i32,
    l_buttons_ex3: i32,
    w_axis_vz: i32,
    w_axis_vbrx: i32,
    w_axis_vbry: i32,
    w_axis_vbrz: i32,
}

const _: () = assert!(std::mem::size_of::<JoystickPositionV3>() == 124);
const _: () = assert!(std::mem::offset_of!(JoystickPositionV3, w_axis_x) == 16);
const _: () = assert!(std::mem::offset_of!(JoystickPositionV3, w_axis_y) == 20);
const _: () = assert!(std::mem::offset_of!(JoystickPositionV3, w_axis_z_rot) == 36);
const _: () = assert!(std::mem::offset_of!(JoystickPositionV3, l_buttons) == 76);

const HATS_NEUTRAL: u32 = u32::MAX;

impl JoystickPositionV3 {
    fn base(device_id: u32, z: AxisRange) -> Self {
        Self {
            b_device: u8::try_from(device_id).unwrap_or(1),
            w_axis_z: z.center(),
            b_hats: HATS_NEUTRAL,
            b_hats_ex1: HATS_NEUTRAL,
            b_hats_ex2: HATS_NEUTRAL,
            b_hats_ex3: HATS_NEUTRAL,
            ..Self::default()
        }
    }

    fn apply(&mut self, frame: &OutputFrame, ranges: &AxisRanges) {
        self.w_axis_x = ranges.steering.map_signed(frame.steering);
        self.w_axis_y = ranges.throttle.map_unit(frame.throttle);
        self.w_axis_z_rot = ranges.brake.map_unit(frame.brake);
        self.l_buttons = frame.buttons as i32;
    }
}

pub struct VJoyLib {
    path: PathBuf,
    _lib: Option<Library>,
    enabled: FnVoidBool,
    driver_match: FnDriverMatch,
    status: FnIdInt,
    acquire: FnIdBool,
    relinquish: FnIdVoid,
    update: FnUpdate,
    axis_exist: FnAxisExist,
    axis_min: FnAxisLimit,
    axis_max: FnAxisLimit,
    button_count: FnIdInt,
    owner_pid: Option<FnIdInt>,
    register_removal: Option<FnRegisterRemoval>,
}

fn symbol<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
    let cname = format!("{name}\0");
    // SAFETY: T, vJoyInterface.h'deki imzayla eşleşen bir fonksiyon işaretçisi
    // tipidir; Library hiç bırakılmadığı için işaretçi geçerli kalır.
    unsafe { lib.get::<T>(cname.as_bytes()) }
        .map(|s| *s)
        .map_err(|_| format!("{DLL_NAME}: '{name}' bulunamadı"))
}

impl VJoyLib {
    fn open(path: &Path) -> Result<Self, String> {
        // SAFETY: vJoyInterface.dll'in DllMain'i yan etkisiz; yalnız bilinen
        // konumlardan ya da standart arama sırasından yüklenir.
        let lib = unsafe { Library::new(path) }.map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self {
            enabled: symbol(&lib, "vJoyEnabled")?,
            driver_match: symbol(&lib, "DriverMatch")?,
            status: symbol(&lib, "GetVJDStatus")?,
            acquire: symbol(&lib, "AcquireVJD")?,
            relinquish: symbol(&lib, "RelinquishVJD")?,
            update: symbol(&lib, "UpdateVJD")?,
            axis_exist: symbol(&lib, "GetVJDAxisExist")?,
            axis_min: symbol(&lib, "GetVJDAxisMin")?,
            axis_max: symbol(&lib, "GetVJDAxisMax")?,
            button_count: symbol(&lib, "GetVJDButtonNumber")?,
            owner_pid: symbol(&lib, "GetOwnerPid").ok(),
            register_removal: symbol(&lib, "RegisterRemovalCB").ok(),
            path: path.to_path_buf(),
            _lib: Some(lib),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn driver_enabled(&self) -> bool {
        // SAFETY: parametresiz sorgu.
        unsafe { (self.enabled)() != 0 }
    }

    pub fn versions(&self) -> Versions {
        let (mut dll, mut driver) = (0u16, 0u16);
        // SAFETY: iki yerel WORD'e yazar.
        let matched = unsafe { (self.driver_match)(&mut dll, &mut driver) } != 0;
        Versions {
            dll,
            driver,
            matched,
        }
    }

    pub fn device_state(&self, id: u32) -> DeviceState {
        // SAFETY: salt okuma sorgusu.
        device_state_from(unsafe { (self.status)(id) })
    }

    fn axis_caps(&self, id: u32) -> AxisCaps {
        // SAFETY: salt okuma sorguları.
        let has = |usage| unsafe { (self.axis_exist)(id, usage) } != 0;
        AxisCaps {
            x: has(HID_USAGE_X),
            y: has(HID_USAGE_Y),
            rz: has(HID_USAGE_RZ),
        }
    }

    fn buttons(&self, id: u32) -> i32 {
        // SAFETY: salt okuma sorgusu.
        unsafe { (self.button_count)(id) }.max(0)
    }

    fn axis_range(&self, id: u32, usage: u32) -> AxisRange {
        let (mut min, mut max) = (0i32, 0i32);
        // SAFETY: sonuçlar yerel değişkenlere yazılır.
        let ok = unsafe {
            (self.axis_min)(id, usage, &mut min) != 0 && (self.axis_max)(id, usage, &mut max) != 0
        };
        if ok {
            AxisRange::new(min, max).unwrap_or(AxisRange::DEFAULT)
        } else {
            AxisRange::DEFAULT
        }
    }

    fn owner(&self, id: u32) -> Option<Owner> {
        let get_pid = self.owner_pid?;
        // SAFETY: salt okuma sorgusu; negatif değerler hata kodudur.
        let pid = u32::try_from(unsafe { get_pid(id) })
            .ok()
            .filter(|&p| p != 0)?;
        Some(Owner {
            pid,
            name: process_name(pid),
        })
    }

    fn register_removal_callback(&self) {
        if let Some(register) = self.register_removal {
            // SAFETY: geri çağrı statik bir fonksiyon; DLL hiç bırakılmaz.
            unsafe { register(on_device_change, std::ptr::null_mut()) };
        }
    }
}

fn device_state_from(raw: i32) -> DeviceState {
    match raw {
        0 => DeviceState::Own,
        1 => DeviceState::Free,
        2 => DeviceState::Busy,
        3 => DeviceState::Missing,
        _ => DeviceState::Unknown,
    }
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: yalnız sorgu hakkıyla açılır ve hemen kapatılır.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    // SAFETY: buf `len` karakter yazılabilir.
    let ok = unsafe {
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok
    }
    .is_ok();
    if !ok {
        return None;
    }
    let full = String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]);
    Path::new(&full)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

static REMOVALS: AtomicU64 = AtomicU64::new(0);
static ARRIVALS: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn on_device_change(removed: Bool, _first: Bool, _data: *mut c_void) {
    let counter = if removed != 0 { &REMOVALS } else { &ARRIVALS };
    counter.fetch_add(1, Ordering::AcqRel);
}

pub fn device_events() -> (u64, u64) {
    (
        REMOVALS.load(Ordering::Acquire),
        ARRIVALS.load(Ordering::Acquire),
    )
}

fn dll_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        out.push(dir.join(DLL_NAME));
    }
    for var in ["ProgramW6432", "ProgramFiles"] {
        if let Some(pf) = std::env::var_os(var) {
            let path = PathBuf::from(pf).join("vJoy").join("x64").join(DLL_NAME);
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

fn choose(matched: &[bool]) -> usize {
    matched.iter().position(|&m| m).unwrap_or(0)
}

struct LoadedLib {
    lib: &'static VJoyLib,
    alternate: Option<PathBuf>,
}

static LOADED: OnceLock<LoadedLib> = OnceLock::new();

fn load() -> Result<&'static LoadedLib, String> {
    if let Some(loaded) = LOADED.get() {
        return Ok(loaded);
    }
    let mut libs: Vec<&'static VJoyLib> = Vec::new();
    let mut last_error = None;
    for path in dll_candidates().iter().filter(|p| p.exists()) {
        match VJoyLib::open(path) {
            Ok(lib) => libs.push(Box::leak(Box::new(lib))),
            Err(e) => last_error = Some(e),
        }
    }
    if libs.is_empty() {
        let lib = VJoyLib::open(Path::new(DLL_NAME)).map_err(|e| last_error.unwrap_or(e))?;
        libs.push(Box::leak(Box::new(lib)));
    }
    let matched: Vec<bool> = libs.iter().map(|l| l.versions().matched).collect();
    let chosen = choose(&matched);
    let lib = libs[chosen];
    let alternate = libs
        .iter()
        .enumerate()
        .find(|(i, _)| *i != chosen)
        .map(|(_, l)| l.path.clone());
    lib.register_removal_callback();
    Ok(LOADED.get_or_init(|| LoadedLib { lib, alternate }))
}

pub fn probe(device_id: u32, buttons_required: i32) -> (SetupReport, Option<VJoyDevice>) {
    let mut report = SetupReport {
        device_id,
        buttons_required,
        ..SetupReport::default()
    };
    let loaded = match load() {
        Ok(l) => l,
        Err(e) => {
            report.dll_error = Some(e);
            return (report, None);
        }
    };
    let device = probe_with(loaded.lib, &mut report);
    report.alternate_dll = loaded.alternate.clone();
    (report, device)
}

fn probe_with(lib: &'static VJoyLib, report: &mut SetupReport) -> Option<VJoyDevice> {
    let id = report.device_id;
    report.dll_path = Some(lib.path.clone());
    let enabled = lib.driver_enabled();
    report.driver_enabled = Some(enabled);
    if !enabled {
        return None;
    }
    report.versions = Some(lib.versions());
    let state = lib.device_state(id);
    report.device_state = Some(state);
    match state {
        DeviceState::Busy => {
            report.owner = lib.owner(id);
            return None;
        }
        DeviceState::Missing | DeviceState::Unknown => return None,
        DeviceState::Free | DeviceState::Own => {}
    }
    report.axes = Some(lib.axis_caps(id));
    report.buttons = Some(lib.buttons(id));
    let device = VJoyDevice::acquire(lib, id);
    report.acquired = device.is_some();
    device
}

pub struct VJoyDevice {
    lib: &'static VJoyLib,
    id: u32,
    ranges: AxisRanges,
    position: JoystickPositionV3,
}

impl VJoyDevice {
    fn acquire(lib: &'static VJoyLib, id: u32) -> Option<Self> {
        // SAFETY: basit çağrı; sonuç BOOL.
        if unsafe { (lib.acquire)(id) } == 0 {
            return None;
        }
        let ranges = AxisRanges {
            steering: lib.axis_range(id, HID_USAGE_X),
            throttle: lib.axis_range(id, HID_USAGE_Y),
            brake: lib.axis_range(id, HID_USAGE_RZ),
        };
        let z = lib.axis_range(id, HID_USAGE_Z);
        let mut device = Self {
            lib,
            id,
            ranges,
            position: JoystickPositionV3::base(id, z),
        };
        let _ = device.write(&OutputFrame::NEUTRAL);
        Some(device)
    }

    pub fn ranges(&self) -> AxisRanges {
        self.ranges
    }
}

impl OutputBackend for VJoyDevice {
    fn write(&mut self, frame: &OutputFrame) -> Result<(), WriteError> {
        self.position.apply(frame, &self.ranges);
        let data = (&mut self.position as *mut JoystickPositionV3).cast::<c_void>();
        // SAFETY: position, DLL'in beklediği V3 düzeninde (const assert'ler)
        // ve çağrı boyunca yazılabilir.
        let ok = unsafe { (self.lib.update)(self.id, data) } != 0;
        if ok { Ok(()) } else { Err(WriteError) }
    }

    fn describe(&self) -> String {
        format!("vJoy #{} ({})", self.id, self.lib.path.display())
    }
}

impl Drop for VJoyDevice {
    fn drop(&mut self) {
        let _ = self.write(&OutputFrame::NEUTRAL);
        // SAFETY: bu süreç cihazı sahiplenmişti.
        unsafe { (self.lib.relinquish)(self.id) };
    }
}
