use std::sync::mpsc::Receiver;

use crate::bind::{BindHelper, BindPhase, BindTarget};
use crate::config::{Config, Tuning};
use crate::diagnostics::{LatencyStats, LatencySummary, LoopStats, LoopSummary, RateMeter};
use crate::input::{DeviceFilter, InputCounters, MouseButton};
use crate::keys::{UNBOUND, VK_LBUTTON, VK_RBUTTON};
use crate::logic::{MouseDriveState, STEERING_RANGE, TickInput};
use crate::output::{BUTTON_GEAR_DOWN, BUTTON_GEAR_UP, OutputFrame};
use crate::priority::{Priorities, ThreadPriority};
use crate::setup::REQUIRED_BUTTONS;
use crate::status::{
    AppStatus, Cue, CuePlanner, Health, SessionState, StatusInputs, derive_status,
};
use crate::telemetry::{Marker, Sample};

use super::connection::ConnectionManager;
use super::guard::{ButtonGuard, CHECK_INTERVAL_MS, DesktopWatch, async_pressed};
use super::io::ControlIo;
use super::{Command, Options, Shared, Snapshot};

const HEALTH_WINDOW_MS: f64 = 1000.0;
const LOOP_STATS_WINDOW: usize = 1000;
const NANOS_PER_MS: f64 = 1_000_000.0;
const RESUME_SETTLE_MS: f64 = 1500.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AbSide {
    A,
    B,
}

impl AbSide {
    fn other(self) -> Self {
        match self {
            AbSide::A => AbSide::B,
            AbSide::B => AbSide::A,
        }
    }
}

struct AbState {
    stored: Tuning,
    side: AbSide,
}

enum Pending {
    Profile { name: String, tuning: Box<Tuning> },
    AbToggle,
}

#[derive(Default)]
struct KeyEdges {
    capture: bool,
    ab: bool,
}

pub(crate) struct Engine {
    config: Config,
    seen_epoch: u64,
    options: Options,
    state: MouseDriveState,
    capture: bool,
    last_now_ms: Option<f64>,
    tick_count: u64,
    bind: Option<BindHelper>,
    capture_before_bind: bool,
    ab: Option<AbState>,
    pending: Option<Pending>,
    keys: KeyEdges,
    keyboard_busy: bool,
    guards: [ButtonGuard; 2],
    guard_elapsed_ms: f64,
    desktop: DesktopWatch,
    session: SessionState,
    settle_left_ms: f64,
    stuck_releases: u64,
    telemetry_dropped: u64,
    conn: ConnectionManager,
    published_setup_seq: u64,
    loop_stats: LoopStats,
    loop_summary: LoopSummary,
    latency: LatencyStats,
    latency_summary: LatencySummary,
    priorities: Priorities,
    rate: RateMeter,
    counters: InputCounters,
    drops_at_center: u64,
    counts_total: i64,
    device_filter: DeviceFilter,
    registration_ok: bool,
    health: Health,
    health_elapsed_ms: f64,
    last_failed_writes: u64,
    last_clipped: u64,
    cues: CuePlanner,
    status: AppStatus,
    marker: Marker,
    corrected_fields: Vec<&'static str>,
    non_finite_logged: bool,
}

fn device_id(c: &Config) -> u32 {
    c.vjoy_device_id.clamp(1, 16) as u32
}

fn buttons_required(c: &Config) -> i32 {
    if c.gear_keys_enabled {
        REQUIRED_BUTTONS
    } else {
        0
    }
}

