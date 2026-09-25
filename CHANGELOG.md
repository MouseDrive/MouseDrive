# Changelog

All notable changes to MouseDrive are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Input fidelity, a robust vJoy backend, profiles and a reworked interface. Items
reference the input/UI/driver research report (G = input and output fixes,
E = ergonomics). The own-driver work (UMDF2/VHF) is deferred.

### Input fidelity

- **G1 — Lossless mouse counts.** Raw X counts are accumulated as integers on
  the Raw Input thread; DPI scale and sensitivity are applied later in `f64`.
  Previously each event was rounded, so slow movements with a DPI scale below
  1 were lost entirely.
- **G10 — No per-event cap.** The fixed 180-count cap is gone. An optional
  *spike filter* (maximum steering rate, %/s; off by default) limits single
  jumps and counts clipped ticks in diagnostics.
- **G9 — Time-based smoothing.** *Smoothed* mode uses a time constant in ms
  instead of a per-tick alpha, so the feel no longer changes with the loop
  rate. Old configs are migrated (`config_version` 2) to the same feel.
- **New *Adaptive* steering mode** — One Euro filter: smooth when the mouse is
  slow, no lag on fast corrections.
- **G11 — Logical clock.** The driving logic is pure: one `TickInput` per tick
  and a single logical clock (tick gaps capped at 50 ms). Ramps and hold phases
  run on real durations; the old compressed `time_scale` is gone.
