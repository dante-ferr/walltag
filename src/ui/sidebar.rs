use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk::Display;
use gtk4::prelude::*;
use gtk4::{
    gio, Box, Button, CheckButton, Orientation, PolicyType, ScrolledWindow, SearchEntry, StringList, Widget,
};
use libadwaita::prelude::*;
use libadwaita::{
    ActionRow, ComboRow, EntryRow, HeaderBar, PreferencesGroup, SpinRow, ToolbarView,
};

use crate::config::{AppConfig, AwwwTransitionKind, BackendKind, ConfigPaths};
use crate::metadata::parse_tag_input;
use crate::ui::gallery::{assign_tags, GalleryModel};
use crate::ui::masonry::MasonryGrid;

const ALL_MONITORS: &str = "All monitors";

/// Sidebar with filename search, output, backend, and awww animation controls. Changes update the shared
/// config immediately and persist to the XDG config file. An **Edit** section at the bottom
/// assigns tags to the wallpapers selected in the gallery.
pub struct Sidebar {
    pub widget: Widget,
    /// Opens the wallpaper folder chooser; the app wires the action.
    pub pick_folder: Button,
}

impl Sidebar {
    pub fn new(
        config: Rc<RefCell<AppConfig>>,
        grid: MasonryGrid,
        model: GalleryModel,
        search: SearchEntry,
    ) -> Self {
        let monitor_row = ComboRow::builder()
            .title("Monitor")
            .subtitle("Output that receives the wallpaper")
            .build();
        let refresh = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh monitor list")
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();
        monitor_row.add_suffix(&refresh);

        let backend_row = ComboRow::builder()
            .title("Backend")
            .subtitle("Wallpaper daemon to use")
            .model(&StringList::new(
                &BackendKind::ALL.map(BackendKind::as_str),
            ))
            .build();
        backend_row.set_selected(
            BackendKind::ALL
                .iter()
                .position(|b| *b == config.borrow().backend)
                .unwrap_or(0) as u32,
        );

        let output_group = PreferencesGroup::builder().title("Output").build();
        output_group.add(&monitor_row);
        output_group.add(&backend_row);

        let names_check = CheckButton::new();
        names_check.set_valign(gtk4::Align::Center);
        names_check.set_active(config.borrow().show_filenames);
        let names_row = ActionRow::builder()
            .title("Show file names")
            .subtitle("Under each wallpaper")
            .activatable_widget(&names_check)
            .build();
        names_row.add_suffix(&names_check);
        let gallery_group = PreferencesGroup::builder().title("Gallery").build();
        gallery_group.add(&names_row);
        grid.set_show_names(config.borrow().show_filenames);
        {
            let config = config.clone();
            let grid = grid.clone();
            names_check.connect_toggled(move |check| {
                let show = check.is_active();
                grid.set_show_names(show);
                update(&config, |cfg| cfg.show_filenames = show);
            });
        }

        let transition_row = ComboRow::builder()
            .title("Transition")
            .model(&StringList::new(
                &AwwwTransitionKind::ALL.map(AwwwTransitionKind::as_str),
            ))
            .build();
        let duration_row = SpinRow::with_range(0.0, 10.0, 0.1);
        duration_row.set_title("Duration");
        duration_row.set_subtitle("Seconds. Ignored by the simple and none transitions");
        duration_row.set_digits(1);
        let fps_row = SpinRow::with_range(1.0, 240.0, 1.0);
        fps_row.set_title("Frame rate");
        let angle_row = SpinRow::with_range(0.0, 360.0, 5.0);
        angle_row.set_title("Angle");
        angle_row.set_subtitle("Degrees, for wipe and wave");

        {
            let cfg = config.borrow();
            transition_row.set_selected(
                AwwwTransitionKind::ALL
                    .iter()
                    .position(|k| *k == cfg.awww.transition)
                    .unwrap_or(0) as u32,
            );
            duration_row.set_value(cfg.transition_secs as f64);
            fps_row.set_value(cfg.awww.fps as f64);
            angle_row.set_value(cfg.awww.angle as f64);
            angle_row.set_visible(cfg.awww.transition.uses_angle());
        }

        let awww_group = PreferencesGroup::builder()
            .title("awww animation")
            .description("Applies when the backend is awww")
            .build();
        awww_group.add(&transition_row);
        awww_group.add(&duration_row);
        awww_group.add(&fps_row);
        awww_group.add(&angle_row);
        awww_group.set_visible(config.borrow().backend == BackendKind::Awww);

        // Monitor list. `monitors` maps combo index to output name (`None` = all).
        let monitors: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(Vec::new()));
        let rebuilding = Rc::new(Cell::new(false));
        let rebuild: Rc<dyn Fn()> = {
            let monitor_row = monitor_row.clone();
            let monitors = monitors.clone();
            let config = config.clone();
            let rebuilding = rebuilding.clone();
            Rc::new(move || {
                rebuilding.set(true);
                let selected = config.borrow().monitor.clone();
                let entries = monitor_entries(connected_monitors(), selected.as_deref());
                let labels: Vec<&str> = entries
                    .iter()
                    .map(|e| e.as_deref().unwrap_or(ALL_MONITORS))
                    .collect();
                monitor_row.set_model(Some(&StringList::new(&labels)));
                monitor_row.set_selected(
                    entries.iter().position(|e| *e == selected).unwrap_or(0) as u32,
                );
                *monitors.borrow_mut() = entries;
                rebuilding.set(false);
            })
        };
        rebuild();

