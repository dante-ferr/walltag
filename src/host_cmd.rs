//! Spawn commands on the host when running inside Flatpak.
//!
//! Wallpaper backends (`swww` / `awww`), `post_command`, and compositor helpers
//! live on the host, not in the sandbox. Detect Flatpak via `/.flatpak-info`
//! and prefix with `flatpak-spawn --host`.

use std::path::Path;
use std::process::Command;

/// True when this process is running inside a Flatpak sandbox.
pub fn in_flatpak() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Build a [`Command`] that runs `program` on the host when sandboxed.
pub fn host_command(program: &str) -> Command {
    if in_flatpak() {
        let mut cmd = Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program);
        cmd
    } else {
        Command::new(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_command_program_is_set() {
        let cmd = host_command("swww");
        // Outside Flatpak the program is `swww`; inside it is `flatpak-spawn`.
        let prog = cmd.get_program().to_string_lossy();
        assert!(prog == "swww" || prog == "flatpak-spawn");
    }
}
