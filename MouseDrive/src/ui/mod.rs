#![deny(unsafe_code)]

mod bind_helper;
mod brake_tab;
mod calibration;
mod closing;
mod dashboard;
mod diag;
mod general_tab;
mod keybind;
mod logo;
mod monitor;
mod notices;
mod overview;
mod pacing;
mod profiles;
mod repaint;
mod setup;
mod startup;
mod status_bar;
mod steering_tab;
mod theme;
mod throttle_tab;
#[cfg(feature = "updater")]
mod update_ui;
mod widgets;

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use eframe::egui::{
    Align, CentralPanel, Context, Frame, Key, Layout, Margin, Modifiers, Panel, RichText,
    ScrollArea, Ui,
};
#[cfg(target_os = "linux")]
use eframe::egui::{OpenUrl, OutputCommand};
use mousedrive::config::{Config, ConfigError};
use mousedrive::control::{Command, KEYBOARD_BINDING, KEYBOARD_TEXT, Shared, Snapshot};
use mousedrive::disk::{DiskWriter, JobState, Pending};
use mousedrive::input::{self, MouseInfo};
use mousedrive::keys::{self, VK_ESCAPE};
use mousedrive::log;
#[cfg(target_os = "linux")]
use mousedrive::platform;
use mousedrive::session::Session;

use self::brake_tab::BrakeTab;
use self::calibration::Calibration;
use self::closing::CloseState;
use self::diag::DiagUi;
use self::keybind::{KeyBinder, KeyInput, Outcome};
use self::monitor::{Export, Monitor};
use self::notices::{Level, Notice, Notices};
use self::pacing::FramePacer;
use self::profiles::ProfilesUi;
use self::repaint::{Load, RepaintPlan, Shown};
use self::setup::SetupUi;
use self::status_bar::{InputHints, LoopWatch};
use self::theme::Palette;
#[cfg(feature = "updater")]
use self::update_ui::UpdaterUi;
use self::widgets::Cx;
use crate::lang::{Lang, fill, strings};
use crate::lifecycle::ExitRequest;

pub use self::startup::Startup;
use self::startup::{crashed_notice, rolled_back_notice, updated_notice};

const NAV_WIDTH: f32 = 176.0;
const WIDE_LAYOUT: f32 = 980.0;
const LIVE_PANEL_WIDTH: f32 = 260.0;
const LIVE_PANEL_ROOM: f32 = 800.0;
#[cfg(target_os = "linux")]
const URL_OPENER: &str = "xdg-open";

pub fn keyboard_flags(text: bool, binding: bool) -> u8 {
    let text = if text { KEYBOARD_TEXT } else { 0 };
    let binding = if binding { KEYBOARD_BINDING } else { 0 };
    text | binding
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Overview,
    Steering,
    Throttle,
    Brake,
    General,
    Monitor,
}

struct ConfigWrite {
    config: Config,
    pending: Pending<Result<(), ConfigError>>,
}

fn lost_write() -> ConfigError {
    ConfigError::Io(io::Error::from(io::ErrorKind::Interrupted))
}

pub struct App {
    shared: Arc<Shared>,
    disk: Arc<DiskWriter>,
    config_write: Option<ConfigWrite>,
    session: Session,
    config_path: Option<PathBuf>,
    snap: Snapshot,
    notices: Notices,
    profiles: ProfilesUi,
    setup: SetupUi,
    diag: DiagUi,
    monitor: Monitor,
    bind_helper_open: bool,
    keybind: KeyBinder,
    calibration: Calibration,
    brake: BrakeTab,
    tab: Tab,
    view: Option<(i32, f64)>,
    close: CloseState,
    mice: Vec<MouseInfo>,
    device_generation: u64,
    hints: InputHints,
    start: Instant,
    #[cfg(feature = "updater")]
    updater: UpdaterUi,
    restart: Arc<AtomicBool>,
    exit_request: Arc<ExitRequest>,
    pacer: FramePacer,
    repaint: RepaintPlan,
    loop_watch: LoopWatch,
    loop_stalled: bool,
}

