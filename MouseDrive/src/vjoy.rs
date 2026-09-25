use libloading::Library;

pub const HID_USAGE_X: u32 = 0x30;
pub const HID_USAGE_Y: u32 = 0x31;
pub const HID_USAGE_RZ: u32 = 0x35;

pub const AXIS_MAX: i32 = 32767;
pub const AXIS_MIN: i32 = 0;
pub const AXIS_CENTER: i32 = 16383;

#[repr(u32)]
#[derive(Debug, PartialEq)]
pub enum VjdStat {
    Own = 0,
    Free = 1,
    Busy = 2,
    Miss = 3,
    Unkn = 4,
}

#[derive(Clone, Debug, PartialEq)]
pub enum VJoyStatus {
    Connected,
    DllNotFound,
    DriverDisabled,
    DeviceBusy,
    DeviceMissing,
    AcquireFailed,
    Unknown,
}

type FnVJoyEnabled = unsafe extern "C" fn() -> bool;
type FnGetVJDStatus = unsafe extern "C" fn(u32) -> u32;
type FnAcquireVJD = unsafe extern "C" fn(u32) -> bool;
type FnRelinquishVJD = unsafe extern "C" fn(u32);
type FnSetAxis = unsafe extern "C" fn(i32, u32, u32) -> bool;
type FnSetBtn = unsafe extern "C" fn(bool, u32, u8) -> bool;
type FnResetVJD = unsafe extern "C" fn(u32) -> bool;

pub struct VJoyApi {
    _lib: Library,
    vjoy_enabled: FnVJoyEnabled,
    get_vjd_status: FnGetVJDStatus,
    acquire_vjd: FnAcquireVJD,
    relinquish_vjd: FnRelinquishVJD,
    set_axis: FnSetAxis,
    set_btn: FnSetBtn,
    reset_vjd: FnResetVJD,
    acquired: std::cell::Cell<Option<u32>>,
}

fn find_vjoy_dll() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let path = dir.join("vJoyInterface.dll");
        if path.exists() {
            return Some(path);
        }
    }

    if let Ok(pf) = std::env::var("ProgramFiles") {
        let path = std::path::PathBuf::from(&pf)
            .join("vJoy")
            .join("x64")
            .join("vJoyInterface.dll");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

impl VJoyApi {
    #[allow(clippy::missing_transmute_annotations)]
    pub fn load() -> Option<Self> {
        unsafe {
            let lib = if let Some(path) = find_vjoy_dll() {
                Library::new(path).ok()?
            } else {
                Library::new("vJoyInterface.dll").ok()?
            };

            macro_rules! sym {
                ($name:literal, $ty:ty) => {
                    std::mem::transmute::<_, $ty>(lib.get::<$ty>($name).ok()?.into_raw().into_raw())
                };
            }

            let api = Self {
                vjoy_enabled: sym!(b"vJoyEnabled", FnVJoyEnabled),
                get_vjd_status: sym!(b"GetVJDStatus", FnGetVJDStatus),
                acquire_vjd: sym!(b"AcquireVJD", FnAcquireVJD),
                relinquish_vjd: sym!(b"RelinquishVJD", FnRelinquishVJD),
                set_axis: sym!(b"SetAxis", FnSetAxis),
                set_btn: sym!(b"SetBtn", FnSetBtn),
                reset_vjd: sym!(b"ResetVJD", FnResetVJD),
                _lib: lib,
                acquired: std::cell::Cell::new(None),
            };
            Some(api)
        }
    }

    pub fn is_enabled(&self) -> bool {
        unsafe { (self.vjoy_enabled)() }
    }
    pub fn acquire(&self, dev: u32) -> bool {
        let ok = unsafe { (self.acquire_vjd)(dev) };
        if ok {
            self.acquired.set(Some(dev));
        }
        ok
    }
    pub fn relinquish(&self, dev: u32) {
        unsafe { (self.relinquish_vjd)(dev) }
        if self.acquired.get() == Some(dev) {
            self.acquired.set(None);
        }
    }
    pub fn reset(&self, dev: u32) -> bool {
        unsafe { (self.reset_vjd)(dev) }
    }

    pub fn set_axis(&self, val: i32, dev: u32, usage: u32) -> bool {
        unsafe { (self.set_axis)(val, dev, usage) }
    }
    pub fn set_btn(&self, on: bool, dev: u32, btn: u8) -> bool {
        unsafe { (self.set_btn)(on, dev, btn) }
    }

    pub fn get_status(&self, dev: u32) -> VjdStat {
        match unsafe { (self.get_vjd_status)(dev) } {
            0 => VjdStat::Own,
            1 => VjdStat::Free,
            2 => VjdStat::Busy,
            3 => VjdStat::Miss,
            _ => VjdStat::Unkn,
        }
    }
}

impl Drop for VJoyApi {
    fn drop(&mut self) {
        if let Some(dev) = self.acquired.get() {
            unsafe { (self.relinquish_vjd)(dev) }
        }
    }
}
