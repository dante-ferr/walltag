use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use libadwaita::prelude::*;
use libadwaita::{
    Application, ApplicationWindow, EntryRow, SpinRow, StyleManager, ToastOverlay,
};
use gtk4::gdk::Display;
use gtk4::gio;
use gtk4::prelude::{ButtonExt, GtkWindowExt};
use gtk4::{
    Align, Box as GtkBox, Button, CssProvider, DropDown, Editable, EventControllerKey,
    FileDialog, GestureClick, ListBoxRow, Orientation, Overlay, Paned, PickFlags, Popover,
    PropagationPhase, Scale, Scrollbar, SpinButton, Switch, ToggleButton, Widget,
    STYLE_PROVIDER_PRIORITY_APPLICATION,
};

use crate::config::{ConfigPaths, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH};
use crate::metadata::MetadataStore;
use crate::scan::scan_wallpapers;
use crate::ui::masonry::{MasonryGrid, MAX_ZOOM, MIN_ZOOM};
use crate::ui::sidebar::Sidebar;
use crate::ui::gallery::{default_app_css, wire_activate, GalleryModel, GalleryView};
use crate::ui::theming::load_custom_css;
use crate::{AppConfig, Cli};

const APP_ID: &str = "com.walltag.Walltag";

pub fn launch(config: AppConfig) -> Result<()> {
    let app = Application::builder().application_id(APP_ID).build();

    let config = Rc::new(RefCell::new(config));
    app.connect_activate(move |app| {
        if let Err(err) = build_ui(app, config.clone()) {
            tracing::error!("Failed to build UI: {err:#}");
        }
    });

    app.run();
    Ok(())
}

fn build_ui(app: &Application, config: Rc<RefCell<AppConfig>>) -> Result<()> {
    install_embedded_css(app);

    {
        let cfg = config.borrow();
        if let Some(css) = &cfg.custom_css {
            if let Some(display) = gtk4::gdk::Display::default() {
                let _ = load_custom_css(&display, css);
            }
        }
    }

    let metadata = MetadataStore::open(config.borrow().metadata_path())?;
    let model = GalleryModel::new(config.clone(), metadata);

    let entries = match scan_wallpapers(&config.borrow().wallpaper_root) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("Scan failed: {err:#}");
            Vec::new()
        }
    };

    let gallery = Rc::new(GalleryView::new(model.clone()));
    gallery.reload_from_scan(entries);
    wire_activate(&gallery.grid, &model);

    let toast_overlay = ToastOverlay::new();

    let sidebar = Sidebar::new(config.clone(), gallery.clone(), model.clone());
    // The sidebar sits in a `Paned` so the user can drag the divider to resize it.
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    let split = Paned::builder()
        .orientation(Orientation::Horizontal)
        .start_child(&sidebar.widget)
        .resize_start_child(false)
        .shrink_start_child(false)
        .resize_end_child(true)
        .shrink_end_child(false)
        .position(config.borrow().sidebar_width)
        .build();
    {
        // Keep the width in memory while dragging and write it once the drag settles.
        let config = config.clone();
        let sidebar_widget = sidebar.widget.clone();
        let pending: Rc<RefCell<Option<gtk4::glib::SourceId>>> = Rc::new(RefCell::new(None));
        split.connect_position_notify(move |split| {
            // A hidden sidebar reports a collapsed position; do not store it.
            if !sidebar_widget.is_visible() {
                return;
            }
            let width = split.position().clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
            if config.borrow().sidebar_width == width {
                return;
            }
            config.borrow_mut().sidebar_width = width;
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let config = config.clone();
            let slot = pending.clone();
            let id = gtk4::glib::timeout_add_local_once(
                std::time::Duration::from_millis(400),
                move || {
                    slot.borrow_mut().take();
                    if let Err(err) = ConfigPaths::save(&config.borrow()) {
                        tracing::error!("Failed to save config: {err:#}");
                    }
                },
            );
            *pending.borrow_mut() = Some(id);
        });
    }

    // No top bar: the sidebar toggle floats over the gallery corner.
    let sidebar_toggle = ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show or hide the sidebar (F9)")
        .halign(Align::Start)
        .valign(Align::Start)
        .margin_top(18)
        .margin_start(18)
        .css_classes(["osd", "circular"])
        .build();
    sidebar
        .widget
        .bind_property("visible", &sidebar_toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    let content = Overlay::new();
    content.set_child(Some(&gallery.root));
    content.add_overlay(&sidebar_toggle);
    content.add_overlay(&build_zoom_controls(&gallery.grid));
    split.set_end_child(Some(&content));
    toast_overlay.set_child(Some(&split));

    let window = ApplicationWindow::builder()
        .application(app)
        .title("Walltag")
        .default_width(1100)
        .default_height(720)
        .content(&toast_overlay)
        .build();

    gallery.show_empty_banner(&config.borrow());

    let toggle_action = gio::SimpleAction::new("toggle-sidebar", None);
    {
        let sidebar_widget = sidebar.widget.clone();
        toggle_action
            .connect_activate(move |_, _| sidebar_widget.set_visible(!sidebar_widget.is_visible()));
    }
    window.add_action(&toggle_action);
    app.set_accels_for_action("win.toggle-sidebar", &["F9"]);

    wire_zoom_actions(app, &window, &gallery.grid);
    wire_clear_selection(&window, &gallery.grid);

    wire_wallpaper_folder_picker(&sidebar.pick_folder, &window, config, model, gallery);

    window.present();
    Ok(())
}

