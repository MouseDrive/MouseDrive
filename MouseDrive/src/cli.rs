#![deny(unsafe_code)]

use mousedrive::input::DeviceReport;

use crate::lang::{Lang, Strings, fill};

pub const UPDATED_FROM: &str = "--updated-from";
const MAX_VERSION_LEN: usize = 32;
const LOCALE_VARS: [&str; 3] = ["LC_ALL", "LC_MESSAGES", "LANG"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    X11,
    Wayland,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::X11 => "x11",
            Backend::Wayland => "wayland",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Run,
    Help,
    Version,
    ListDevices,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub action: Action,
    pub test_counter: bool,
    pub allow_injected: bool,
    pub allow_root: bool,
    pub backend: Option<Backend>,
    pub updated_from: Option<String>,
    pub unknown: Vec<String>,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Args {
    let mut args = args.into_iter().peekable();
    let mut parsed = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => parsed.action = Action::Help,
            "-V" | "--version" if parsed.action == Action::Run => {
                parsed.action = Action::Version;
            }
            "-V" | "--version" => {}
            "--list-devices" if parsed.action != Action::Help => {
                parsed.action = Action::ListDevices;
            }
            "--list-devices" => {}
            "--x11" => parsed.backend = Some(Backend::X11),
            "--wayland" => parsed.backend = Some(Backend::Wayland),
            "--test-counter" => parsed.test_counter = true,
            "--allow-injected" => parsed.allow_injected = true,
            "--allow-root" => parsed.allow_root = true,
            UPDATED_FROM => {
                if let Some(version) = args.next_if(|v| is_version(v)) {
                    parsed.updated_from = Some(version);
                }
            }
            _ => parsed.unknown.push(arg),
        }
    }
    parsed
}

fn is_version(text: &str) -> bool {
    text.len() <= MAX_VERSION_LEN
        && text.starts_with(|c: char| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

pub fn version_line() -> String {
    format!("MouseDrive {}", env!("CARGO_PKG_VERSION"))
}

pub fn help_text(s: &Strings) -> String {
    fill(s.cli_help, &[("version", env!("CARGO_PKG_VERSION"))])
}

pub fn device_list(devices: &[DeviceReport], s: &Strings) -> String {
    if devices.is_empty() {
        return s.dev_list_empty.to_string();
    }
    let rows: Vec<[String; 6]> = std::iter::once(s.dev_list_columns.map(String::from))
        .chain(devices.iter().map(|d| device_row(d, s)))
        .collect();
    let widths: [usize; 6] =
        std::array::from_fn(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0));
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:<width$}"))
                .collect();
            cells.join("  ").trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn device_row(d: &DeviceReport, s: &Strings) -> [String; 6] {
    [
        d.node.clone(),
        format!("{:04X}:{:04X}", d.vendor, d.product),
        bus_label(d.bus),
        d.class
            .map_or_else(|| "-".to_string(), |c| s.device_class(c).to_string()),
        d.skip
            .map_or_else(|| s.dev_in_use.to_string(), |skip| s.device_skip(skip)),
        d.name.clone(),
    ]
}

fn bus_label(bus: u16) -> String {
    let name = match bus {
        0 => "-",
        0x01 => "pci",
        0x03 => "usb",
        0x05 => "bluetooth",
        0x06 => "virtual",
        0x10 => "isa",
        0x11 => "i8042",
        0x13 => "rs232",
        0x18 => "i2c",
        0x19 => "host",
        0x1C => "spi",
        0x1D => "rmi",
        0x1F => "ishtp",
        _ => return format!("{bus:#04x}"),
    };
    name.to_string()
}

pub fn unknown_warning(s: &Strings, arg: &str) -> String {
    fill(s.cli_unknown_arg, &[("arg", arg)])
}

pub fn env_lang() -> Lang {
    let locale = LOCALE_VARS
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    locale_lang(locale.as_deref())
}

fn locale_lang(locale: Option<&str>) -> Lang {
    let turkish = locale.is_some_and(|l| {
        l.get(..2)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("tr"))
    });
    if turkish { Lang::Tr } else { Lang::En }
}
