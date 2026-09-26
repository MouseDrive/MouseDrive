#![deny(unsafe_code)]

use eframe::egui::{
    Button, ComboBox, Context, Id, Key, Modal, RichText, TextEdit, Ui, WidgetInfo, WidgetType,
};
use mousedrive::control::{AbSide, Command, Shared, Snapshot};
use mousedrive::profiles::{MAX_NAME_LEN, NameError, ProfileError, ProfileStore, validate_name};
use mousedrive::session::Session;

use super::notices::Notices;
use super::startup::profile_corrected_notice;
use super::widgets::Cx;
use crate::lang::{Strings, fill};

const DIALOG_WIDTH: f32 = 340.0;
#[cfg(windows)]
const FILE_MANAGER: &str = "explorer";
#[cfg(target_os = "linux")]
const FILE_MANAGER: &str = "xdg-open";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameAction {
    SaveAs,
    Duplicate,
    Rename,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Dialog {
    Name(NameAction),
    Switch(String),
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Primary,
    Secondary,
    Cancel,
}

pub struct Env<'a> {
    pub session: &'a mut Session,
    pub shared: &'a Shared,
    pub snap: &'a Snapshot,
    pub notices: &'a mut Notices,
}

pub struct ProfilesUi {
    store: Option<ProfileStore>,
    names: Vec<String>,
    dialog: Option<Dialog>,
    input: String,
    focus_input: bool,
}

impl ProfilesUi {
    pub fn new(store: Option<ProfileStore>, s: &Strings, notices: &mut Notices) -> Self {
        let mut profiles = Self {
            store,
            names: Vec::new(),
            dialog: None,
            input: String::new(),
            focus_input: false,
        };
        profiles.refresh(s, notices);
        profiles
    }

    pub fn refresh(&mut self, s: &Strings, notices: &mut Notices) {
        let Some(store) = &self.store else {
            return;
        };
        match store.list() {
            Ok(names) => self.names = names,
            Err(e) => notices.error(fill(s.err_profile_list, &[("error", &e.to_string())])),
        }
    }

    pub fn bar(&mut self, ui: &mut Ui, cx: &Cx, env: &mut Env) {
        if self.store.is_none() {
            return;
        }
        self.combo(ui, cx, env);
        let dirty = env.session.is_dirty();
        let save = if dirty {
            format!("{} •", cx.s.btn_save)
        } else {
            cx.s.btn_save.to_string()
        };
        let tip = if dirty {
            cx.s.tip_unsaved
        } else {
            cx.s.btn_save
        };
        if ui
            .add_enabled(dirty, Button::new(save))
            .on_hover_text(tip)
            .clicked()
        {
            self.save_active(cx.s, env.session, env.notices);
        }
        self.menu(ui, cx, env);
        history_buttons(ui, cx, env.session);
        ab_controls(ui, cx, env);
        if env.snap.pending_apply {
            ui.spinner();
            ui.label(RichText::new(cx.s.pending_apply).weak());
        }
    }

    fn combo(&mut self, ui: &mut Ui, cx: &Cx, env: &mut Env) {
        let active = env.session.config().active_profile.clone();
        let mut chosen = None;
        let label = ui.label(cx.s.profile);
        ui.add_enabled_ui(env.snap.ab_side.is_none(), |ui| {
            ComboBox::from_id_salt("profile_combo")
                .selected_text(&active)
                .show_ui(ui, |ui| {
                    for name in &self.names {
                        if ui.selectable_label(*name == active, name).clicked() {
                            chosen = Some(name.clone());
                        }
                    }
                })
                .response
                .labelled_by(label.id);
        });
        if let Some(name) = chosen {
            self.request_switch(name, cx.s, env);
        }
    }

