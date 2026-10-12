#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use eframe::egui::{CollapsingHeader, Frame, Id, Label, RichText, ScrollArea, TextWrapMode, Ui};
use mousedrive::config::Config;
use mousedrive::input::{self, Class, DeviceReport, Skip};
use mousedrive::keys;
use mousedrive::setup::udev::{
    self, KEYBOARD_RULE, KEYBOARD_RULE_FILE, MOUSE_RULE, OWN_RULE, OwnRule, RulesFacts, STEAM_RULE,
    UINPUT_RULE, WheelAccess,
};
use mousedrive::setup::{CheckState, SetupReport};

use super::super::widgets::Cx;
use super::Row;
use crate::lang::{Strings, fill};

const COPIED_VISIBLE_S: f64 = 3.0;
const WHEEL_REFRESH_S: f64 = 1.0;
const WHEEL_SETTLE_REFRESH_S: f64 = 0.2;
const RULES_REFRESH_S: f64 = 3.0;
const RULES_DIR: &str = "/etc/udev/rules.d";
const MODULES_LOAD_FILE: &str = "/etc/modules-load.d/mousedrive.conf";
const RELOAD_RULES: &str = "sudo udevadm control --reload-rules";
const TRIGGER_ALL: &str = "sudo udevadm trigger --subsystem-match=misc --subsystem-match=input";
const TRIGGER_INPUT: &str = "sudo udevadm trigger --subsystem-match=input";
const NIX_RULES_FILE: &str = "/lib/udev/rules.d/70-mousedrive.rules";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub explain: &'static str,
    pub commands: Vec<String>,
}

fn write_rules(lines: &[&str], file: &str) -> String {
    let quoted: String = lines
        .iter()
        .map(|line| format!("  '{line}' \\\n"))
        .collect();
    format!("printf '%s\\n' \\\n{quoted}  | sudo tee {RULES_DIR}/{file}")
}

pub fn setup_steps(s: &Strings) -> Vec<Step> {
    vec![
        Step {
            explain: s.step_uinput,
            commands: vec![
                "sudo modprobe uinput".into(),
                format!("echo uinput | sudo tee {MODULES_LOAD_FILE}"),
            ],
        },
        Step {
            explain: s.step_rule,
            commands: vec![write_rules(&[UINPUT_RULE, MOUSE_RULE], OWN_RULE)],
        },
        Step {
            explain: s.step_apply,
            commands: vec![RELOAD_RULES.into(), TRIGGER_ALL.into()],
        },
    ]
}

pub fn keyboard_steps(s: &Strings) -> Vec<Step> {
    vec![
        Step {
            explain: s.step_keyboard_rule,
            commands: vec![write_rules(&[KEYBOARD_RULE], KEYBOARD_RULE_FILE)],
        },
        Step {
            explain: s.step_keyboard_apply,
            commands: vec![RELOAD_RULES.into(), TRIGGER_INPUT.into()],
        },
    ]
}

pub fn script(steps: &[Step]) -> String {
    steps
        .iter()
        .flat_map(|step| &step.commands)
        .map(|command| format!("{command}\n"))
        .collect()
}

pub fn nixos_snippet() -> String {
    [
        "boot.kernelModules = [ \"uinput\" ];".to_string(),
        "services.udev.packages = [".into(),
        "  (pkgs.writeTextFile {".into(),
        "    name = \"mousedrive-udev-rules\";".into(),
        format!("    destination = \"{NIX_RULES_FILE}\";"),
        "    text = ''".into(),
        format!("      {UINPUT_RULE}"),
        format!("      {MOUSE_RULE}"),
        "    '';".into(),
        "  })".into(),
        "];".into(),
    ]
    .map(|line| line + "\n")
    .concat()
}

pub fn binds_keyboard_keys(cfg: &Config) -> bool {
    let keyboard = |vk: i32| vk != keys::UNBOUND && keys::mouse_button_number(vk).is_none();
    let gears = cfg.gear_keys_enabled && (keyboard(cfg.gear_up_key) || keyboard(cfg.gear_down_key));
    keyboard(cfg.capture_toggle_key) || keyboard(cfg.ab_toggle_key) || gears
}

