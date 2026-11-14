mod awww;
mod swww;
mod traits;

pub use traits::{ApplyOptions, WallpaperBackend, WallpaperState};

use crate::config::BackendKind;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

pub fn backend_for(kind: BackendKind) -> Arc<dyn WallpaperBackend> {
    match kind {
        BackendKind::Swww => Arc::new(swww::SwwwBackend),
        BackendKind::Awww => Arc::new(awww::AwwwBackend),
    }
}

pub fn apply_wallpaper(kind: BackendKind, image: &Path, options: &ApplyOptions) -> Result<()> {
    backend_for(kind).set_wallpaper(image, options)?;
    run_post_command(image, options)
}

pub fn restore_wallpaper(kind: BackendKind, image: &Path, options: &ApplyOptions) -> Result<()> {
    backend_for(kind).restore(image, options)?;
    run_post_command(image, options)
}

/// Expand `$wallpaper` / `$monitor` and run the configured shell command.
fn run_post_command(image: &Path, options: &ApplyOptions) -> Result<()> {
    let template = options.post_command.trim();
    if template.is_empty() {
        return Ok(());
    }
    let wallpaper = image.to_string_lossy();
    let monitor = options.monitor.as_deref().unwrap_or("");
    let command = template
        .replace("$wallpaper", &wallpaper)
        .replace("$monitor", monitor);
    tracing::info!("Running post_command: {command}");
    let status = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .status()
        .with_context(|| format!("spawn post_command: {command}"))?;
    if !status.success() {
        anyhow::bail!("post_command exited with {status}: {command}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AwwwOptions;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn post_command_expands_wallpaper_and_monitor() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let out = std::env::temp_dir().join(format!("walltag-post-{stamp}.txt"));
        let _ = fs::remove_file(&out);
        let options = ApplyOptions {
            monitor: Some("DP-1".into()),
            transition_secs: 1.0,
            awww: AwwwOptions::default(),
            post_command: format!(
                "printf '%s\\n' \"$wallpaper\" \"$monitor\" > '{}'",
                out.display()
            ),
        };
        run_post_command(Path::new("/tmp/wall.png"), &options).unwrap();
        let text = fs::read_to_string(&out).unwrap();
        assert_eq!(text, "/tmp/wall.png\nDP-1\n");
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn empty_post_command_is_a_no_op() {
        let options = ApplyOptions {
            monitor: None,
            transition_secs: 1.0,
            awww: AwwwOptions::default(),
            post_command: String::new(),
        };
        run_post_command(Path::new("/tmp/wall.png"), &options).unwrap();
    }
}