        if let Some(display) = Display::default() {
            let rebuild = rebuild.clone();
            display
                .monitors()
                .connect_items_changed(move |_, _, _, _| rebuild());
        }
        {
            let rebuild = rebuild.clone();
            refresh.connect_clicked(move |_| rebuild());
        }

        {
            let config = config.clone();
            let monitors = monitors.clone();
            let rebuilding = rebuilding.clone();
            monitor_row.connect_selected_notify(move |row| {
                if rebuilding.get() {
                    return;
                }
                let choice = monitors
                    .borrow()
                    .get(row.selected() as usize)
                    .cloned()
                    .flatten();
                update(&config, |cfg| cfg.monitor = choice);
            });
        }
        {
            let config = config.clone();
            let awww_group = awww_group.clone();
            backend_row.connect_selected_notify(move |row| {
                let Some(kind) = BackendKind::ALL.get(row.selected() as usize).copied() else {
                    return;
                };
                awww_group.set_visible(kind == BackendKind::Awww);
                update(&config, |cfg| cfg.backend = kind);
            });
        }
        {
            let config = config.clone();
            let angle_row = angle_row.clone();
            transition_row.connect_selected_notify(move |row| {
                let Some(kind) = AwwwTransitionKind::ALL
                    .get(row.selected() as usize)
                    .copied()
                else {
                    return;
                };
                angle_row.set_visible(kind.uses_angle());
                update(&config, |cfg| cfg.awww.transition = kind);
            });
        }
        {
            let config = config.clone();
            duration_row.connect_value_notify(move |row| {
                let secs = row.value() as f32;
                update(&config, |cfg| cfg.transition_secs = secs);
            });
        }
        {
            let config = config.clone();
            fps_row.connect_value_notify(move |row| {
                let fps = row.value() as u32;
                update(&config, |cfg| cfg.awww.fps = fps);
            });
        }
        {
            let config = config.clone();
            angle_row.connect_value_notify(move |row| {
                let angle = row.value() as u32;
                update(&config, |cfg| cfg.awww.angle = angle);
            });
        }

        let content = Box::new(Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&search);
        content.append(&output_group);
        content.append(&gallery_group);
        content.append(&awww_group);

        let scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vexpand(true)
            .child(&content)
            .build();

        // Slim header: no title. It keeps the window controls and the folder action.
        let pick_folder = Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Choose wallpaper folder")
            .build();
        let header = HeaderBar::new();
        header.set_title_widget(Some(&Box::new(Orientation::Horizontal, 0)));
        header.set_show_end_title_buttons(false);
        header.pack_end(&pick_folder);
        let toolbar = ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.set_content(Some(&scroll));
        toolbar.add_bottom_bar(&build_edit_section(grid, model));

        Self {
            widget: toolbar.upcast(),
            pick_folder,
        }
    }
}

const EMPTY_SELECTION_HINT: &str =
    "No wallpapers selected. Right-click a wallpaper, or drag a box around several, to select them.";