fn validated(mut config: Config, last: &[&'static str]) -> (Config, Vec<&'static str>) {
    let corrected = config.validate();
    if !corrected.is_empty() && corrected != last {
        crate::log::warn(&format!(
            "geçersiz ayarlar düzeltildi: {}",
            corrected.join(", ")
        ));
    }
    (config, corrected)
}

impl Engine {
    pub(crate) fn new(config: Config, epoch: u64, options: Options) -> Self {
        let (config, corrected_fields) = validated(config, &[]);
        let conn = ConnectionManager::new(device_id(&config), buttons_required(&config));
        Self {
            config,
            seen_epoch: epoch,
            options,
            state: MouseDriveState::new(),
            capture: true,
            last_now_ms: None,
            tick_count: 0,
            bind: None,
            capture_before_bind: true,
            ab: None,
            pending: None,
            keys: KeyEdges::default(),
            keyboard_busy: false,
            guards: [ButtonGuard::default(); 2],
            guard_elapsed_ms: 0.0,
            desktop: DesktopWatch::default(),
            session: SessionState::Active,
            settle_left_ms: 0.0,
            stuck_releases: 0,
            telemetry_dropped: 0,
            conn,
            published_setup_seq: 0,
            loop_stats: LoopStats::new(LOOP_STATS_WINDOW),
            loop_summary: LoopSummary::default(),
            latency: LatencyStats::new(LOOP_STATS_WINDOW),
            latency_summary: LatencySummary::default(),
            priorities: Priorities::default(),
            rate: RateMeter::default(),
            counters: InputCounters::default(),
            drops_at_center: 0,
            counts_total: 0,
            device_filter: DeviceFilter::AllMice,
            registration_ok: true,
            health: Health::default(),
            health_elapsed_ms: 0.0,
            last_failed_writes: 0,
            last_clipped: 0,
            cues: CuePlanner::default(),
            status: AppStatus::SetupRequired,
            marker: Marker::None,
            corrected_fields,
            non_finite_logged: false,
        }
    }

    pub(crate) fn interval_ms(&self) -> f64 {
        f64::from(self.config.thread_interval_ms.clamp(1, 20))
    }

    pub(crate) fn record_wake_late(&mut self, late_ms: f64) {
        self.latency.record_wake_late(late_ms);
    }

    pub(crate) fn record_work(&mut self, work_ms: f64) {
        self.latency.record_work(work_ms);
    }

    pub(crate) fn tick(
        &mut self,
        now_ms: f64,
        io: &mut impl ControlIo,
        shared: &Shared,
        commands: &Receiver<Command>,
    ) {
        let dt_ms = self.advance_clock(now_ms, io);
        self.watch_priorities(io);
        self.pick_up_config(shared, io, now_ms);
        while let Ok(command) = commands.try_recv() {
            self.handle_command(command, io, now_ms);
        }
        self.watch_session(io);
        self.poll_hotkeys(io, shared);
        self.conn.poll(now_ms, io);
        self.publish_setup(shared);
        let input = self.read_input(dt_ms, io);
        self.apply_pending(shared, io, &input);
        let frame = self.next_frame(input, dt_ms, io);
        self.conn.write(&frame, now_ms);
        self.update_health(dt_ms, io);
        self.update_status(dt_ms, now_ms, io);
        self.push_telemetry(shared, now_ms, input, &frame);
        shared.store_snapshot(self.snapshot(&frame, input, now_ms));
    }

    fn advance_clock(&mut self, now_ms: f64, io: &mut impl ControlIo) -> f64 {
        let dt_ms = match self.last_now_ms {
            Some(prev) => (now_ms - prev).max(0.0),
            None => {
                self.apply_side_effects(io, now_ms);
                self.mark_centered(io);
                self.interval_ms()
            }
        };
        self.last_now_ms = Some(now_ms);
        self.tick_count += 1;
        self.loop_stats.record(dt_ms);
        dt_ms
    }

    fn pick_up_config(&mut self, shared: &Shared, io: &mut impl ControlIo, now_ms: f64) {
        if shared.config_epoch() == self.seen_epoch {
            return;
        }
        let (epoch, published) = shared.config_arc();
        self.seen_epoch = epoch;
        let (config, corrected) = validated(Config::clone(&published), &self.corrected_fields);
        self.corrected_fields = corrected;
        if config.thread_interval_ms != self.config.thread_interval_ms {
            self.reset_loop_stats();
        }
        self.state
            .reseed_after_config_change(&self.config.tuning, &config.tuning);
        self.config = config;
        self.apply_side_effects(io, now_ms);
    }

    fn reset_loop_stats(&mut self) {
        self.loop_stats.clear();
        self.latency.clear();
        self.health_elapsed_ms = 0.0;
        self.loop_summary = LoopSummary::default();
        self.latency_summary = LatencySummary::default();
    }

    fn watch_priorities(&mut self, io: &impl ControlIo) {
        let now = io.priorities();
        if now == self.priorities {
            return;
        }
        let threads = [
            ("kontrol", self.priorities.control, now.control),
            ("girdi", self.priorities.input, now.input),
        ];
        for (label, before, after) in threads {
            if after == ThreadPriority::Demoted && before != after {
                crate::log::warn(&format!(
                    "{label} thread'i ara vermeden çok uzun çalıştığı için (RLIMIT_RTTIME) gerçek zamanlı önceliği geri alındı"
                ));
            }
        }
        self.priorities = now;
    }

    fn apply_side_effects(&mut self, io: &mut impl ControlIo, now_ms: f64) {
        let c = &self.config;
        io.apply_input_settings(c.input_sink_enabled, &c.mouse_device);
        self.conn
            .set_target(device_id(c), buttons_required(c), now_ms);
    }

    fn apply_pending(&mut self, shared: &Shared, io: &mut impl ControlIo, input: &TickInput) {
        if !self.session.is_active() || (self.capture && !self.state.pedals_idle(input)) {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let (next, ab) = match &pending {
            Pending::Profile { name, tuning } => {
                let mut next = self.config.with_tuning((**tuning).clone());
                next.active_profile = name.clone();
                (next, None)
            }
            Pending::AbToggle => {
                let Some(ab) = &self.ab else {
                    return;
                };
                let stored = AbState {
                    stored: self.config.tuning.clone(),
                    side: ab.side.other(),
                };
                (self.config.with_tuning(ab.stored.clone()), Some(stored))
            }
        };
        let Some(epoch) = shared.replace_config(&next, self.seen_epoch) else {
            self.pending = Some(pending);
            return;
        };
        self.seen_epoch = epoch;
        self.state
            .reseed_after_config_change(&self.config.tuning, &next.tuning);
        self.config = next;
        self.ab = ab;
        self.marker = Marker::ProfileChanged;
        self.play(io, Cue::ProfileChanged);
    }

    fn handle_command(&mut self, command: Command, io: &mut impl ControlIo, now_ms: f64) {
        match command {
            Command::Reconnect => self.conn.recheck(now_ms),
            Command::ResetSteering => self.recenter(io),
            Command::SetCapture(on) => self.set_capture(on, io),
            Command::ToggleCapture => self.toggle_capture(io),
            Command::StartBind(target) => self.start_bind(target, io),
            Command::CancelBind => self.finish_bind(io),
            Command::ApplyProfile { name, tuning } => {
                self.pending = Some(Pending::Profile { name, tuning });
            }
            Command::StartAb => {
                self.ab = Some(AbState {
                    stored: self.config.tuning.clone(),
                    side: AbSide::B,
                });
            }
            Command::ToggleAb => self.request_ab_toggle(),
            Command::EndAb => {
                self.ab = None;
                if matches!(self.pending, Some(Pending::AbToggle)) {
                    self.pending = None;
                }
            }
            Command::PlayCue(cue) => self.play(io, cue),
        }
    }

    fn request_ab_toggle(&mut self) {
        if self.ab.is_some() {
            self.pending = Some(Pending::AbToggle);
        }
    }

    fn watch_session(&mut self, io: &mut impl ControlIo) {
        let session = io.session();
        if session == self.session {
            return;
        }
        let was_active = self.session.is_active();
        self.session = session;
        crate::log::info(session_change_note(session));
        if was_active == session.is_active() {
            return;
        }
        self.restart_input(io);
        if session.is_active() {
            self.settle_left_ms = RESUME_SETTLE_MS;
            io.request_register();
        }
    }

    fn poll_hotkeys(&mut self, io: &mut impl ControlIo, shared: &Shared) {
        self.keyboard_busy = shared.gui_keyboard() != 0 && io.app_foreground();
        let hotkeys_live = !self.keyboard_busy && self.session.is_active();
        let capture_down = io.key_down(self.config.capture_toggle_key);
        if capture_down && !self.keys.capture && hotkeys_live {
            self.toggle_capture(io);
        }
        self.keys.capture = capture_down;

        let ab_key = self.config.ab_toggle_key;
        let ab_down = ab_key != UNBOUND && io.key_down(ab_key);
        if ab_down && !self.keys.ab && hotkeys_live {
            self.request_ab_toggle();
        }
        self.keys.ab = ab_down;
    }

    fn toggle_capture(&mut self, io: &mut impl ControlIo) {
        if self.bind.is_some() {
            self.finish_bind(io);
        } else {
            self.set_capture(!self.capture, io);
        }
    }

    fn set_capture(&mut self, on: bool, io: &mut impl ControlIo) {
        if on == self.capture {
            return;
        }
        self.capture = on;
        self.restart_input(io);
        self.marker = if on {
            Marker::CaptureOn
        } else {
            Marker::CaptureOff
        };
    }

    fn restart_input(&mut self, io: &mut impl ControlIo) {
        self.state.reset();
        self.mark_centered(io);
        io.clear_input();
        self.guards = [ButtonGuard::default(); 2];
    }

    fn steering_live(&self) -> bool {
        self.capture && self.session.is_active()
    }

    fn start_bind(&mut self, target: BindTarget, io: &mut impl ControlIo) {
        if self.bind.is_none() {
            self.capture_before_bind = self.capture;
        }
        self.set_capture(false, io);
        self.bind = Some(BindHelper::new(target));
    }

    fn finish_bind(&mut self, io: &mut impl ControlIo) {
        if self.bind.take().is_some() {
            self.set_capture(self.capture_before_bind, io);
        }
    }

    fn recenter(&mut self, io: &impl ControlIo) {
        self.state.reset_steering();
        self.mark_centered(io);
    }

    fn mark_centered(&mut self, io: &impl ControlIo) {
        self.drops_at_center = io.input_counters().dropped_reports;
    }

    fn drops_since_center(&self) -> u64 {
        if !self.capture {
            return 0;
        }
        self.counters
            .dropped_reports
            .saturating_sub(self.drops_at_center)
    }

    fn read_input(&mut self, dt_ms: f64, io: &mut impl ControlIo) -> TickInput {
        let motion = io.take_motion();
        if let Some(oldest_ns) = motion.oldest_ns {
            let age_ns = io.now_ns().saturating_sub(oldest_ns);
            self.latency.record_input_age(age_ns as f64 / NANOS_PER_MS);
        }
        let counts = motion.counts;
        self.counts_total = self.counts_total.wrapping_add(counts);
        if io.take_middle_click() && self.session.is_active() {
            self.recenter(io);
        }
        self.check_stuck_buttons(dt_ms, io);
        let counters = io.input_counters();
        let new_events = counters.move_events.wrapping_sub(self.counters.move_events);
        self.rate.update(new_events, dt_ms);
        self.counters = counters;
        let (left, right) = io.take_buttons();
        if !self.steering_live() {
            return TickInput {
                dt_ms,
                ..TickInput::default()
            };
        }
        TickInput {
            counts_x: counts,
            throttle_pressed: left,
            brake_pressed: right,
            dt_ms,
        }
    }

    fn check_stuck_buttons(&mut self, dt_ms: f64, io: &mut impl ControlIo) {
        self.guard_elapsed_ms += dt_ms;
        if self.guard_elapsed_ms < CHECK_INTERVAL_MS {
            return;
        }
        self.guard_elapsed_ms = 0.0;
        if self.desktop.check(io.input_desktop_active()) {
            for button in [MouseButton::Left, MouseButton::Right] {
                self.release_stuck(button, "girdi masaüstü değişti", io);
            }
            return;
        }
        if !self.session.is_active() {
            return;
        }
        let (left, right) = io.buttons();
        let swapped = io.buttons_swapped();
        let (l_vk, r_vk) = (io.key_down(VK_LBUTTON), io.key_down(VK_RBUTTON));
        let [left_guard, right_guard] = &mut self.guards;
        let stuck = [
            (
                MouseButton::Left,
                left_guard.check(left, async_pressed(l_vk, r_vk, swapped)),
            ),
            (
                MouseButton::Right,
                right_guard.check(right, async_pressed(r_vk, l_vk, swapped)),
            ),
        ];
        for (button, released) in stuck {
            if released {
                self.release_stuck(button, "bırakma olayı kaçtı", io);
            }
        }
    }

    fn release_stuck(&mut self, button: MouseButton, reason: &str, io: &mut impl ControlIo) {
        if io.force_release(button) {
            self.stuck_releases += 1;
            crate::log::warn(&format!("takılı buton bırakıldı ({reason}): {button:?}"));
        }
    }

    fn next_frame(&mut self, input: TickInput, dt_ms: f64, io: &mut impl ControlIo) -> OutputFrame {
        self.state.tick(&self.config, input);
        self.repair_state();
        if !self.session.is_active() {
            return OutputFrame::NEUTRAL;
        }
        if let Some(frame) = self.bind_frame(dt_ms, io) {
            return frame;
        }
        if self.options.test_counter {
            return OutputFrame::test_pattern(self.tick_count);
        }
        if !self.capture {
            return OutputFrame::NEUTRAL;
        }
        let buttons = self.gear_buttons(io);
        self.state.output_frame(&self.config.tuning, buttons)
    }

    fn repair_state(&mut self) {
        if self.state.repair_non_finite() && !self.non_finite_logged {
            self.non_finite_logged = true;
            crate::log::warn("mantık durumunda sonlu olmayan değer bulundu; sıfırlandı");
        }
    }

    fn gear_buttons(&self, io: &impl ControlIo) -> u32 {
        let c = &self.config;
        if !c.gear_keys_enabled || self.keyboard_busy {
            return 0;
        }
        let up = if io.key_down(c.gear_up_key) {
            BUTTON_GEAR_UP
        } else {
            0
        };
        let down = if io.key_down(c.gear_down_key) {
            BUTTON_GEAR_DOWN
        } else {
            0
        };
        up | down
    }

    fn bind_frame(&mut self, dt_ms: f64, io: &mut impl ControlIo) -> Option<OutputFrame> {
        let helper = self.bind.as_mut()?;
        let was_countdown = matches!(helper.phase(), BindPhase::Countdown(_));
        helper.advance(dt_ms);
        let phase = helper.phase();
        let frame = helper.frame();
        if was_countdown && matches!(phase, BindPhase::Sweep(_)) {
            self.play(io, Cue::Bind);
        }
        if phase == BindPhase::Done {
            self.finish_bind(io);
        }
        Some(frame)
    }

    fn update_health(&mut self, dt_ms: f64, io: &mut impl ControlIo) {
        self.health_elapsed_ms += dt_ms;
        if self.health_elapsed_ms < HEALTH_WINDOW_MS {
            return;
        }
        self.health_elapsed_ms = 0.0;
        self.loop_summary = self.loop_stats.summary();
        self.latency_summary = self.latency.summary();
        let writes = self.conn.writes();
        let clipped = self.state.clipped_ticks;
        self.health = Health {
            write_failures: writes.failed_writes.saturating_sub(self.last_failed_writes),
            loop_hz: self.loop_summary.hz,
            target_hz: 1000.0 / self.interval_ms(),
            clipped_ticks: clipped.saturating_sub(self.last_clipped),
            version_mismatch: self.conn.report().versions.is_some_and(|v| !v.matched),
        };
        self.last_failed_writes = writes.failed_writes;
        self.last_clipped = clipped;
        self.registration_ok = io.registration_ok();
        if !self.registration_ok {
            io.request_register();
        }
        if self.device_filter == DeviceFilter::SelectedMissing {
            io.refresh_filter();
        }
        if self.priorities.any_realtime() {
            io.keep_cpu_budget();
        }
    }

    fn settling(&mut self, dt_ms: f64, io: &impl ControlIo) -> bool {
        if self.settle_left_ms <= 0.0 || !self.session.is_active() {
            return false;
        }
        self.settle_left_ms -= dt_ms;
        if self.settle_left_ms > 0.0 {
            return true;
        }
        self.registration_ok = io.registration_ok();
        false
    }

    fn update_status(&mut self, dt_ms: f64, now_ms: f64, io: &mut impl ControlIo) {
        self.device_filter = io.device_filter();
        let settling = self.settling(dt_ms, io);
        let sink = self.config.input_sink_enabled;
        let inputs = StatusInputs {
            connection: self.conn.connection(),
            setup_failures: self.conn.setup_failures(),
            capture_enabled: self.capture,
            binding: self.bind.is_some(),
            input_sink_enabled: sink,
            app_foreground: !sink && io.app_foreground(),
            mouse_missing: !settling && self.device_filter == DeviceFilter::SelectedMissing,
            pointer_readable: settling || self.registration_ok,
            session: self.session,
            health: self.health,
        };
        self.status = derive_status(&inputs);
        if !self.session.is_active() {
            return;
        }
        if let Some(cue) = self.cues.on_status(self.status, now_ms) {
            self.play(io, cue);
        }
    }

    fn play(&self, io: &mut impl ControlIo, cue: Cue) {
        if self.config.sounds_enabled {
            io.play(cue, self.config.sound_volume.clamp(0, 100) as u8);
        }
    }

    fn publish_setup(&mut self, shared: &Shared) {
        let seq = self.conn.report_seq();
        if seq != self.published_setup_seq {
            self.published_setup_seq = seq;
            shared.store_setup(self.conn.report().clone(), self.conn.backend_desc());
        }
    }

    fn push_telemetry(
        &mut self,
        shared: &Shared,
        now_ms: f64,
        input: TickInput,
        frame: &OutputFrame,
    ) {
        let s = &self.state;
        let brake_scale = self.config.tuning.brake_max_output;
        let marker = self.marker;
        let pushed = shared.push_telemetry(Sample {
            seq: 0,
            t_ms: now_ms,
            counts_x: input
                .counts_x
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            steering_raw: (s.steering / STEERING_RANGE) as f32,
            steering_out: frame.steering as f32,
            throttle_pressed: input.throttle_pressed,
            throttle_target: s.throttle_target as f32,
            throttle_out: frame.throttle as f32,
            brake_pressed: input.brake_pressed,
            brake_floor: (s.brake_floor * brake_scale) as f32,
            brake_out: frame.brake as f32,
            brake_phase: s.brake_phase(),
            capture: self.capture,
            marker,
        });
        if pushed {
            self.marker = Marker::None;
        } else {
            self.telemetry_dropped += 1;
        }
    }

    fn snapshot(&self, frame: &OutputFrame, input: TickInput, now_ms: f64) -> Snapshot {
        let s = &self.state;
        Snapshot {
            status: self.status,
            connection: self.conn.connection(),
            capture: self.capture,
            binding: self.bind.map(|b| (b.target, b.phase())),
            ab_side: self.ab.as_ref().map(|a| a.side),
            pending_apply: self.pending.is_some(),
            frame: *frame,
            steering_raw: s.steering / STEERING_RANGE,
            throttle_target: s.throttle_target,
            throttle_pressed: input.throttle_pressed,
            brake_pressed: input.brake_pressed,
            brake_floor: s.brake_floor * self.config.tuning.brake_max_output,
            brake_phase: s.brake_phase(),
            loop_summary: self.loop_summary,
            latency: self.latency_summary,
            priorities: self.priorities,
            target_hz: 1000.0 / self.interval_ms(),
            writes: self.conn.writes(),
            reconnects: self.conn.reconnects(),
            retry_in_ms: self.conn.retry_in_ms(now_ms),
            clipped_ticks: s.clipped_ticks,
            mouse_hz: self.rate.peak_hz(),
            absolute_events: self.counters.absolute_events,
            ignored_events: self.counters.ignored_events,
            dropped_reports: self.counters.dropped_reports,
            drops_since_center: self.drops_since_center(),
            reconcile_releases: self.counters.reconcile_releases,
            input_stopped: self.counters.thread_stopped,
            device_filter: self.device_filter,
            registration_ok: self.registration_ok,
            session: self.session,
            stuck_releases: self.stuck_releases,
            telemetry_dropped: self.telemetry_dropped,
            counts_total: self.counts_total,
            health: self.health,
            test_counter: self.options.test_counter,
            tick: self.tick_count,
        }
    }
}

fn session_change_note(session: SessionState) -> &'static str {
    match session {
        SessionState::Active => "oturum yeniden etkin; girdi okunmaya devam ediyor",
        SessionState::Locked => "ekran kilitlendi; girdi duraklatıldı, oyuna nötr değerler gidiyor",
        SessionState::Inactive => {
            "oturum etkin değil (başka bir kullanıcı ya da konsol önde); girdi duraklatıldı, oyuna nötr değerler gidiyor"
        }
    }
}