impl App {
    #[cfg_attr(not(feature = "updater"), allow(unused_mut))]
    pub fn new(
        startup: Startup,
        shared: Arc<Shared>,
        disk: Arc<DiskWriter>,
        restart: Arc<AtomicBool>,
        exit_request: Arc<ExitRequest>,
    ) -> Self {
        let Startup {
            config,
            disk: on_disk,
            config_path,
            store,
            notices: startup_notices,
        } = startup;
        let s = strings(Lang::from_i32(config.language));
        let mut notices = Notices::default();
        startup_notices.into_iter().for_each(|n| notices.push(n));
        let mut session = Session::new(config, shared.config_epoch(), on_disk);
        #[cfg(feature = "updater")]
        let updater = UpdaterUi::new(session.config_mut());
        let profiles = ProfilesUi::new(store, Arc::clone(&disk), s, &mut notices);
        let device_generation = input::device_generation();
        Self {
            shared,
            disk,
            config_write: None,
            session,
            config_path,
            snap: Snapshot::default(),
            notices,
            profiles,
            setup: SetupUi::default(),
            diag: DiagUi::default(),
            monitor: Monitor::default(),
            bind_helper_open: false,
            keybind: KeyBinder::default(),
            calibration: Calibration::default(),
            brake: BrakeTab::default(),
            tab: Tab::Overview,
            view: None,
            close: CloseState::default(),
            mice: input::list_mice(),
            device_generation,
            hints: InputHints::default(),
            start: Instant::now(),
            #[cfg(feature = "updater")]
            updater,
            restart,
            exit_request,
            pacer: FramePacer::default(),
            repaint: RepaintPlan::default(),
            loop_watch: LoopWatch::default(),
            loop_stalled: false,
        }
    }

    pub fn limit_frame_rate(&mut self) {
        self.pacer = FramePacer::new(repaint::FAST);
    }

    pub fn report_launch_error(&mut self, error: &str) {
        let s = self.cx().s;
        self.notices.push(Notice::new(
            Level::Error,
            fill(s.err_launch, &[("error", error)]),
        ));
    }

    pub fn report_update(&mut self, from: &str) {
        let s = self.cx().s;
        if let Some(notice) = updated_notice(s, from) {
            self.notices.push(notice);
        }
    }

    pub fn report_rollback(&mut self) {
        let s = self.cx().s;
        self.notices.push(rolled_back_notice(s));
    }

    pub fn report_crash(&mut self, report: &Path) {
        let s = self.cx().s;
        self.notices.push(crashed_notice(s, report));
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
        let interval_ms = f64::from(self.session.config().thread_interval_ms.clamp(1, 20));
        self.loop_stalled = self.loop_watch.stalled(self.snap.tick, interval_ms, now_ms);
        self.setup.pull(&self.shared);
        self.session.sync_in(&*self.shared);
        self.pull_devices();
        self.brake.tick(&self.snap, now_ms);
        if self.tab == Tab::Monitor {
            self.monitor.pull(&self.shared);
        } else {
            self.monitor.release();
        }
    }

    fn pull_devices(&mut self) {
        let generation = input::device_generation();
        if generation != self.device_generation {
            self.device_generation = generation;
            self.mice = input::list_mice();
        }
        self.hints = InputHints::read(&self.session.config().mouse_device);
    }

    fn apply_view(&mut self, ctx: &Context) {
        let cfg = self.session.config();
        let wanted = (cfg.language, cfg.ui_zoom);
        let dragging = ctx.input(|i| i.pointer.any_down());
        if self.view == Some(wanted) || (dragging && self.view.is_some()) {
            return;
        }
        theme::apply(ctx, wanted.1);
        self.view = Some(wanted);
    }

    fn poll_keybind(&mut self, ctx: &Context) {
        if !self.keybind.listening() {
            return;
        }
        let (window_escape, now_s) = ctx.input(|i| {
            let escape = i.key_pressed(Key::Escape) || i.key_down(Key::Escape);
            (escape, i.time)
        });
        let input = KeyInput {
            pressed: keys::first_pressed_key(),
            escape: window_escape || keys::is_key_down(VK_ESCAPE),
            now_s,
        };
        if let Some(Outcome::Bound(field, vk)) = self.keybind.poll(input) {
            *field.slot(self.session.config_mut()) = vk;
        }
    }

