use std::sync::mpsc::Receiver;

use crate::bind::{BindHelper, BindPhase, BindTarget};
use crate::config::{Config, Tuning};
use crate::diagnostics::{LoopStats, LoopSummary, RateMeter};
use crate::input::{DeviceFilter, InputCounters, MouseButton};
use crate::keys::{UNBOUND, VK_LBUTTON, VK_RBUTTON};
use crate::logic::{MouseDriveState, STEERING_RANGE, TickInput};
use crate::output::{BUTTON_GEAR_DOWN, BUTTON_GEAR_UP, OutputFrame};
use crate::setup::REQUIRED_BUTTONS;
use crate::status::{AppStatus, Cue, CuePlanner, Health, StatusInputs, derive_status};
use crate::telemetry::{Marker, Sample};

use super::connection::ConnectionManager;
use super::guard::{ButtonGuard, CHECK_INTERVAL_MS, async_pressed};
use super::io::ControlIo;
use super::{Command, Options, Shared, Snapshot};

const HEALTH_WINDOW_MS: f64 = 1000.0;
const LOOP_STATS_WINDOW: usize = 1000;

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
    stuck_releases: u64,
    conn: ConnectionManager,
    published_setup_seq: u64,
    loop_stats: LoopStats,
    loop_summary: LoopSummary,
    rate: RateMeter,
    counters: InputCounters,
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

