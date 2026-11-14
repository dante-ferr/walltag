use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::traits::{ApplyOptions, WallpaperBackend};

pub struct AwwwBackend;

impl WallpaperBackend for AwwwBackend {
    fn name(&self) -> &'static str {
        "awww"
    }

    fn set_wallpaper(&self, image: &Path, options: &ApplyOptions) -> Result<()> {
        let image = image
            .canonicalize()
            .with_context(|| format!("resolve wallpaper path {}", image.display()))?;
        // awww uses a subcommand layout that may vary by distro; this matches common installs.
        let status = Command::new("awww")
            .args(build_args(&image, options))
            .status()
            .context("run awww img")?;
        if !status.success() {
            anyhow::bail!("awww exited with status {status}");
        }
        Ok(())
    }
}

fn build_args(image: &Path, options: &ApplyOptions) -> Vec<String> {
    let awww = &options.awww;
    let mut args = vec![
        "img".to_string(),
        image.to_string_lossy().into_owned(),
        "--transition-type".to_string(),
        awww.transition.as_str().to_string(),
        "--transition-duration".to_string(),
        format!("{:.2}", options.transition_secs),
        "--transition-fps".to_string(),
        awww.fps.to_string(),
    ];
    if awww.transition.uses_angle() {
        args.push("--transition-angle".to_string());
        args.push(awww.angle.to_string());
    }
    if let Some(monitor) = &options.monitor {
        args.push("--outputs".to_string());
        args.push(monitor.clone());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AwwwOptions, AwwwTransitionKind};

    fn options(kind: AwwwTransitionKind, monitor: Option<&str>) -> ApplyOptions {
        ApplyOptions {
            monitor: monitor.map(str::to_string),
            transition_secs: 1.5,
            awww: AwwwOptions {
                transition: kind,
                fps: 60,
                angle: 90,
            },
            post_command: String::new(),
        }
    }

    #[test]
    fn passes_transition_flags() {
        let args = build_args(
            Path::new("/a.png"),
            &options(AwwwTransitionKind::Grow, None),
        );
        let joined = args.join(" ");
        assert!(joined.contains("--transition-type grow"));
        assert!(joined.contains("--transition-duration 1.50"));
        assert!(joined.contains("--transition-fps 60"));
        assert!(!joined.contains("--transition-angle"));
        assert!(!joined.contains("--outputs"));
    }

    #[test]
    fn passes_angle_for_wipe_and_output() {
        let args = build_args(
            Path::new("/a.png"),
            &options(AwwwTransitionKind::Wipe, Some("HDMI-A-1")),
        );
        let joined = args.join(" ");
        assert!(joined.contains("--transition-angle 90"));
        assert!(joined.contains("--outputs HDMI-A-1"));
    }
}