    fn zoom_keys(&mut self, ctx: &Context) {
        let Some(key) = theme::zoom_shortcut(ctx) else {
            return;
        };
        let cfg = self.session.config_mut();
        cfg.ui_zoom = theme::zoomed(cfg.ui_zoom, key);
        ctx.request_repaint();
    }

    fn shortcuts(&mut self, ctx: &Context, cx: &Cx) {
        if ctx.egui_wants_keyboard_input() || self.keybind.listening() {
            return;
        }
        self.zoom_keys(ctx);
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
                .save_active(cx.s, &self.session, &mut self.notices);
        }
    }

    fn top_panel(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        Panel::top("top")
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(22, 10)),
            )
            .show(ui, |ui| {
                let action = status_bar::show(
                    ui,
                    cx,
                    &self.snap,
                    &self.setup.info,
                    self.session.config(),
                    self.loop_stalled,
                    &self.hints,
                );
                if let Some(action) = action {
                    self.run_status_action(action);
                }
                if self.close.second_close_armed(now_ms) {
                    let warn = ui.visuals().warn_fg_color;
                    ui.label(RichText::new(cx.s.close_again_hint).color(warn));
                }
                ui.add_space(2.0);
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
            Action::UseAllMice => self.session.config_mut().mouse_device.clear(),
            Action::Details => self.diag.open = !self.diag.open,
            Action::SaveMovedMouse => {
                if let Some(moved) = &self.hints.moved {
                    self.session.config_mut().mouse_device = moved.new.clone();
                }
            }
        }
    }

    fn bottom_panel(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        Panel::bottom("diag").show(ui, |ui| {
            let facts = (&self.snap, &self.setup.info, self.session.config());
            self.diag.show(ui, cx, facts, now_ms);
        });
    }

    fn navigation(&mut self, ui: &mut Ui, cx: &Cx, horizontal: bool) {
        let items = [
            (Tab::Overview, cx.s.nav_home),
            (Tab::Steering, cx.s.tab_steering),
            (Tab::Throttle, cx.s.tab_throttle),
            (Tab::Brake, cx.s.tab_brake),
            (Tab::Monitor, cx.s.nav_monitor),
            (Tab::General, cx.s.nav_settings),
        ];
        let mut buttons = |ui: &mut Ui| {
            for (tab, label) in items {
                let width = if horizontal {
                    0.0
                } else {
                    ui.available_width()
                };
                if theme::nav_button(ui, label, self.tab == tab, width).clicked() {
                    self.tab = tab;
                }
            }
        };
        if horizontal {
            ui.horizontal_wrapped(buttons);
        } else {
            buttons(ui);
        }
    }

    fn side_panel(&mut self, ui: &mut Ui, cx: &Cx) {
        if ui.available_width() < WIDE_LAYOUT {
            return;
        }
        Panel::left("navigation")
            .exact_size(NAV_WIDTH)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::same(14)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new(cx.s.nav_hint).small().color(theme::MUTED));
                ui.add_space(8.0);
                self.navigation(ui, cx, false);
                ui.add_space(20.0);
                ui.separator();
                ui.add_space(8.0);
                theme::hint(ui, cx.s.save_short);
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    if ui.button(cx.s.btn_setup).clicked() {
                        self.setup.open = true;
                    }
                });
            });
        if !matches!(self.tab, Tab::Overview | Tab::Monitor)
            && ui.available_width() >= LIVE_PANEL_ROOM
        {
            Panel::right("live_output")
                .exact_size(LIVE_PANEL_WIDTH)
                .resizable(false)
                .frame(
                    Frame::new()
                        .fill(theme::BACKGROUND)
                        .inner_margin(Margin::same(16)),
                )
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .id_salt("live_scroll")
                        .show(ui, |ui| {
                            dashboard::compact(ui, cx, &self.snap, self.session.config());
                        });
                });
        }
    }

    fn tab_content(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        let steer_abs = self.snap.frame.steering.abs();
        match self.tab {
            Tab::Overview => self.overview(ui, cx),
            Tab::Monitor => {
                let cfg = self.session.config();
                let export = Export {
                    disk: &self.disk,
                    profile: &cfg.active_profile,
                    unsaved: self.session.is_dirty(),
                    tuning: &cfg.tuning,
                };
                theme::card(ui, |ui| self.monitor.show(ui, cx, &export));
            }
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

    fn central_panel(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(24)),
            )
            .show(ui, |ui| {
                if ui.ctx().content_rect().width() < WIDE_LAYOUT {
                    self.navigation(ui, cx, true);
                    ui.add_space(14.0);
                }
                ScrollArea::vertical()
                    .id_salt(format!("workspace_scroll_{:?}", self.tab))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let (title, description) = match self.tab {
                            Tab::Overview => (cx.s.home_title, cx.s.home_desc),
                            Tab::Steering => (cx.s.tab_steering, cx.s.steering_intro),
                            Tab::Throttle => (cx.s.tab_throttle, cx.s.throttle_intro),
                            Tab::Brake => (cx.s.tab_brake, cx.s.brake_intro),
                            Tab::General => (cx.s.nav_settings, cx.s.general_intro),
                            Tab::Monitor => (cx.s.nav_monitor, cx.s.monitor_intro),
                        };
                        theme::heading(ui, title);
                        theme::hint(ui, description);
                        ui.add_space(12.0);
                        ui.push_id(format!("page_{:?}", self.tab), |ui| {
                            self.tab_content(ui, cx, now_ms)
                        });
                    });
            });
    }

    fn render_panels(&mut self, ui: &mut Ui, cx: &Cx, now_ms: f64) {
        self.top_panel(ui, cx, now_ms);
        self.bottom_panel(ui, cx, now_ms);
        self.side_panel(ui, cx);
        self.central_panel(ui, cx, now_ms);
    }

    fn run_dashboard_action(&mut self, action: dashboard::Action) {
        use dashboard::Action;
        match action {
            Action::ResetSteering => self.shared.send(Command::ResetSteering),
            Action::OpenMonitor => self.tab = Tab::Monitor,
        }
    }

    fn windows(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        if self.setup.open {
            let env = setup::Env {
                shared: &self.shared,
                snap: &self.snap,
                #[cfg(target_os = "linux")]
                keyboard_keys: setup::binds_keyboard_keys(self.session.config()),
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
            let counts_total = self.snap.counts_total;
            self.calibration.show(ctx, cx, counts_total, now_ms, dpi);
        }
        let mut env = profiles::Env {
            session: &mut self.session,
            shared: &self.shared,
            snap: &self.snap,
            notices: &mut self.notices,
        };
        self.profiles.dialogs(ctx, cx, &mut env);
    }

    fn push(&mut self, ctx: &Context, now_ms: f64) {
        let text = ctx.egui_wants_keyboard_input();
        self.shared
            .set_gui_keyboard(keyboard_flags(text, self.keybind.listening()));
        let interacting = !window_hidden(ctx) && (text || ctx.input(|i| i.pointer.any_down()));
        self.session.sync_out(&*self.shared, interacting);
        self.write_disk(now_ms, interacting);
    }

    fn write_disk(&mut self, now_ms: f64, interacting: bool) {
        if self.config_write.is_some() {
            return;
        }
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let Some(config) = self.session.disk_write_due(now_ms, interacting) else {
            return;
        };
        let written = config.clone();
        let pending = self.disk.submit(move || {
            let result = written.save_to_file(&path);
            if let Err(e) = &result {
                log::error(&format!("config yazılamadı: {e}"));
            }
            result
        });
        self.config_write = Some(ConfigWrite { config, pending });
    }

    fn collect_writes(&mut self, cx: &Cx, now_ms: f64) {
        self.profiles
            .poll(cx.s, &mut self.session, &mut self.notices);
        self.monitor.collect_export();
        let Some(write) = self.config_write.take() else {
            return;
        };
        let result = match write.pending.state() {
            JobState::Running => {
                self.config_write = Some(write);
                return;
            }
            JobState::Done(result) => result,
            JobState::Lost => Err(lost_write()),
        };
        match result {
            Ok(()) => self.session.disk_written(write.config),
            Err(e) => {
                self.session.disk_write_failed(now_ms);
                self.notices
                    .error(fill(cx.s.err_config_save, &[("error", &e.to_string())]));
            }
        }
    }

    fn finish_writes(&mut self) {
        let s = self.cx().s;
        self.profiles
            .settle(s, &mut self.session, &mut self.notices);
        if let Some(write) = self.config_write.take() {
            match write.pending.wait().unwrap_or_else(|| Err(lost_write())) {
                Ok(()) => self.session.disk_written(write.config),
                Err(e) => log::error(&format!("çıkışta bekleyen config yazımı başarısız: {e}")),
            }
        }
        if let (Some(path), Some(config)) =
            (self.config_path.clone(), self.session.disk_write_needed())
        {
            let written = config.clone();
            match self.disk.run(move || config.save_to_file(&path)) {
                Some(Ok(())) => self.session.disk_written(written),
                Some(Err(e)) => log::error(&format!("çıkışta config yazılamadı: {e}")),
                None => log::error("çıkışta config yazılamadı: disk işi yarıda kaldı"),
            }
        }
        self.disk.flush();
    }

    fn busy(&self) -> bool {
        #[cfg(feature = "updater")]
        let updating = self.updater.busy();
        #[cfg(not(feature = "updater"))]
        let updating = false;
        updating
            || self.keybind.listening()
            || self.snap.binding.is_some()
            || self.calibration.is_measuring()
            || self.profiles.is_saving()
            || self.config_write.is_some()
            || self.monitor.exporting()
    }

    fn animating(&self) -> bool {
        match self.tab {
            Tab::Monitor => self.monitor.moving(),
            Tab::Brake => self.snap.brake_pressed,
            _ => false,
        }
    }

    fn schedule_repaint(&mut self, ctx: &Context, now_ms: f64) {
        let load = Load {
            focused: ctx.input(|i| i.viewport().focused).unwrap_or(true),
            busy: self.busy(),
            animating: self.animating(),
        };
        let after = self.repaint.next(Shown::of(&self.snap), now_ms, load);
        ctx.request_repaint_after(after);
    }
}