impl Engine {
    pub(crate) fn new(config: Config, epoch: u64, options: Options) -> Self {
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
            stuck_releases: 0,
            conn,
            published_setup_seq: 0,
            loop_stats: LoopStats::new(LOOP_STATS_WINDOW),
            loop_summary: LoopSummary::default(),
            rate: RateMeter::default(),
            counters: InputCounters::default(),
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
        }
    }

    pub(crate) fn interval_ms(&self) -> f64 {
        f64::from(self.config.thread_interval_ms.clamp(1, 20))
    }

    pub(crate) fn tick(
        &mut self,
        now_ms: f64,
        io: &mut impl ControlIo,
        shared: &Shared,
        commands: &Receiver<Command>,
    ) {
        let dt_ms = self.advance_clock(now_ms, io);
        self.pick_up_config(shared, io, now_ms);
        while let Ok(command) = commands.try_recv() {
            self.handle_command(command, io, now_ms);
        }
        self.poll_hotkeys(io, shared);
        self.conn.poll(now_ms, io);
        self.publish_setup(shared);
        let input = self.read_input(dt_ms, io);
        self.apply_pending(shared, io);
        let frame = self.next_frame(input, dt_ms, io);
        self.conn.write(&frame, now_ms);
        self.update_health(dt_ms, io);
        self.update_status(now_ms, io);
        self.push_telemetry(shared, now_ms, input, &frame);
        shared.store_snapshot(self.snapshot(&frame, input, now_ms));
    }

    fn advance_clock(&mut self, now_ms: f64, io: &mut impl ControlIo) -> f64 {
        let dt_ms = match self.last_now_ms {
            Some(prev) => (now_ms - prev).max(0.0),
            None => {
                self.apply_side_effects(io, now_ms);
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
        let (epoch, config) = shared.config();
        self.seen_epoch = epoch;
        self.state
            .reseed_after_config_change(&self.config.tuning, &config.tuning);
        self.config = config;
        self.apply_side_effects(io, now_ms);
    }

    fn apply_side_effects(&mut self, io: &mut impl ControlIo, now_ms: f64) {
        let c = &self.config;
        io.apply_input_settings(c.input_sink_enabled, &c.mouse_device);
        io.overlay_config(c.overlay_enabled, c.overlay_corner);
        self.conn
            .set_target(device_id(c), buttons_required(c), now_ms);
    }

    fn apply_pending(&mut self, shared: &Shared, io: &mut impl ControlIo) {
        if self.capture && !self.state.pedals_idle() {
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
            Command::Reconnect => self.conn.reconnect_now(now_ms),
            Command::ResetSteering => self.state.reset_steering(),
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

    fn poll_hotkeys(&mut self, io: &mut impl ControlIo, shared: &Shared) {
        self.keyboard_busy = shared.gui_keyboard() != 0 && io.app_foreground();
        let capture_down = io.key_down(self.config.capture_toggle_key);
        if capture_down && !self.keys.capture && !self.keyboard_busy {
            self.toggle_capture(io);
        }
        self.keys.capture = capture_down;

        let ab_key = self.config.ab_toggle_key;
        let ab_down = ab_key != UNBOUND && io.key_down(ab_key);
        if ab_down && !self.keys.ab && !self.keyboard_busy {
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
        self.state.reset();
        io.clear_input();
        self.guards = [ButtonGuard::default(); 2];
        self.marker = if on {
            Marker::CaptureOn
        } else {
            Marker::CaptureOff
        };
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

    fn read_input(&mut self, dt_ms: f64, io: &mut impl ControlIo) -> TickInput {
        let counts = io.take_counts();
        self.counts_total = self.counts_total.wrapping_add(counts);
        if io.take_middle_click() {
            self.state.reset_steering();
        }
        self.check_stuck_buttons(dt_ms, io);
        let counters = io.input_counters();
        let new_events = counters.move_events.wrapping_sub(self.counters.move_events);
        self.rate.update(new_events, dt_ms);
        self.counters = counters;
        if !self.capture {
            return TickInput {
                dt_ms,
                ..TickInput::default()
            };
        }
        let (left, right) = io.buttons();
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
        let (left, right) = io.buttons();
        let swapped = io.buttons_swapped();
        let (l_vk, r_vk) = (io.key_down(VK_LBUTTON), io.key_down(VK_RBUTTON));
        let checks = [
            (MouseButton::Left, left, async_pressed(l_vk, r_vk, swapped)),
            (
                MouseButton::Right,
                right,
                async_pressed(r_vk, l_vk, swapped),
            ),
        ];
        for (guard, (button, raw, async_down)) in self.guards.iter_mut().zip(checks) {
            if guard.check(raw, async_down) && io.force_release(button) {
                self.stuck_releases += 1;
                crate::log::line(&format!("takılı buton bırakıldı: {button:?}"));
            }
        }
    }

    fn next_frame(&mut self, input: TickInput, dt_ms: f64, io: &mut impl ControlIo) -> OutputFrame {
        self.state.tick(&self.config, input);
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
        self.device_filter = io.device_filter();
        self.registration_ok = io.registration_ok();
        if !self.registration_ok {
            io.request_register();
        }
    }

    fn update_status(&mut self, now_ms: f64, io: &mut impl ControlIo) {
        let sink = self.config.input_sink_enabled;
        let inputs = StatusInputs {
            connection: self.conn.connection(),
            setup_failures: self.conn.setup_failures(),
            capture_enabled: self.capture,
            binding: self.bind.is_some(),
            input_sink_enabled: sink,
            app_foreground: !sink && io.app_foreground(),
            health: self.health,
        };
        self.status = derive_status(&inputs);
        if let Some(cue) = self.cues.on_status(self.status, now_ms) {
            self.play(io, cue);
        }
        io.overlay_status(self.status);
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
        shared.push_telemetry(Sample {
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
            marker: std::mem::take(&mut self.marker),
        });
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
            target_hz: 1000.0 / self.interval_ms(),
            writes: self.conn.writes(),
            reconnects: self.conn.reconnects(),
            retry_in_ms: self.conn.retry_in_ms(now_ms),
            clipped_ticks: s.clipped_ticks,
            mouse_hz: self.rate.peak_hz(),
            absolute_events: self.counters.absolute_events,
            ignored_events: self.counters.ignored_events,
            device_filter: self.device_filter,
            registration_ok: self.registration_ok,
            stuck_releases: self.stuck_releases,
            counts_total: self.counts_total,
            health: self.health,
            test_counter: self.options.test_counter,
        }
    }
}
