#![deny(unsafe_code)]

use std::path::{Path, PathBuf};

use mousedrive::config::{self, Config, LoadNotice};
use mousedrive::profiles::ProfileStore;

use super::notices::{Level, Notice};
use crate::lang::{Lang, Strings, fill, strings};

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
        let mut notices: Vec<Notice> = loaded
            .notice
            .iter()
            .flat_map(|n| load_notices(n, s))
            .collect();
        let disk = (file_exists && loaded.notice.is_none()).then(|| loaded.config.clone());
        let store = config_path
            .as_deref()
            .and_then(Path::parent)
            .map(ProfileStore::new);
        let config = match &store {
            Some(store) => {
                let (config, profile_notices) = init_profiles(store, loaded.config, s);
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
            config_path,
            store,
            notices,
        }
    }
}

fn load_notices(notice: &LoadNotice, s: &Strings) -> Vec<Notice> {
    match notice {
        LoadNotice::Corrected(fields) => {
            let text = fill(
                s.notice_corrected,
                &[
                    ("n", &fields.len().to_string()),
                    ("fields", &fields.join(", ")),
                ],
            );
            vec![Notice::new(Level::Warn, text)]
        }
        LoadNotice::Unreadable { error, backup } => {
            let mut out = vec![Notice::new(
                Level::Error,
                fill(s.notice_unreadable, &[("error", error)]),
            )];
            if let Some(path) = backup {
                let text = fill(s.notice_backup, &[("path", &path.display().to_string())]);
                out.push(Notice::new(Level::Info, text));
            }
            out
        }
    }
}

pub fn profile_corrected_notice(s: &Strings, name: &str, fields: &[&str]) -> Notice {
    let text = fill(
        s.profile_corrected,
        &[("name", name), ("fields", &fields.join(", "))],
    );
    Notice::new(Level::Warn, text)
}

fn error_notice(template: &str, error: &dyn std::fmt::Display) -> Notice {
    Notice::new(
        Level::Error,
        fill(template, &[("error", &error.to_string())]),
    )
}

fn init_profiles(store: &ProfileStore, config: Config, s: &Strings) -> (Config, Vec<Notice>) {
    if let Err(e) = store.ensure_initialized(&config.tuning) {
        return (config, vec![error_notice(s.err_profile_list, &e)]);
    }
    let Some(active) = store.resolve_active(&config.active_profile) else {
        return (config, Vec::new());
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
        Err(e) => (config, vec![error_notice(s.err_profile_load, &e)]),
    }
}