#[cfg(target_os = "linux")]
fn open_requested_urls(ctx: &Context, opener: &str) {
    let unopened: Vec<OutputCommand> = take_url_commands(ctx)
        .into_iter()
        .filter(|open| !open_url(opener, &open.url))
        .map(OutputCommand::OpenUrl)
        .collect();
    if !unopened.is_empty() {
        ctx.output_mut(|output| output.commands.extend(unopened));
    }
}

#[cfg(target_os = "linux")]
fn open_url(opener: &str, url: &str) -> bool {
    match platform::spawn_reaped(platform::child_command(opener).arg(url)) {
        Ok(_) => true,
        Err(e) => {
            log::warn(&format!(
                "bağlantı {opener} ile açılamadı ({url}): {e}; eframe'e bırakıldı"
            ));
            false
        }
    }
}

#[cfg(target_os = "linux")]
fn take_url_commands(ctx: &Context) -> Vec<OpenUrl> {
    ctx.output_mut(|output| {
        let (urls, rest) = std::mem::take(&mut output.commands)
            .into_iter()
            .partition::<Vec<_>, _>(|command| matches!(command, OutputCommand::OpenUrl(_)));
        output.commands = rest;
        urls.into_iter()
            .filter_map(|command| match command {
                OutputCommand::OpenUrl(open) => Some(open),
                _ => None,
            })
            .collect()
    })
}

fn window_hidden(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().visible()) == Some(false)
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.pacer.pace();
        let now_ms = self.now_ms();
        #[cfg(target_os = "linux")]
        mousedrive::keys::set_app_focused(ctx.input(|i| i.viewport().focused).unwrap_or(true));
        self.pull(now_ms);
        self.apply_view(ctx);
        let cx = self.cx();
        self.collect_writes(&cx, now_ms);
        self.poll_keybind(ctx);
        self.handle_close(ctx, &cx, now_ms);
        #[cfg(feature = "updater")]
        self.handle_restart(ctx);
        if window_hidden(ctx) {
            self.push(ctx, now_ms);
            self.schedule_repaint(ctx, now_ms);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now_ms = self.now_ms();
        let cx = self.cx();
        self.shortcuts(&ctx, &cx);
        self.render_panels(ui, &cx, now_ms);
        self.windows(&ctx, &cx, now_ms);
        self.close_dialogs(&ctx, &cx);
        self.push(&ctx, now_ms);
        #[cfg(target_os = "linux")]
        open_requested_urls(&ctx, URL_OPENER);
        self.schedule_repaint(&ctx, now_ms);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.finish_writes();
    }
}
