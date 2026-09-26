use windows::Win32::System::Threading::GetCurrentProcessId;
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
