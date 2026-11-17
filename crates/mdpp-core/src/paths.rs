//! Where MD++ keeps its files: the platform's configuration and data folders (XDG on Linux,
//! AppData on Windows). Portable mode (v1.0) will redirect both next to the executable.

use std::path::PathBuf;

use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};

/// Folder name under the platform's config/data roots.
const APP_NAME: &str = "mdpp";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// settings.toml, keymap.toml
    pub config: PathBuf,
    /// session, backups (hot exit), logs
    pub data: PathBuf,
}

impl Paths {
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
