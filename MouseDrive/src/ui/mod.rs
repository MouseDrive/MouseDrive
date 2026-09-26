#![deny(unsafe_code)]

mod bind_helper;
mod brake_tab;
mod calibration;
mod dashboard;
mod diag;
mod general_tab;
mod keybind;
mod monitor;
mod notices;
mod profiles;
mod setup;
mod startup;
mod status_bar;
mod steering_tab;
mod theme;
mod throttle_tab;
#[cfg(feature = "updater")]
mod update_ui;
mod widgets;

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(feature = "updater")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{
    CentralPanel, Context, Id, Key, Modal, Modifiers, RichText, ScrollArea, SidePanel,
    TopBottomPanel, Ui, ViewportCommand,
};
use mousedrive::control::{Command, KEYBOARD_BINDING, KEYBOARD_TEXT, Shared, Snapshot};
use mousedrive::input::{self, MouseInfo};
use mousedrive::keys::{self, VK_ESCAPE};
use mousedrive::session::Session;
use mousedrive::status::AppStatus;
use mousedrive::{log, overlay};

use self::brake_tab::BrakeTab;
use self::calibration::Calibration;
use self::diag::DiagUi;
use self::keybind::{KeyBinder, Outcome};
use self::monitor::Monitor;
use self::notices::{Level, Notice, Notices};
use self::profiles::ProfilesUi;
use self::setup::SetupUi;
use self::theme::Palette;
#[cfg(feature = "updater")]
use self::update_ui::UpdaterUi;
use self::widgets::Cx;
use crate::lang::{Lang, fill, strings};

pub use self::startup::Startup;

const FAST_REPAINT: Duration = Duration::from_millis(16);
const SLOW_REPAINT: Duration = Duration::from_millis(250);
const SIDE_PANEL_WIDTH: f32 = 440.0;
const SIDE_PANEL_MIN_WIDTH: f32 = 360.0;
const CLOSE_DIALOG_WIDTH: f32 = 360.0;

pub fn status_labels(lang: Lang) -> [String; AppStatus::ALL.len()] {
    strings(lang).status_labels.map(String::from)
}

