#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub const RULES_DIRS: [&str; 5] = [
    "/etc/udev/rules.d",
    "/run/udev/rules.d",
    "/usr/local/lib/udev/rules.d",
    "/usr/lib/udev/rules.d",
    "/lib/udev/rules.d",
];
pub const OWN_RULE: &str = "70-mousedrive.rules";
pub const KEYBOARD_RULE_FILE: &str = "71-mousedrive-keyboard.rules";
pub const STEAM_RULE: &str = "60-steam-input.rules";
pub const UINPUT_RULE: &str =
    r#"KERNEL=="uinput", SUBSYSTEM=="misc", OPTIONS+="static_node=uinput", TAG+="uaccess""#;
pub const MOUSE_RULE: &str =
    r#"SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_MOUSE}=="1", TAG+="uaccess""#;
pub const KEYBOARD_RULE: &str =
    r#"SUBSYSTEM=="input", KERNEL=="event*", ENV{ID_INPUT_KEY}=="1", TAG+="uaccess""#;
const DATA_DIR: &str = "/run/udev/data";
const RULES_SUFFIX: &str = ".rules";
const JOYSTICK_PROPERTY: &str = "E:ID_INPUT_JOYSTICK=1";
const UACCESS_TAGS: [&str; 2] = ["G:uaccess", "Q:uaccess"];
const UACCESS: &str = "TAG+=\"uaccess\"";
const UINPUT_MATCH: &str = "KERNEL==\"uinput\"";
const INPUT_MATCH: &str = "SUBSYSTEM==\"input\"";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NodeTags {
    pub joystick: bool,
    pub uaccess: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelAccess {
    Ready,
    Pending,
    NotJoystick,
    NoSessionAccess { tagged: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OwnRule {
    #[default]
    Missing,
    Narrow,
    Broad,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RulesFacts {
    pub own: OwnRule,
    pub uinput_from: Vec<String>,
}

impl RulesFacts {
    pub fn uinput_from_others(&self) -> Option<&[String]> {
        let own = self.uinput_from.iter().any(|name| name == OWN_RULE);
        (!own && !self.uinput_from.is_empty()).then_some(self.uinput_from.as_slice())
    }
}

pub fn parse_node_data(text: &str) -> NodeTags {
    text.lines()
        .map(str::trim_end)
        .fold(NodeTags::default(), |tags, line| NodeTags {
            joystick: tags.joystick || line == JOYSTICK_PROPERTY,
            uaccess: tags.uaccess || UACCESS_TAGS.contains(&line),
        })
}

pub fn node_data_path(data_dir: &Path, rdev: u64) -> PathBuf {
    data_dir.join(format!("c{}:{}", libc::major(rdev), libc::minor(rdev)))
}

pub fn wheel_access(tags: Option<NodeTags>, readable: bool) -> WheelAccess {
    match tags {
        None => WheelAccess::Pending,
        Some(t) if !t.joystick => WheelAccess::NotJoystick,
        Some(_) if readable => WheelAccess::Ready,
        Some(t) => WheelAccess::NoSessionAccess { tagged: t.uaccess },
    }
}

pub fn check_wheel(node: &Path) -> WheelAccess {
    check_wheel_in(Path::new(DATA_DIR), node)
}

fn check_wheel_in(data_dir: &Path, node: &Path) -> WheelAccess {
    let tags = fs::metadata(node)
        .ok()
        .and_then(|m| fs::read_to_string(node_data_path(data_dir, m.rdev())).ok())
        .map(|text| parse_node_data(&text));
    wheel_access(tags, fs::File::open(node).is_ok())
}

fn rule_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

pub fn grants_uinput(text: &str) -> bool {
    rule_lines(text).any(|line| line.contains(UINPUT_MATCH) && line.contains(UACCESS))
}

fn grants_all_input(line: &str) -> bool {
    line.contains(INPUT_MATCH) && line.contains(UACCESS) && !line.contains('{')
}

pub fn own_rule(text: &str) -> OwnRule {
    let mut lines = rule_lines(text).peekable();
    if lines.peek().is_none() {
        return OwnRule::Missing;
    }
    if lines.any(grants_all_input) {
        OwnRule::Broad
    } else {
        OwnRule::Narrow
    }
}

pub fn effective_rule_files(dirs: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    dirs.iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.ends_with(RULES_SUFFIX).then(|| (name, entry.path()))
        })
        .fold(BTreeMap::new(), |mut files, (name, path)| {
            files.entry(name).or_insert(path);
            files
        })
}

pub fn scan_rules_in(dirs: &[PathBuf]) -> RulesFacts {
    let texts: Vec<(String, String)> = effective_rule_files(dirs)
        .into_iter()
        .filter_map(|(name, path)| fs::read_to_string(path).ok().map(|text| (name, text)))
        .collect();
    let own = texts
        .iter()
        .find(|(name, _)| name == OWN_RULE)
        .map_or(OwnRule::Missing, |(_, text)| own_rule(text));
    let uinput_from = texts
        .into_iter()
        .filter(|(_, text)| grants_uinput(text))
        .map(|(name, _)| name)
        .collect();
    RulesFacts { own, uinput_from }
}

pub fn scan_rules() -> RulesFacts {
    let dirs: Vec<PathBuf> = RULES_DIRS.iter().map(PathBuf::from).collect();
    scan_rules_in(&dirs)
}
