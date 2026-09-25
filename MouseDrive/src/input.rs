use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Devices::HumanInterfaceDevice::HidD_GetProductString;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST,
};
use windows::Win32::UI::Input::{
    GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList, GetRegisteredRawInputDevices,
    HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTDEVICELIST, RAWINPUTHEADER,
    RID_INPUT, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDI_DEVICENAME, RIM_TYPEMOUSE,
    RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostMessageW,
    PostThreadMessageW, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_INPUT,
    WM_INPUT_DEVICE_CHANGE, WM_QUIT, WNDCLASSW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::logic::{COUNT_ACCUM_LIMIT, accumulate_counts};

const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;

const WM_APP_REGISTER: u32 = WM_APP + 1;
const WM_APP_FILTER: u32 = WM_APP + 2;

pub struct InputState {
    counts_x: AtomicI64,
    left: AtomicBool,
    right: AtomicBool,
    middle_clicked: AtomicBool,
    move_events: AtomicU64,
    absolute_events: AtomicU64,
    ignored_events: AtomicU64,
    last_device: AtomicIsize,
    accepted_device: AtomicIsize,
    filter_missing: AtomicBool,
    allow_injected: AtomicBool,
    input_sink: AtomicBool,
    hwnd: AtomicIsize,
    thread_id: AtomicU32,
    filter_path: Mutex<String>,
}

impl InputState {
    pub const fn new() -> Self {
        Self {
            counts_x: AtomicI64::new(0),
            left: AtomicBool::new(false),
            right: AtomicBool::new(false),
            middle_clicked: AtomicBool::new(false),
            move_events: AtomicU64::new(0),
            absolute_events: AtomicU64::new(0),
            ignored_events: AtomicU64::new(0),
            last_device: AtomicIsize::new(0),
            accepted_device: AtomicIsize::new(0),
            filter_missing: AtomicBool::new(false),
            allow_injected: AtomicBool::new(false),
            input_sink: AtomicBool::new(true),
            hwnd: AtomicIsize::new(0),
            thread_id: AtomicU32::new(0),
            filter_path: Mutex::new(String::new()),
        }
    }

    fn accepts(&self, device: isize) -> bool {
        let wanted = self.accepted_device.load(Ordering::Relaxed);
        if wanted == 0 {
            return true;
        }
        if device == 0 {
            return self.allow_injected.load(Ordering::Relaxed);
        }
        device == wanted
    }

    fn process(&self, device: isize, flags: u16, dx: i32, dy: i32, button_flags: u16) {
        if !self.accepts(device) {
            self.ignored_events.fetch_add(1, Ordering::Relaxed);
            return;
        }
        if flags & MOUSE_MOVE_ABSOLUTE.0 != 0 {
            self.absolute_events.fetch_add(1, Ordering::Relaxed);
        } else if dx != 0 || dy != 0 {
            let _ = self
                .counts_x
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |acc| {
                    Some(accumulate_counts(acc, dx))
                });
            self.move_events.fetch_add(1, Ordering::Relaxed);
            self.last_device.store(device, Ordering::Relaxed);
        }
        self.apply_buttons(button_flags);
    }

    fn apply_buttons(&self, bf: u16) {
        let updates = [
            (RI_MOUSE_LEFT_BUTTON_DOWN, &self.left, true),
            (RI_MOUSE_LEFT_BUTTON_UP, &self.left, false),
            (RI_MOUSE_RIGHT_BUTTON_DOWN, &self.right, true),
            (RI_MOUSE_RIGHT_BUTTON_UP, &self.right, false),
        ];
        for (mask, flag, value) in updates {
            if bf & mask != 0 {
                flag.store(value, Ordering::Release);
            }
        }
        if bf & RI_MOUSE_MIDDLE_BUTTON_DOWN != 0 {
            self.middle_clicked.store(true, Ordering::Release);
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

static STATE: InputState = InputState::new();

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputCounters {
    pub move_events: u64,
    pub absolute_events: u64,
    pub ignored_events: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DeviceFilter {
    #[default]
    AllMice,
    Selected,
    SelectedMissing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MouseInfo {
    pub path: String,
    pub label: String,
}

pub fn start(
    input_sink: bool,
    device_path: &str,
    allow_injected: bool,
) -> std::io::Result<JoinHandle<()>> {
    STATE.input_sink.store(input_sink, Ordering::Release);
    STATE
        .allow_injected
        .store(allow_injected, Ordering::Release);
    store_filter_path(device_path);
    std::thread::Builder::new()
        .name("raw-input".into())
        .spawn(thread_main)
}

pub fn stop() {
    let id = STATE.thread_id.load(Ordering::Acquire);
    if id != 0 {
        // SAFETY: yalnız bir mesaj gönderilir; thread yoksa çağrı başarısız olur.
        let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

pub fn take_counts() -> i64 {
    STATE
        .counts_x
        .swap(0, Ordering::Relaxed)
        .clamp(-COUNT_ACCUM_LIMIT, COUNT_ACCUM_LIMIT)
}

pub fn buttons() -> (bool, bool) {
    (
        STATE.left.load(Ordering::Acquire),
        STATE.right.load(Ordering::Acquire),
    )
}

pub fn take_middle_click() -> bool {
    STATE.middle_clicked.swap(false, Ordering::Acquire)
}

pub fn force_release(button: MouseButton) -> bool {
    let flag = match button {
        MouseButton::Left => &STATE.left,
        MouseButton::Right => &STATE.right,
    };
    flag.compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub fn clear() {
    STATE.counts_x.store(0, Ordering::Relaxed);
    STATE.left.store(false, Ordering::Release);
    STATE.right.store(false, Ordering::Release);
    STATE.middle_clicked.store(false, Ordering::Release);
}

pub fn counters() -> InputCounters {
    InputCounters {
        move_events: STATE.move_events.load(Ordering::Relaxed),
        absolute_events: STATE.absolute_events.load(Ordering::Relaxed),
        ignored_events: STATE.ignored_events.load(Ordering::Relaxed),
    }
}

pub fn set_input_sink(enabled: bool) {
    let previous = STATE.input_sink.swap(enabled, Ordering::AcqRel);
    if previous != enabled {
        request_register();
    }
}

pub fn set_device_filter(path: &str) {
    if store_filter_path(path) {
        post_to_window(WM_APP_FILTER);
    }
}

pub fn device_filter() -> DeviceFilter {
    if STATE.filter_missing.load(Ordering::Acquire) {
        DeviceFilter::SelectedMissing
    } else if STATE.accepted_device.load(Ordering::Acquire) != 0 {
        DeviceFilter::Selected
    } else {
        DeviceFilter::AllMice
    }
}

pub fn request_register() {
    post_to_window(WM_APP_REGISTER);
}

pub fn registration_ok() -> bool {
    let hwnd = STATE.hwnd.load(Ordering::Acquire);
    if hwnd == 0 {
        return false;
    }
    let sink = STATE.input_sink.load(Ordering::Acquire);
    registered_devices().iter().any(|d| {
        d.usUsagePage == 0x01
            && d.usUsage == 0x02
            && d.hwndTarget.0 as isize == hwnd
            && ((d.dwFlags.0 & RIDEV_INPUTSINK.0) != 0) == sink
    })
}

pub fn last_moved_device() -> Option<String> {
    let handle = STATE.last_device.load(Ordering::Relaxed);
    (handle != 0).then(|| device_path(HANDLE(handle as *mut c_void)))?
}

pub fn list_mice() -> Vec<MouseInfo> {
    let paths: Vec<String> = mouse_handles()
        .into_iter()
        .filter_map(device_path)
        .collect();
    let labels: Vec<String> = paths
        .iter()
        .map(|p| label_for(p, product_name(p)))
        .collect();
    paths
        .into_iter()
        .zip(disambiguate(labels))
        .map(|(path, label)| MouseInfo { path, label })
        .collect()
}

fn hex_after(path: &str, key: &str, digits: usize) -> Option<u16> {
    let upper = path.to_ascii_uppercase();
    let start = upper.find(key)? + key.len();
    let run: String = upper[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    let tail = run.get(run.len().checked_sub(digits)?..)?;
    u16::from_str_radix(tail, 16).ok()
}

pub fn parse_vid_pid(path: &str) -> Option<(u16, u16)> {
    let vid = hex_after(path, "VID_", 4).or_else(|| hex_after(path, "VID&", 4))?;
    let pid = hex_after(path, "PID_", 4).or_else(|| hex_after(path, "PID&", 4))?;
    Some((vid, pid))
}

pub fn label_for(path: &str, product: Option<String>) -> String {
    let ids = parse_vid_pid(path).map(|(v, p)| format!("{v:04X}:{p:04X}"));
    match (product.filter(|s| !s.trim().is_empty()), ids) {
        (Some(name), Some(ids)) => format!("{} ({ids})", name.trim()),
        (Some(name), None) => name.trim().to_string(),
        (None, Some(ids)) => format!("USB {ids}"),
        (None, None) => path.rsplit('#').nth(1).unwrap_or(path).to_string(),
    }
}

pub fn disambiguate(labels: Vec<String>) -> Vec<String> {
    let mut seen: Vec<(String, usize)> = Vec::new();
    labels
        .iter()
        .map(|label| {
            let total = labels.iter().filter(|l| *l == label).count();
            if total == 1 {
                return label.clone();
            }
            let n = match seen.iter_mut().find(|(l, _)| l == label) {
                Some(entry) => {
                    entry.1 += 1;
                    entry.1
                }
                None => {
                    seen.push((label.clone(), 1));
                    1
                }
            };
            format!("{label} #{n}")
        })
        .collect()
}

#[repr(C, align(8))]
struct RawBuf([u8; 128]);

const HEADER_SIZE: u32 = std::mem::size_of::<RAWINPUTHEADER>() as u32;

fn store_filter_path(path: &str) -> bool {
    let Ok(mut current) = STATE.filter_path.lock() else {
        return false;
    };
    if current.as_str() == path {
        return false;
    }
    path.clone_into(&mut current);
    true
}

fn post_to_window(msg: u32) {
    let hwnd = STATE.hwnd.load(Ordering::Acquire);
    if hwnd != 0 {
        // SAFETY: pencere bu süreçte; mesaj yalnız kuyruğa eklenir.
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut c_void)), msg, WPARAM(0), LPARAM(0)) };
    }
}

fn thread_main() {
    // SAFETY: yalnız bu thread'in kimliği ve önceliği.
    unsafe {
        STATE
            .thread_id
            .store(GetCurrentThreadId(), Ordering::Release);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
    let Some(hwnd) = create_window() else {
        crate::log::line("Raw Input penceresi oluşturulamadı");
        return;
    };
    STATE.hwnd.store(hwnd.0 as isize, Ordering::Release);
    if !register(hwnd) {
        crate::log::line("Raw Input kaydı başarısız");
    }
    resolve_filter();

    let mut msg = MSG::default();
    // SAFETY: standart mesaj döngüsü; msg yerel. -1 (hata) döngüyü bitirir.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    STATE.hwnd.store(0, Ordering::Release);
}

fn create_window() -> Option<HWND> {
    let class_name = w!("MouseDriveRawInput");
    // SAFETY: sınıf yapısı geçerli ve statik bir sınıf adı kullanıyor; pencere
    // yalnız bu thread'de kullanılır.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(wc.hInstance),
            None,
        )
        .ok()
    }
}

fn register(hwnd: HWND) -> bool {
    let sink = STATE.input_sink.load(Ordering::Acquire);
    let flags = if sink {
        RIDEV_DEVNOTIFY | RIDEV_INPUTSINK
    } else {
        RIDEV_DEVNOTIFY
    };
    let rid = RAWINPUTDEVICE {
        usUsagePage: 0x01,
        usUsage: 0x02,
        dwFlags: flags,
        hwndTarget: hwnd,
    };
    // SAFETY: tek elemanlı geçerli dizi ve doğru yapı boyutu.
    unsafe { RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32) }.is_ok()
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_INPUT => read_raw_input(HRAWINPUT(lparam.0 as *mut c_void)),
        WM_INPUT_DEVICE_CHANGE | WM_APP_FILTER => {
            resolve_filter();
            return LRESULT(0);
        }
        WM_APP_REGISTER => {
            register(hwnd);
            return LRESULT(0);
        }
        _ => {}
    }
    // SAFETY: parametreler pencere prosedürüne gelenlerin aynısı.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

fn read_raw_input(handle: HRAWINPUT) {
    let mut buf = RawBuf([0; 128]);
    let mut size = std::mem::size_of::<RawBuf>() as u32;
    // SAFETY: buf `size` bayt yazılabilir ve hizalı; tek çağrı yeterli.
    let copied = unsafe {
        GetRawInputData(
            handle,
            RID_INPUT,
            Some(buf.0.as_mut_ptr().cast()),
            &mut size,
            HEADER_SIZE,
        )
    };
    if copied == u32::MAX || copied < HEADER_SIZE {
        return;
    }
    // SAFETY: buf 8 bayt hizalı ve GetRawInputData tam bir RAWINPUT yazdı.
    let raw = unsafe { &*(buf.0.as_ptr() as *const RAWINPUT) };
    if raw.header.dwType != RIM_TYPEMOUSE.0 {
        return;
    }
    // SAFETY: tip fare olduğu için birliğin `mouse` alanı geçerli.
    let mouse = unsafe { &raw.data.mouse };
    // SAFETY: usButtonFlags her fare kaydında dolu.
    let buttons = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
    STATE.process(
        raw.header.hDevice.0 as isize,
        mouse.usFlags.0,
        mouse.lLastX,
        mouse.lLastY,
        buttons,
    );
}

fn resolve_filter() {
    let path = STATE
        .filter_path
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();
    let (accepted, missing) = if path.is_empty() {
        (0, false)
    } else {
        let found = mouse_handles()
            .into_iter()
            .find(|&h| device_path(h).is_some_and(|p| p.eq_ignore_ascii_case(&path)));
        match found {
            Some(h) => (h.0 as isize, false),
            None => (0, true),
        }
    };
    let previous = STATE.accepted_device.swap(accepted, Ordering::AcqRel);
    STATE.filter_missing.store(missing, Ordering::Release);
    if previous != accepted {
        STATE.left.store(false, Ordering::Release);
        STATE.right.store(false, Ordering::Release);
    }
}

fn registered_devices() -> Vec<RAWINPUTDEVICE> {
    let size = std::mem::size_of::<RAWINPUTDEVICE>() as u32;
    let mut count = 0u32;
    // SAFETY: yalnız sayı sorgusu.
    unsafe { GetRegisteredRawInputDevices(None, &mut count, size) };
    let mut list = vec![RAWINPUTDEVICE::default(); count as usize];
    if list.is_empty() {
        return list;
    }
    // SAFETY: liste `count` eleman kapasiteli.
    let n = unsafe { GetRegisteredRawInputDevices(Some(list.as_mut_ptr()), &mut count, size) };
    if n == u32::MAX {
        return Vec::new();
    }
    list.truncate(n as usize);
    list
}

fn mouse_handles() -> Vec<HANDLE> {
    let size = std::mem::size_of::<RAWINPUTDEVICELIST>() as u32;
    let mut count = 0u32;
    // SAFETY: yalnız sayı sorgusu.
    if unsafe { GetRawInputDeviceList(None, &mut count, size) } == u32::MAX {
        return Vec::new();
    }
    let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
    if list.is_empty() {
        return Vec::new();
    }
    // SAFETY: liste `count` eleman kapasiteli.
    let n = unsafe { GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut count, size) };
    if n == u32::MAX {
        return Vec::new();
    }
    list.truncate(n as usize);
    list.into_iter()
        .filter(|d| d.dwType == RIM_TYPEMOUSE)
        .map(|d| d.hDevice)
        .collect()
}

fn device_path(handle: HANDLE) -> Option<String> {
    let mut len = 0u32;
    // SAFETY: yalnız uzunluk sorgusu (karakter sayısı).
    unsafe { GetRawInputDeviceInfoW(Some(handle), RIDI_DEVICENAME, None, &mut len) };
    if len == 0 || len > 1024 {
        return None;
    }
    let mut buf = vec![0u16; len as usize];
    // SAFETY: buf `len` karakter yazılabilir.
    let written = unsafe {
        GetRawInputDeviceInfoW(
            Some(handle),
            RIDI_DEVICENAME,
            Some(buf.as_mut_ptr().cast()),
            &mut len,
        )
    };
    if written == 0 || written == u32::MAX {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

fn product_name(path: &str) -> Option<String> {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: wide sıfır sonlu; 0 erişim hakkıyla açmak yalnız sorguya izin verir.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .ok()?;
    let mut buf = [0u16; 127];
    // SAFETY: buf'ın bayt boyutu verilir; tanıtıcı hemen kapatılır.
    let ok = unsafe {
        let ok = HidD_GetProductString(
            handle,
            buf.as_mut_ptr().cast(),
            std::mem::size_of_val(&buf) as u32,
        );
        let _ = CloseHandle(handle);
        ok
    };
    if !ok {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}
