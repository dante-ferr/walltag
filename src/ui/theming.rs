use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use gtk4::gdk::Display;
use gtk4::{CssProvider, STYLE_PROVIDER_PRIORITY_USER};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

pub fn load_custom_css(display: &Display, css_path: &Path) -> Result<()> {
    if !css_path.exists() {
        tracing::warn!("Custom CSS file not found: {}", css_path.display());
        return Ok(());
    }
    apply_css_file(display, css_path)?;
    watch_css_file(display, css_path)?;
    Ok(())
}

fn apply_css_file(display: &Display, css_path: &Path) -> Result<()> {
    let provider = CssProvider::new();
    provider.load_from_path(css_path);
    gtk4::style_context_add_provider_for_display(display, &provider, STYLE_PROVIDER_PRIORITY_USER);
    tracing::info!("Loaded custom CSS from {}", css_path.display());
    Ok(())
}

fn watch_css_file(display: &Display, css_path: &Path) -> Result<()> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default().with_poll_interval(Duration::from_millis(400)),
    )?;

    let parent = css_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| css_path.to_path_buf());
    watcher.watch(&parent, RecursiveMode::NonRecursive)?;

    let display = display.clone();
    let css_path = css_path.to_path_buf();
    gtk4::glib::spawn_future_local(async move {
        while let Ok(Ok(event)) = rx.recv() {
            if matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            ) && event.paths.iter().any(|p| p == &css_path)
            {
                tracing::info!("Reloading custom CSS");
                let _ = apply_css_file(&display, &css_path);
            }
        }
    });

    std::mem::forget(watcher);
    Ok(())
}
