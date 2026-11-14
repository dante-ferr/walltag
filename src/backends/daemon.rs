//! Start `swww-daemon` / `awww-daemon` when the configured backend needs them.
//!
//! Matches Waypaper: check whether the client can talk to a daemon, spawn one
//! if missing, then wait briefly for the socket before `img`.

use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::host_cmd::host_command;

const READY_TIMEOUT: Duration = Duration::from_secs(3);
const READY_POLL: Duration = Duration::from_millis(50);

/// Ensure `daemon` is running and accepting `client query` before wallpaper calls.
pub fn ensure_running(daemon: &str, client: &str) -> Result<()> {
    if client_ready(client) {
        return Ok(());
    }

    if !daemon_alive(daemon) {
        tracing::info!("starting {daemon}");
        host_command(daemon)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("start {daemon}"))?;
    }

    wait_until_ready(client)
}

fn daemon_alive(daemon: &str) -> bool {
    host_command("pgrep")
        .args(["-x", daemon])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn client_ready(client: &str) -> bool {
    host_command(client)
        .arg("query")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn wait_until_ready(client: &str) -> Result<()> {
    let start = Instant::now();
    loop {
        if client_ready(client) {
            return Ok(());
        }
        if start.elapsed() >= READY_TIMEOUT {
            bail!("{client} daemon did not become ready within {READY_TIMEOUT:?}");
        }
        thread::sleep(READY_POLL);
    }
}
