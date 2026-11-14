use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::traits::{ApplyOptions, WallpaperBackend};

pub struct SwwwBackend;

impl WallpaperBackend for SwwwBackend {
    fn name(&self) -> &'static str {
        "swww"
    }

    fn set_wallpaper(&self, image: &Path, options: &ApplyOptions) -> Result<()> {
        let image = image
            .canonicalize()
            .with_context(|| format!("resolve wallpaper path {}", image.display()))?;
        let status = Command::new("swww")
            .args(build_args(&image, options))
            .status()
            .context("run swww img")?;
        if !status.success() {
            anyhow::bail!("swww exited with status {status}");
        }
        Ok(())
    }
}

fn build_args(image: &Path, options: &ApplyOptions) -> Vec<String> {
    let mut args = vec![
        "img".to_string(),
        image.to_string_lossy().into_owned(),
        "--transition-type".to_string(),
        "fade".to_string(),
        "--transition-duration".to_string(),
        format!("{:.2}", options.transition_secs),
    ];
    if let Some(monitor) = &options.monitor {
        args.push("--outputs".to_string());
        args.push(monitor.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AwwwOptions;

    fn options(monitor: Option<&str>) -> ApplyOptions {
        ApplyOptions {
            monitor: monitor.map(str::to_string),
            transition_secs: 1.0,
            awww: AwwwOptions::default(),
        }
    }

    #[test]
    fn targets_all_outputs_by_default() {
        let args = build_args(Path::new("/a.png"), &options(None));
        assert!(!args.contains(&"--outputs".to_string()));
    }

    #[test]
    fn targets_selected_output() {
        let args = build_args(Path::new("/a.png"), &options(Some("DP-1")));
        let pos = args.iter().position(|a| a == "--outputs").unwrap();
        assert_eq!(args[pos + 1], "DP-1");
    }
}
