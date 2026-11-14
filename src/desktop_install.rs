//! Install a FreeDesktop launcher so Walltag appears in app menus / rofi.
//!
//! System packages (AUR, Flatpak) already ship the `.desktop` file and icon.
//! This path covers `cargo install` and local builds by writing into the user's
//! XDG data directories on GUI launch.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use directories::BaseDirs;

const APP_ID: &str = "io.github.dante_ferr.Walltag";
const DESKTOP_NAME: &str = "io.github.dante_ferr.Walltag.desktop";
const ICON_NAME: &str = "io.github.dante_ferr.Walltag.svg";

const EMBEDDED_ICON: &str = include_str!("../assets/io.github.dante_ferr.Walltag.svg");

/// Ensure a user-local `.desktop` entry and hicolor icon exist when the app was
/// not installed by a system package that already provides them.
pub fn ensure_user_launcher() -> Result<()> {
    if env::var_os("FLATPAK_ID").is_some() {
        return Ok(());
    }

    if system_launcher_present() {
        return Ok(());
    }

    let Some(base) = BaseDirs::new() else {
        tracing::debug!("no XDG base dirs; skip desktop launcher install");
        return Ok(());
    };

    let data = base.data_dir();
    let desktop_path = data.join("applications").join(DESKTOP_NAME);
    let icon_path = data
        .join("icons/hicolor/scalable/apps")
        .join(ICON_NAME);

    let exe = env::current_exe()
        .context("resolve walltag executable path")?
        .canonicalize()
        .unwrap_or_else(|_| env::current_exe().expect("current_exe"));

    write_if_changed(&icon_path, EMBEDDED_ICON.as_bytes())?;
    write_if_changed(&desktop_path, desktop_file_for(&exe).as_bytes())?;

    // Best-effort cache refresh for menus that use the desktop database.
    let _ = Command::new("update-desktop-database")
        .arg(data.join("applications"))
        .status();
    let _ = Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(data.join("icons/hicolor"))
        .status();

    tracing::debug!(
        desktop = %desktop_path.display(),
        icon = %icon_path.display(),
        "installed user desktop launcher"
    );
    Ok(())
}

fn system_launcher_present() -> bool {
    [
        PathBuf::from("/usr/share/applications").join(DESKTOP_NAME),
        PathBuf::from("/usr/local/share/applications").join(DESKTOP_NAME),
    ]
    .iter()
    .any(|p| p.is_file())
}

fn desktop_file_for(exe: &Path) -> String {
    format!(
        "\
[Desktop Entry]
Type=Application
Name=Walltag
Comment=Minimalist wallpaper manager and visual gallery for Wayland
Exec={}
TryExec={}
Icon={APP_ID}
Terminal=false
Categories=Graphics;GTK;
StartupNotify=true
StartupWMClass={APP_ID}
Keywords=wallpaper;gallery;wayland;tags;
",
        shell_quote(exe),
        shell_quote(exe),
    )
}

fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.chars().any(|c| c.is_whitespace() || "\"'\\$`".contains(c)) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.into_owned()
    }
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == contents {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = fs::File::create(path).with_context(|| format!("write {}", path.display()))?;
    file.write_all(contents)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}
