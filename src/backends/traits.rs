use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::{AppConfig, AwwwOptions};

/// Per-call settings a backend needs to apply a wallpaper.
#[derive(Debug, Clone)]
pub struct ApplyOptions {
    /// Output name. `None` applies to all monitors.
    pub monitor: Option<String>,
    pub transition_secs: f32,
    pub awww: AwwwOptions,
}

impl ApplyOptions {
    pub fn from_config(config: &AppConfig) -> Self {
        Self {
            monitor: config.monitor.clone().filter(|m| !m.is_empty()),
            transition_secs: config.transition_secs,
            awww: config.awww,
        }
    }
}

pub trait WallpaperBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn set_wallpaper(&self, image: &Path, options: &ApplyOptions) -> Result<()>;
    fn restore(&self, image: &Path, options: &ApplyOptions) -> Result<()> {
        self.set_wallpaper(image, options)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WallpaperState {
    pub last_image: PathBuf,
    #[serde(default)]
    pub backend: Option<String>,
}

impl WallpaperState {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(Some(serde_json::from_str(&raw)?))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let pretty = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{pretty}\n"))?;
        Ok(())
    }
}
