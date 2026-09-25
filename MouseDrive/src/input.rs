use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
    TranslateMessage, WINDOW_EX_STYLE, WM_INPUT, WNDCLASSW, WS_POPUP,
};
use windows::core::PCWSTR;

const RI_MOUSE_LEFT_BUTTON_DOWN: u16 = 0x0001;
const RI_MOUSE_LEFT_BUTTON_UP: u16 = 0x0002;
const RI_MOUSE_RIGHT_BUTTON_DOWN: u16 = 0x0004;
const RI_MOUSE_RIGHT_BUTTON_UP: u16 = 0x0008;
const RI_MOUSE_MIDDLE_BUTTON_DOWN: u16 = 0x0010;

pub static MOUSE_DELTA_X: AtomicI64 = AtomicI64::new(0);
pub static LEFT_BUTTON: AtomicBool = AtomicBool::new(false);
pub static RIGHT_BUTTON: AtomicBool = AtomicBool::new(false);
pub static MIDDLE_BUTTON_CLICKED: AtomicBool = AtomicBool::new(false);
pub static RAW_INPUT_HWND: AtomicI64 = AtomicI64::new(0);
pub static RAW_INPUT_THREAD_ID: AtomicU32 = AtomicU32::new(0);
pub static INPUT_SINK_ENABLED: AtomicBool = AtomicBool::new(true);
pub static MOUSE_DELTA_CAP: AtomicI32 = AtomicI32::new(180);

static MOUSE_DPI_SCALE: AtomicU64 = AtomicU64::new(0x3FF0000000000000);

pub fn load_dpi_scale() -> f64 {
    f64::from_bits(MOUSE_DPI_SCALE.load(Ordering::Relaxed))
}
pub fn store_dpi_scale(v: f64) {
    MOUSE_DPI_SCALE.store(v.to_bits(), Ordering::Relaxed);
}

#[repr(C, align(8))]
struct AlignedBuf([u8; 256]);

unsafe extern "system" fn raw_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        unsafe {
            let mut size: u32 = 0;
            let _ = GetRawInputData(
                HRAWINPUT(lparam.0 as *mut c_void),
                RID_INPUT,
                None,
                &mut size,
                std::mem::size_of::<RAWINPUTHEADER>() as u32,
            );

            if size > 0 && (size as usize) <= std::mem::size_of::<AlignedBuf>() {
                let mut buf = AlignedBuf([0u8; 256]);
                let result = GetRawInputData(
                    HRAWINPUT(lparam.0 as *mut c_void),
                    RID_INPUT,
                    Some(buf.0.as_mut_ptr() as *mut c_void),
                    &mut size,
                    std::mem::size_of::<RAWINPUTHEADER>() as u32,
                );

                if result == size {
                    let raw = &*(buf.0.as_ptr() as *const RAWINPUT);
                    if raw.header.dwType == RIM_TYPEMOUSE.0 {
                        let mouse = &raw.data.mouse;

                        if (mouse.usFlags.0 & 0x01) == 0 {
                            let dx = mouse.lLastX;
                            let cap = MOUSE_DELTA_CAP.load(Ordering::Relaxed);
                            let dx_clamped = dx.clamp(-cap, cap);
                            let dpi_scale = load_dpi_scale();
                            let dx_scaled = (dx_clamped as f64 * dpi_scale).round() as i64;
                            let limit = (cap as i64) * 100;
                            let _ = MOUSE_DELTA_X.fetch_update(
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                                |old| Some((old + dx_scaled).clamp(-limit, limit)),
                            );
                        }

                        let bf = mouse.Anonymous.Anonymous.usButtonFlags;
                        if (bf & RI_MOUSE_LEFT_BUTTON_DOWN) != 0 {
                            LEFT_BUTTON.store(true, Ordering::Release);
                        }
                        if (bf & RI_MOUSE_LEFT_BUTTON_UP) != 0 {
                            LEFT_BUTTON.store(false, Ordering::Release);
                        }
                        if (bf & RI_MOUSE_RIGHT_BUTTON_DOWN) != 0 {
                            RIGHT_BUTTON.store(true, Ordering::Release);
                        }
                        if (bf & RI_MOUSE_RIGHT_BUTTON_UP) != 0 {
                            RIGHT_BUTTON.store(false, Ordering::Release);
                        }
                        if (bf & RI_MOUSE_MIDDLE_BUTTON_DOWN) != 0 {
                            MIDDLE_BUTTON_CLICKED.store(true, Ordering::Release);
                        }
                    }
                }
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

pub fn start_raw_input_thread() -> std::thread::JoinHandle<()> {
    std::thread::spawn(|| unsafe {
        RAW_INPUT_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);

        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(2));

        let class_name: Vec<u16> = "RawInputHostWindow\0".encode_utf16().collect();

        let wc = WNDCLASSW {
            lpfnWndProc: Some(raw_wnd_proc),
            hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            hbrBackground: HBRUSH(std::ptr::null_mut()),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
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
        .unwrap_or(HWND(std::ptr::null_mut()));

        RAW_INPUT_HWND.store(hwnd.0 as i64, Ordering::SeqCst);
        register_raw_input(hwnd, INPUT_SINK_ENABLED.load(Ordering::Acquire));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    })
}

pub fn register_raw_input(hwnd: HWND, input_sink: bool) {
    unsafe {
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: if input_sink {
                RIDEV_INPUTSINK
            } else {
                Default::default()
            },
            hwndTarget: hwnd,
        };
        let _ = RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
    }
}

pub fn is_key_down(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } & 0x8000u16 as i16) != 0
}