    fn menu(&mut self, ui: &mut Ui, cx: &Cx, env: &mut Env) {
        let s = cx.s;
        let active = env.session.config().active_profile.clone();
        ui.menu_button("…", |ui| {
            let actions = [
                (s.btn_save_as, NameAction::SaveAs),
                (s.btn_duplicate, NameAction::Duplicate),
                (s.btn_rename, NameAction::Rename),
            ];
            for (label, action) in actions {
                if ui.button(label).clicked() {
                    self.open_name_dialog(action, &active);
                    ui.close_menu();
                }
            }
            if ui
                .add_enabled(self.names.len() > 1, Button::new(s.btn_delete))
                .clicked()
            {
                self.dialog = Some(Dialog::Delete);
                ui.close_menu();
            }
            ui.separator();
            let revert = ui.add_enabled(env.session.is_dirty(), Button::new(s.btn_revert));
            if revert.on_hover_text(s.tip_revert).clicked() {
                env.session.revert_to_saved();
                ui.close_menu();
            }
            if ui.button(s.btn_open_folder).clicked() {
                self.open_folder(s, env.notices);
                ui.close_menu();
            }
        });
    }

    fn open_name_dialog(&mut self, action: NameAction, active: &str) {
        self.input = match action {
            NameAction::SaveAs => String::new(),
            NameAction::Duplicate => unique_copy_name(active, &self.names),
            NameAction::Rename => active.to_string(),
        };
        self.focus_input = true;
        self.dialog = Some(Dialog::Name(action));
    }

    fn open_folder(&self, s: &Strings, notices: &mut Notices) {
        let Some(store) = &self.store else {
            return;
        };
        let result = std::fs::create_dir_all(store.dir()).and_then(|()| {
            std::process::Command::new(FILE_MANAGER)
                .arg(store.dir())
                .spawn()
        });
        if let Err(e) = result {
            notices.error(fill(s.err_launch, &[("error", &e.to_string())]));
        }
    }

    pub fn save_active(
        &mut self,
        s: &Strings,
        session: &mut Session,
        notices: &mut Notices,
    ) -> bool {
        let Some(store) = &self.store else {
            notices.error(s.err_no_config_dir);
            return false;
        };
        let name = session.config().active_profile.clone();
        match store.save(&name, &session.config().tuning) {
            Ok(()) => {
                session.mark_saved();
                self.refresh(s, notices);
                true
            }
            Err(e) => {
                notices.error(fill(
                    s.err_profile_save,
                    &[("error", &profile_error_text(&e, s))],
                ));
                false
            }
        }
    }

    fn request_switch(&mut self, name: String, s: &Strings, env: &mut Env) {
        if name == env.session.config().active_profile {
            return;
        }
        if env.session.is_dirty() {
            self.dialog = Some(Dialog::Switch(name));
        } else {
            self.switch_to(&name, s, env);
        }
    }

    fn switch_to(&mut self, name: &str, s: &Strings, env: &mut Env) -> bool {
        let Some(store) = &self.store else {
            return false;
        };
        let interval = f64::from(env.session.config().thread_interval_ms);
        match store.load(name, interval) {
            Ok(loaded) => {
                if !loaded.corrected.is_empty() {
                    env.notices
                        .push(profile_corrected_notice(s, name, &loaded.corrected));
                }
                let tuning = Box::new(loaded.tuning);
                env.shared.send(Command::ApplyProfile {
                    name: name.to_string(),
                    tuning,
                });
                true
            }
            Err(e) => {
                env.notices.error(fill(
                    s.err_profile_load,
                    &[("error", &profile_error_text(&e, s))],
                ));
                false
            }
        }
    }

