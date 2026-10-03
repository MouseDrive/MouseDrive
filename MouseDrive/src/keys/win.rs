use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::StationsAndDesktops::{
    CloseDesktop, DESKTOP_CONTROL_FLAGS, DESKTOP_READOBJECTS, GetThreadDesktop,
    GetUserObjectInformationW, HDESK, OpenInputDesktop, UOI_NAME,
};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyNameTextW, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, SM_SWAPBUTTON,
};

pub fn is_key_down(vk: i32) -> bool {
    if !(1..=254).contains(&vk) {
        return false;
    }
    // SAFETY: GetAsyncKeyState'in önkoşulu yok; vk 1..=254 aralığında.
    let state = unsafe { GetAsyncKeyState(vk) };
    (state as u16 & 0x8000) != 0
}

pub fn key_name_lparam(scan_ex: u32) -> i32 {
    let extended = (scan_ex & 0xFF00) == 0xE000 || (scan_ex & 0xFF00) == 0xE100;
    let scan = (scan_ex & 0xFF) as i32;
    (scan << 16) | if extended { 1 << 24 } else { 0 }
}

pub(super) fn keyboard_key_name(vk: i32) -> Option<String> {
    let vk = u32::try_from(vk).ok()?;
    // SAFETY: saf çeviri sorgusu.
    let scan_ex = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX) };
    if scan_ex == 0 {
        return None;
    }
    let mut buf = [0u16; 64];
    // SAFETY: buf yazılabilir; uzunluğu dilimden alınır.
    let len = unsafe { GetKeyNameTextW(key_name_lparam(scan_ex), &mut buf) };
    let len = usize::try_from(len).ok().filter(|&n| n > 0)?;
    Some(String::from_utf16_lossy(&buf[..len.min(buf.len())]))
}

pub fn buttons_swapped() -> bool {
    // SAFETY: salt okuma sorgusu.
    unsafe { GetSystemMetrics(SM_SWAPBUTTON) != 0 }
}

pub fn app_is_foreground() -> bool {
    // SAFETY: salt okuma sorguları; pid yerel bir değişkene yazılır.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

pub fn input_desktop_active() -> bool {
    // SAFETY: girdi masaüstü yalnız ad sorgusu için açılır ve hemen kapatılır.
    let opened = unsafe { OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS) };
    let Ok(input) = opened else {
        return false;
    };
    let input_name = desktop_name(input);
    // SAFETY: tutamaç yukarıda açıldı ve başka yerde kullanılmıyor. Kapatma
    // başarısız olursa yapılacak bir şey yok; bedeli sızan bir tutamaç.
    let _ = unsafe { CloseDesktop(input) };
    // SAFETY: thread'in masaüstü tutamacı sistemindir, kapatılmaz.
    let own = unsafe { GetThreadDesktop(GetCurrentThreadId()) }
        .ok()
        .and_then(desktop_name);
    input_name.is_some() && input_name == own
}

fn desktop_name(desktop: HDESK) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: buf yazılabilir; uzunluk bayt cinsinden verilir.
    unsafe {
        GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(buf.as_mut_ptr().cast()),
            std::mem::size_of_val(&buf) as u32,
            None,
        )
    }
    .ok()?;
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}
