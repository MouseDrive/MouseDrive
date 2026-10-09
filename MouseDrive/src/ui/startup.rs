#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use mousedrive::config::{
    self, Backup, CURRENT_CONFIG_VERSION, Config, LoadNotice, PORTABLE_MARKER, Portable,
};
use mousedrive::profiles::{ProfileError, ProfileStore};

use super::notices::{Level, Notice};
use super::profiles::profile_error_text;
use crate::lang::{Lang, Strings, fill, strings};

const CHANGELOG_URL: &str = "https://github.com/MouseDrive/MouseDrive/blob/main/CHANGELOG.md";

pub struct Startup {
    pub config: Config,
    pub disk: Option<Config>,
    pub config_path: Option<PathBuf>,
    pub store: Option<ProfileStore>,
    pub notices: Vec<Notice>,
}

impl Startup {
    pub fn load() -> Self {
        let config_path = config::config_path();
        let loaded = config::load_startup(config_path.as_deref());
        let file_exists = config_path.as_deref().is_some_and(Path::exists);
        let s = strings(Lang::from_i32(loaded.config.language));
        let exe_dir = config::exe_dir();
        let mut notices: Vec<Notice> =
            portable_notice(config::portable_mode(), exe_dir.as_deref(), s)
                .into_iter()
                .chain(loaded.notices.iter().flat_map(|n| load_notices(n, s)))
                .collect();
        let rewrite = loaded.notices.iter().any(LoadNotice::rewrites_file);
        let disk = (file_exists && !rewrite && !loaded.outdated).then(|| loaded.config.clone());
        let store = config_path
            .as_deref()
            .and_then(Path::parent)
            .map(ProfileStore::new);
        let config = match &store {
            Some(store) => {
                let (config, profile_notices) =
                    init_profiles(store, loaded.config, loaded.outdated, s);
                notices.extend(profile_notices);
                config
            }
            None => {
                notices.push(Notice::new(Level::Error, s.err_no_config_dir));
                loaded.config
            }
        };
        Self {
            config,
            disk,
            config_path: config_path.filter(|_| loaded.writable),
            store,
            notices,
        }
    }
}

fn portable_notice(mode: Portable, dir: Option<&Path>, s: &Strings) -> Option<Notice> {
    let (level, template) = match mode {
        Portable::IgnoredUnmarked => (Level::Info, s.notice_portable_unmarked),
        Portable::IgnoredForeign { marked: true } => (Level::Warn, s.notice_portable_foreign),
        _ => return None,
    };
    let text = fill(
        template,
        &[
            ("dir", &dir?.display().to_string()),
            ("marker", PORTABLE_MARKER),
        ],
    );
    Some(Notice::new(level, text))
}

fn load_notices(notice: &LoadNotice, s: &Strings) -> Vec<Notice> {
    let main = load_notice(notice, s);
    let backup = notice
        .backup()
        .map(|backup| backup_result_notice(notice, backup, s));
    std::iter::once(main).chain(backup).collect()
}

fn load_notice(notice: &LoadNotice, s: &Strings) -> Notice {
    match notice {
        LoadNotice::Corrected(fields) => {
            let text = fill(
                s.notice_corrected,
                &[
                    ("n", &fields.len().to_string()),
                    ("fields", &fields.join(", ")),
                ],
            );
            Notice::new(Level::Warn, text)
        }
        LoadNotice::Unreadable { error, .. } => {
            Notice::new(Level::Error, fill(s.notice_unreadable, &[("error", error)]))
        }
        LoadNotice::Newer { version, .. } => {
            let text = fill(
                s.notice_newer,
                &[
                    ("version", &version.to_string()),
                    ("current", &CURRENT_CONFIG_VERSION.to_string()),
                ],
            );
            Notice::new(Level::Warn, text)
        }
        LoadNotice::UnknownKeys { keys, .. } => Notice::new(
            Level::Warn,
            fill(s.notice_unknown_keys, &[("keys", &keys.join(", "))]),
        ),
        LoadNotice::Foreign { .. } => Notice::new(Level::Warn, s.notice_foreign),
    }
}

fn backup_result_notice(notice: &LoadNotice, backup: &Backup, s: &Strings) -> Notice {
    match backup {
        Ok(path) => {
            let template = match notice {
                LoadNotice::Unreadable { .. } => s.notice_backup,
                _ => s.notice_backup_copy,
            };
            let text = fill(template, &[("path", &path.display().to_string())]);
            Notice::new(Level::Info, text)
        }
        Err(error) => Notice::new(
            Level::Error,
            fill(s.notice_backup_failed, &[("error", error)]),
        ),
    }
}