#[derive(Default)]
pub struct Live {
    node: Option<PathBuf>,
    wheel: Option<WheelAccess>,
    rules: RulesFacts,
    wheel_at: Option<f64>,
    rules_at: Option<f64>,
}

fn due(last: Option<f64>, now: f64, every: f64) -> bool {
    last.is_none_or(|t| now - t >= every || now < t)
}

impl Live {
    fn wheel_interval(&self) -> f64 {
        match self.wheel {
            Some(WheelAccess::Ready) | None => WHEEL_REFRESH_S,
            Some(_) => WHEEL_SETTLE_REFRESH_S,
        }
    }

    pub fn refresh(&mut self, now: f64, node: Option<&Path>) {
        if self.node.as_deref() != node || due(self.wheel_at, now, self.wheel_interval()) {
            self.node = node.map(Path::to_path_buf);
            self.wheel = node.map(udev::check_wheel);
            self.wheel_at = Some(now);
        }
        if due(self.rules_at, now, RULES_REFRESH_S) {
            self.rules = udev::scan_rules();
            self.rules_at = Some(now);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PointerAccess {
    pub readable: bool,
    pub denied: Vec<String>,
    pub touchpads: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardAccess {
    pub in_use: usize,
    pub denied: Vec<String>,
    pub needed: bool,
}

fn is_key_class(class: Class) -> bool {
    matches!(class, Class::Keyboard | Class::OtherKeys)
}

fn denied_labels(devices: &[DeviceReport], wanted: fn(Class) -> bool) -> Vec<String> {
    devices
        .iter()
        .filter(|d| d.denied_as(wanted))
        .map(DeviceReport::label)
        .collect()
}

pub fn pointer_access(readable: bool, devices: &[DeviceReport]) -> PointerAccess {
    PointerAccess {
        readable,
        denied: denied_labels(devices, |c| c.is_pointer() && c != Class::Touchpad),
        touchpads: denied_labels(devices, |c| c == Class::Touchpad),
    }
}

pub fn keyboard_access(devices: &[DeviceReport], needed: bool) -> KeyboardAccess {
    KeyboardAccess {
        in_use: devices
            .iter()
            .filter(|d| d.class == Some(Class::Keyboard) && d.skip.is_none())
            .count(),
        denied: denied_labels(devices, is_key_class),
        needed,
    }
}

fn row(state: CheckState, title: &str, detail: String, fix: Option<&str>) -> Row {
    Row {
        state,
        title: title.into(),
        detail,
        fix: fix.map(Into::into),
    }
}

pub fn wheel_row(access: Option<WheelAccess>, node: Option<&Path>, s: &Strings) -> Row {
    let (Some(access), Some(node)) = (access, node) else {
        return row(
            CheckState::NotRun,
            s.chk_wheel_access,
            s.chk_not_run.into(),
            None,
        );
    };
    let node = node.display().to_string();
    let detail = |template: &str| fill(template, &[("node", &node)]);
    let (state, detail, fix) = match access {
        WheelAccess::Ready => (CheckState::Pass, detail(s.wheel_ready), None),
        WheelAccess::Pending => (CheckState::Warn, detail(s.wheel_pending), None),
        WheelAccess::NotJoystick => (CheckState::Warn, detail(s.wheel_not_joystick), None),
        WheelAccess::NoSessionAccess { tagged } => {
            let text = if tagged {
                s.wheel_no_seat
            } else {
                s.wheel_no_tag
            };
            (CheckState::Warn, detail(text), Some(s.fix_wheel_access))
        }
    };
    row(state, s.chk_wheel_access, detail, fix)
}

pub fn mouse_access_row(p: &PointerAccess, s: &Strings) -> Row {
    let state = match (p.readable, p.denied.is_empty()) {
        (true, true) => CheckState::Pass,
        (true, false) => CheckState::Warn,
        (false, _) => CheckState::Fail,
    };
    let (detail, fix) = if !p.denied.is_empty() {
        let mice = p.denied.join(", ");
        (
            fill(s.mouse_access_denied, &[("mice", &mice)]),
            Some(s.fix_mouse_access),
        )
    } else if p.readable && p.touchpads.is_empty() {
        (String::new(), None)
    } else if p.readable {
        (s.mouse_access_touchpad_skipped.into(), None)
    } else if !p.touchpads.is_empty() {
        let touchpads = p.touchpads.join(", ");
        (
            fill(s.mouse_access_touchpad_only, &[("touchpads", &touchpads)]),
            Some(s.fix_touchpad_only),
        )
    } else {
        (s.mouse_access_none.into(), None)
    };
    row(state, s.chk_mouse_access, detail, fix)
}

pub fn keyboard_access_row(k: &KeyboardAccess, s: &Strings) -> Row {
    let keyboards = k.denied.join(", ");
    let (state, detail, fix) = match (k.denied.is_empty(), k.needed) {
        (false, true) => (
            CheckState::Warn,
            fill(s.keyboard_access_denied, &[("keyboards", &keyboards)]),
            Some(s.fix_keyboard_access),
        ),
        (false, false) => (CheckState::Pass, s.keyboard_not_needed.into(), None),
        (true, _) if k.in_use > 0 => (CheckState::Pass, String::new(), None),
        (true, true) => (CheckState::Warn, s.keyboard_access_none.into(), None),
        (true, false) => (CheckState::Pass, s.keyboard_access_none.into(), None),
    };
    row(state, s.chk_keyboard_access, detail, fix)
}

pub fn setup_rule_row(rules: &RulesFacts, s: &Strings) -> Option<Row> {
    (rules.own == OwnRule::Broad).then(|| {
        row(
            CheckState::Warn,
            s.chk_setup_rule,
            s.setup_rule_broad.into(),
            Some(s.fix_setup_rule_broad),
        )
    })
}

fn rule_label(name: &str) -> String {
    if name == STEAM_RULE {
        format!("{name} (Steam)")
    } else {
        name.to_string()
    }
}

pub fn uinput_note(report: &SetupReport, rules: &RulesFacts, s: &Strings) -> Option<String> {
    if report.driver_enabled != Some(true) {
        return None;
    }
    let others = rules.uinput_from_others()?;
    let labels: Vec<String> = others.iter().map(|name| rule_label(name)).collect();
    Some(fill(s.uinput_via_rules, &[("rules", &labels.join(", "))]))
}

pub struct Facts<'a> {
    pub report: &'a SetupReport,
    pub live: &'a Live,
    pub pointer: PointerAccess,
    pub keyboard: KeyboardAccess,
}

impl<'a> Facts<'a> {
    pub fn read(report: &'a SetupReport, live: &'a Live, keyboard_keys: bool) -> Self {
        let devices = input::devices();
        Self {
            report,
            live,
            pointer: pointer_access(input::registration_ok(), &devices),
            keyboard: keyboard_access(&devices, keyboard_keys),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveRows {
    pub rows: Vec<Row>,
    pub note: Option<String>,
    pub setup_open: bool,
    pub keyboard_open: bool,
}

pub fn live_rows(f: &Facts, s: &Strings) -> LiveRows {
    let wheel = wheel_row(f.live.wheel, f.live.node.as_deref(), s);
    let mouse = mouse_access_row(&f.pointer, s);
    let keyboard = keyboard_access_row(&f.keyboard, s);
    let rule = setup_rule_row(&f.live.rules, s);
    let setup_open = f.report.has_failures() || !f.pointer.denied.is_empty() || rule.is_some();
    let keyboard_open = keyboard.state == CheckState::Warn && rule.is_none();
    LiveRows {
        rows: [Some(wheel), Some(mouse), Some(keyboard), rule]
            .into_iter()
            .flatten()
            .collect(),
        note: uinput_note(f.report, &f.live.rules, s),
        setup_open,
        keyboard_open,
    }
}

pub fn mouse_blocked() -> bool {
    !input::registration_ok() && !input::denied_mice().is_empty()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceTone {
    InUse,
    Denied,
    Unused,
}

fn setup_grants(class: Option<Class>) -> bool {
    matches!(
        class,
        Some(Class::Mouse | Class::AbsolutePointer | Class::Keyboard | Class::OtherKeys)
    )
}

pub fn device_line(d: &DeviceReport, s: &Strings) -> (String, DeviceTone) {
    let class = d.class.map(|c| s.device_class(c));
    let status = d
        .skip
        .map_or_else(|| s.dev_in_use.to_string(), |skip| s.device_skip(skip));
    let tone = match d.skip {
        None => DeviceTone::InUse,
        Some(Skip::OpenFailed(_)) => DeviceTone::Denied,
        Some(Skip::PermissionDenied) if setup_grants(d.class) => DeviceTone::Denied,
        Some(_) => DeviceTone::Unused,
    };
    let text = match class {
        Some(class) => format!("{} · {class} · {status}", d.name),
        None => format!("{} · {status}", d.name),
    };
    (text, tone)
}

pub fn device_list(ui: &mut Ui, cx: &Cx) {
    let devices: Vec<DeviceReport> = input::devices()
        .into_iter()
        .filter(|d| d.class.is_some())
        .collect();
    let title = fill(cx.s.setup_devices, &[("n", &devices.len().to_string())]);
    CollapsingHeader::new(title)
        .id_salt("setup_devices")
        .show(ui, |ui| {
            for d in &devices {
                let (text, tone) = device_line(d, cx.s);
                let text = match tone {
                    DeviceTone::InUse => RichText::new(text).color(cx.pal.ok),
                    DeviceTone::Denied => RichText::new(text).color(ui.visuals().warn_fg_color),
                    DeviceTone::Unused => RichText::new(text).weak(),
                };
                ui.label(text).on_hover_text(&d.node);
            }
        });
}

pub fn note(ui: &mut Ui, note: Option<&str>) {
    if let Some(note) = note {
        ui.label(RichText::new(note).weak());
    }
}

pub fn commands(ui: &mut Ui, cx: &Cx, rows: &LiveRows) {
    let s = cx.s;
    CollapsingHeader::new(s.setup_commands)
        .id_salt(("setup_commands", rows.setup_open))
        .default_open(rows.setup_open)
        .show(ui, |ui| {
            ui.label(RichText::new(s.setup_commands_intro).weak());
            let steps = setup_steps(s);
            steps_ui(ui, "setup_step", &steps);
            copy_row(ui, cx, s.btn_setup_tool, &script(&steps), "setup_copied");
        });
    CollapsingHeader::new(s.keyboard_tier)
        .id_salt(("keyboard_tier", rows.keyboard_open))
        .default_open(rows.keyboard_open)
        .show(ui, |ui| {
            let warn = ui.visuals().warn_fg_color;
            ui.label(RichText::new(s.keyboard_tier_warning).strong().color(warn));
            ui.label(s.keyboard_tier_why);
            let steps = keyboard_steps(s);
            steps_ui(ui, "keyboard_step", &steps);
            copy_row(
                ui,
                cx,
                s.btn_copy_keyboard,
                &script(&steps),
                "keyboard_copied",
            );
        });
    CollapsingHeader::new(s.nixos_title)
        .id_salt("setup_nixos")
        .show(ui, |ui| {
            ui.label(s.nixos_note);
            code_block(ui, Id::new("setup_nixos_code"), &nixos_snippet());
        });
}

fn steps_ui(ui: &mut Ui, salt: &str, steps: &[Step]) {
    for (i, step) in steps.iter().enumerate() {
        ui.add_space(4.0);
        ui.label(format!("{}. {}", i + 1, step.explain));
        code_block(ui, Id::new((salt, i)), &script(std::slice::from_ref(step)));
    }
}

fn code_block(ui: &mut Ui, id: Id, text: &str) {
    Frame::group(ui.style())
        .fill(ui.visuals().extreme_bg_color)
        .show(ui, |ui| {
            ScrollArea::horizontal().id_salt(id).show(ui, |ui| {
                let text = RichText::new(text.trim_end()).monospace();
                ui.add(
                    Label::new(text)
                        .selectable(true)
                        .wrap_mode(TextWrapMode::Extend),
                );
            });
        });
}

fn copy_row(ui: &mut Ui, cx: &Cx, label: &str, text: &str, salt: &str) {
    let id = Id::new(salt);
    ui.horizontal_wrapped(|ui| {
        let now = ui.input(|i| i.time);
        if ui.button(label).clicked() {
            ui.ctx().copy_text(text.to_string());
            ui.ctx().data_mut(|d| d.insert_temp(id, now));
        }
        let copied_at: Option<f64> = ui.ctx().data(|d| d.get_temp(id));
        if copied_at.is_some_and(|t| now - t < COPIED_VISIBLE_S) {
            ui.label(RichText::new(cx.s.copied).weak());
        }
    });
}
