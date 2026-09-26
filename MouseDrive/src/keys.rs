#[cfg(target_os = "linux")]
pub(crate) mod linux;
#[cfg(windows)]
mod win;

#[cfg(target_os = "linux")]
use linux as sys;
#[cfg(windows)]
use win as sys;

#[cfg(target_os = "linux")]
pub use linux::set_app_focused;
pub use sys::{app_is_foreground, buttons_swapped, is_key_down};
#[cfg(windows)]
pub use win::key_name_lparam;

pub const VK_LBUTTON: i32 = 0x01;
pub const VK_RBUTTON: i32 = 0x02;
pub const VK_MBUTTON: i32 = 0x04;
pub const VK_XBUTTON1: i32 = 0x05;
pub const VK_XBUTTON2: i32 = 0x06;
pub const VK_ESCAPE: i32 = 0x1B;

pub const UNBOUND: i32 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyLabel {
    Unbound,
    Mouse(u8),
    Key(String),
}

pub fn is_bindable(vk: i32) -> bool {
    matches!(vk, 0x03..=0xFE)
        && !matches!(
            vk,
            0x07 | 0x0A | 0x0B | 0x0E | 0x0F | 0x10..=0x12 | VK_ESCAPE
        )
}

pub fn first_pressed_key() -> Option<i32> {
    (0x03..=0xFE)
        .filter(|&vk| is_bindable(vk))
        .find(|&vk| is_key_down(vk))
}

pub fn mouse_button_number(vk: i32) -> Option<u8> {
    match vk {
        VK_MBUTTON => Some(3),
        VK_XBUTTON1 => Some(4),
        VK_XBUTTON2 => Some(5),
        _ => None,
    }
}

pub fn key_label(vk: i32) -> KeyLabel {
    if vk == UNBOUND {
        return KeyLabel::Unbound;
    }
    if let Some(n) = mouse_button_number(vk) {
        return KeyLabel::Mouse(n);
    }
    KeyLabel::Key(sys::keyboard_key_name(vk).unwrap_or_else(|| format!("VK {vk:#04X}")))
}
