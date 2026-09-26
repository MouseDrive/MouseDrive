use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
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

use super::{MouseInfo, STATE, disambiguate, label_for};

const WM_APP_REGISTER: u32 = WM_APP + 1;
const WM_APP_FILTER: u32 = WM_APP + 2;

static HWND_VALUE: AtomicIsize = AtomicIsize::new(0);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);

#[repr(C, align(8))]
struct RawBuf([u8; 128]);

const HEADER_SIZE: u32 = std::mem::size_of::<RAWINPUTHEADER>() as u32;

pub(super) fn spawn() -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("raw-input".into())
        .spawn(thread_main)
}

pub(super) fn stop() {
    let id = THREAD_ID.load(Ordering::Acquire);
    if id != 0 {
        // SAFETY: yalnız bir mesaj gönderilir; thread yoksa çağrı başarısız olur.
        let _ = unsafe { PostThreadMessageW(id, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

pub(super) fn request_register() {
    post_to_window(WM_APP_REGISTER);
}

pub(super) fn filter_changed() {
    post_to_window(WM_APP_FILTER);
}

pub(super) fn registration_ok() -> bool {
    let hwnd = HWND_VALUE.load(Ordering::Acquire);
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

pub(super) fn device_path(handle: isize) -> Option<String> {
    handle_path(HANDLE(handle as *mut c_void))
}

pub(super) fn list_mice() -> Vec<MouseInfo> {
    let paths: Vec<String> = mouse_handles()
        .into_iter()
        .filter_map(handle_path)
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

fn post_to_window(msg: u32) {
    let hwnd = HWND_VALUE.load(Ordering::Acquire);
    if hwnd != 0 {
        // SAFETY: pencere bu süreçte; mesaj yalnız kuyruğa eklenir.
        let _ = unsafe { PostMessageW(Some(HWND(hwnd as *mut c_void)), msg, WPARAM(0), LPARAM(0)) };
    }
}

fn thread_main() {
    // SAFETY: yalnız bu thread'in kimliği ve önceliği.
    unsafe {
        THREAD_ID.store(GetCurrentThreadId(), Ordering::Release);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
    let Some(hwnd) = create_window() else {
        crate::log::line("Raw Input penceresi oluşturulamadı");
        return;
    };
    HWND_VALUE.store(hwnd.0 as isize, Ordering::Release);
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
    HWND_VALUE.store(0, Ordering::Release);
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
        mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 != 0,
        mouse.lLastX,
        mouse.lLastY,
        buttons,
    );
}

fn resolve_filter() {
    STATE.resolve_filter(|path| {
        mouse_handles()
            .into_iter()
            .find(|&h| handle_path(h).is_some_and(|p| p.eq_ignore_ascii_case(path)))
            .map(|h| h.0 as isize)
    });
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

fn handle_path(handle: HANDLE) -> Option<String> {
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
