use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicUsize, Ordering};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
    DeleteObject, DrawTextW, Ellipse, EndPaint, FW_SEMIBOLD, FillRect, GetDC, GetDeviceCaps,
    GetStockObject, HDC, InvalidateRect, LOGPIXELSX, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    ReleaseDC, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, LWA_COLORKEY, MA_NOACTIVATE, MSG, PM_NOREMOVE,
    PeekMessageW, PostThreadMessageW, RegisterClassW, SPI_GETWORKAREA, SW_HIDE, SWP_NOACTIVATE,
    SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetLayeredWindowAttributes, SetTimer,
    SetWindowPos, ShowWindow, SystemParametersInfoW, TranslateMessage, WM_APP, WM_DISPLAYCHANGE,
    WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_PAINT, WM_QUIT, WM_SETTINGCHANGE, WM_TIMER,
    WM_USER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::status::{AppStatus, status_rgb};

const BASE_WIDTH: i32 = 190;
const BASE_HEIGHT: i32 = 28;
const BASE_MARGIN: i32 = 12;
const ALPHA: u8 = 230;
const TOPMOST_REFRESH_MS: u32 = 2000;
const TIMER_ID: usize = 1;

const KEY: COLORREF = rgb([255, 0, 255]);
const BACKGROUND: COLORREF = rgb([28, 28, 32]);
const TEXT: COLORREF = rgb([240, 240, 240]);

const WM_APP_CONFIG: u32 = WM_APP + 1;
const WM_APP_REDRAW: u32 = WM_APP + 2;

const fn rgb(c: [u8; 3]) -> COLORREF {
    COLORREF(c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16)
}

struct OverlayState {
    thread_id: AtomicU32,
    enabled: AtomicBool,
    corner: AtomicI32,
    status: AtomicUsize,
    labels: Mutex<[String; AppStatus::ALL.len()]>,
}

static STATE: OverlayState = OverlayState {
    thread_id: AtomicU32::new(0),
    enabled: AtomicBool::new(false),
    corner: AtomicI32::new(1),
    status: AtomicUsize::new(usize::MAX),
    labels: Mutex::new([const { String::new() }; AppStatus::ALL.len()]),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

pub fn place(work: Area, width: i32, height: i32, corner: i32, margin: i32) -> (i32, i32) {
    let left = work.left + margin;
    let right = work.right - margin - width;
    let top = work.top + margin;
    let bottom = work.bottom - margin - height;
    match corner {
        0 => (left, top),
        2 => (left, bottom),
        3 => (right, bottom),
        _ => (right, top),
    }
}

fn scaled(v: i32, dpi: i32) -> i32 {
    (i64::from(v) * i64::from(dpi.max(96)) / 96) as i32
}

pub fn start() -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("overlay".into())
        .spawn(thread_main)
}

pub fn configure(enabled: bool, corner: i32) {
    let was_enabled = STATE.enabled.swap(enabled, Ordering::AcqRel);
    let old_corner = STATE.corner.swap(corner, Ordering::AcqRel);
    if was_enabled != enabled || old_corner != corner {
        post(WM_APP_CONFIG);
    }
}

pub fn set_status(status: AppStatus) {
    let previous = STATE.status.swap(status.index(), Ordering::AcqRel);
    if previous != status.index() && STATE.enabled.load(Ordering::Acquire) {
        post(WM_APP_REDRAW);
    }
}

pub fn set_labels(labels: [String; AppStatus::ALL.len()]) {
    if let Ok(mut current) = STATE.labels.lock() {
        *current = labels;
    }
    post(WM_APP_REDRAW);
}

pub fn stop() {
    post(WM_QUIT);
}

fn post(msg: u32) {
    let id = STATE.thread_id.load(Ordering::Acquire);
    if id != 0 {
        // SAFETY: yalnız mesaj gönderilir; thread yoksa çağrı başarısız olur.
        let _ = unsafe { PostThreadMessageW(id, msg, WPARAM(0), LPARAM(0)) };
    }
}

fn thread_main() {
    let mut msg = MSG::default();
    // SAFETY: mesaj kuyruğunu oluşturur (PostThreadMessageW için gerekli).
    unsafe {
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        STATE
            .thread_id
            .store(GetCurrentThreadId(), Ordering::Release);
    }
    let mut window: Option<HWND> = None;
    apply_config(&mut window);
    // SAFETY: standart mesaj döngüsü; -1 (hata) ve 0 (WM_QUIT) döngüyü bitirir.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            if msg.hwnd.0.is_null() {
                handle_thread_message(msg.message, &mut window);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(hwnd) = window {
            let _ = DestroyWindow(hwnd);
        }
    }
    STATE.thread_id.store(0, Ordering::Release);
}

fn handle_thread_message(message: u32, window: &mut Option<HWND>) {
    match message {
        WM_APP_CONFIG => apply_config(window),
        WM_APP_REDRAW => {
            if let Some(hwnd) = *window {
                // SAFETY: pencere bu thread'e ait.
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                assert_topmost(hwnd);
            }
        }
        _ => {}
    }
}