/// Floating zoom buttons in the top-right corner of the gallery: zoom out, reset (shows the
/// current percentage), zoom in.
fn build_zoom_controls(grid: &MasonryGrid) -> GtkBox {
    let out = Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text("Zoom out (Ctrl+-)")
        .build();
    let reset = Button::builder()
        .label("100%")
        .tooltip_text("Reset zoom (Ctrl+0)")
        .build();
    let zoom_in = Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text("Zoom in (Ctrl++)")
        .build();

    let controls = GtkBox::new(Orientation::Horizontal, 0);
    controls.set_halign(Align::End);
    controls.set_valign(Align::Start);
    controls.set_margin_top(18);
    controls.set_margin_end(18);
    controls.add_css_class("osd");
    controls.add_css_class("linked");
    controls.add_css_class("walltag-zoom-controls");
    controls.append(&out);
    controls.append(&reset);
    controls.append(&zoom_in);

    let refresh = {
        let (out, reset, zoom_in) = (out.clone(), reset.clone(), zoom_in.clone());
        move |zoom: f32| {
            reset.set_label(&format!("{}%", (zoom * 100.0).round() as i32));
            out.set_sensitive(zoom > MIN_ZOOM + f32::EPSILON);
            zoom_in.set_sensitive(zoom < MAX_ZOOM - f32::EPSILON);
        }
    };
    refresh(grid.zoom());
    grid.connect_zoom_changed(refresh);

    {
        let grid = grid.clone();
        out.connect_clicked(move |_| grid.zoom_out());
    }
    {
        let grid = grid.clone();
        reset.connect_clicked(move |_| grid.reset_zoom());
    }
    {
        let grid = grid.clone();
        zoom_in.connect_clicked(move |_| grid.zoom_in());
    }
    controls
}

/// Ctrl+`+`, Ctrl+`=`, and Ctrl+numpad plus zoom in; Ctrl+`-` and Ctrl+numpad minus zoom
/// out; Ctrl+`0` resets.
fn wire_zoom_actions(app: &Application, window: &ApplicationWindow, grid: &MasonryGrid) {
    type Handler = fn(&MasonryGrid);
    let actions: [(&str, &[&str], Handler); 3] = [
        (
            "zoom-in",
            &[
                "<Control>plus",
                "<Control>equal",
                "<Control><Shift>equal",
                "<Control>KP_Add",
            ],
            MasonryGrid::zoom_in,
        ),
        (
            "zoom-out",
            &["<Control>minus", "<Control>KP_Subtract"],
            MasonryGrid::zoom_out,
        ),
        (
            "zoom-reset",
            &["<Control>0", "<Control>KP_0"],
            MasonryGrid::reset_zoom,
        ),
    ];
    for (name, accels, handler) in actions {
        let action = gio::SimpleAction::new(name, None);
        let grid = grid.clone();
        action.connect_activate(move |_, _| handler(&grid));
        window.add_action(&action);
        app.set_accels_for_action(&format!("win.{name}"), accels);
    }
}

/// Clears every selection in the app. Add any new selectable state here so Esc and
/// blank-space clicks keep covering it.
fn clear_all_selection(grid: &MasonryGrid) {
    grid.clear_selection();
}

