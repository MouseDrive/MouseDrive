use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static HOLDERS: [AtomicU8; 256] = [const { AtomicU8::new(0) }; 256];
static APP_FOCUSED: AtomicBool = AtomicBool::new(false);

struct Key {
    code: u16,
    vk: i32,
    name: &'static str,
}

const fn key(code: u16, vk: i32, name: &'static str) -> Key {
    Key { code, vk, name }
}

const KEYS: &[Key] = &[
    key(1, 0x1B, "Esc"),
    key(2, 0x31, "1"),
    key(3, 0x32, "2"),
    key(4, 0x33, "3"),
    key(5, 0x34, "4"),
    key(6, 0x35, "5"),
    key(7, 0x36, "6"),
    key(8, 0x37, "7"),
    key(9, 0x38, "8"),
    key(10, 0x39, "9"),
    key(11, 0x30, "0"),
    key(12, 0xBD, "-"),
    key(13, 0xBB, "="),
    key(14, 0x08, "Backspace"),
    key(15, 0x09, "Tab"),
    key(16, 0x51, "Q"),
    key(17, 0x57, "W"),
    key(18, 0x45, "E"),
    key(19, 0x52, "R"),
    key(20, 0x54, "T"),
    key(21, 0x59, "Y"),
    key(22, 0x55, "U"),
    key(23, 0x49, "I"),
    key(24, 0x4F, "O"),
    key(25, 0x50, "P"),
    key(26, 0xDB, "["),
    key(27, 0xDD, "]"),
    key(28, 0x0D, "Enter"),
    key(29, 0xA2, "Left Ctrl"),
    key(30, 0x41, "A"),
    key(31, 0x53, "S"),
    key(32, 0x44, "D"),
    key(33, 0x46, "F"),
    key(34, 0x47, "G"),
    key(35, 0x48, "H"),
    key(36, 0x4A, "J"),
    key(37, 0x4B, "K"),
    key(38, 0x4C, "L"),
    key(39, 0xBA, ";"),
    key(40, 0xDE, "'"),
    key(41, 0xC0, "`"),
    key(42, 0xA0, "Left Shift"),
    key(43, 0xDC, "\\"),
    key(44, 0x5A, "Z"),
    key(45, 0x58, "X"),
    key(46, 0x43, "C"),
    key(47, 0x56, "V"),
    key(48, 0x42, "B"),
    key(49, 0x4E, "N"),
    key(50, 0x4D, "M"),
    key(51, 0xBC, ","),
    key(52, 0xBE, "."),
    key(53, 0xBF, "/"),
    key(54, 0xA1, "Right Shift"),
    key(55, 0x6A, "Num *"),
    key(56, 0xA4, "Left Alt"),
    key(57, 0x20, "Space"),
    key(58, 0x14, "Caps Lock"),
    key(59, 0x70, "F1"),
    key(60, 0x71, "F2"),
    key(61, 0x72, "F3"),
    key(62, 0x73, "F4"),
    key(63, 0x74, "F5"),
    key(64, 0x75, "F6"),
    key(65, 0x76, "F7"),
    key(66, 0x77, "F8"),
    key(67, 0x78, "F9"),
    key(68, 0x79, "F10"),
    key(69, 0x90, "Num Lock"),
    key(70, 0x91, "Scroll Lock"),
    key(71, 0x67, "Num 7"),
    key(72, 0x68, "Num 8"),
    key(73, 0x69, "Num 9"),
    key(74, 0x6D, "Num -"),
    key(75, 0x64, "Num 4"),
    key(76, 0x65, "Num 5"),
    key(77, 0x66, "Num 6"),
    key(78, 0x6B, "Num +"),
    key(79, 0x61, "Num 1"),
    key(80, 0x62, "Num 2"),
    key(81, 0x63, "Num 3"),
    key(82, 0x60, "Num 0"),
    key(83, 0x6E, "Num Del"),
    key(86, 0xE2, "<"),
    key(87, 0x7A, "F11"),
    key(88, 0x7B, "F12"),
    key(96, 0x0D, "Enter"),
    key(97, 0xA3, "Right Ctrl"),
    key(98, 0x6F, "Num /"),
    key(99, 0x2C, "Print Screen"),
    key(100, 0xA5, "Right Alt"),
    key(102, 0x24, "Home"),
    key(103, 0x26, "Up"),
    key(104, 0x21, "Page Up"),
    key(105, 0x25, "Left"),
    key(106, 0x27, "Right"),
    key(107, 0x23, "End"),
    key(108, 0x28, "Down"),
    key(109, 0x22, "Page Down"),
    key(110, 0x2D, "Insert"),
    key(111, 0x2E, "Delete"),
    key(113, 0xAD, "Mute"),
    key(114, 0xAE, "Volume Down"),
    key(115, 0xAF, "Volume Up"),
    key(119, 0x13, "Pause"),
    key(125, 0x5B, "Left Super"),
    key(126, 0x5C, "Right Super"),
    key(127, 0x5D, "Menu"),
    key(163, 0xB0, "Next Track"),
    key(164, 0xB3, "Play/Pause"),
    key(165, 0xB1, "Previous Track"),
    key(166, 0xB2, "Stop"),
    key(183, 0x7C, "F13"),
    key(184, 0x7D, "F14"),
    key(185, 0x7E, "F15"),
    key(186, 0x7F, "F16"),
    key(187, 0x80, "F17"),
    key(188, 0x81, "F18"),
    key(189, 0x82, "F19"),
    key(190, 0x83, "F20"),
    key(191, 0x84, "F21"),
    key(192, 0x85, "F22"),
    key(193, 0x86, "F23"),
    key(194, 0x87, "F24"),
    key(0x110, 0x01, "Mouse 1"),
    key(0x111, 0x02, "Mouse 2"),
    key(0x112, 0x04, "Mouse 3"),
    key(0x113, 0x05, "Mouse 4"),
    key(0x114, 0x06, "Mouse 5"),
    key(0x115, 0x06, "Mouse 5"),
    key(0x116, 0x05, "Mouse 4"),
];

pub(crate) fn vk_for_code(code: u16) -> Option<i32> {
    KEYS.iter().find(|k| k.code == code).map(|k| k.vk)
}

fn holder(vk: i32) -> Option<&'static AtomicU8> {
    usize::try_from(vk).ok().and_then(|i| HOLDERS.get(i))
}

pub(crate) fn press(vk: i32) {
    if let Some(h) = holder(vk) {
        let _ = h.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1));
    }
}

pub(crate) fn release(vk: i32) {
    if let Some(h) = holder(vk) {
        let _ = h.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
    }
}

pub fn is_key_down(vk: i32) -> bool {
    (1..=254).contains(&vk) && holder(vk).is_some_and(|h| h.load(Ordering::Acquire) > 0)
}

pub(super) fn keyboard_key_name(vk: i32) -> Option<String> {
    KEYS.iter().find(|k| k.vk == vk).map(|k| k.name.to_string())
}

pub fn buttons_swapped() -> bool {
    false
}

pub fn app_is_foreground() -> bool {
    APP_FOCUSED.load(Ordering::Acquire)
}

pub fn set_app_focused(focused: bool) {
    APP_FOCUSED.store(focused, Ordering::Release);
}
