# Changelog

All notable changes to MouseDrive are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Safer Linux input, narrower device access, a safer updater and proper Linux
packages.

### Updating from 0.7.0

The updater of 0.7.0 installs the next release as before. Two things differ
after it:

- **Narrower udev rule (Linux).** The old rule gave your session access to every
  input device, keyboards and game controllers included. Setup shows a *Setup
  rule* warning while it is installed; run the setup commands again and restart
  the computer. Keyboard access is now an optional second rule (see *Added*).
- **Portable mode needs a marker (Windows and Linux).** An empty file named
  `mousedrive.portable` next to the program turns it on. A `config.toml` next to
  the program that MouseDrive wrote itself (it has a `config_version` key)
  keeps working without the marker. A `config.toml` there without that key is
  not used and not rewritten; if it holds MouseDrive settings, a notice tells
  you to add the marker.

### Added

- **Centre response for Expo steering.** A new *Centre response (%)* setting
  mixes a straight line into the Expo curve, so the wheel starts turning from
  the first mouse count instead of after a numb stretch around the centre. It
  is 0 % by default, which keeps the previous curve.
- **Narrower device access on Linux.** The setup rule now covers `/dev/uinput`
  and mice only. Keyboards, touchpads and game controllers are no longer
  included. Keyboard access, which the capture key, the A/B key and the gear
  keys need, is a separate optional rule that Setup labels with its risk: every
  program in your session can then read all keystrokes; it also covers media-key
  devices, so media keys can be bound. The README explains
  what the rules allow (read, inject, grab) and how to set them up on NixOS and
  without systemd-logind.
- **Setup shows the commands.** The Linux Setup window lists the commands as
  selectable text with one sentence per step, instead of only copying them.
  They work in bash, zsh and fish. New rows check keyboard access and whether
  games can read the virtual wheel, and Setup names Steam's own uinput rule
  when it is the only reason `/dev/uinput` works.
- **Command-line options.** `--version` and `--help` print and exit before
  anything else starts. On Linux, `--list-devices` lists every input device
  with its class and the reason it is not used, `--x11` and `--wayland` choose
  the window backend and `--allow-root` permits running as root. Unknown
  options are reported on stderr and in the log, then ignored.
- **One MouseDrive per user.** A second start says *MouseDrive is already
  running* and exits. After an update the new version waits a few seconds for
  the old one to let go.
- **Clear start-up problems on Linux.** MouseDrive refuses to run as root
  (unless `--allow-root`), says so when no desktop session is available, and
  names the missing system libraries with the package to install. Messages
  also appear through `notify-send`, `zenity` or `kdialog`. If the Wayland
  window cannot open, MouseDrive restarts once through XWayland.
- **Clean shutdown on signals (Linux).** `SIGTERM`, `SIGINT` and `SIGHUP`
  close MouseDrive properly: app settings are saved and the virtual wheel is
  removed.
- **Event log with timestamps.** Lines carry a UTC time in milliseconds, a
  level and the thread, and are written by a background thread that never
  makes the control loop wait. On Linux the log lives in
  `~/.local/state/mousedrive/`. After a crash the next start shows a notice and
  keeps the report as `last_crash.old.txt`.
- **Lost mouse events are detected (Linux).** When the kernel drops events
  because MouseDrive was too slow, held buttons are read again from the kernel
  and released if their release was lost, and the status line suggests
  recentring. A pedal can no longer stay pressed after such an overflow. While
  a pedal is held, the buttons are also re-checked every 100 ms. Diagnostics
  count both cases.
- **Pause on a locked screen or another user's session (Linux).** Input is
  ignored, neutral values go to the game, held keys are released and the status
  shows PAUSED with the reason; MouseDrive continues by itself.
