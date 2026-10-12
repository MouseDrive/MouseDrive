use std::collections::BTreeSet;
use std::path::Path;

use evdev::{AbsoluteAxisCode, BusType, KeyCode, RelativeAxisCode};

use super::super::{Class, Skip};
use crate::keys::linux as keys;

const OWN_DEVICE_PREFIX: &str = "MouseDrive";
const UINPUT_DEVICES: &str = "/devices/virtual/input/";
const PHYS_INPUT: &str = "/input";
const WORD_BITS: usize = libc::c_ulong::BITS as usize;
const BUS_VIRTUAL: u16 = BusType::BUS_VIRTUAL.0;
const BTN_LEFT: u16 = KeyCode::BTN_LEFT.0;
const BTN_TOOL_FINGER: u16 = KeyCode::BTN_TOOL_FINGER.0;
const BTN_GAMEPAD: u16 = KeyCode::BTN_SOUTH.0;
const REL_X: u16 = RelativeAxisCode::REL_X.0;
const REL_Y: u16 = RelativeAxisCode::REL_Y.0;
const ABS_X: u16 = AbsoluteAxisCode::ABS_X.0;
const ABS_MT_SLOT: u16 = AbsoluteAxisCode::ABS_MT_SLOT.0;
const KEY_ESC: u16 = KeyCode::KEY_ESC.0;
const KEY_S: u16 = KeyCode::KEY_S.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DeviceCaps {
    pub(super) name: String,
    pub(super) bus: u16,
    pub(super) vendor: u16,
    pub(super) product: u16,
    pub(super) keys: BTreeSet<u16>,
    pub(super) rel: BTreeSet<u16>,
    pub(super) abs: BTreeSet<u16>,
    pub(super) props: BTreeSet<u16>,
    pub(super) phys: String,
    pub(super) uniq: String,
    pub(super) synthetic: bool,
}

impl DeviceCaps {
    pub(super) fn from_sysfs(dir: &Path) -> Option<Self> {
        let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
        let text = |file: &str| read(file).map(|s| s.trim().to_string()).unwrap_or_default();
        let hex = |file: &str| read(file).and_then(|s| parse_hex(&s)).unwrap_or(0);
        let bits = |file: &str| read(file).map(|s| parse_bitmap(&s)).unwrap_or_default();
        let name = read("name")?.trim().to_string();
        Some(Self {
            name,
            bus: hex("id/bustype"),
            vendor: hex("id/vendor"),
            product: hex("id/product"),
            keys: bits("capabilities/key"),
            rel: bits("capabilities/rel"),
            abs: bits("capabilities/abs"),
            props: bits("properties"),
            phys: text("phys"),
            uniq: text("uniq"),
            synthetic: dir.canonicalize().is_ok_and(|p| is_synthetic_path(&p)),
        })
    }

    pub(super) fn is_virtual(&self) -> bool {
        self.bus == BUS_VIRTUAL || self.synthetic
    }

    fn is_gamepad(&self) -> bool {
        self.keys.contains(&BTN_GAMEPAD)
    }

    fn is_typing_keyboard(&self) -> bool {
        (KEY_ESC..=KEY_S).all(|key| self.keys.contains(&key))
    }

    fn has_mapped_key(&self) -> bool {
        self.keys
            .iter()
            .any(|&key| keys::vk_for_code(key).is_some())
    }

    fn pointer_class(&self) -> Option<Class> {
        if !self.keys.contains(&BTN_LEFT) {
            return None;
        }
        if self.rel.contains(&REL_X) && self.rel.contains(&REL_Y) {
            return Some(Class::Mouse);
        }
        if !self.abs.contains(&ABS_X) {
            return None;
        }
        if self.abs.contains(&ABS_MT_SLOT) || self.keys.contains(&BTN_TOOL_FINGER) {
            Some(Class::Touchpad)
        } else {
            Some(Class::AbsolutePointer)
        }
    }

    fn shares_parent(&self, other: &DeviceCaps) -> bool {
        let uniq = self.uniq.trim();
        let same_uniq = !uniq.is_empty() && uniq.eq_ignore_ascii_case(other.uniq.trim());
        same_uniq || phys_parent(&self.phys).is_some_and(|p| phys_parent(&other.phys) == Some(p))
    }
}

pub(super) fn is_synthetic_path(path: &Path) -> bool {
    path.to_string_lossy().contains(UINPUT_DEVICES)
}

pub(super) fn classify(caps: &DeviceCaps) -> Result<Class, Skip> {
    if caps.name.trim().starts_with(OWN_DEVICE_PREFIX) {
        return Err(Skip::OwnDevice);
    }
    if let Some(class) = caps.pointer_class() {
        return Ok(class);
    }
    if caps.is_gamepad() {
        return Err(Skip::Gamepad);
    }
    if caps.is_typing_keyboard() {
        return Ok(Class::Keyboard);
    }
    if caps.has_mapped_key() {
        return Ok(Class::OtherKeys);
    }
    Err(Skip::NotPointerNoKeys)
}

pub(super) fn classify_among(caps: &DeviceCaps, all: &[DeviceCaps]) -> Result<Class, Skip> {
    let class = classify(caps)?;
    let gamepad_part = class == Class::Touchpad
        && all
            .iter()
            .any(|other| other.is_gamepad() && caps.shares_parent(other));
    if gamepad_part {
        return Err(Skip::GamepadTouchpad);
    }
    Ok(class)
}

fn phys_parent(phys: &str) -> Option<&str> {
    phys.trim()
        .rsplit_once(PHYS_INPUT)
        .map(|(parent, _)| parent)
        .filter(|parent| !parent.is_empty())
}

fn parse_hex(text: &str) -> Option<u16> {
    u16::from_str_radix(text.trim(), 16).ok()
}

fn parse_bitmap(text: &str) -> BTreeSet<u16> {
    let words: Result<Vec<u64>, _> = text
        .split_whitespace()
        .rev()
        .map(|word| u64::from_str_radix(word, 16))
        .collect();
    let Ok(words) = words else {
        return BTreeSet::new();
    };
    words
        .iter()
        .enumerate()
        .flat_map(|(i, &word)| {
            (0..WORD_BITS)
                .filter(move |&bit| word >> bit & 1 == 1)
                .map(move |bit| i * WORD_BITS + bit)
        })
        .filter_map(|bit| u16::try_from(bit).ok())
        .collect()
}
