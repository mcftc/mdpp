//! Where MD++ keeps its files: the platform's configuration and data folders (XDG on Linux,
//! AppData on Windows) — or, in portable mode, a folder next to the executable: a file named
//! `mdpp-portable` beside it turns that on (a USB stick, a zip unpacked anywhere).

use std::path::{Path, PathBuf};

use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};

/// Folder name under the platform's config/data roots.
const APP_NAME: &str = "mdpp";
/// A file with this name next to the executable turns on portable mode.
pub const PORTABLE_MARKER: &str = "mdpp-portable";
/// Portable mode keeps everything in this folder next to the executable.
const PORTABLE_FOLDER: &str = "mdpp-data";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// settings.toml, keymap.toml
    pub config: PathBuf,
    /// session, backups (hot exit), logs
    pub data: PathBuf,
}

impl Paths {
    /// Portable folders when the marker is next to the running executable, else the platform's.
    pub fn find() -> Option<Self> {
        let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
        Self::portable(&exe_dir).or_else(Self::platform)
    }

    /// The portable folders beside `exe_dir`, when the marker file is there.
    pub fn portable(exe_dir: &Path) -> Option<Self> {
        exe_dir.join(PORTABLE_MARKER).is_file().then(|| {
            let root = exe_dir.join(PORTABLE_FOLDER);
            Paths {
                config: root.join("config"),
                data: root.join("data"),
            }
        })
    }

    /// `None` when the platform reports no home folder (then MD++ runs without persistence).
    pub fn platform() -> Option<Self> {
        let strategy = choose_app_strategy(AppStrategyArgs {
            top_level_domain: "io.github".to_string(),
            author: APP_NAME.to_string(),
            app_name: APP_NAME.to_string(),
        })
        .ok()?;
        Some(Paths {
            config: strategy.config_dir(),
            data: strategy.data_dir(),
        })
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    pub fn keymap_file(&self) -> PathBuf {
        self.config.join("keymap.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_only_with_the_marker() {
        let dir = std::env::temp_dir().join(format!("mdpp-portable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Paths::portable(&dir), None);
        std::fs::write(dir.join(PORTABLE_MARKER), "").unwrap();
        let paths = Paths::portable(&dir).unwrap();
        assert_eq!(
            paths.settings_file(),
            dir.join("mdpp-data/config/settings.toml")
        );
        assert_eq!(paths.data, dir.join("mdpp-data/data"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