- **Device recognition (Linux).** Devices are classified (mouse, absolute
  pointer, touchpad, keyboard, other keys) and every skipped device keeps its
  reason, shown in Setup, in the log and by `--list-devices`. The touchpad of a
  DualShock or DualSense controller no longer acts as a throttle. When the
  selected mouse is missing and exactly one mouse of the same model shows up
  on another port, that mouse is read and a *Save* button keeps it selected; a
  warning appears when input-remapper, keyd or evsieve has grabbed the
  selected mouse. Virtual devices created by other programs are treated as
  injected whatever bus they claim. Mouse lists refresh by themselves when
  devices are plugged in or out. More keys are recognised (Brazilian and
  Japanese layout keys, browser and launch keys).
- **Thread priority on Linux.** The control and input threads use real-time
  scheduling where the system allows it (with a CPU-time budget so a runaway
  thread cannot freeze the desktop), otherwise a lower nice value, otherwise
  rtkit. The result, and the wake-up delay, tick work and input age of the
  control loop (median, p99, maximum), appear in *Connection details*. The
  control loop sleeps to absolute deadlines.
- **Bigger diagnostics report.** *Copy diagnostics* adds, on Linux, the
  distribution, kernel, glibc, desktop and session type, mouse access, every
  device with class and reason, ignored and lost events, released buttons,
  thread priorities, timing, test options, the single-instance lock and
  portable mode. Bluetooth addresses are masked.
- **Input monitor export.** *Save last 30 s as CSV* writes the samples to the
  log folder, headed by the app version and the active profile settings.
- **Linux packages.** The release zip has a `MouseDrive-vX.Y.Z` top folder with
  the program, README, LICENSE, CHANGELOG, `THIRD_PARTY_LICENSES.html` and
  `packaging/` (udev rules, `uinput` module setting, menu entry, icon,
  AppStream data). Releases also include `.deb` and `.rpm` packages, and a
  PKGBUILD for the AUR package `mousedrive-bin` is provided. The packages
  install the setup rule for `/dev/uinput` and mice, not the optional keyboard
  rule, and are updated by the package manager. The
  window has the app ID `io.github.MouseDrive.MouseDrive`, so desktops match it
  to the menu entry and icon.
- **Signed checksums.** Releases carry build provenance attestations and, when
  a signing key is configured, a minisign signature of `SHA256SUMS.txt` that
  names the release version. The updater checks the signature once a public
  key is embedded in a release; until then a missing signature is only
  logged.

### Changed

- **The brake lets go at once by default.** *Hold after release* now defaults
  to 0 %, so releasing the brake button drops the brake straight to zero instead
  of holding 6 % for half a second. A `config.toml` that already has a value
  keeps it; *Reset this page to defaults* on the brake tab applies the new
  default.
- **The steering speed limit delays instead of dropping.** The optional limit
  (maximum steering rate, formerly called *spike filter*) used to discard mouse
  movement above the limit. Now the surplus is kept and applied on the
  following ticks, so the wheel catches up and the total travel is unchanged.
  The limit is still off by default, and limited ticks are still counted in
  diagnostics.
- **Setup required when no mouse can be read.** The status no longer says
  ACTIVE while capture is on but no mouse can be read, for example on Linux
  without access to the mouse; the top bar's one-click action opens Setup. On
  Linux, touchpads without relative movement now also appear in the list of
  denied devices.
- **Safer updates.** The release is chosen by operating system and CPU, and a
  build for another CPU is never installed. MouseDrive checks that it can write
  to its own folder before downloading, starts the new program with
  `--version` before replacing the old one, keeps the previous program as
  `<program>.old` and puts it back when the new version does not start. Package
  manager installs (under `/usr`, except `/usr/local`, and `/nix/store`) and
  AppImages only get a link to the release page. Failures are named in the log
  and in the update notice. Downloads use the HTTP proxy from `https_proxy`,
  `all_proxy` or `http_proxy` (SOCKS proxies are not supported; `no_proxy` is
  honoured) and, on Linux, also trust the system certificates.
