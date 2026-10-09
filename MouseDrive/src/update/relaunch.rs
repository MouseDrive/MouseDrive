#![deny(unsafe_code)]

use std::path::Path;
use std::time::Duration;

use mousedrive::{log, platform};

use super::install;

const ROLLBACK_WINDOW: Duration = Duration::from_secs(3);
const BACKUP_KEEP: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relaunch {
    Started,
    RolledBack,
    RestoredOnly,
    Broken,
}

pub fn relaunch(exe: &Path, new_args: &[&str], old_args: &[&str]) -> Relaunch {
    relaunch_within(exe, new_args, old_args, ROLLBACK_WINDOW)
}

fn relaunch_within(exe: &Path, new_args: &[&str], old_args: &[&str], window: Duration) -> Relaunch {
    let failure = match install::spawn(platform::child_command(exe).args(new_args)) {
        Err(e) => format!("yeni sürüm başlatılamadı: {e}"),
        Ok(mut child) => match install::wait_for(&mut child, window) {
            Ok(None) => {
                log::info(&format!(
                    "yeni sürüm başlatıldı (pid {}); ilk {} s içinde kapanmadı",
                    child.id(),
                    window.as_secs_f32()
                ));
                return Relaunch::Started;
            }
            Ok(Some(status)) if status.success() => {
                log::info(&format!(
                    "yeni sürüm başlatıldı ve kendiliğinden kapandı: {status}"
                ));
                return Relaunch::Started;
            }
            Ok(Some(status)) => format!(
                "yeni sürüm ilk {} s içinde hatayla kapandı: {status}",
                window.as_secs_f32()
            ),
            Err(e) => {
                log::warn(&format!("yeni sürüm izlenemedi: {e}"));
                return Relaunch::Started;
            }
        },
    };
    log::error(&failure);
    roll_back(exe, old_args)
}

fn roll_back(exe: &Path, old_args: &[&str]) -> Relaunch {
    if let Err(e) = install::restore_backup(exe) {
        log::error(&format!(
            "önceki sürüm geri yüklenemedi ({}): {e}",
            install::backup_path(exe).display()
        ));
        return Relaunch::Broken;
    }
    log::warn(&format!("önceki sürüm geri yüklendi: {}", exe.display()));
    match platform::spawn_reaped(platform::child_command(exe).args(old_args)) {
        Ok(pid) => {
            log::info(&format!("önceki sürüm yeniden başlatıldı (pid {pid})"));
            Relaunch::RolledBack
        }
        Err(e) => {
            log::error(&format!("önceki sürüm başlatılamadı: {e}"));
            Relaunch::RestoredOnly
        }
    }
}

pub fn forget_backup_later() {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            log::warn(&format!("önceki sürümün yedeği aranamadı: {e}"));
            return;
        }
    };
    if std::fs::symlink_metadata(install::backup_path(&exe)).is_err() {
        return;
    }
    let generation = install::backup_generation();
    let spawned = std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(move || {
            std::thread::sleep(BACKUP_KEEP);
            install::forget_backup(&exe, generation);
        });
    if let Err(e) = spawned {
        log::warn(&format!("yedek temizleme thread'i başlatılamadı: {e}"));
    }
}