/// App-wide selection clearing: Esc anywhere in the window, and a press on blank space.
///
/// Both gestures run in the capture phase and never claim the event, so every widget
/// still receives it. The gallery grid is skipped for presses because it already clears
/// on a blank press and handles tile clicks itself.
fn wire_clear_selection(window: &ApplicationWindow, grid: &MasonryGrid) {
    let key = EventControllerKey::new();
    key.set_propagation_phase(PropagationPhase::Capture);
    {
        let grid = grid.clone();
        key.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                clear_all_selection(&grid);
            }
            gtk4::glib::Propagation::Proceed
        });
    }
    window.add_controller(key);

    let click = GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(PropagationPhase::Capture);
    let window_weak = window.downgrade();
    let grid = grid.clone();
    click.connect_pressed(move |_, _, x, y| {
        let Some(window) = window_weak.upgrade() else {
            return;
        };
        let Some(target) = window.pick(x, y, PickFlags::DEFAULT) else {
            return;
        };
        if is_inside(&target, grid.upcast_ref()) || is_interactive(&target) {
            return;
        }
        clear_all_selection(&grid);
    });
    window.add_controller(click);
}

fn is_inside(widget: &Widget, ancestor: &Widget) -> bool {
    widget == ancestor || widget.is_ancestor(ancestor)
}

/// Whether a press on `target` belongs to a control (button, entry, row, scrollbar,
/// popover, or the sidebar divider) instead of blank space.
fn is_interactive(target: &Widget) -> bool {
    if target.is::<Paned>() {
        // The only part of the `Paned` itself that can be hit is its divider.
        return true;
    }
    let mut current = Some(target.clone());
    while let Some(w) = current {
        let control = w.is::<Button>()
            || w.is::<Editable>()
            || w.is::<Scrollbar>()
            || w.is::<Scale>()
            || w.is::<Switch>()
            || w.is::<SpinButton>()
            || w.is::<DropDown>()
            || w.is::<Popover>()
            || w.is::<EntryRow>()
            || w.is::<SpinRow>()
            || w.downcast_ref::<ListBoxRow>().is_some_and(|row| row.is_activatable());
        if control {
            return true;
        }
        current = w.parent();
    }
    false
}

fn wire_wallpaper_folder_picker(
    button: &Button,
    window: &ApplicationWindow,
    config: Rc<RefCell<AppConfig>>,
    model: GalleryModel,
    gallery: Rc<GalleryView>,
) {
    let window = window.clone();
    button.connect_clicked(move |_| {
        let dialog = FileDialog::new();
        dialog.set_title("Choose wallpaper folder");
        dialog.set_modal(true);
        {
            let cfg = config.borrow();
            if cfg.wallpaper_root.is_dir() {
                let initial = gio::File::for_path(&cfg.wallpaper_root);
                dialog.set_initial_folder(Some(&initial));
            }
        }

        let window = window.clone();
        let config = config.clone();
        let model = model.clone();
        let gallery = gallery.clone();

        dialog.select_folder(Some(&window), None::<&gio::Cancellable>, move |result| {
            let path = match result {
                Ok(file) => match file.path() {
                    Some(path) => path,
                    None => {
                        tracing::warn!("Folder chooser returned a file without a local path");
                        return;
                    }
                },
                Err(err) => {
                    tracing::debug!("Folder chooser dismissed: {err}");
                    return;
                }
            };

            if let Err(err) = apply_wallpaper_root(&config, &model, &gallery, path) {
                tracing::error!("Failed to change wallpaper folder: {err:#}");
            }
        });
    });
}

fn apply_wallpaper_root(
    config: &Rc<RefCell<AppConfig>>,
    model: &GalleryModel,
    gallery: &GalleryView,
    new_root: PathBuf,
) -> Result<()> {
    {
        let mut cfg = config.borrow_mut();
        cfg.wallpaper_root = new_root;
        cfg.expand_paths()?;
        ConfigPaths::save(&cfg)?;
    }

    Cli::ensure_wallpaper_root(&config.borrow())?;

    let metadata = MetadataStore::open(config.borrow().metadata_path())?;
    model.set_metadata(metadata);

    gallery.clear_empty_banner();

    let entries = scan_wallpapers(&config.borrow().wallpaper_root).unwrap_or_else(|err| {
        tracing::warn!("Scan failed: {err:#}");
        Vec::new()
    });

    gallery.reload_from_scan(entries);
    gallery.show_empty_banner(&config.borrow());

    tracing::info!(
        "Wallpaper folder set to {}",
        config.borrow().wallpaper_root.display()
    );
    Ok(())
}

fn install_embedded_css(app: &Application) {
    let provider = CssProvider::new();
    provider.load_from_string(default_app_css());
    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("default display"),
        &provider,
        STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let _ = StyleManager::default();
    let _ = app;
}
