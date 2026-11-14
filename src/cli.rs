use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use crate::backends::{restore_wallpaper, ApplyOptions, WallpaperState};
use crate::config::AppConfig;

#[derive(Parser, Debug)]
#[command(name = "walltag", version, about = "Wallpaper manager for Wayland")]
pub struct Cli {
    /// Restore the last wallpaper without opening the GUI (for compositor autostart).
    #[arg(long)]
    pub restore: bool,

    /// Override the wallpaper root for this run.
    #[arg(long)]
    pub wallpaper_root: Option<PathBuf>,
}

impl Cli {
    pub fn run_headless(self, mut config: AppConfig) -> Result<()> {
        if let Some(root) = self.wallpaper_root {
            config.wallpaper_root = root;
            config.expand_paths()?;
        }

        let state_path = config.state_path();
        let state = WallpaperState::load(&state_path)?
            .context("no saved wallpaper state; open the app and set a wallpaper first")?;

        if !state.last_image.exists() {
            anyhow::bail!(
                "saved wallpaper no longer exists: {}",
                state.last_image.display()
            );
        }

        restore_wallpaper(
            config.backend,
            &state.last_image,
            &ApplyOptions::from_config(&config),
        )?;
        tracing::info!(
            "Restored wallpaper via {}: {}",
            config.backend.as_str(),
            state.last_image.display()
        );
        Ok(())
    }

    pub fn persist_last_wallpaper(config: &AppConfig, image: &std::path::Path) -> Result<()> {
        let state = WallpaperState {
            last_image: image.to_path_buf(),
            backend: Some(config.backend.as_str().to_string()),
        };
        state
            .save(&config.state_path())
            .context("write wallpaper state.json")?;
        Ok(())
    }

    pub fn ensure_wallpaper_root(config: &AppConfig) -> Result<()> {
        fs::create_dir_all(&config.wallpaper_root).with_context(|| {
            format!(
                "create wallpaper root {}",
                config.wallpaper_root.display()
            )
        })?;
        Ok(())
    }
}