- **Config and profile files are handled more carefully.** Saves are written in
  order by a background thread, so a slow disk never freezes the window, and
  everything pending is written before MouseDrive quits. A symlinked
  `config.toml` or profile stays a symlink and keeps its permissions, the
  folder is flushed after a save, and an earlier backup is never overwritten. A
  `config.toml` from a newer MouseDrive, or one with settings this version does
  not know, is backed up and reported before anything is written; if the backup
  fails, settings are not saved in that session. Profile names are matched
  exactly first, then ignoring letter case; if two profile files differ only in
  case, a name that matches both is refused.
- **The DPI measurement ends by itself.** It finishes 600 ms after the mouse
  stops once it has moved; Space and Enter finish it too, and *Done* has the
  keyboard focus.
- **Window and accessibility polish.** Clicking the key button again, pressing
  Esc, or waiting 10 s ends key listening. The window repaints at about 7 Hz
  while nothing changes. With *Exit on close* off, a second close within 3 s
  quits when the window manager cannot minimise. Ctrl+Plus, Ctrl+Minus and
  Ctrl+0 change the saved interface scale. Screen readers announce status
  changes (urgently for SETUP REQUIRED and CONNECTION LOST). On Linux the
  window keeps the dark theme and asks for dark window decorations; the *full
  guide* link opens the Linux README, which now explains `evtest` and
  `jstest-gtk`.
- **Control loop details.** Steering blends smoothly when the mode is changed
  while driving. The self-centering spring strength is capped at its maximum.
  The brake is never raised when it enters the release hold. A button pressed
  and released between two ticks is no longer lost. Loop statistics restart
  when the loop rate changes. A stalled control loop is reported, and the input
  monitor notes samples it had to drop. The virtual wheel is registered with
  neutral axis values, and a Setup recheck keeps a working wheel instead of
  recreating it.
- **Build.** MouseDrive uses eframe and egui 0.36 (still on the glow renderer),
  rustls 0.23.45 and the pinned Rust 1.98.0 toolchain; the minimum supported
  Rust version is 1.95. The Linux release is built on Ubuntu 22.04 and checked
  to need no glibc newer than 2.35. CI also runs the tests, a dependency
  audit (`cargo deny`) and a tag-against-version check.

### Fixed

- **A pedal no longer sticks after lost evdev events (Linux).** A burst of
  mouse input above the kernel's buffer could lose a button release and leave
  the throttle or brake pressed.
- **Typing on the lock screen no longer reaches the game (Linux).** A password
  typed on the lock screen could trigger the capture key or the gear keys, and
  mouse input reached the game while the screen was locked. Input devices whose
  access was revoked are now closed, which also releases the keys held on
  them.
- **Unreadable devices are not reopened on every rescan (Linux).** Denied and
  unused input nodes are remembered until their permissions change. A
  temporary polling error no longer ends the input thread for good; it
  retries, and diagnostics say so if the thread does stop.
- **A `config.toml` of another program next to MouseDrive is left alone.** A
  file next to the program that has none of MouseDrive's settings is neither
  used nor rewritten.
- **Interrupted updates leave nothing behind.** Leftover files of an update that
  was cut off are removed before the next update starts, and the cleanup of an
  old backup can no longer delete the backup of a newer update.

### Documentation

- The README describes the Linux setup in full: the read, inject and grab risk
  of the udev rules, the optional keyboard rule, NixOS, systems without
  systemd-logind, Steam Deck, Wayland and XWayland, priority and timing, and
  all command-line options.
- **Proton and Wine show the brake as the Z axis**, not Rz, so controls saved on
  Windows do not carry over axis by axis; the README and Setup say so.

## [0.7.0] - 2026-10-03

Linux support, a redesigned main window and safer updates.

### Updating from 0.6.0

0.6.0's updater installs this version, so the fixes below apply from the next
update on. With 0.6.0, MouseDrive restarts as soon as the download finishes
and does not ask about unsaved changes: save your profile and leave the game
before you click *Update*.

### Added

