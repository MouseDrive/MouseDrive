use std::ffi::OsString;
use std::io::{self, Write as _};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);
const DIALOG_SETTLE: Duration = Duration::from_millis(500);
const POLL_STEP: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Notification,
    Dialog,
}

impl Style {
    fn settle_time(self) -> Duration {
        match self {
            Style::Notification => NOTIFY_TIMEOUT,
            Style::Dialog => DIALOG_SETTLE,
        }
    }

    fn running_means_shown(self) -> bool {
        self == Style::Dialog
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Notifier {
    program: OsString,
    args: Vec<OsString>,
    style: Style,
}

impl Notifier {
    fn new(program: &str, args: &[String], style: Style) -> Self {
        Self {
            program: program.into(),
            args: args.iter().map(OsString::from).collect(),
            style,
        }
    }
}

fn notifiers(title: &str, body: &str) -> [Notifier; 3] {
    [
        Notifier::new(
            "notify-send",
            &[
                "--app-name=MouseDrive".into(),
                "--urgency=critical".into(),
                "--icon=dialog-error".into(),
                "--".into(),
                title.into(),
                body.into(),
            ],
            Style::Notification,
        ),
        Notifier::new(
            "zenity",
            &[
                "--error".into(),
                "--no-markup".into(),
                format!("--title={title}"),
                format!("--text={body}"),
            ],
            Style::Dialog,
        ),
        Notifier::new(
            "kdialog",
            &[
                "--title".into(),
                title.into(),
                "--error".into(),
                body.into(),
            ],
            Style::Dialog,
        ),
    ]
}

pub fn alert(title: &str, body: &str) {
    let _ = writeln!(io::stderr(), "{title}: {body}");
    let _ = show_first(&notifiers(title, body));
}

fn show_first(notifiers: &[Notifier]) -> Option<usize> {
    notifiers.iter().position(show)
}

fn show(notifier: &Notifier) -> bool {
    let spawned = super::child_command(&notifier.program)
        .args(&notifier.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match spawned {
        Ok(child) => settle(child, notifier.style),
        Err(_) => false,
    }
}

fn settle(mut child: Child, style: Style) -> bool {
    let deadline = Instant::now() + style.settle_time();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_STEP),
            Ok(None) => {
                crate::platform::reap_in_background(child);
                return style.running_means_shown();
            }
            Err(e) => {
                crate::log::warn(&format!("uyarı programı izlenemedi: {e}"));
                crate::platform::reap_in_background(child);
                return false;
            }
        }
    }
}
