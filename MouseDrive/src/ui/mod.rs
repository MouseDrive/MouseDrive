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
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use eframe::egui::{
    Align, CentralPanel, Context, Frame, Key, Layout, Margin, Modifiers, RichText, ScrollArea,
    SidePanel, TopBottomPanel, Ui,
};
use mousedrive::control::{Command, KEYBOARD_BINDING, KEYBOARD_TEXT, Shared, Snapshot};
use mousedrive::input::{self, MouseInfo};
use mousedrive::keys::{self, VK_ESCAPE};
use mousedrive::session::Session;
use mousedrive::status::AppStatus;
use mousedrive::{log, overlay};

use self::brake_tab::BrakeTab;
use self::calibration::Calibration;
use self::closing::CloseState;
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
const NAV_WIDTH: f32 = 176.0;
const WIDE_LAYOUT: f32 = 980.0;
const LIVE_PANEL_WIDTH: f32 = 260.0;
const LIVE_PANEL_ROOM: f32 = 800.0;

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
    Overview,
    Steering,
    Throttle,
    Brake,
    General,
    Monitor,
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
    restart: Arc<AtomicBool>,
}

impl App {
    #[cfg_attr(not(feature = "updater"), allow(unused_mut))]
    pub fn new(startup: Startup, shared: Arc<Shared>, restart: Arc<AtomicBool>) -> Self {
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
            bind_helper_open: false,
            keybind: KeyBinder::default(),
            calibration: Calibration::default(),
            brake: BrakeTab::default(),
            tab: Tab::Overview,
            view: None,
            close: CloseState::default(),
            mice: input::list_mice(),
            start: Instant::now(),
            #[cfg(feature = "updater")]
            updater,
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
        if self.tab == Tab::Monitor {
            self.monitor.pull(&self.shared);
        } else {
            self.monitor.release();
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
        TopBottomPanel::top("top")
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(22, 10)),
            )
            .show(ctx, |ui| {
                let action =
                    status_bar::show(ui, cx, &self.snap, &self.setup.info, self.session.config());
                if let Some(action) = action {
                    self.run_status_action(action);
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
        }
    }

    fn bottom_panel(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        TopBottomPanel::bottom("diag").show(ctx, |ui| {
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

    fn side_panel(&mut self, ctx: &Context, cx: &Cx) {
        if ctx.available_rect().width() < WIDE_LAYOUT {
            return;
        }
        SidePanel::left("navigation")
            .exact_width(NAV_WIDTH)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::same(14)),
            )
            .show(ctx, |ui| {
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
            && ctx.available_rect().width() >= LIVE_PANEL_ROOM
        {
            SidePanel::right("live_output")
                .exact_width(LIVE_PANEL_WIDTH)
                .resizable(false)
                .frame(
                    Frame::new()
                        .fill(theme::BACKGROUND)
                        .inner_margin(Margin::same(16)),
                )
                .show(ctx, |ui| {
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
                theme::card(ui, |ui| self.monitor.show(ui, cx));
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

    fn central_panel(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(theme::BACKGROUND)
                    .inner_margin(Margin::same(24)),
            )
            .show(ctx, |ui| {
                if ctx.screen_rect().width() < WIDE_LAYOUT {
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

    fn render_panels(&mut self, ctx: &Context, cx: &Cx, now_ms: f64) {
        self.top_panel(ctx, cx);
        self.bottom_panel(ctx, cx, now_ms);
        self.side_panel(ctx, cx);
        self.central_panel(ctx, cx, now_ms);
    }

    fn run_dashboard_action(&mut self, action: dashboard::Action) {
        use dashboard::Action;
        match action {
            Action::ResetSteering => self.shared.send(Command::ResetSteering),
            Action::OpenMonitor => self.tab = Tab::Monitor,
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
        self.render_panels(ctx, &cx, now_ms);
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
