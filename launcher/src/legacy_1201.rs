//! Minecraft 1.20.1 development profile. Offline play uses local Minecraft resources; the
//! authenticated download and online-server paths keep their normal account checks.
use std::path::{Path, PathBuf};

pub const ENABLED: bool = cfg!(feature = "mc-1201-dev");
pub const TITLE: &str = name("Horizon", "Horizon 1.20.1 (Rust Edition)");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const LAUNCHER_DIR: &str = name("horizon-launcher", "horizon-1201-dev-launcher");
pub const GAME_DIR: &str = name("horizon", "horizon-1201-dev");
pub const LOCATION_DIR: &str = name("horizon-launcher-location", "horizon-1201-dev-location");
pub const KEYCHAIN: &str = name("dev.playhorizon.launcher", "local.horizon.mc1201-dev");

const fn name(upstream: &'static str, fork: &'static str) -> &'static str {
    if ENABLED {
        fork
    } else {
        upstream
    }
}

/// Called first in main, before threads start. An explicit environment always wins.
/// The Windows archive carries this marker so its bundled engine is already installed.
pub fn portable_environment() {
    if !ENABLED {
        return;
    }
    let Some(data) = std::env::current_exe()
        .ok()
        .and_then(|exe| portable_data(&exe))
    else {
        return;
    };
    for (key, value) in [
        ("HORIZON_LAUNCHER_HOME", data.join("launcher")),
        ("HORIZON_HOME", data.join("game")),
        ("HORIZON_LAUNCHER_LOCATION_FILE", data.join("location.json")),
    ] {
        if std::env::var_os(key).is_none_or(|v| v.is_empty()) {
            std::env::set_var(key, value);
        }
    }
}

fn portable_data(exe: &Path) -> Option<PathBuf> {
    let folder = exe.parent()?;
    folder
        .join("horizon-portable.txt")
        .is_file()
        .then(|| folder.join("data"))
}

/// A fresh profile starts with a local name; existing names and account choices stay intact.
pub fn initialize_player() -> Result<(), String> {
    if !ENABLED || !crate::backend::Settings::load().dev_offline {
        return Ok(());
    }
    crate::instances::ensure_legacy_1201_instance(
        &crate::backend::game_home(),
        &crate::backend::data_dir(),
    );
    let path = crate::account::Accounts::path();
    if path.exists() {
        return Ok(());
    }
    crate::account::Accounts {
        offline: vec!["Developer".into()],
        offline_selected: Some("Developer".into()),
        ..Default::default()
    }
    .save_to(&path)?;
    Ok(())
}

/// Finds the horizon-1201-server executable.
pub fn server_binary() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let direct = dir.join(if cfg!(windows) {
                "horizon-1201-server.exe"
            } else {
                "horizon-1201-server"
            });
            if direct.is_file() {
                return Some(direct);
            }
        }
    }
    for candidate in [
        "target/debug/horizon-1201-server.exe",
        "target/release/horizon-1201-server.exe",
        "../target/debug/horizon-1201-server.exe",
        "../target/release/horizon-1201-server.exe",
    ] {
        let p = PathBuf::from(candidate);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Deprecated: single-player runs through the native integrated server inside `horizon-client`.
pub fn ensure_server_running() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_paths_follow_the_executable_and_require_a_marker() {
        let root = std::env::temp_dir().join(format!("horizon-portable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let exe = root.join("horizon-launcher.exe");
        assert_eq!(portable_data(&exe), None);
        std::fs::write(root.join("horizon-portable.txt"), "portable").unwrap();
        assert_eq!(portable_data(&exe), Some(root.join("data")));
        let moved = root.with_file_name(format!("horizon-moved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&moved);
        std::fs::rename(&root, &moved).unwrap();
        assert_eq!(
            portable_data(&moved.join("horizon-launcher.exe")),
            Some(moved.join("data"))
        );
        std::fs::remove_dir_all(moved).unwrap();
    }
}
