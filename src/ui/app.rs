use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use gtk4::gdk::Display;
use gtk4::gio;
use gtk4::prelude::{ButtonExt, GtkWindowExt};
use gtk4::{
    Align, Box as GtkBox, Button, CheckButton, CssProvider, DropDown, Editable, EventControllerKey,
    FileDialog, GestureClick, ListBoxRow, Orientation, Paned, PickFlags, Popover, PropagationPhase,
    Scale, Scrollbar, SpinButton, Switch, ToggleButton, Widget, STYLE_PROVIDER_PRIORITY_USER,
};
use libadwaita::prelude::*;
use libadwaita::{
    Application, ApplicationWindow, Dialog, EntryRow, HeaderBar, SpinRow, StyleManager,
    ToastOverlay, ToolbarView,
};

use crate::config::{sidebar_limits, ConfigPaths};
use crate::metadata::MetadataStore;
use crate::scan::scan_wallpapers;
use crate::ui::gallery::{default_app_css, wire_activate, GalleryModel, GalleryView};
use crate::ui::gallery_scroll::wire_gallery_arrow_keys;
use crate::ui::gui_scale::{install_scale_provider, wire_gui_scale};
use crate::ui::masonry::{MasonryGrid, MAX_ZOOM, MIN_ZOOM};
use crate::ui::overlay::{close_focused_dialog, overlay_css};
use crate::ui::sidebar::Sidebar;
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
    gallery.grid.set_zoom(config.borrow().zoom);
    gallery.reload_from_scan(entries, false);
    wire_activate(&gallery.grid, &model);
    wire_persist_zoom(config.clone(), &gallery.grid);

    let toast_overlay = ToastOverlay::new();

    let sidebar = Sidebar::new(config.clone(), gallery.clone(), model.clone());
    {
        let (min, _) = sidebar_limits(config.borrow().gui_scale);
        sidebar.widget.set_size_request(min, -1);
    }

    let sidebar_visible = config.borrow().sidebar_visible;
    let split = Paned::builder()
        .orientation(Orientation::Horizontal)
        .start_child(&sidebar.widget)
        .resize_start_child(false)
        // Shrink only while hidden, so a drag or a narrow window cannot push the
        // open sidebar below its size request. GtkPaned applies that limit itself;
        // setting the position from notify::position is overwritten.
        .shrink_start_child(!sidebar_visible)
        .resize_end_child(true)
        .shrink_end_child(true)
        .position(if sidebar_visible {
            config.borrow().sidebar_width
        } else {
            0
        })
        .build();
    // While a show/hide animation runs, ignore drag-driven width saves.
    let animating_sidebar = Rc::new(Cell::new(false));
    {
        let config = config.clone();
        let animating = animating_sidebar.clone();
        let pending: Rc<RefCell<Option<gtk4::glib::SourceId>>> = Rc::new(RefCell::new(None));
        split.connect_position_notify(move |split| {
            if animating.get() {
                return;
            }
            let raw = split.position();
            let (min, max) = sidebar_limits(config.borrow().gui_scale);
            let width = raw.clamp(min, max);
            if raw != width {
                // The paned writes the dragged position after this signal returns.
                if !split.shrinks_start_child() {
                    let split = split.clone();
                    gtk4::glib::idle_add_local_once(move || {
                        if split.shrinks_start_child() || split.position() == width {
                            return;
                        }
                        split.set_position(width);
                    });
                }
                return;
            }
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

    let sidebar_toggle = ToggleButton::builder()
        .icon_name("sidebar-show-symbolic")
        .tooltip_text("Show or hide the sidebar (F9)")
        .active(sidebar_visible)
        .css_classes(["flat", "circular", "walltag-topbar-btn"])
        .build();
    {
        let config = config.clone();
        let split = split.clone();
        let animating = animating_sidebar.clone();
        sidebar_toggle.connect_toggled(move |btn| {
            let show = btn.is_active();
            let target = if show {
                let cfg = config.borrow();
                let (min, max) = sidebar_limits(cfg.gui_scale);
                cfg.sidebar_width.clamp(min, max)
            } else {
                0
            };
            // Allow the divider to travel below the minimum while it animates shut
            // or open. The animation locks the minimum again once the sidebar is open.
            if !show {
                split.set_shrink_start_child(true);
            }
            let open_min = sidebar_limits(config.borrow().gui_scale).0;
            animate_paned_position(&split, target, &animating, open_min);
            if config.borrow().sidebar_visible != show {
                config.borrow_mut().sidebar_visible = show;
                if let Err(err) = ConfigPaths::save(&config.borrow()) {
                    tracing::error!("Failed to save config: {err:#}");
                }
            }
        });
    }
    // Selecting wallpapers opens the sidebar so the Edit section is reachable.
    {
        let toggle = sidebar_toggle.clone();
        let grid = gallery.grid.clone();
        gallery.grid.connect_selection_changed(move || {
            if !grid.selected_keys().is_empty() && !toggle.is_active() {
                toggle.set_active(true);
            }
        });
    }

    let top = HeaderBar::new();
    top.add_css_class("walltag-topbar");
    top.set_show_end_title_buttons(true);
    top.set_title_widget(Some(&GtkBox::new(Orientation::Horizontal, 0)));
    top.pack_start(&sidebar_toggle);
    top.pack_end(&build_zoom_controls(&gallery.grid));
    gallery.spinner.set_valign(Align::Center);
    top.pack_end(&gallery.spinner);

    split.set_end_child(Some(&gallery.root));

    let shell = ToolbarView::new();
    shell.add_css_class("walltag-main-shell");
    shell.add_top_bar(&top);
    shell.set_content(Some(&split));
    toast_overlay.set_child(Some(&shell));

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
        let toggle = sidebar_toggle.clone();
        toggle_action.connect_activate(move |_, _| toggle.set_active(!toggle.is_active()));
    }
    window.add_action(&toggle_action);
    app.set_accels_for_action("win.toggle-sidebar", &["F9"]);

    let gui_scale_provider = install_scale_provider(&Display::default().expect("default display"));
    wire_gui_scale(
        &window,
        gallery.root.upcast_ref(),
        &gallery.grid,
        &[sidebar.content.clone(), sidebar.edit_content.clone()],
        &sidebar.widget,
        &split,
        config.clone(),
        &gui_scale_provider,
    );
    wire_clear_selection(&window, &gallery.grid);
    wire_gallery_arrow_keys(window.upcast_ref(), gallery.scroller());

    wire_wallpaper_folder_picker(&sidebar.pick_folder, &window, config, model, gallery);

    window.present();
    Ok(())
}

/// Keeps an open sidebar at or above the scaled minimum. A hidden sidebar may shrink to 0.
fn lock_open_sidebar(split: &Paned, target: i32, open_min: i32) {
    split.set_shrink_start_child(target < open_min);
    split.queue_resize();
}

/// Animates the paned divider to `target` so hide/show frees or restores gallery space.
fn animate_paned_position(split: &Paned, target: i32, animating: &Rc<Cell<bool>>, open_min: i32) {
    const DURATION_MS: f64 = 220.0;
    let start = split.position() as f64;
    let end = target as f64;
    // The divider has to pass through widths below the minimum while it opens or closes.
    if start < f64::from(open_min) || end < f64::from(open_min) {
        split.set_shrink_start_child(true);
    }
    if (start - end).abs() < 0.5 {
        split.set_position(target);
        animating.set(false);
        lock_open_sidebar(split, target, open_min);
        return;
    }
    animating.set(true);
    let split = split.clone();
    let animating = animating.clone();
    let t0 = std::time::Instant::now();
    split.add_tick_callback(move |split, _| {
        let t = (t0.elapsed().as_secs_f64() * 1000.0 / DURATION_MS).min(1.0);
        // Ease-out cubic.
        let eased = 1.0 - (1.0 - t).powi(3);
        let pos = (start + (end - start) * eased).round() as i32;
        split.set_position(pos);
        if t >= 1.0 {
            split.set_position(target);
            animating.set(false);
            lock_open_sidebar(split, target, open_min);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

/// Compact zoom controls for the thin top bar: out, discrete percent, in.
fn build_zoom_controls(grid: &MasonryGrid) -> GtkBox {
    let out = Button::builder()
        .icon_name("zoom-out-symbolic")
        .tooltip_text("Gallery zoom out (Ctrl+- over gallery)")
        .css_classes(["flat", "walltag-topbar-btn"])
        .build();
    let reset = Button::builder()
        .label("100")
        .tooltip_text("Reset gallery zoom (Ctrl+0 over gallery)")
        .css_classes(["flat", "walltag-zoom-label"])
        .build();
    let zoom_in = Button::builder()
        .icon_name("zoom-in-symbolic")
        .tooltip_text("Gallery zoom in (Ctrl++ over gallery)")
        .css_classes(["flat", "walltag-topbar-btn"])
        .build();

    let controls = GtkBox::new(Orientation::Horizontal, 0);
    controls.add_css_class("walltag-zoom-controls");
    controls.append(&out);
    controls.append(&reset);
    controls.append(&zoom_in);

    let refresh = {
        let (out, reset, zoom_in) = (out.clone(), reset.clone(), zoom_in.clone());
        move |zoom: f32| {
            reset.set_label(&format!("{}", (zoom * 100.0).round() as i32));
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

/// Clears selection and drops keyboard focus from inputs. Add any new selectable state
/// here so Esc and blank-space clicks keep covering it.
fn clear_focus_and_selection(window: &ApplicationWindow, grid: &MasonryGrid) {
    GtkWindowExt::set_focus(window, None::<&Widget>);
    grid.clear_selection();
}

/// App-wide clear: Esc anywhere, and a press on blank space.
///
/// Both gestures run in the capture phase and never claim the event, so every widget
/// still receives it. Tile clicks inside the gallery still go to the masonry grid for
/// selection/activate; this path only clears focus (and selection outside the grid).
fn wire_clear_selection(window: &ApplicationWindow, grid: &MasonryGrid) {
    let key = EventControllerKey::new();
    key.set_propagation_phase(PropagationPhase::Capture);
    {
        let window = window.clone();
        let grid = grid.clone();
        key.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                if close_focused_dialog(&window) {
                    return gtk4::glib::Propagation::Stop;
                }
                clear_focus_and_selection(&window, &grid);
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
            clear_focus_and_selection(&window, &grid);
            return;
        };
        if is_interactive(&target) {
            return;
        }
        // Always drop focus when clicking outside controls (including gallery tiles).
        GtkWindowExt::set_focus(&window, None::<&Widget>);
        // Selection clear for gallery blanks is handled by the masonry grid; elsewhere
        // clear the edit batch here.
        if !is_inside(&target, grid.upcast_ref()) {
            grid.clear_selection();
        }
    });
    window.add_controller(click);
}

fn is_inside(widget: &Widget, ancestor: &Widget) -> bool {
    widget == ancestor || widget.is_ancestor(ancestor)
}

/// Whether a press on `target` belongs to a control (button, entry, row, scrollbar,
/// popover, dialog, or the sidebar divider) instead of blank space.
fn is_interactive(target: &Widget) -> bool {
    if target.is::<Paned>() {
        // The only part of the `Paned` itself that can be hit is its divider.
        return true;
    }
    let mut current = Some(target.clone());
    while let Some(w) = current {
        let control = w.is::<Button>()
            || w.is::<Editable>()
            || w.is::<CheckButton>()
            || w.is::<Scrollbar>()
            || w.is::<Scale>()
            || w.is::<Switch>()
            || w.is::<SpinButton>()
            || w.is::<DropDown>()
            || w.is::<Popover>()
            || w.is::<Dialog>()
            || w.is::<EntryRow>()
            || w.is::<SpinRow>()
            || w.downcast_ref::<ListBoxRow>()
                .is_some_and(|row| row.is_activatable());
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

    gallery.reload_from_scan(entries, true);
    gallery.show_empty_banner(&config.borrow());

    tracing::info!(
        "Wallpaper folder set to {}",
        config.borrow().wallpaper_root.display()
    );
    Ok(())
}

/// Persists gallery zoom a short time after the last change so rapid Ctrl+wheel
/// bursts do not rewrite config.json on every step.
fn wire_persist_zoom(config: Rc<RefCell<AppConfig>>, grid: &MasonryGrid) {
    let pending: Rc<RefCell<Option<gtk4::glib::SourceId>>> = Rc::new(RefCell::new(None));
    grid.connect_zoom_changed(move |zoom| {
        if (config.borrow().zoom - zoom).abs() < f32::EPSILON {
            return;
        }
        config.borrow_mut().zoom = zoom;
        if let Some(id) = pending.borrow_mut().take() {
            id.remove();
        }
        let config = config.clone();
        let slot = pending.clone();
        let id =
            gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                slot.borrow_mut().take();
                if let Err(err) = ConfigPaths::save(&config.borrow()) {
                    tracing::error!("Failed to save config: {err:#}");
                }
            });
        *pending.borrow_mut() = Some(id);
    });
}

fn install_embedded_css(app: &Application) {
    let provider = CssProvider::new();
    provider.load_from_string(&format!("{}{}", default_app_css(), overlay_css()));
    gtk4::style_context_add_provider_for_display(
        &Display::default().expect("default display"),
        &provider,
        // USER priority so the dropdown and sidebar rules beat a global gtk.css theme.
        STYLE_PROVIDER_PRIORITY_USER,
    );
    let _ = StyleManager::default();
    let _ = app;
}
