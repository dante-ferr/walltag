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

use crate::config::{AppConfig, AwwwTransitionKind, BackendKind, ConfigPaths, GroupBy};
use crate::metadata::parse_tag_input;
use crate::ui::filter::{collect_folders, collect_tags, folder_label};
use crate::ui::gallery::{assign_tags, set_random_wallpaper, GalleryModel, GalleryView};
use crate::ui::masonry::MasonryGrid;

const ALL_MONITORS: &str = "All monitors";
const ALL_TAGS: &str = "All tags";
const ALL_FOLDERS: &str = "All folders";

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
        gallery: Rc<GalleryView>,
        model: GalleryModel,
    ) -> Self {
        let grid = gallery.grid.clone();
        let search = gallery.search.clone();
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

        let search_group = build_search_section(config.clone(), gallery.clone(), model.clone(), search);

        let content = Box::new(Orientation::Vertical, 18);
        content.set_margin_top(12);
        content.set_margin_bottom(12);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.append(&search_group);
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
        toolbar.add_bottom_bar(&build_edit_section(gallery.clone(), grid, model));

        Self {
            widget: toolbar.upcast(),
            pick_folder,
        }
    }
}

const EMPTY_SELECTION_HINT: &str =
    "No wallpapers selected. Right-click a wallpaper, or drag a box around several, to select them.";

/// Search session: filename search, group-by, tag/folder filters, and random pick.
fn build_search_section(
    config: Rc<RefCell<AppConfig>>,
    gallery: Rc<GalleryView>,
    model: GalleryModel,
    search: SearchEntry,
) -> PreferencesGroup {
    let group = PreferencesGroup::builder()
        .title("Search")
        .description("Filter and group the gallery")
        .build();

    let search_row = ActionRow::builder()
        .title("Filename")
        .activatable(false)
        .build();
    search.set_hexpand(true);
    search_row.add_suffix(&search);
    group.add(&search_row);

    let group_row = ComboRow::builder()
        .title("Group by")
        .subtitle("Section headers in the gallery")
        .model(&StringList::new(
            &GroupBy::ALL.map(GroupBy::label),
        ))
        .build();
    group_row.set_selected(
        GroupBy::ALL
            .iter()
            .position(|g| *g == config.borrow().group_by)
            .unwrap_or(0) as u32,
    );
    group.add(&group_row);

    let tag_row = ComboRow::builder()
        .title("Tag")
        .subtitle("Show only wallpapers with this tag")
        .build();
    group.add(&tag_row);

    let folder_row = ComboRow::builder()
        .title("Folder")
        .subtitle("Show only wallpapers in this workspace folder")
        .build();
    group.add(&folder_row);

    let random = Button::builder()
        .label("Random")
        .tooltip_text("Set a random wallpaper from the current search results")
        .halign(gtk4::Align::End)
        .css_classes(["pill"])
        .build();
    let random_row = ActionRow::builder()
        .title("Random wallpaper")
        .subtitle("Uses the current filters")
        .activatable(false)
        .build();
    random_row.add_suffix(&random);
    group.add(&random_row);

    // Tag filter options: index 0 = all; then each tag string.
    let tag_options: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(vec![None]));
    // Folder filter options: index 0 = all; then parent_relative keys.
    let folder_options: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(vec![None]));
    let rebuilding = Rc::new(Cell::new(false));

    let rebuild_filters: Rc<dyn Fn()> = {
        let tag_row = tag_row.clone();
        let folder_row = folder_row.clone();
        let tag_options = tag_options.clone();
        let folder_options = folder_options.clone();
        let model = model.clone();
        let rebuilding = rebuilding.clone();
        Rc::new(move || {
            rebuilding.set(true);
            let entries = model.entries().borrow().clone();
            let metadata = model.metadata();
            let metadata = metadata.borrow();
            let filter = model.filter().borrow().clone();

            let tags = collect_tags(&entries, &metadata);
            let mut tag_labels = vec![ALL_TAGS.to_owned()];
            tag_labels.extend(tags.iter().cloned());
            let mut tag_opts = vec![None];
            tag_opts.extend(tags.into_iter().map(Some));
            let tag_sel = filter
                .tag
                .as_ref()
                .and_then(|t| tag_opts.iter().position(|o| o.as_ref() == Some(t)))
                .unwrap_or(0);
            tag_row.set_model(Some(&StringList::new(
                &tag_labels.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )));
            tag_row.set_selected(tag_sel as u32);
            *tag_options.borrow_mut() = tag_opts;

            let folders = collect_folders(&entries);
            let mut folder_labels = vec![ALL_FOLDERS.to_owned()];
            folder_labels.extend(folders.iter().map(|f| folder_label(f)));
            let mut folder_opts = vec![None];
            folder_opts.extend(folders.into_iter().map(Some));
            let folder_sel = filter
                .folder
                .as_ref()
                .and_then(|f| folder_opts.iter().position(|o| o.as_ref() == Some(f)))
                .unwrap_or(0);
            folder_row.set_model(Some(&StringList::new(
                &folder_labels.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )));
            folder_row.set_selected(folder_sel as u32);
            *folder_options.borrow_mut() = folder_opts;
            rebuilding.set(false);
        })
    };
    rebuild_filters();
    gallery.connect_repopulated({
        let rebuild_filters = rebuild_filters.clone();
        move || rebuild_filters()
    });

    {
        let config = config.clone();
        let gallery = gallery.clone();
        let model = model.clone();
        group_row.connect_selected_notify(move |row| {
            let Some(kind) = GroupBy::ALL.get(row.selected() as usize).copied() else {
                return;
            };
            model.filter().borrow_mut().group_by = kind;
            update(&config, |cfg| cfg.group_by = kind);
            gallery.refresh(true);
        });
    }
    {
        let gallery = gallery.clone();
        let model = model.clone();
        let tag_options = tag_options.clone();
        let rebuilding = rebuilding.clone();
        tag_row.connect_selected_notify(move |row| {
            if rebuilding.get() {
                return;
            }
            let choice = tag_options
                .borrow()
                .get(row.selected() as usize)
                .cloned()
                .flatten();
            model.filter().borrow_mut().tag = choice;
            gallery.refresh(true);
        });
    }
    {
        let gallery = gallery.clone();
        let model = model.clone();
        let folder_options = folder_options.clone();
        let rebuilding = rebuilding.clone();
        folder_row.connect_selected_notify(move |row| {
            if rebuilding.get() {
                return;
            }
            let choice = folder_options
                .borrow()
                .get(row.selected() as usize)
                .cloned()
                .flatten();
            model.filter().borrow_mut().folder = choice;
            gallery.refresh(true);
        });
    }
    {
        let model = model.clone();
        random.connect_clicked(move |_| match set_random_wallpaper(&model) {
            Ok(Some(key)) => tracing::info!("Random wallpaper: {key}"),
            Ok(None) => tracing::warn!("No wallpapers match the current filters"),
            Err(err) => tracing::error!("Failed to set random wallpaper: {err:#}"),
        });
    }

    group
}

/// Edit section pinned to the bottom of the sidebar. It shows how many wallpapers are in
/// the gallery's edit batch and adds the typed tags to each of them.
fn build_edit_section(gallery: Rc<GalleryView>, grid: MasonryGrid, model: GalleryModel) -> Box {
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
        let gallery = gallery.clone();
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
                    // Rebuild so tag groups and the tag filter list stay current.
                    gallery.refresh(false);
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