- **G13 — Mouse selection.** Read all mice or only one device ("pick the last
  moved mouse"); absolute-position events (tablets, remote desktop) are
  ignored and reported once.
- **G6 — Deadline pacing.** The control loop schedules each tick from the
  previous deadline instead of sleeping a relative interval, so it holds 250 Hz
  instead of ~230 Hz. Measured loop rate and p99 are shown in diagnostics.
- **G15 — Power throttling opt-out.** The process asks Windows not to apply
  EcoQoS/timer throttling when it runs in the background.

### vJoy backend

- **G5 — One report per tick.** All axes and buttons are written with a single
  `UpdateVJD` call, so a game can never read steering from one tick and brake
  from another. Measured at 250 Hz: 0 torn reports in 1252.
- **G2 — Correct FFI types.** `BOOL` returns are `i32`, matching the SDK.
- **G3 — Health tracking and reconnect.** Write results are checked; 25
  consecutive failures or a vJoy removal notice (`RegisterRemovalCB`) switch
  to CONNECTION LOST, and MouseDrive reconnects on its own. Setup problems are
  retried every 2 s, so fixing vJoy needs no restart.
- **G4 — Connection checks.** Driver enabled, DLL/driver version match, device
  status (free/own/busy/missing), required axes and buttons — each reported in
  the new setup window. When several `vJoyInterface.dll` copies exist, the one
  matching the driver is preferred.
- **G14 — Axis range from the driver** (`GetVJDAxisMin/Max`) instead of a fixed
  0–32767.
- **Neutral frame** on acquire and on normal exit (wheel centred, pedals at 0).
- **§5.9 — `OutputBackend` trait** with a normalised `OutputFrame`; vJoy is one
  backend, and a future own driver or USB dongle can plug in without touching
  the logic.

### Safety

- **G8 — Stuck-button guard.** Raw Input misses button releases while the
  secure desktop (UAC, Ctrl+Alt+Del) is up; the control thread cross-checks with
  `GetAsyncKeyState` every ~100 ms and releases a stuck pedal (swapped mouse
  buttons handled).
- **G7 — Configurable gear keys**, sent only while capture is on and suppressed
  while typing in MouseDrive.
- Profile and A/B switches are applied only when both pedals are released.

### Ergonomics

- **E1 — Steering speed in cm per full lock**, with a type-in field and a
  *Measure DPI* tool (move the mouse along a ruler).
- **E2 — Axis bind helper:** 5 s countdown, then only the chosen axis or gear
  button moves for 4 s, so the game binds the right one.
- **E3 — In-game settings checklist** in *General*; its *Full guide* button
  opens the [In-game settings](README.md#in-game-settings) section.
- **Brake ceiling** (`brake_max_output`, max brake %) scales every brake phase
  at the output.

### Profiles and tuning workflow

- **Profiles** in `profiles\<name>.toml` hold only driving settings, so they can
  be shared; app settings stay in `config.toml`. Save, save as, duplicate,
  rename, delete, with name validation and an unsaved-changes prompt.
- **Undo / redo** (Ctrl+Z, Ctrl+Y / Ctrl+Shift+Z), *Revert to saved*, per-setting
  *reset to default* and *reset this tab*.
- **A/B comparison** switchable by hotkey while driving.

### Interface

- **Single status model** — eight statuses (ACTIVE, PAUSED, SETUP REQUIRED,
  DEVICE BUSY, CONNECTION LOST, MOUSE NOT READ, DEGRADED, BINDING), each with a
  sentence and one action; derived on the control thread.
- **Status sounds** (synthesised tones, own audio thread) and an optional
  **status overlay** (native click-through topmost window on its own thread).
- **Setup and health check** window with ✔/⚠/✖ rows, fixes, live axis test and
  next steps.
- **Input monitor** — 5/10/30 s chart of raw input against the output sent to
  the game, with capture/profile markers and freeze. Fed by a lock-free
  telemetry ring buffer on the control thread.
- **Settings rework** — basic/advanced split, mode-dependent sliders, units on
  every value, tooltips with range and default, changed markers.
- **Throttle tab** — the corner cut is explained in one sentence plus a mini
  chart; **Brake tab** — five phases in order under a live timeline simulated by
  the real logic.
- **Curve editor** — numeric entry, keyboard control (Page Up/Down, arrows,
  Delete) and screen-reader descriptions.
- **Diagnostics line** and **Copy diagnostics** for bug reports.
- **Accessibility** — colour-blind palette (Okabe-Ito), brake always dashed,
  AccessKit labels on custom widgets, rebindable hotkeys including mouse side
  buttons, interface scale 75–200 %.
- **Microcopy** — labels with units, verb buttons, errors that say what happened
  and what to do; Turkish strings with proper Turkish characters.
- **No silent fallbacks** — corrected values and unreadable files are reported
  by name; broken files are backed up; all writes are atomic.

### Tooling

- **Measured output** — SendInput → HID report latency p50 2.3 ms, p99 4.3 ms
  at 250 Hz.
- **CI** — `cargo fmt --check` and clippy with and without the updater feature;
  multi-line steps run in bash so every command's exit code counts. The release
  zip contains `mousedrive.exe`, README and LICENSE; `mousedrive.exe` is also
  attached on its own, and `SHA256SUMS.txt` lists both (auto-update still
  installs from the zip).
- Code split into focused modules (`control/`, `logic/`, `ui/`, `lang/`, …).

### Documentation

- README: new *In-game settings* section (checklist, binding, `joy.cpl` check).
- README troubleshooting now covers process death: if MouseDrive is killed,
  vJoy keeps the last axis values until MouseDrive starts again.

### License

- MouseDrive is now licensed under GPL-3.0-or-later. Releases up to v0.5.0
  remain available under the MIT License.
- The license grants no rights to the MouseDrive name or logo; modified versions
  must not be presented as the original.

## [0.5.0] - 2026-06-14

The headline of this release is the **decoupled control architecture**: the
250 Hz vJoy feed now runs on its own thread, completely independent of the GUI.

### Performance & architecture

- **Control loop decoupled from the GUI thread** — `control.rs` owns the vJoy
  handle and runs a 250 Hz loop on a dedicated `THREAD_PRIORITY_HIGHEST` thread.
  The vJoy feed no longer stalls when the window is minimized or the repaint
  lags — previously the steering axis froze and the accumulated mouse delta was
  applied as one jump on restore. The GUI now only reads a shared lock-free
  snapshot and publishes config changes; control math is byte-identical (all 41
  logic tests unchanged).
- **Lazy GUI repaint** — ~60 Hz focused, ~4 Hz backgrounded. Control cadence is
  fully independent of repaint rate.
- **`panic = "abort"`** in the release profile (smaller binary, no unwind tables).
- Configurable **vJoy device id** (1–16) in Settings > General — no longer stuck
  on DeviceBusy when Device 1 is in use.
- **Lightweight file logging** (`mousedrive.log` next to the config) for
  connect/reconnect outcomes, missing DLL/symbol, and update failures; never on
  the hot path. `VJoyApi` relinquishes the device on `Drop` as a shutdown backstop.
- **`exit_on_close` now works** — unchecked → minimize (vJoy feed keeps running);
  default stays "exit".
- Config validation **surfaces the corrected-field count** in the UI; added a
  `config_version` migration hook for future schema changes.
- **Updater is an optional `updater` cargo feature** (default on).
  `--no-default-features` drops ureq/rustls/zip/sha2/self-replace → lean build
  (~3.9 MB vs ~5.3 MB).

### UI / visual

- **Modern dashboard redesign**: accent blue theme (`#378ADD`), vertical
  colour-coded gauges (throttle green / brake red), bidirectional steering bar
  (custom painter, centre-fill), status and input pill badges.
- **Readability fix**: selected tabs / combo-box items now show white text on blue
  background. Previously `selection.stroke.color = ACCENT` made text invisible
  (blue-on-blue).
- Localization: added `steer_left` / `steer_right` (TR "Sol/Sağ", EN "Left/Right").

### Added

- **Graphical envelope curve editors** for throttle rise/fall, brake apply, and
  brake post-hold drop. Drag control points (2–8) directly on the graph,
  double-click to add, right-click to remove. Two interpolation modes: Linear
  and Smooth (monotone cubic / PCHIP — guaranteed no overshoot). Presets:
  Linear, S-Curve, Aggressive, Progressive. A live marker travels along the
  active curve while driving. (Design notes: `graph.md`)
- **Phase-tracking ramp algorithm**: throttle/brake envelopes follow the curves
  with inverse re-seeding on every direction change, so output is continuous
  across press/release/steering-cut transitions. Default identity curves
  reproduce the previous linear behavior exactly — old configs feel identical.
- **Automatic update checks**: once per day on startup (configurable), in a
  background thread that never blocks the control loop. Manual "Check now"
  button in Settings > General.
- **One-click self-update**: a green Update button appears in the top bar when
  a new release is available. Clicking it downloads the release zip, verifies
  it against `SHA256SUMS.txt`, swaps the running executable and restarts
  automatically. Falls back to opening the release page if the release lacks
  standardized assets or installation fails. "Skip" silences a given version.
  (Design notes: `auto-update.md`)
- **CI/CD pipeline** (`.github/workflows/ci.yml`): tests + clippy on every
  push/PR; pushing a `vX.Y.Z` tag builds with the version taken from the tag,
  packages `MouseDrive-vX.Y.Z-windows-x64.zip` + `SHA256SUMS.txt`, and
  publishes a GitHub release automatically.
- New config options: `auto_check_updates`, `skipped_version`, and per-curve
  tables (`throttle_rise_curve`, `throttle_fall_curve`, `brake_apply_curve`,
  `brake_posthold_curve`). Existing `config.toml` files load unchanged.
- `.gitignore` and `CHANGELOG.md`.

### Changed

- Throttle and brake ramp steps are now evaluated through envelope curves; the
  existing `*_ms` sliders remain the time base, curves only shape the profile.
- The brake post-hold falloff curve composes on top of the existing
  "Release Accel Power" exponent (set the exponent to 1.0 for purely graphical
  control). Button-release decay intentionally stays linear.
- Test suite grew from 16 to 41 unit tests (curve math, monotonicity, inverse
  round-trips, legacy-equivalence regression, config compatibility, update
  parsing/verification).

### Fixed

- `main` failed to compile: `Win32_Media` and `Win32_System_Threading` features
  were missing from `Cargo.toml` (regression relative to the v0.4.0 release).
- `Cargo.toml` package version (stuck at 0.3.0) brought in line with the actual
  release line; from now on CI stamps the version from the git tag at build time.

## [0.4.0] - 2026-03-21

- Steering, throttle and brake tuning improvements; UI polish.
- See the [v0.4.0 release](https://github.com/Toxpox/MouseDrive/releases/tag/v0.4.0).

## [0.3.0] - 2026-02-09

- Rust rewrite of the original C++ version: raw-input capture thread, atomic
  lock-free state sharing, eframe/egui interface, TOML configuration, TR/EN
  localization.
- See the [V0.3.0 release](https://github.com/Toxpox/MouseDrive/releases/tag/V0.3.0).

## [0.1.0-alpha] - 2026-02-04

- First public alpha.
- See the [V0.1.0-alpha release](https://github.com/Toxpox/MouseDrive/releases/tag/V0.1.0-alpha).

[0.5.0]: https://github.com/Toxpox/MouseDrive/compare/v0.4.0...main
[0.4.0]: https://github.com/Toxpox/MouseDrive/compare/V0.3.0...v0.4.0
[0.3.0]: https://github.com/Toxpox/MouseDrive/compare/V0.1.0-alpha...V0.3.0
[0.1.0-alpha]: https://github.com/Toxpox/MouseDrive/releases/tag/V0.1.0-alpha