pub fn profile_corrected_notice(s: &Strings, name: &str, fields: &[&str]) -> Notice {
    let text = fill(
        s.profile_corrected,
        &[("name", name), ("fields", &fields.join(", "))],
    );
    Notice::new(Level::Warn, text)
}

pub fn updated_notice(s: &Strings, from: &str) -> Option<Notice> {
    let to = env!("CARGO_PKG_VERSION");
    (from != to).then(|| {
        let text = fill(s.notice_updated, &[("from", from), ("to", to)]);
        Notice::new(Level::Info, text).with_link(s.link_changelog, CHANGELOG_URL)
    })
}

pub fn rolled_back_notice(s: &Strings) -> Notice {
    let text = fill(
        s.notice_update_rolled_back,
        &[("version", env!("CARGO_PKG_VERSION"))],
    );
    Notice::new(Level::Warn, text)
}

pub fn crashed_notice(s: &Strings, report: &Path) -> Notice {
    let text = fill(s.notice_crashed, &[("path", &report.display().to_string())]);
    Notice::new(Level::Warn, text)
}

fn error_notice(template: &str, error: &str) -> Notice {
    Notice::new(Level::Error, fill(template, &[("error", error)]))
}

fn init_profiles(
    store: &ProfileStore,
    config: Config,
    outdated: bool,
    s: &Strings,
) -> (Config, Vec<Notice>) {
    let migrated = if outdated {
        migrate_profiles(store, &config, s)
    } else {
        Vec::new()
    };
    let (config, activated) = activate_profile(store, config, s);
    (config, migrated.into_iter().chain(activated).collect())
}

fn migrate_profiles(store: &ProfileStore, config: &Config, s: &Strings) -> Vec<Notice> {
    let report = match store.reset_legacy_deadzone(f64::from(config.thread_interval_ms)) {
        Ok(report) => report,
        Err(e) => return vec![error_notice(s.err_profile_list, &profile_error_text(&e, s))],
    };
    let changed = (!report.changed.is_empty()).then(|| {
        let names = report.changed.join(", ");
        Notice::new(
            Level::Info,
            fill(s.notice_deadzone_reset, &[("names", &names)]),
        )
    });
    let failed = report.failed.iter().map(|(name, e)| {
        let text = fill(
            s.err_deadzone_reset,
            &[("name", name), ("error", &e.to_string())],
        );
        Notice::new(Level::Error, text)
    });
    changed.into_iter().chain(failed).collect()
}

fn activate_profile(store: &ProfileStore, config: Config, s: &Strings) -> (Config, Vec<Notice>) {
    if let Err(e) = store.ensure_initialized(&config.tuning) {
        let text = profile_error_text(&e, s);
        return (config, vec![error_notice(s.err_profile_list, &text)]);
    }
    let active = match store.resolve_active(&config.active_profile) {
        Ok(Some(active)) => active,
        Ok(None) => return (config, Vec::new()),
        Err(e) => {
            let text = profile_error_text(&e, s);
            return (config, vec![error_notice(s.err_profile_load, &text)]);
        }
    };
    match store.load(&active, f64::from(config.thread_interval_ms)) {
        Ok(loaded) => {
            let notices = if loaded.corrected.is_empty() {
                Vec::new()
            } else {
                vec![profile_corrected_notice(s, &active, &loaded.corrected)]
            };
            let mut next = config.with_tuning(loaded.tuning);
            next.active_profile = active;
            (next, notices)
        }
        Err(e) => {
            let load = error_notice(s.err_profile_load, &profile_error_text(&e, s));
            let backup = backup_notice(store, &active, s);
            (config, std::iter::once(load).chain(backup).collect())
        }
    }
}

pub(super) fn backup_notice(store: &ProfileStore, name: &str, s: &Strings) -> Option<Notice> {
    match store.backup(name) {
        Ok(path) => {
            let text = fill(s.notice_backup, &[("path", &path.display().to_string())]);
            Some(Notice::new(Level::Info, text))
        }
        Err(ProfileError::NotFound(_) | ProfileError::Ambiguous { .. }) => None,
        Err(e) => Some(error_notice(
            s.err_profile_backup,
            &profile_error_text(&e, s),
        )),
    }
}