    pub fn dialogs(&mut self, ctx: &Context, cx: &Cx, env: &mut Env) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        let done = match dialog {
            Dialog::Name(action) => self.name_dialog(ctx, cx, env, action),
            Dialog::Switch(name) => self.switch_dialog(ctx, cx, env, &name),
            Dialog::Delete => self.delete_dialog(ctx, cx, env),
        };
        if done {
            self.dialog = None;
        }
    }

    fn name_dialog(&mut self, ctx: &Context, cx: &Cx, env: &mut Env, action: NameAction) -> bool {
        let s = cx.s;
        let active = env.session.config().active_profile.clone();
        let keep = (action == NameAction::Rename).then_some(active.as_str());
        let error = input_error(&self.input, &self.names, keep, s);
        let title = match action {
            NameAction::SaveAs => s.dlg_new_name,
            NameAction::Duplicate => s.dlg_duplicate,
            NameAction::Rename => s.dlg_rename,
        };
        let mut choice = None;
        let response = Modal::new(Id::new("profile_name_dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            let label = ui.label(RichText::new(title).strong());
            let edit = ui
                .add(TextEdit::singleline(&mut self.input).char_limit(MAX_NAME_LEN))
                .labelled_by(label.id);
            if std::mem::take(&mut self.focus_input) {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if let Some(e) = error.as_ref().filter(|_| !self.input.is_empty()) {
                ui.label(RichText::new(e).color(ui.visuals().error_fg_color));
            }
            choice = two_buttons(ui, (s.btn_ok, error.is_none()), None, s.btn_cancel);
            if enter && error.is_none() {
                choice = Some(Choice::Primary);
            }
        });
        match close_choice(choice, response.should_close()) {
            None => false,
            Some(Choice::Primary) => {
                let name = self.input.clone();
                self.run_name_action(action, &name, s, env)
            }
            Some(_) => true,
        }
    }

    fn run_name_action(
        &mut self,
        action: NameAction,
        name: &str,
        s: &Strings,
        env: &mut Env,
    ) -> bool {
        let Some(store) = &self.store else {
            return true;
        };
        let active = env.session.config().active_profile.clone();
        let result = match action {
            NameAction::SaveAs => store.save_new(name, &env.session.config().tuning),
            NameAction::Duplicate => store.duplicate(&active, name),
            NameAction::Rename if name == active => return true,
            NameAction::Rename => store.rename(&active, name),
        };
        if let Err(e) = result {
            env.notices.error(fill(
                s.err_profile_save,
                &[("error", &profile_error_text(&e, s))],
            ));
            return false;
        }
        if action != NameAction::Duplicate {
            env.session.config_mut().active_profile = name.to_string();
        }
        if action == NameAction::SaveAs {
            env.session.mark_saved();
        }
        self.refresh(s, env.notices);
        true
    }

    fn switch_dialog(&mut self, ctx: &Context, cx: &Cx, env: &mut Env, name: &str) -> bool {
        let s = cx.s;
        let active = env.session.config().active_profile.clone();
        let mut choice = None;
        let response = Modal::new(Id::new("profile_switch_dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.label(fill(s.switch_unsaved, &[("name", &active)]));
            ui.add_space(6.0);
            choice = two_buttons(ui, (s.btn_save, true), Some(s.btn_dont_save), s.btn_cancel);
        });
        match close_choice(choice, response.should_close()) {
            None => false,
            Some(Choice::Primary) => {
                if self.save_active(s, env.session, env.notices) {
                    self.switch_to(name, s, env);
                }
                true
            }
            Some(Choice::Secondary) => {
                self.switch_to(name, s, env);
                true
            }
            Some(Choice::Cancel) => true,
        }
    }

    fn delete_dialog(&mut self, ctx: &Context, cx: &Cx, env: &mut Env) -> bool {
        let s = cx.s;
        let active = env.session.config().active_profile.clone();
        let mut choice = None;
        let response = Modal::new(Id::new("profile_delete_dialog")).show(ctx, |ui| {
            ui.set_width(DIALOG_WIDTH);
            ui.label(fill(s.delete_confirm, &[("name", &active)]));
            ui.add_space(6.0);
            choice = two_buttons(ui, (s.btn_delete, true), None, s.btn_cancel);
        });
        let choice = close_choice(choice, response.should_close());
        if choice == Some(Choice::Primary) {
            self.delete_active(&active, s, env);
        }
        choice.is_some()
    }

    fn delete_active(&mut self, active: &str, s: &Strings, env: &mut Env) {
        let Some(next) = next_profile(&self.names, active) else {
            return;
        };
        if !self.switch_to(&next, s, env) {
            return;
        }
        let Some(store) = &self.store else {
            return;
        };
        match store.delete(active) {
            Ok(()) => self.refresh(s, env.notices),
            Err(e) => {
                env.notices.error(fill(
                    s.err_profile_delete,
                    &[("error", &profile_error_text(&e, s))],
                ));
            }
        }
    }
}