/// Edit section pinned to the bottom of the sidebar. It shows how many wallpapers are in
/// the gallery's edit batch and adds the typed tags to each of them.
fn build_edit_section(grid: MasonryGrid, model: GalleryModel) -> Box {
    let clear = Button::builder()
        .label("Clear")
        .tooltip_text("Clear selection (Esc)")
        .valign(gtk4::Align::Center)
        .css_classes(["flat"])
        .build();
    let tags_row = EntryRow::builder()
        .title("Add tags (comma or space separated)")
        .show_apply_button(true)
        .build();
    let group = PreferencesGroup::builder().title("Edit").build();
    group.set_header_suffix(Some(&clear));
    group.add(&tags_row);

    let refresh: Rc<dyn Fn()> = {
        let grid = grid.clone();
        let group = group.clone();
        let tags_row = tags_row.clone();
        let clear = clear.clone();
        Rc::new(move || {
            let count = grid.selected_keys().len();
            let description = match count {
                0 => EMPTY_SELECTION_HINT.to_owned(),
                1 => "1 wallpaper selected".to_owned(),
                n => format!("{n} wallpapers selected"),
            };
            group.set_description(Some(&description));
            tags_row.set_sensitive(count > 0);
            clear.set_sensitive(count > 0);
        })
    };
    refresh();
    {
        let refresh = refresh.clone();
        grid.connect_selection_changed(move || refresh());
    }
    {
        let grid = grid.clone();
        clear.connect_clicked(move |_| grid.clear_selection());
    }
    {
        let group = group.clone();
        tags_row.connect_apply(move |row| {
            let tags = parse_tag_input(&row.text());
            let keys = grid.selected_keys();
            if tags.is_empty() || keys.is_empty() {
                return;
            }
            match assign_tags(&grid, &model, &keys, &tags) {
                Ok(()) => {
                    row.set_text("");
                    group.set_description(Some(&format!(
                        "Added {} to {} selected",
                        tags.join(", "),
                        keys.len()
                    )));
                }
                Err(err) => {
                    tracing::error!("Failed to save tags: {err:#}");
                    group.set_description(Some("Could not save tags. See the log for details."));
                }
            }
        });
    }

    let section = Box::new(Orientation::Vertical, 0);
    section.set_margin_top(12);
    section.set_margin_bottom(12);
    section.set_margin_start(12);
    section.set_margin_end(12);
    section.append(&group);
    section
}

/// Applies `change` to the shared config and writes it to disk.
fn update(config: &Rc<RefCell<AppConfig>>, change: impl FnOnce(&mut AppConfig)) {
    let mut cfg = config.borrow_mut();
    change(&mut cfg);
    if let Err(err) = ConfigPaths::save(&cfg) {
        tracing::error!("Failed to save config: {err:#}");
    }
}

/// Combo entries for the monitor row: `None` (all monitors) first, then each connected
/// output. A saved choice that is not connected right now stays in the list, so the
/// selection survives a restart while the monitor is unplugged.
fn monitor_entries(mut connected: Vec<String>, saved: Option<&str>) -> Vec<Option<String>> {
    if let Some(name) = saved {
        if !connected.iter().any(|c| c == name) {
            connected.push(name.to_owned());
        }
    }
    let mut entries = vec![None];
    entries.extend(connected.into_iter().map(Some));
    entries
}

/// Output names (for example `DP-1`) reported by GDK. On Wayland these match the names
/// swww and awww accept for `--outputs`.
fn connected_monitors() -> Vec<String> {
    let Some(display) = Display::default() else {
        return Vec::new();
    };
    let list: gio::ListModel = display.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i))
        .filter_map(|obj| obj.downcast::<gtk4::gdk::Monitor>().ok())
        .filter_map(|m| m.connector())
        .map(|c| c.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_monitor_is_selectable_even_when_disconnected() {
        let entries = monitor_entries(vec!["DP-1".into()], Some("HDMI-A-1"));
        assert_eq!(
            entries,
            vec![None, Some("DP-1".into()), Some("HDMI-A-1".into())]
        );
    }

    #[test]
    fn saved_monitor_is_not_duplicated() {
        let entries = monitor_entries(vec!["DP-1".into()], Some("DP-1"));
        assert_eq!(entries, vec![None, Some("DP-1".into())]);
    }
}