fn apply_config(window: &mut Option<HWND>) {
    if !STATE.enabled.load(Ordering::Acquire) {
        if let Some(hwnd) = *window {
            // SAFETY: pencere bu thread'e ait.
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
        return;
    }
    if window.is_none() {
        *window = create_window();
    }
    if let Some(hwnd) = *window {
        reposition(hwnd);
    }
}

fn create_window() -> Option<HWND> {
    let class_name = w!("MouseDriveOverlay");
    // SAFETY: sınıf yapısı geçerli, sınıf adı statik; pencere yalnız bu
    // thread'de kullanılır.
    unsafe {
        let instance = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class_name,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            BASE_WIDTH,
            BASE_HEIGHT,
            None,
            None,
            Some(wc.hInstance),
            None,
        )
        .ok()?;
        let _ = SetLayeredWindowAttributes(hwnd, KEY, ALPHA, LWA_COLORKEY | LWA_ALPHA);
        SetTimer(Some(hwnd), TIMER_ID, TOPMOST_REFRESH_MS, None);
        Some(hwnd)
    }
}

fn system_dpi() -> i32 {
    // SAFETY: ekran DC'si alınır ve hemen bırakılır.
    unsafe {
        let hdc = GetDC(None);
        let dpi = GetDeviceCaps(Some(hdc), LOGPIXELSX);
        ReleaseDC(None, hdc);
        dpi
    }
}

fn work_area() -> Area {
    let mut rc = RECT::default();
    // SAFETY: SPI_GETWORKAREA bir RECT yazar.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&mut rc as *mut RECT).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if !ok {
        rc = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
    }
    Area {
        left: rc.left,
        top: rc.top,
        right: rc.right,
        bottom: rc.bottom,
    }
}

fn reposition(hwnd: HWND) {
    let dpi = system_dpi();
    let (w, h) = (scaled(BASE_WIDTH, dpi), scaled(BASE_HEIGHT, dpi));
    let corner = STATE.corner.load(Ordering::Acquire);
    let (x, y) = place(work_area(), w, h, corner, scaled(BASE_MARGIN, dpi));
    // SAFETY: pencere bu thread'e ait; odak almadan gösterilir.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn assert_topmost(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{SWP_NOMOVE, SWP_NOSIZE};
    if !STATE.enabled.load(Ordering::Acquire) {
        return;
    }
    // SAFETY: pencere bu thread'e ait; konum ve boyut değişmez.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_TIMER => {
            assert_topmost(hwnd);
            LRESULT(0)
        }
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
            if STATE.enabled.load(Ordering::Acquire) {
                reposition(hwnd);
            }
            // SAFETY: işlenmeyen kısım varsayılan işleyiciye.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        // SAFETY: işlenmeyen mesajlar varsayılan işleyiciye.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn current_label(status: AppStatus) -> Vec<u16> {
    let text = STATE
        .labels
        .lock()
        .ok()
        .and_then(|l| l.get(status.index()).cloned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{status:?}"));
    format!("MouseDrive · {text}").encode_utf16().collect()
}

fn paint(hwnd: HWND) {
    let status = AppStatus::from_index(STATE.status.load(Ordering::Acquire));
    let mut ps = PAINTSTRUCT::default();
    let mut rc = RECT::default();
    // SAFETY: WM_PAINT içinde BeginPaint/EndPaint çifti; oluşturulan GDI
    // nesneleri seçimden çıkarıldıktan sonra silinir.
    unsafe {
        let hdc = BeginPaint(hwnd, &mut ps);
        if GetClientRect(hwnd, &mut rc).is_ok() {
            draw(hdc, rc, status);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// # Safety
/// `hdc` geçerli bir boyama DC'si olmalı.
unsafe fn draw(hdc: HDC, rc: RECT, status: AppStatus) {
    let h = rc.bottom - rc.top;
    let d = h / 2;
    let pad = (h - d) / 2;
    let text_rc = RECT {
        left: rc.left + pad * 2 + d,
        top: rc.top,
        right: rc.right - pad,
        bottom: rc.bottom,
    };
    // SAFETY: çağıranın geçerli DC sözleşmesi yardımcılara aynen geçer.
    unsafe {
        draw_pill(hdc, rc, status, pad, d);
        draw_label(hdc, text_rc, h, status);
    }
}

/// # Safety
/// `hdc` geçerli bir boyama DC'si olmalı.
unsafe fn draw_pill(hdc: HDC, rc: RECT, status: AppStatus, pad: i32, d: i32) {
    let h = rc.bottom - rc.top;
    // SAFETY: çağıran geçerli bir DC verir; her nesne seçimden çıkarıldıktan
    // sonra silinir.
    unsafe {
        let key = CreateSolidBrush(KEY);
        FillRect(hdc, &rc, key);
        let _ = DeleteObject(key.into());

        let bg = CreateSolidBrush(BACKGROUND);
        let old_brush = SelectObject(hdc, bg.into());
        let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
        let _ = RoundRect(hdc, rc.left, rc.top, rc.right + 1, rc.bottom + 1, h, h);

        let dot = CreateSolidBrush(rgb(status_rgb(status)));
        SelectObject(hdc, dot.into());
        let _ = Ellipse(
            hdc,
            rc.left + pad,
            rc.top + pad,
            rc.left + pad + d,
            rc.top + pad + d,
        );
        SelectObject(hdc, old_brush);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(bg.into());
        let _ = DeleteObject(dot.into());
    }
}

/// # Safety
/// `hdc` geçerli bir boyama DC'si olmalı.
unsafe fn draw_label(hdc: HDC, mut text_rc: RECT, h: i32, status: AppStatus) {
    // SAFETY: çağıran geçerli bir DC verir; yazı tipi seçimden çıkarıldıktan
    // sonra silinir.
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, TEXT);
        let font = CreateFontW(
            -(h * 15 / 28),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let old_font = SelectObject(hdc, font.into());
        let mut text = current_label(status);
        DrawTextW(
            hdc,
            &mut text,
            &mut text_rc,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
        let _ = DeleteObject(font.into());
    }
}