fn two_buttons(
    ui: &mut Ui,
    (primary, enabled): (&str, bool),
    secondary: Option<&str>,
    cancel: &str,
) -> Option<Choice> {
    let mut choice = None;
    ui.horizontal(|ui| {
        if ui.add_enabled(enabled, Button::new(primary)).clicked() {
            choice = Some(Choice::Primary);
        }
        if let Some(label) = secondary
            && ui.button(label).clicked()
        {
            choice = Some(Choice::Secondary);
        }
        if ui.button(cancel).clicked() {
            choice = Some(Choice::Cancel);
        }
    });
    choice
}

fn close_choice(choice: Option<Choice>, should_close: bool) -> Option<Choice> {
    choice.or(should_close.then_some(Choice::Cancel))
}

fn history_buttons(ui: &mut Ui, cx: &Cx, session: &mut Session) {
    let (can_undo, can_redo) = (session.can_undo(), session.can_redo());
    let undo = ui
        .add_enabled(can_undo, Button::new("⟲"))
        .on_hover_text(cx.s.btn_undo);
    undo.widget_info(|| WidgetInfo::labeled(WidgetType::Button, can_undo, cx.s.btn_undo));
    if undo.clicked() {
        session.undo();
    }
    let redo = ui
        .add_enabled(can_redo, Button::new("⟳"))
        .on_hover_text(cx.s.btn_redo);
    redo.widget_info(|| WidgetInfo::labeled(WidgetType::Button, can_redo, cx.s.btn_redo));
    if redo.clicked() {
        session.redo();
    }
}

fn ab_controls(ui: &mut Ui, cx: &Cx, env: &mut Env) {
    let s = cx.s;
    let Some(side) = env.snap.ab_side else {
        if ui
            .button(s.ab_start)
            .on_hover_text(s.tip_ab_start)
            .clicked()
        {
            env.shared.send(Command::StartAb);
        }
        return;
    };
    let label = match side {
        AbSide::A => "A/B: A",
        AbSide::B => "A/B: B",
    };
    if ui
        .selectable_label(true, label)
        .on_hover_text(s.tip_ab_toggle)
        .clicked()
    {
        env.shared.send(Command::ToggleAb);
    }
    if ui.button(s.ab_end).on_hover_text(s.tip_ab_end).clicked() {
        env.shared.send(Command::EndAb);
    }
}

pub fn name_error_text(e: &NameError, s: &Strings) -> String {
    match e {
        NameError::Empty => s.name_empty.to_string(),
        NameError::TooLong => s.name_too_long.to_string(),
        NameError::InvalidChar(c) => {
            let shown = if c.is_control() {
                format!("U+{:04X}", u32::from(*c))
            } else {
                c.to_string()
            };
            fill(s.name_invalid_char, &[("c", &shown)])
        }
        NameError::TrailingDotOrSpace => s.name_trailing.to_string(),
        NameError::Reserved => s.name_reserved.to_string(),
        NameError::Duplicate => s.name_duplicate.to_string(),
    }
}

pub fn profile_error_text(e: &ProfileError, s: &Strings) -> String {
    match e {
        ProfileError::Name(n) => name_error_text(n, s),
        other => other.to_string(),
    }
}

pub fn input_error(
    input: &str,
    names: &[String],
    keep: Option<&str>,
    s: &Strings,
) -> Option<String> {
    if let Err(e) = validate_name(input) {
        return Some(name_error_text(&e, s));
    }
    let wanted = input.to_lowercase();
    let is_self = keep.is_some_and(|k| k.to_lowercase() == wanted);
    let taken = !is_self && names.iter().any(|n| n.to_lowercase() == wanted);
    taken.then(|| name_error_text(&NameError::Duplicate, s))
}

pub fn next_profile(names: &[String], active: &str) -> Option<String> {
    let i = names.iter().position(|n| n == active)?;
    names
        .get(i + 1)
        .or_else(|| i.checked_sub(1).and_then(|j| names.get(j)))
        .cloned()
}

pub fn unique_copy_name(base: &str, names: &[String]) -> String {
    let taken = |candidate: &str| {
        let wanted = candidate.to_lowercase();
        names.iter().any(|n| n.to_lowercase() == wanted)
    };
    (2..=names.len() + 2)
        .map(|i| format!("{base} ({i})"))
        .find(|c| !taken(c))
        .unwrap_or_else(|| format!("{base} (copy)"))
}
