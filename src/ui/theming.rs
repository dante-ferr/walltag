use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
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
    let provider_slot: Rc<RefCell<Option<CssProvider>>> = Rc::new(RefCell::new(None));
    apply_css_file(display, css_path, &provider_slot)?;
    watch_css_file(display, css_path, provider_slot)?;
    Ok(())
}

fn apply_css_file(
    display: &Display,
    css_path: &Path,
    provider_slot: &Rc<RefCell<Option<CssProvider>>>,
) -> Result<()> {
    // load_from_path keeps a file monitor on the GTK thread. Read the sheet
    // and parse it in memory so the first frame is not stalled.
    let css = std::fs::read_to_string(css_path)?;
    let provider = CssProvider::new();
    provider.load_from_string(&css);
    if let Some(old) = provider_slot.borrow_mut().take() {
        gtk4::style_context_remove_provider_for_display(display, &old);
    }
    gtk4::style_context_add_provider_for_display(display, &provider, STYLE_PROVIDER_PRIORITY_USER);
    *provider_slot.borrow_mut() = Some(provider);
    tracing::info!("Loaded custom CSS from {}", css_path.display());
    Ok(())
}

fn watch_css_file(
    display: &Display,
    css_path: &Path,
    provider_slot: Rc<RefCell<Option<CssProvider>>>,
) -> Result<()> {
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
    // notify delivers on its own thread. Poll here so a blocking recv cannot
    // stall the GTK loop before the window is mapped.
    gtk4::glib::timeout_add_local(Duration::from_millis(200), move || {
        while let Ok(event) = rx.try_recv() {
            let Ok(event) = event else {
                continue;
            };
            if matches!(
                event.kind,
                notify::EventKind::Modify(_) | notify::EventKind::Create(_)
            ) && event.paths.iter().any(|p| p == &css_path)
            {
                tracing::info!("Reloading custom CSS");
                let _ = apply_css_file(&display, &css_path, &provider_slot);
            }
        }
        gtk4::glib::ControlFlow::Continue
    });

    std::mem::forget(watcher);
    Ok(())
}