pub fn keyboard_flags(text: bool, binding: bool) -> u8 {
    let text = if text { KEYBOARD_TEXT } else { 0 };
    let binding = if binding { KEYBOARD_BINDING } else { 0 };
    text | binding
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Steering,
    Throttle,
    Brake,
    General,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseDecision {
    Quit,
    Minimize,
    Confirm,
}

pub fn close_decision(exit_on_close: bool, dirty: bool) -> CloseDecision {
    match (exit_on_close, dirty) {
        (false, _) => CloseDecision::Minimize,
        (true, true) => CloseDecision::Confirm,
        (true, false) => CloseDecision::Quit,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CloseChoice {
    SaveAndQuit,
    Quit,
    Cancel,
}

#[derive(Default)]
struct CloseState {
    allowed: bool,
    confirm: bool,
}

pub struct App {
    shared: Arc<Shared>,
    session: Session,
    config_path: Option<PathBuf>,
    snap: Snapshot,
    notices: Notices,
    profiles: ProfilesUi,
    setup: SetupUi,
    diag: DiagUi,
    monitor: Monitor,
    monitor_open: bool,
    bind_helper_open: bool,
    keybind: KeyBinder,
    calibration: Calibration,
    brake: BrakeTab,
    tab: Tab,
    view: Option<(i32, f64)>,
    close: CloseState,
    mice: Vec<MouseInfo>,
    start: Instant,
    #[cfg(feature = "updater")]
    updater: UpdaterUi,
    #[cfg(feature = "updater")]
    restart: Arc<AtomicBool>,
}

impl App {
    #[cfg_attr(not(feature = "updater"), allow(unused_variables, unused_mut))]
    pub fn new(
        startup: Startup,
        shared: Arc<Shared>,
        restart: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let Startup {
            config,
            disk,
            config_path,
            store,
            notices: startup_notices,
        } = startup;
        let s = strings(Lang::from_i32(config.language));
        let mut notices = Notices::default();
        startup_notices.into_iter().for_each(|n| notices.push(n));
        let mut session = Session::new(config, shared.config_epoch(), disk);
        #[cfg(feature = "updater")]
        let updater = UpdaterUi::new(session.config_mut());
        let profiles = ProfilesUi::new(store, s, &mut notices);
        Self {
            shared,
            session,
            config_path,
            snap: Snapshot::default(),
            notices,
            profiles,
            setup: SetupUi::default(),
            diag: DiagUi::default(),
            monitor: Monitor::default(),
            monitor_open: false,
            bind_helper_open: false,
            keybind: KeyBinder::default(),
            calibration: Calibration::default(),
            brake: BrakeTab::default(),
            tab: Tab::Steering,
            view: None,
            close: CloseState::default(),
            mice: input::list_mice(),
            start: Instant::now(),
            #[cfg(feature = "updater")]
            updater,
            #[cfg(feature = "updater")]
            restart,
        }
    }

    pub fn report_launch_error(&mut self, error: &str) {
        let s = self.cx().s;
        self.notices.push(Notice::new(
            Level::Error,
            fill(s.err_launch, &[("error", error)]),
        ));
    }

    fn cx(&self) -> Cx {
        let cfg = self.session.config();
        let lang = Lang::from_i32(cfg.language);
        Cx {
            s: strings(lang),
            lang,
            pal: Palette::new(cfg.colorblind_palette),
        }
    }

    fn now_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn pull(&mut self, now_ms: f64) {
        self.snap = self.shared.snapshot();
        self.setup.pull(&self.shared);
        self.session.sync_in(&*self.shared);
        self.brake.tick(&self.snap, now_ms);
        if self.monitor_open {
            self.monitor.pull(&self.shared);
        }
    }

    fn apply_view(&mut self, ctx: &Context) {
        let cfg = self.session.config();
        let wanted = (cfg.language, cfg.ui_zoom);
        let dragging = ctx.input(|i| i.pointer.any_down());
        if self.view == Some(wanted) || (dragging && self.view.is_some()) {
            return;
        }
        if self.view.map(|(lang, _)| lang) != Some(wanted.0) {
            overlay::set_labels(status_labels(Lang::from_i32(wanted.0)));
        }
        theme::apply(ctx, wanted.1);
        self.view = Some(wanted);
    }

    fn poll_keybind(&mut self) {
        if !self.keybind.listening() {
            return;
        }
        let pressed = keys::first_pressed_key();
        let escape = keys::is_key_down(VK_ESCAPE);
        if let Some(Outcome::Bound(field, vk)) = self.keybind.poll(pressed, escape) {
            *field.slot(self.session.config_mut()) = vk;
        }
    }

    fn shortcuts(&mut self, ctx: &Context, cx: &Cx) {
        if ctx.wants_keyboard_input() || self.keybind.listening() {
            return;
        }
        let redo = ctx.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                || i.consume_key(Modifiers::COMMAND, Key::Y)
        });
        let undo = !redo && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z));
        let save = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S));
        if redo {
            self.session.redo();
        }
        if undo {
            self.session.undo();
        }
        if save && self.session.is_dirty() {
            self.profiles
                .save_active(cx.s, &mut self.session, &mut self.notices);
        }
    }

    fn top_panel(&mut self, ctx: &Context, cx: &Cx) {
        TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(4.0);
            let action =
                status_bar::show(ui, cx, &self.snap, &self.setup.info, self.session.config());
            if let Some(action) = action {
                self.run_status_action(action);
            }
            ui.horizontal_wrapped(|ui| {
                let mut env = profiles::Env {
                    session: &mut self.session,
                    shared: &self.shared,
                    snap: &self.snap,
                    notices: &mut self.notices,
                };
                self.profiles.bar(ui, cx, &mut env);
                #[cfg(feature = "updater")]
                self.updater.button(ui, cx, self.session.config_mut());
            });
            self.notices.show(ui, cx);
            ui.add_space(2.0);
        });
    }

    fn run_status_action(&mut self, action: status_bar::Action) {
        use status_bar::Action;
        match action {
            Action::OpenSetup => self.setup.open = true,
            Action::Reconnect => self.shared.send(Command::Reconnect),
            Action::CancelBind => self.shared.send(Command::CancelBind),
            Action::Start => self.shared.send(Command::SetCapture(true)),
            Action::Pause => self.shared.send(Command::SetCapture(false)),
            Action::EnableSink => self.session.config_mut().input_sink_enabled = true,
            Action::Details => self.diag.open = !self.diag.open,
        }
    }

    fn bottom_panel(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        TopBottomPanel::bottom("diag").show(ctx, |ui| {
            let facts = (&self.snap, &self.setup.info, self.session.config());
            self.diag.show(ui, cx, facts, now_ms);
        });
    }

    fn side_panel(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        SidePanel::right("settings")
            .default_width(SIDE_PANEL_WIDTH)
            .min_width(SIDE_PANEL_MIN_WIDTH)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let s = cx.s;
                    let tabs = [
                        (Tab::Steering, s.tab_steering),
                        (Tab::Throttle, s.tab_throttle),
                        (Tab::Brake, s.tab_brake),
                        (Tab::General, s.tab_general),
                    ];
                    for (tab, label) in tabs {
                        ui.selectable_value(&mut self.tab, tab, label);
                    }
                });
                ui.separator();
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.tab_content(ui, cx, now_ms));
            });
    }

    fn tab_content(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        let steer_abs = self.snap.frame.steering.abs();
        match self.tab {
            Tab::Steering => steering_tab::show(ui, cx, self.session.config_mut()),
            Tab::Throttle => {
                throttle_tab::show(ui, cx, &mut self.session.config_mut().tuning, steer_abs)
            }
            Tab::Brake => {
                let tuning = &mut self.session.config_mut().tuning;
                self.brake.show(ui, cx, tuning, &self.snap, now_ms);
            }
            Tab::General => {
                let env = general_tab::Env {
                    snap: &self.snap,
                    shared: &self.shared,
                    mice: &mut self.mice,
                    keybind: &mut self.keybind,
                    calibration: &mut self.calibration,
                    #[cfg(feature = "updater")]
                    updater: &mut self.updater,
                };
                general_tab::show(ui, cx, self.session.config_mut(), env);
            }
        }
    }

    fn central_panel(&mut self, ctx: &Context, cx: &Cx) {
        CentralPanel::default().show(ctx, |ui| {
            ScrollArea::vertical().show(ui, |ui| {
                let action =
                    dashboard::show(ui, cx, &self.snap, self.session.config(), self.monitor_open);
                if let Some(action) = action {
                    self.run_dashboard_action(action);
                }
                if self.monitor_open {
                    ui.separator();
                    self.monitor.show(ui, cx);
                }
            });
        });
    }

    fn run_dashboard_action(&mut self, action: dashboard::Action) {
        use dashboard::Action;
        match action {
            Action::ResetSteering => self.shared.send(Command::ResetSteering),
            Action::OpenBindHelper => self.bind_helper_open = true,
            Action::OpenSetup => self.setup.open = true,
            Action::ToggleMonitor => {
                self.monitor_open = !self.monitor_open;
                if !self.monitor_open {
                    self.monitor.clear();
                }
            }
        }
    }

    fn windows(&mut self, ctx: &Context, cx: &Cx) {
        if self.setup.open {
            let env = setup::Env {
                shared: &self.shared,
                snap: &self.snap,
                device_id: &mut self.session.config_mut().vjoy_device_id,
                notices: &mut self.notices,
                open_bind_helper: &mut self.bind_helper_open,
            };
            self.setup.show(ctx, cx, env);
        }
        if self.bind_helper_open {
            bind_helper::show(
                ctx,
                cx,
                &mut self.bind_helper_open,
                &self.snap,
                &self.shared,
            );
        }
        if self.calibration.is_open() {
            let dpi = &mut self.session.config_mut().mouse_dpi;
            self.calibration.show(ctx, cx, self.snap.counts_total, dpi);
        }
        let mut env = profiles::Env {
            session: &mut self.session,
            shared: &self.shared,
            snap: &self.snap,
            notices: &mut self.notices,
        };
        self.profiles.dialogs(ctx, cx, &mut env);
    }

    fn push(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        let text = ctx.wants_keyboard_input();
        self.shared
            .set_gui_keyboard(keyboard_flags(text, self.keybind.listening()));
        let interacting = text || ctx.input(|i| i.pointer.any_down());
        self.session.sync_out(&*self.shared, interacting);
        self.write_disk(cx, now_ms, interacting);
    }

    fn write_disk(&mut self, cx: &Cx, now_ms: f64, interacting: bool) {
        let Some(path) = &self.config_path else {
            return;
        };
        let Some(config) = self.session.disk_write_due(now_ms, interacting) else {
            return;
        };
        match config.save_to_file(path) {
            Ok(()) => self.session.disk_written(config),
            Err(e) => {
                self.session.disk_write_failed(now_ms);
                self.notices
                    .error(fill(cx.s.err_config_save, &[("error", &e.to_string())]));
            }
        }
    }

    fn handle_close(&mut self, ctx: &Context, cx: &Cx) {
        let requested = ctx.input(|i| i.viewport().close_requested());
        if requested && !self.close.allowed {
            let cfg = self.session.config();
            match close_decision(cfg.exit_on_close, self.session.is_dirty()) {
                CloseDecision::Quit => {}
                CloseDecision::Minimize => {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                }
                CloseDecision::Confirm => {
                    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                    self.close.confirm = true;
                }
            }
        }
        if self.close.confirm {
            self.close_dialog(ctx, cx);
        }
    }

    fn close_dialog(&mut self, ctx: &Context, cx: &Cx) {
        let s = cx.s;
        let name = self.session.config().active_profile.clone();
        let mut choice = None;
        let response = Modal::new(Id::new("close_dialog")).show(ctx, |ui| {
            ui.set_width(CLOSE_DIALOG_WIDTH);
            ui.label(RichText::new(s.close_title).strong());
            ui.label(fill(s.close_body, &[("name", &name)]));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(s.btn_save_quit).clicked() {
                    choice = Some(CloseChoice::SaveAndQuit);
                }
                if ui.button(s.btn_quit_nosave).clicked() {
                    choice = Some(CloseChoice::Quit);
                }
                if ui.button(s.btn_cancel).clicked() {
                    choice = Some(CloseChoice::Cancel);
                }
            });
        });
        match choice.or(response.should_close().then_some(CloseChoice::Cancel)) {
            None => {}
            Some(CloseChoice::SaveAndQuit) => {
                if self
                    .profiles
                    .save_active(s, &mut self.session, &mut self.notices)
                {
                    self.quit(ctx);
                } else {
                    self.close.confirm = false;
                }
            }
            Some(CloseChoice::Quit) => self.quit(ctx),
            Some(CloseChoice::Cancel) => self.close.confirm = false,
        }
    }

    fn quit(&mut self, ctx: &Context) {
        self.close = CloseState {
            allowed: true,
            confirm: false,
        };
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }

    #[cfg(feature = "updater")]
    fn handle_restart(&mut self, ctx: &Context) {
        if self.updater.ready_to_restart() && !self.close.allowed {
            self.restart.store(true, Ordering::Release);
            self.quit(ctx);
        }
    }

    fn busy(&self) -> bool {
        #[cfg(feature = "updater")]
        let updating = self.updater.busy();
        #[cfg(not(feature = "updater"))]
        let updating = false;
        updating || self.keybind.listening() || self.snap.binding.is_some()
    }

    fn schedule_repaint(&self, ctx: &Context) {
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let after = if focused || self.busy() {
            FAST_REPAINT
        } else {
            SLOW_REPAINT
        };
        ctx.request_repaint_after(after);
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let now_ms = self.now_ms();
        #[cfg(target_os = "linux")]
        mousedrive::keys::set_app_focused(ctx.input(|i| i.viewport().focused).unwrap_or(true));
        self.pull(now_ms);
        self.apply_view(ctx);
        let cx = self.cx();
        self.poll_keybind();
        self.shortcuts(ctx, &cx);
        self.top_panel(ctx, &cx);
        self.bottom_panel(ctx, &cx, now_ms);
        self.side_panel(ctx, &cx, now_ms);
        self.central_panel(ctx, &cx);
        self.windows(ctx, &cx);
        self.push(ctx, &cx, now_ms);
        self.handle_close(ctx, &cx);
        #[cfg(feature = "updater")]
        self.handle_restart(ctx);
        self.schedule_repaint(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let (Some(path), Some(config)) = (&self.config_path, self.session.disk_write_needed())
        else {
            return;
        };
        if let Err(e) = config.save_to_file(path) {
            log::line(&format!("çıkışta config yazılamadı: {e}"));
        }
    }
}