- **Linux support.** A native x86-64 build reads mice and keyboards through
  evdev (hotplug through inotify) and writes to a uinput virtual joystick,
  *MouseDrive Virtual Wheel* (X, Y, Rz, 8 buttons, vJoy's axis range). Sounds
  use ALSA and settings live in `~/.config/mousedrive/`. The Setup window
  checks uinput, its permission and mouse access, and copies the one-time udev
  commands.
- Releases include `MouseDrive-vX.Y.Z-linux-x64.zip`; auto-update installs the
  build for the running platform.
- **MouseDrive says when it was updated.** After the restart, a notice names
  the old and the new version and links the changelog (from the next update
  on).

### Changed

- **Redesigned main window.** A navigation bar on the left replaces the tabs:
  *Overview*, *Steering*, *Throttle*, *Brake*, *Input monitor* and *App
  settings* (formerly *General*). The top bar shows the state, the one action
  that fixes it and short explanations; settings pages keep a *Live output*
  panel on the right. The bottom line shows the output, loop rate and p99
  latency and lists problems only when they happen; *Connection details*
  opens the full diagnostics and *Copy diagnostics*. *Overview* combines the
  live gauges with three getting-started steps and tips. In narrow windows the
  navigation moves above the page.
- New dark theme (carbon black, cream and red); colorblind mode keeps the
  Okabe-Ito colours. The top bar shows the MouseDrive logo: the wheel and
  cursor mark next to a two-tone *MouseDrive* wordmark, in the theme colours.
  The window opens at 1180×680, so it fits 1366×768 screens.
- **Steering deadzone is off by default** and marked experimental; it did not
  help in practice. Profiles still on the old 2 % default are set to 0 once
  after the update, and a notice lists them; other values are kept.
- **The control loop no longer waits for the input monitor.** While the
  monitor copies its history, the control thread skips that tick's sample
  instead of waiting; *Copy diagnostics* shows how many were skipped.

### Removed

- **Status overlay.** The optional status label over the game was a topmost,
  click-through window, the kind anti-cheat software may flag as a cheat
  overlay. It is removed so MouseDrive cannot trigger anti-cheat that way.
  Status sounds and the top bar remain. The `overlay_enabled` and
  `overlay_corner` keys in `config.toml` are ignored and dropped on the next
  save.

### Fixed

- **Updates no longer drop unsaved profile changes.** If the active profile
  has unsaved changes when an update finishes installing, MouseDrive asks
  first (*Save and restart*, *Restart without saving*, *Later*) instead of
  restarting at once. After *Later*, *Restart now* in the top bar finishes the
  update; otherwise it applies on the next start.
- **Closing during an update install no longer breaks the installation.**
  MouseDrive stays open with an *Installing update* notice and closes by
  itself when the installation ends. *Quit now* leaves without waiting;
  MouseDrive stays intact even if the update does not finish. *Keep open*
  cancels the close. The new version is written to a fresh, owner-only
  temporary file.
- **Updates no longer restart MouseDrive while you drive.** If another window,
  such as the game, is in front when an update finishes installing,
  MouseDrive neither restarts nor asks; *Restart now* in the top bar finishes
  the update, otherwise it applies on the next start. Hovering *Update*
  explains what happens.
- **A failed restart after an update is reported.** If MouseDrive cannot
  start the new version, a message asks you to start it yourself instead of
  MouseDrive silently staying closed (on Linux the message is printed to the
  terminal). The restart uses the path MouseDrive was launched with, which
  also works on Linux, where the running file is gone once the update is
  installed.
- **Unreadable profiles are backed up.** If the active profile cannot be read
  at startup, or a profile picked from the list cannot be loaded, it is copied
  to `<name>.toml.<timestamp>.bak` before a later Save can overwrite it (once
  per content, not on every start); a failed backup is reported.
- **Profile and A/B switches wait until the pedals are really idle.** A
  switch no longer slips through when a pedal is pressed in the same moment,
  or while the throttle is still fading out after release; the new curve
  takes over once the throttle reaches zero.
- **An unplugged selected mouse no longer hands steering to other mice.** If
  the mouse chosen in *App settings → Mouse* is not connected, MouseDrive
  reads no mouse and shows *MOUSE NOT READ* with *Use all mice*, instead of
  quietly reading every mouse (a touchpad could steer). Buttons held at that
  moment are released. *Use all mice* clears the saved selection. MouseDrive
  keeps looking for the selected mouse while it is missing, and *Pick the last
  moved mouse* now works in that state too.
- **A pedal no longer stays pressed after a UAC prompt or the lock screen.**
  When the secure desktop (UAC, Ctrl+Alt+Del) or the lock screen takes over
  input, both pedals are released within about 200 ms (Windows). Before, a
  press made just before the switch, which the stuck-button guard had not yet
  confirmed, could stay held.

## [0.6.0] - 2026-09-25

Input fidelity, a robust vJoy backend, profiles and a reworked interface.

### Input fidelity

- **Lossless mouse counts.** Raw X counts are accumulated as integers on
  the Raw Input thread; DPI scale and sensitivity are applied later in `f64`.
  Previously each event was scaled and rounded on its own, so slow movements
  came out too small or too large depending on the DPI scale.
- **No per-event cap.** The fixed 180-count cap is gone. An optional
  *spike filter* (maximum steering rate, %/s; off by default) limits single
  jumps and counts clipped ticks in diagnostics.
- **Time-based smoothing.** *Smoothed* mode uses a time constant in ms
  instead of a per-tick alpha, so the feel no longer changes with the loop
  rate. Old configs are migrated (`config_version` 2) to the same feel.
- **New *Adaptive* steering mode** — One Euro filter: smooth when the mouse is
  slow, little lag on fast corrections.
- **Logical clock.** The driving logic is pure: one `TickInput` per tick
  and a single logical clock (tick gaps capped at 50 ms). Ramps and hold phases
  run on real durations; the old compressed `time_scale` is gone.
- **Mouse selection.** Read all mice or only one device ("pick the last
  moved mouse"); absolute-position events (tablets, remote desktop) do not
  steer and are reported once; their buttons still count.
- **Deadline pacing.** The control loop schedules each tick from the
  previous deadline instead of sleeping a relative interval, so it holds 250 Hz
  instead of ~230 Hz. Measured loop rate and p99 are shown in diagnostics.
- **Power throttling opt-out.** The process asks Windows not to apply
  EcoQoS/timer throttling when it runs in the background.

### vJoy backend

- **One report per tick.** All axes and buttons are written with a single
  `UpdateVJD` call, so a game can never read steering from one tick and brake
  from another. Developer measurement (Windows 11, vJoy 2.2.2.0, 250 Hz):
  0 torn reports in 1252.
- **Correct FFI types.** `BOOL` returns are `i32`, matching the SDK.
- **Health tracking and reconnect.** Write results are checked; 25
  consecutive failures or a vJoy removal notice (`RegisterRemovalCB`) switch
  to CONNECTION LOST, and MouseDrive reconnects on its own. Setup problems are
  retried every 2 s, so fixing vJoy needs no restart.
- **Connection checks.** Driver enabled, DLL/driver version match, device
  status (free/own/busy/missing), required axes and buttons — each reported in
  the new setup window. When several `vJoyInterface.dll` copies exist, the one
  matching the driver is preferred.
- **Axis range from the driver** (`GetVJDAxisMin/Max`) instead of a fixed
  0–32767.
- **Neutral frame** on acquire and on normal exit (wheel centred, pedals at 0).
- **`OutputBackend` trait** with a normalised `OutputFrame`; vJoy is one
  backend, and a future own driver or USB dongle can plug in without touching
  the logic.

### Safety

- **Stuck-button guard.** Raw Input misses button releases while the
  secure desktop (UAC, Ctrl+Alt+Del) is up; the control thread cross-checks with
  `GetAsyncKeyState` every ~100 ms and releases a stuck pedal (swapped mouse
  buttons handled).
- **Configurable gear keys**, sent only while capture is on and suppressed
  while typing in MouseDrive.
- Profile and A/B switches are applied only when both pedals are released.

### Ergonomics

- **Steering speed in cm per full lock**, with a type-in field and a
  *Measure DPI* tool (move the mouse along a ruler).
- **Axis bind helper:** 5 s countdown, then only the chosen axis or gear
  button moves for 4 s, so the game binds the right one.
- **In-game settings checklist** in *General*; its *Full guide* button
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
  the game, with capture/profile markers and freeze. Fed by a preallocated
  telemetry ring buffer on the control thread.
- **Settings rework** — basic/advanced split, mode-dependent sliders, units on
  every value, tooltips with range and default, changed markers.
- **Throttle tab** — the corner cut is explained in one sentence plus a mini
  chart; **Brake tab** — five phases in order under a live timeline simulated by
  the real logic.
- **Curve editor** — numeric entry, keyboard control (Page Up/Down, arrows,
  Delete) and screen-reader descriptions.
- **Diagnostics line** and **Copy diagnostics** for bug reports.
- **Logo and app icon** — the window, the taskbar and `mousedrive.exe` show
  the new MouseDrive icon; the README shows the logo.
- **Accessibility** — colour-blind palette (Okabe-Ito), brake always dashed,
  AccessKit labels on custom widgets, rebindable hotkeys including mouse side
  buttons, interface scale 75–200 %.
- **Microcopy** — labels with units, verb buttons, errors that say what happened
  and what to do; Turkish strings with proper Turkish characters.
- **No silent fallbacks** — corrected values and unreadable files are reported
  by name; broken files are backed up; config and profile saves are atomic.

### Tooling

- **Measured output** — SendInput → HID report latency p50 2.3 ms, p99 4.3 ms
  at 250 Hz (developer measurement, Windows 11, vJoy 2.2.2.0; not repeated
  in CI).
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

### License and repository

- MouseDrive is now licensed under GPL-3.0-or-later. Releases up to v0.5.0
  remain available under the MIT License.
- The license grants no rights to the MouseDrive name or logo; modified versions
  must not be presented as the original.
- The repository moved to [MouseDrive/MouseDrive](https://github.com/MouseDrive/MouseDrive).
  Old `Toxpox/MouseDrive` links and the update check of earlier versions are
  redirected.

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
  active curve while driving.
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
- See the [v0.4.0 release](https://github.com/MouseDrive/MouseDrive/releases/tag/v0.4.0).

## [0.3.0] - 2026-02-09

- Rust rewrite of the original C++ version: raw-input capture thread, atomic
  lock-free state sharing, eframe/egui interface, TOML configuration, TR/EN
  localization.
- See the [V0.3.0 release](https://github.com/MouseDrive/MouseDrive/releases/tag/V0.3.0).

## [0.1.0-alpha] - 2026-02-04

- First public alpha.
- See the [V0.1.0-alpha release](https://github.com/MouseDrive/MouseDrive/releases/tag/V0.1.0-alpha).

[Unreleased]: https://github.com/MouseDrive/MouseDrive/compare/v0.7.0...HEAD
[0.7.0]: https://github.com/MouseDrive/MouseDrive/compare/v0.6.0...v0.7.0
[0.6.0]: https://github.com/MouseDrive/MouseDrive/compare/v0.5.0...v0.6.0
[0.5.0]: https://github.com/MouseDrive/MouseDrive/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/MouseDrive/MouseDrive/compare/V0.3.0...v0.4.0
[0.3.0]: https://github.com/MouseDrive/MouseDrive/compare/V0.1.0-alpha...V0.3.0
[0.1.0-alpha]: https://github.com/MouseDrive/MouseDrive/releases/tag/V0.1.0-alpha
