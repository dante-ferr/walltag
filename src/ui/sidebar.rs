use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::gdk::Display;
use gtk4::glib;
use gtk4::glib::object::IsA;
use gtk4::prelude::*;
use gtk4::{
    gio, Align, Box, Button, CheckButton, Orientation, PolicyType, Revealer,
    RevealerTransitionType, ScrolledWindow, SearchEntry, StringList, Widget,
};
use libadwaita::prelude::*;
use libadwaita::{ActionRow, ComboRow, EntryRow, PreferencesGroup, SpinRow};

use crate::config::{AppConfig, AwwwTransitionKind, BackendKind, ConfigPaths, GroupBy, SortBy};
use crate::metadata::parse_tag_input;
use crate::ui::filter::{
    collect_folders, collect_tags, folder_filter_summary, folder_label, group_by_summary,
    ALL_FOLDERS_LABEL,
};
use crate::ui::gallery::{
    assign_tags, remove_tags, set_favorite, set_random_wallpaper, GalleryModel, GalleryView,
};
use crate::ui::masonry::MasonryGrid;
use crate::ui::overlay::FloatingOverlay;

const ALL_MONITORS: &str = "All monitors";
const ALL_TAGS: &str = "All tags";

/// Sidebar with filename search, output, backend, and awww animation controls. Changes update the shared
/// config immediately and persist to the XDG config file. **Edit** and **Shared tags** sit in a
/// bottom pane (at most half the sidebar height).
pub struct Sidebar {
    pub widget: Widget,
    /// Browse / Output / Gallery stack; section spacing follows GUI scale.
    pub content: Box,
    /// Edit / Shared tags stack; section spacing follows GUI scale.
    pub edit_content: Box,
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
            .tooltip_text("Output that receives the wallpaper")
            .build();
        let refresh = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh monitor list")
            .valign(gtk4::Align::Center)
            .css_classes(["flat"])
            .build();

        let backend_row = ComboRow::builder()
            .title("Backend")
            .tooltip_text("Wallpaper daemon to use")
            .model(&StringList::new(&BackendKind::ALL.map(BackendKind::as_str)))
            .build();
        backend_row.set_selected(
            BackendKind::ALL
                .iter()
                .position(|b| *b == config.borrow().backend)
                .unwrap_or(0) as u32,
        );

        let post_command_row = EntryRow::builder()
            .title("Post command")
            .tooltip_text(
                "Shell command after setting a wallpaper. Use $wallpaper and $monitor. Leave empty to skip.",
            )
            .show_apply_button(true)
            .build();
        post_command_row.set_text(&config.borrow().post_command);

        let output_group = PreferencesGroup::builder().title("Output").build();
        output_group.add(&monitor_row);
        output_group.add(&backend_row);
        output_group.add(&post_command_row);

        let gallery_group =
            build_gallery_section(config.clone(), gallery.clone(), model.clone(), &grid);

        let transition_row = ComboRow::builder()
            .title("Transition")
            .model(&StringList::new(
                &AwwwTransitionKind::ALL.map(AwwwTransitionKind::as_str),
            ))
            .build();
        let duration_row = SpinRow::with_range(0.0, 10.0, 0.1);
        duration_row.set_title("Duration");
        duration_row.set_tooltip_text(Some("Seconds. Ignored by the simple and none transitions"));
        duration_row.set_digits(1);
        let fps_row = SpinRow::with_range(1.0, 240.0, 1.0);
        fps_row.set_title("Frame rate");
        let angle_row = SpinRow::with_range(0.0, 360.0, 5.0);
        angle_row.set_title("Angle");
        angle_row.set_tooltip_text(Some("Degrees, for wipe and wave"));

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

        let anim_group = PreferencesGroup::builder().build();
        anim_group.add(&transition_row);
        anim_group.add(&duration_row);
        anim_group.add(&fps_row);
        anim_group.add(&angle_row);

        let anim_overlay = FloatingOverlay::builder("Animation")
            .content_width(300)
            .body_class("walltag-sidebar")
            .child(&anim_group)
            .build();
        let anim_btn = Button::builder()
            .icon_name("preferences-system-symbolic")
            .tooltip_text("awww animation settings")
            .valign(Align::Center)
            .css_classes(["flat"])
            .build();
        anim_overlay.bind_trigger(&anim_btn);
        let awww_selected = config.borrow().backend == BackendKind::Awww;
        anim_btn.set_visible(awww_selected);

        // Refresh + animation share the Output header so controls stay in-section.
        let output_header = Box::new(Orientation::Horizontal, 2);
        output_header.set_valign(Align::Center);
        output_header.append(&anim_btn);
        output_header.append(&refresh);
        output_group.set_header_suffix(Some(&output_header));

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
                monitor_row
                    .set_selected(entries.iter().position(|e| *e == selected).unwrap_or(0) as u32);
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
            let anim_btn = anim_btn.clone();
            let anim_overlay = anim_overlay.clone();
            backend_row.connect_selected_notify(move |row| {
                let Some(kind) = BackendKind::ALL.get(row.selected() as usize).copied() else {
                    return;
                };
                if kind != BackendKind::Awww {
                    anim_overlay.force_close();
                }
                reveal_widget(&anim_btn, kind == BackendKind::Awww);
                update(&config, |cfg| cfg.backend = kind);
            });
        }
        {
            let config = config.clone();
            post_command_row.connect_apply(move |row| {
                let text = row.text().to_string();
                update(&config, |cfg| cfg.post_command = text);
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
                reveal_widget(&angle_row, kind.uses_angle());
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

        let pick_folder = Button::builder()
            .label("Choose folder…")
            .tooltip_text("Choose the wallpaper root directory")
            .halign(gtk4::Align::End)
            .css_classes(["flat"])
            .build();
        let search_group = build_search_section(
            config.clone(),
            gallery.clone(),
            model.clone(),
            search,
            pick_folder.clone(),
        );

        let content = Box::new(Orientation::Vertical, 10);
        content.add_css_class("walltag-sidebar");
        content.append(&search_group);
        content.append(&output_group);
        content.append(&gallery_group);

        let top_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&content)
            .build();

        let edit_content = build_edit_section(gallery.clone(), grid, model);
        edit_content.add_css_class("walltag-sidebar");

        let edit_scroll = ScrolledWindow::builder()
            .hscrollbar_policy(PolicyType::Never)
            .vscrollbar_policy(PolicyType::Automatic)
            .vexpand(false)
            .hexpand(true)
            .propagate_natural_height(true)
            .child(&edit_content)
            .build();
        edit_scroll.add_css_class("walltag-sidebar-edit");

        let shell = Box::new(Orientation::Vertical, 0);
        shell.add_css_class("walltag-sidebar-shell");
        shell.append(&top_scroll);
        shell.append(&edit_scroll);
        wire_edit_pane_max_height(&shell, &edit_scroll);

        Self {
            widget: shell.upcast(),
            content,
            edit_content,
            pick_folder,
        }
    }
}

const SIDEBAR_REVEAL_MS: f64 = 200.0;

/// Fades a header control in or out, then toggles visibility so layout can settle.
fn reveal_widget(widget: &impl IsA<Widget>, show: bool) {
    let widget = widget.as_ref().clone();
    if show == widget.is_visible() && (show || widget.opacity() >= 0.99) {
        if show {
            widget.set_opacity(1.0);
        }
        return;
    }
    if show {
        widget.set_visible(true);
        widget.set_opacity(0.0);
        let t0 = std::time::Instant::now();
        widget.add_tick_callback(move |w, _| {
            let t = (t0.elapsed().as_secs_f64() * 1000.0 / SIDEBAR_REVEAL_MS).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            w.set_opacity(eased);
            if t >= 1.0 {
                w.set_opacity(1.0);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    } else {
        let start = widget.opacity();
        let t0 = std::time::Instant::now();
        widget.add_tick_callback(move |w, _| {
            let t = (t0.elapsed().as_secs_f64() * 1000.0 / SIDEBAR_REVEAL_MS).min(1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            w.set_opacity(start * (1.0 - eased));
            if t >= 1.0 {
                w.set_opacity(1.0);
                w.set_visible(false);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
}

/// Keeps the edit pane at most half the sidebar height while letting short content shrink.
fn wire_edit_pane_max_height(shell: &Box, edit_scroll: &ScrolledWindow) {
    let sync = {
        let shell = shell.clone();
        let edit_scroll = edit_scroll.clone();
        Rc::new(move || {
            let h = shell.height();
            if h <= 1 {
                return;
            }
            let max = h / 2;
            if edit_scroll.max_content_height() != max {
                edit_scroll.set_max_content_height(max);
            }
        })
    };
    {
        let sync = sync.clone();
        shell.connect_map(move |_| sync());
    }
    shell.connect_realize({
        let sync = sync.clone();
        let shell_weak = shell.downgrade();
        move |shell| {
            sync();
            let Some(native) = shell.native() else {
                return;
            };
            let Some(surface) = native.surface() else {
                return;
            };
            let sync = sync.clone();
            let shell_weak = shell_weak.clone();
            surface.connect_layout(move |_, _, _| {
                if shell_weak.upgrade().is_none() {
                    return;
                }
                let sync = sync.clone();
                glib::idle_add_local_once(move || sync());
            });
        }
    });
}

const EMPTY_SELECTION_HINT: &str =
    "No wallpapers selected. Right-click a wallpaper, or drag a box around several, to select them.";

/// Browse session: wallpaper root, filename search, folder filter, and random pick.
fn build_search_section(
    config: Rc<RefCell<AppConfig>>,
    gallery: Rc<GalleryView>,
    model: GalleryModel,
    search: SearchEntry,
    pick_folder: Button,
) -> PreferencesGroup {
    let group = PreferencesGroup::builder().title("Browse").build();

    let folder_pick_row = ActionRow::builder()
        .title("Wallpaper folder")
        .tooltip_text("Root directory scanned for images")
        .activatable(false)
        .build();
    folder_pick_row.add_suffix(&pick_folder);
    group.add(&folder_pick_row);

    let search_row = ActionRow::builder()
        .title("Filename")
        .activatable(false)
        .build();
    search.set_hexpand(true);
    search_row.add_suffix(&search);
    group.add(&search_row);

    let folder_tooltip = "Show wallpapers in the checked folders. All folders shows every folder.";
    let (folder_row, folder_open) = checklist_trigger_row(
        "Folder",
        folder_tooltip,
        &folder_filter_summary(&model.filter().borrow().folders),
    );
    group.add(&folder_row);

    let folder_search = SearchEntry::new();
    folder_search.set_placeholder_text(Some("Search folders…"));
    folder_search.set_hexpand(true);
    folder_search.set_tooltip_text(Some("Filter the folder checklist"));

    let folder_list = PreferencesGroup::builder().build();
    let folder_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(PolicyType::Never)
        .vscrollbar_policy(PolicyType::Automatic)
        .propagate_natural_height(true)
        .max_content_height(360)
        .vexpand(true)
        .child(&folder_list)
        .build();
    let folder_body = Box::new(Orientation::Vertical, 8);
    folder_body.append(&folder_search);
    folder_body.append(&folder_scroll);
    let folder_overlay = FloatingOverlay::builder("Folder")
        .content_width(300)
        .body_class("walltag-sidebar")
        .child(&folder_body)
        .build();
    {
        let overlay = folder_overlay.clone();
        let folder_search = folder_search.clone();
        folder_open.connect_clicked(move |btn| {
            overlay.present(btn);
            folder_search.grab_focus();
        });
    }
    // SearchEntry can consume Escape (stop-search) inside the dialog focus scope.
    // Closing from that signal covers the case where capture did not reach the sheet.
    folder_search.connect_stop_search({
        let overlay = folder_overlay.clone();
        move |_| {
            let overlay = overlay.clone();
            glib::idle_add_local_once(move || overlay.force_close());
        }
    });
    folder_overlay.dialog().connect_closed({
        let folder_search = folder_search.clone();
        move |_| {
            if !folder_search.text().is_empty() {
                folder_search.set_text("");
            }
        }
    });

    let random = Button::builder()
        .label("Random")
        .tooltip_text("Set a random wallpaper from the current search results")
        .halign(gtk4::Align::End)
        .css_classes(["pill"])
        .build();
    let random_row = ActionRow::builder()
        .title("Random wallpaper")
        .tooltip_text("Uses the current filters")
        .activatable(false)
        .build();
    random_row.add_suffix(&random);
    group.add(&random_row);

    let folder_rows: Rc<RefCell<Vec<ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rebuilding = Rc::new(Cell::new(false));
    // Folders, checked ids, and search text. Gallery refreshes often; rebuild only when
    // this triple changes.
    let shown: Rc<RefCell<Option<(Vec<String>, Vec<String>, String)>>> =
        Rc::new(RefCell::new(None));

    let rebuild_filters: Rc<dyn Fn()> = {
        let folder_row = folder_row.clone();
        let folder_list = folder_list.clone();
        let folder_rows = folder_rows.clone();
        let folder_search = folder_search.clone();
        let model = model.clone();
        let gallery = gallery.clone();
        let config = config.clone();
        let rebuilding = rebuilding.clone();
        let shown = shown.clone();
        Rc::new(move || {
            let entries = model.entries().borrow().clone();
            let current = model.filter().borrow().folders.clone();
            let folders = collect_folders(&entries);
            let query = folder_search.text().to_string();
            // Drop folders that the latest scan no longer contains. An empty scan keeps
            // the saved filter so a failed scan does not wipe it.
            let pruned = if folders.is_empty() {
                current.clone()
            } else {
                current
                    .iter()
                    .filter(|folder| folders.iter().any(|known| known == *folder))
                    .cloned()
                    .collect::<Vec<_>>()
            };
            if pruned != current {
                model.filter().borrow_mut().folders = pruned.clone();
                update(&config, |cfg| cfg.filter_folders = pruned.clone());
                refresh_soon(&gallery);
            }
            let selected = if pruned != current { pruned } else { current };
            folder_row.set_subtitle(&folder_filter_summary(&selected));
            let signature = (folders.clone(), selected.clone(), query.clone());
            if shown.borrow().as_ref() == Some(&signature) {
                return;
            }
            *shown.borrow_mut() = Some(signature);
            let items: Vec<(String, String)> = folders
                .iter()
                .filter(|folder| folder_matches_query(folder, &query))
                .map(|folder| (folder.clone(), folder_label(folder)))
                .collect();
            let on_change: Rc<dyn Fn(Vec<String>)> = Rc::new({
                let gallery = gallery.clone();
                let model = model.clone();
                let config = config.clone();
                let folder_row = folder_row.clone();
                let shown = shown.clone();
                move |selected| {
                    folder_row.set_subtitle(&folder_filter_summary(&selected));
                    if model.filter().borrow().folders == selected {
                        return;
                    }
                    model.filter().borrow_mut().folders = selected.clone();
                    if let Some((_, picked, _)) = shown.borrow_mut().as_mut() {
                        *picked = selected.clone();
                    }
                    update(&config, |cfg| cfg.filter_folders = selected);
                    refresh_soon(&gallery);
                }
            });
            install_exclusive_checks(
                &folder_list,
                &folder_rows,
                ALL_FOLDERS_LABEL,
                &items,
                &selected,
                &rebuilding,
                on_change,
            );
        })
    };
    rebuild_filters();
    gallery.connect_repopulated({
        let rebuild_filters = rebuild_filters.clone();
        move || rebuild_filters()
    });
    folder_search.connect_search_changed({
        let rebuild_filters = rebuild_filters.clone();
        move |_| rebuild_filters()
    });
    {
        let model = model.clone();
        random.connect_clicked(move |_| match set_random_wallpaper(&model) {
            Some(key) => tracing::info!("Random wallpaper: {key}"),
            None => tracing::warn!("No wallpapers match the current filters"),
        });
    }

    group
}

/// Gallery section: group-by, sort, tag filter, and display options.
fn build_gallery_section(
    config: Rc<RefCell<AppConfig>>,
    gallery: Rc<GalleryView>,
    model: GalleryModel,
    grid: &MasonryGrid,
) -> PreferencesGroup {
    let group = PreferencesGroup::builder().title("Gallery").build();

    let selected_kinds = GroupBy::normalize(&config.borrow().group_by);
    let group_tooltip =
        "Section headers. Check folder, tag, or both. None leaves the gallery flat.";
    let (group_row, group_open) = checklist_trigger_row(
        "Group by",
        group_tooltip,
        &group_by_summary(&selected_kinds),
    );
    group.add(&group_row);

    let group_list = PreferencesGroup::builder().build();
    let group_overlay = FloatingOverlay::builder("Group by")
        .content_width(300)
        .body_class("walltag-sidebar")
        .child(&group_list)
        .build();
    group_overlay.bind_trigger(&group_open);

    let group_items = [
        (
            GroupBy::Folder.as_str().to_owned(),
            GroupBy::Folder.label().to_owned(),
        ),
        (
            GroupBy::Tag.as_str().to_owned(),
            GroupBy::Tag.label().to_owned(),
        ),
    ];
    let selected_ids: Vec<String> = selected_kinds
        .iter()
        .map(|kind| kind.as_str().to_owned())
        .collect();
    let group_rows: Rc<RefCell<Vec<ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let group_rebuilding = Rc::new(Cell::new(false));
    install_exclusive_checks(
        &group_list,
        &group_rows,
        GroupBy::None.label(),
        &group_items,
        &selected_ids,
        &group_rebuilding,
        Rc::new({
            let config = config.clone();
            let gallery = gallery.clone();
            let model = model.clone();
            let group_row = group_row.clone();
            move |ids| {
                let kinds = GroupBy::normalize(
                    &ids.iter()
                        .filter_map(|id| GroupBy::parse(id))
                        .collect::<Vec<_>>(),
                );
                group_row.set_subtitle(&group_by_summary(&kinds));
                if model.filter().borrow().group_by == kinds {
                    return;
                }
                model.filter().borrow_mut().group_by = kinds.clone();
                update(&config, |cfg| cfg.group_by = kinds);
                refresh_soon(&gallery);
            }
        }),
    );

    let sort_row = ComboRow::builder()
        .title("Sort by")
        .tooltip_text("Orders groups and wallpapers inside each group")
        .model(&StringList::new(&SortBy::ALL.map(SortBy::label)))
        .build();
    sort_row.set_selected(
        SortBy::ALL
            .iter()
            .position(|s| *s == config.borrow().sort_by)
            .unwrap_or(0) as u32,
    );
    group.add(&sort_row);

    let tag_row = ComboRow::builder()
        .title("Tag")
        .tooltip_text("Show only wallpapers with this tag")
        .build();
    group.add(&tag_row);

    let names_check = CheckButton::new();
    names_check.set_valign(gtk4::Align::Center);
    names_check.set_active(config.borrow().show_filenames);
    let names_row = ActionRow::builder()
        .title("Show file names")
        .tooltip_text("Show the file name under each wallpaper")
        .activatable_widget(&names_check)
        .build();
    names_row.add_suffix(&names_check);
    group.add(&names_row);
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

    let tag_options: Rc<RefCell<Vec<Option<String>>>> = Rc::new(RefCell::new(vec![None]));
    let rebuilding = Rc::new(Cell::new(false));
    let rebuild_tags: Rc<dyn Fn()> = {
        let tag_row = tag_row.clone();
        let tag_options = tag_options.clone();
        let model = model.clone();
        let rebuilding = rebuilding.clone();
        Rc::new(move || {
            rebuilding.set(true);
            let entries = model.entries().borrow().clone();
            let metadata = model.metadata();
            let metadata = metadata.borrow();
            let filter = model.filter().borrow().clone();
            let tags = collect_tags(&entries, &metadata);
            let mut labels = vec![ALL_TAGS.to_owned()];
            labels.extend(tags.iter().cloned());
            let mut opts = vec![None];
            opts.extend(tags.into_iter().map(Some));
            let sel = filter
                .tag
                .as_ref()
                .and_then(|t| opts.iter().position(|o| o.as_ref() == Some(t)))
                .unwrap_or(0);
            tag_row.set_model(Some(&StringList::new(
                &labels.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            )));
            tag_row.set_selected(sel as u32);
            *tag_options.borrow_mut() = opts;
            rebuilding.set(false);
        })
    };
    rebuild_tags();
    gallery.connect_repopulated({
        let rebuild_tags = rebuild_tags.clone();
        move || rebuild_tags()
    });

    {
        let config = config.clone();
        let gallery = gallery.clone();
        let model = model.clone();
        sort_row.connect_selected_notify(move |row| {
            let Some(kind) = SortBy::ALL.get(row.selected() as usize).copied() else {
                return;
            };
            if model.filter().borrow().sort_by == kind {
                return;
            }
            model.filter().borrow_mut().sort_by = kind;
            update(&config, |cfg| cfg.sort_by = kind);
            gallery.refresh(true);
        });
    }
    {
        let gallery = gallery.clone();
        let model = model.clone();
        let tag_options = tag_options.clone();
        let rebuilding = rebuilding.clone();
        let config = config.clone();
        tag_row.connect_selected_notify(move |row| {
            if rebuilding.get() {
                return;
            }
            let choice = tag_options
                .borrow()
                .get(row.selected() as usize)
                .cloned()
                .flatten();
            if model.filter().borrow().tag == choice {
                return;
            }
            model.filter().borrow_mut().tag = choice.clone();
            update(&config, |cfg| cfg.filter_tag = choice);
            gallery.refresh(true);
        });
    }

    group
}

/// Edit + shared tags for the bottom-anchored sidebar pane.
fn build_edit_section(gallery: Rc<GalleryView>, grid: MasonryGrid, model: GalleryModel) -> Box {
    let clear = Button::builder()
        .label("Clear")
        .tooltip_text("Clear selection (Esc)")
        .valign(Align::Center)
        .css_classes(["flat"])
        .build();
    let tags_row = EntryRow::builder()
        .title("Add tags (comma or space separated)")
        .show_apply_button(true)
        .build();
    let favorite_check = CheckButton::new();
    favorite_check.set_valign(Align::Center);
    let favorite_row = ActionRow::builder()
        .title("Favorite")
        .subtitle("Appears in the Favorites group")
        .activatable_widget(&favorite_check)
        .build();
    favorite_row.add_suffix(&favorite_check);

    let group = PreferencesGroup::builder().title("Edit").build();
    group.set_header_suffix(Some(&clear));
    group.add(&favorite_row);
    group.add(&tags_row);

    let shared_group = PreferencesGroup::builder()
        .title("Shared tags")
        .description("Tags on every selected wallpaper. Remove deletes them from the selection.")
        .build();
    shared_group.add_css_class("walltag-shared-tags");
    let shared_revealer = Revealer::builder()
        .transition_type(RevealerTransitionType::SlideDown)
        .transition_duration(SIDEBAR_REVEAL_MS as u32)
        .reveal_child(false)
        .child(&shared_group)
        .build();
    let shared_rows: Rc<RefCell<Vec<ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let syncing = Rc::new(Cell::new(false));
    // Filled after `refresh` is built so remove / favorite handlers can call it.
    let refresh_slot: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));

    let refresh: Rc<dyn Fn()> = {
        let grid = grid.clone();
        let group = group.clone();
        let shared_group = shared_group.clone();
        let shared_revealer = shared_revealer.clone();
        let tags_row = tags_row.clone();
        let clear = clear.clone();
        let favorite_check = favorite_check.clone();
        let favorite_row = favorite_row.clone();
        let model = model.clone();
        let gallery = gallery.clone();
        let shared_rows = shared_rows.clone();
        let syncing = syncing.clone();
        let refresh_slot = refresh_slot.clone();
        Rc::new(move || {
            let keys = grid.selected_keys();
            let count = keys.len();
            let description = match count {
                0 => EMPTY_SELECTION_HINT.to_owned(),
                1 => "1 wallpaper selected".to_owned(),
                n => format!("{n} wallpapers selected"),
            };
            group.set_description(Some(&description));
            tags_row.set_sensitive(count > 0);
            clear.set_sensitive(count > 0);
            favorite_row.set_sensitive(count > 0);

            syncing.set(true);
            match model.metadata().borrow().shared_favorite(&keys) {
                Some(value) => {
                    favorite_check.set_inconsistent(false);
                    favorite_check.set_active(value);
                }
                None if count > 0 => {
                    favorite_check.set_inconsistent(true);
                    favorite_check.set_active(false);
                }
                None => {
                    favorite_check.set_inconsistent(false);
                    favorite_check.set_active(false);
                }
            }
            syncing.set(false);

            for row in shared_rows.borrow_mut().drain(..) {
                shared_group.remove(&row);
            }
            let shared = if count > 0 {
                model.metadata().borrow().shared_tags(&keys)
            } else {
                Vec::new()
            };
            shared_revealer.set_reveal_child(count > 0);
            if shared.is_empty() {
                shared_group.set_description(Some(if count == 0 {
                    "Select wallpapers to manage shared tags."
                } else {
                    "No tags in common across the selection."
                }));
            } else {
                shared_group.set_description(Some(
                    "Tags on every selected wallpaper. Remove deletes them from the selection.",
                ));
                for tag in shared {
                    let row = ActionRow::builder()
                        .title(tag.clone())
                        .activatable(false)
                        .build();
                    let remove = Button::builder()
                        .icon_name("list-remove-symbolic")
                        .tooltip_text(format!("Remove “{tag}” from selection"))
                        .valign(Align::Center)
                        .css_classes(["flat"])
                        .build();
                    {
                        let grid = grid.clone();
                        let model = model.clone();
                        let gallery = gallery.clone();
                        let group = group.clone();
                        let refresh_slot = refresh_slot.clone();
                        let tag = tag.clone();
                        remove.connect_clicked(move |_| {
                            let keys = grid.selected_keys();
                            if keys.is_empty() {
                                return;
                            }
                            match remove_tags(&grid, &model, &keys, &[tag.clone()]) {
                                Ok(()) => {
                                    group.set_description(Some(&format!(
                                        "Removed “{tag}” from {} selected",
                                        keys.len()
                                    )));
                                    gallery.refresh(false);
                                    if let Some(refresh) = refresh_slot.borrow().as_ref() {
                                        refresh();
                                    }
                                }
                                Err(err) => {
                                    tracing::error!("Failed to remove tag: {err:#}");
                                    group.set_description(Some(
                                        "Could not save tags. See the log for details.",
                                    ));
                                }
                            }
                        });
                    }
                    row.add_suffix(&remove);
                    shared_group.add(&row);
                    shared_rows.borrow_mut().push(row);
                }
            }
        })
    };
    *refresh_slot.borrow_mut() = Some(refresh.clone());
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
        let grid = grid.clone();
        let model = model.clone();
        let gallery = gallery.clone();
        let group = group.clone();
        let syncing = syncing.clone();
        let refresh = refresh.clone();
        favorite_check.connect_toggled(move |check| {
            if syncing.get() {
                return;
            }
            let keys = grid.selected_keys();
            if keys.is_empty() {
                return;
            }
            check.set_inconsistent(false);
            let favorite = check.is_active();
            match set_favorite(&grid, &model, &keys, favorite) {
                Ok(()) => {
                    group.set_description(Some(&format!(
                        "{} {} selected",
                        if favorite { "Favorited" } else { "Unfavorited" },
                        keys.len()
                    )));
                    gallery.refresh(false);
                    refresh();
                }
                Err(err) => {
                    tracing::error!("Failed to save favorite: {err:#}");
                    group
                        .set_description(Some("Could not save favorite. See the log for details."));
                }
            }
        });
    }
    {
        let group = group.clone();
        let gallery = gallery.clone();
        let model = model.clone();
        let grid = grid.clone();
        let refresh = refresh.clone();
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
                    refresh();
                }
                Err(err) => {
                    tracing::error!("Failed to save tags: {err:#}");
                    group.set_description(Some("Could not save tags. See the log for details."));
                }
            }
        });
    }

    let section = Box::new(Orientation::Vertical, 10);
    section.append(&group);
    section.append(&shared_revealer);
    section
}

/// Rebuilds the gallery on the next idle turn so a check handler can finish first.
fn refresh_soon(gallery: &Rc<GalleryView>) {
    let gallery = Rc::clone(gallery);
    let _ = glib::idle_add_local_once(move || gallery.refresh(true));
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

/// Result of toggling one row in an exclusive checklist.
///
/// Index 0 is the "all / none" row. It is on exactly when the returned indexes are
/// empty. Every other row can be on at the same time. `active` is the state after the
/// click, and `toggled` is the row that changed.
#[cfg(test)]
fn exclusive_after_toggle(active: &[bool], toggled: usize) -> Vec<usize> {
    if active.is_empty() || toggled >= active.len() {
        return Vec::new();
    }
    let mut on = active.to_vec();
    if toggled == 0 {
        if on[0] {
            for flag in &mut on[1..] {
                *flag = false;
            }
        } else {
            on[0] = true;
        }
    } else if on[toggled] {
        on[0] = false;
    } else if !on[1..].iter().any(|flag| *flag) {
        on[0] = true;
    }
    on.iter()
        .enumerate()
        .skip(1)
        .filter_map(|(index, flag)| flag.then_some(index))
        .collect()
}

/// Whether a folder checklist row matches the overlay search box.
fn folder_matches_query(folder: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || folder_label(folder).to_lowercase().contains(&q)
}

/// Check states and full selection after a toggle.
///
/// `visible_ids[0]` is always `None` (the All row). `hidden_selected` are checked ids
/// the search filter is hiding. Checking All clears those too. Unchecking the last
/// visible item keeps All off while any hidden selection remains.
fn exclusive_selection_after_toggle(
    active: &[bool],
    toggled: usize,
    visible_ids: &[Option<String>],
    hidden_selected: &[String],
) -> (Vec<bool>, Vec<String>) {
    if active.is_empty() || toggled >= active.len() || visible_ids.len() != active.len() {
        return (active.to_vec(), hidden_selected.to_vec());
    }
    let mut on = active.to_vec();
    if toggled == 0 {
        if on[0] {
            for flag in &mut on[1..] {
                *flag = false;
            }
        } else {
            on[0] = true;
        }
        return (on, Vec::new());
    }
    if on[toggled] {
        on[0] = false;
    } else if !on[1..].iter().any(|flag| *flag) && hidden_selected.is_empty() {
        on[0] = true;
    }
    if on[0] {
        return (on, Vec::new());
    }
    let mut next = hidden_selected.to_vec();
    for (index, flag) in on.iter().enumerate().skip(1) {
        if !*flag {
            continue;
        }
        let Some(id) = visible_ids[index].clone() else {
            continue;
        };
        if !next.contains(&id) {
            next.push(id);
        }
    }
    (on, next)
}

/// Writes the exclusive toggle result onto the checks and returns the full selection.
fn apply_exclusive_toggle(
    checks: &[(Option<String>, CheckButton)],
    toggled: usize,
    hidden_selected: &[String],
) -> Vec<String> {
    let active: Vec<bool> = checks.iter().map(|(_, check)| check.is_active()).collect();
    let visible_ids: Vec<Option<String>> = checks.iter().map(|(id, _)| id.clone()).collect();
    let (on, selected) =
        exclusive_selection_after_toggle(&active, toggled, &visible_ids, hidden_selected);
    for (index, flag) in on.iter().enumerate() {
        if checks[index].1.is_active() != *flag {
            checks[index].1.set_active(*flag);
        }
    }
    selected
}

fn check_row(title: &str, check: &CheckButton) -> ActionRow {
    check.set_valign(gtk4::Align::Center);
    let row = ActionRow::builder()
        .title(title)
        .activatable_widget(check)
        .build();
    row.add_suffix(check);
    row
}

/// Sidebar row that opens a checklist overlay. The subtitle shows the current selection.
fn checklist_trigger_row(title: &str, tooltip: &str, subtitle: &str) -> (ActionRow, Button) {
    let open = Button::builder()
        .icon_name("go-next-symbolic")
        .tooltip_text(tooltip)
        .valign(Align::Center)
        .css_classes(["flat"])
        .build();
    let row = ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .tooltip_text(tooltip)
        .activatable_widget(&open)
        .build();
    row.add_suffix(&open);
    (row, open)
}

/// Replaces the list rows with an exclusive "all" row plus one check per item.
///
/// `selected` is the full selection, including ids that the search filter hides.
/// An empty `selected` checks the all-row. Checking an item clears the all-row.
/// Checking the all-row clears every selected id. Unchecking the last visible item
/// checks the all-row only when nothing stays selected outside the filter.
fn install_exclusive_checks(
    list: &PreferencesGroup,
    stored_rows: &Rc<RefCell<Vec<ActionRow>>>,
    all_label: &str,
    items: &[(String, String)],
    selected: &[String],
    rebuilding: &Rc<Cell<bool>>,
    on_change: Rc<dyn Fn(Vec<String>)>,
) {
    rebuilding.set(true);
    for row in stored_rows.borrow_mut().drain(..) {
        list.remove(&row);
    }

    let hidden: Rc<Vec<String>> = Rc::new(
        selected
            .iter()
            .filter(|id| !items.iter().any(|(visible, _)| visible == *id))
            .cloned()
            .collect(),
    );
    let mut checks: Vec<(Option<String>, CheckButton)> = Vec::new();
    let mut ui_rows = Vec::new();

    let all_check = CheckButton::new();
    all_check.set_active(selected.is_empty());
    checks.push((None, all_check.clone()));
    let all_row = check_row(all_label, &all_check);
    list.add(&all_row);
    ui_rows.push(all_row);

    for (id, label) in items {
        let check = CheckButton::new();
        check.set_active(selected.iter().any(|picked| picked == id));
        checks.push((Some(id.clone()), check.clone()));
        let row = check_row(label, &check);
        list.add(&row);
        ui_rows.push(row);
    }
    *stored_rows.borrow_mut() = ui_rows;

    let checks = Rc::new(checks);
    for idx in 0..checks.len() {
        let check = checks[idx].1.clone();
        let checks = Rc::clone(&checks);
        let hidden = Rc::clone(&hidden);
        let rebuilding = rebuilding.clone();
        let on_change = Rc::clone(&on_change);
        check.connect_toggled(move |_| {
            if rebuilding.get() {
                return;
            }
            rebuilding.set(true);
            let selected = apply_exclusive_toggle(&checks, idx, &hidden);
            rebuilding.set(false);
            on_change(selected);
        });
    }
    rebuilding.set(false);
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

    #[test]
    fn all_row_clears_items() {
        assert_eq!(
            exclusive_after_toggle(&[true, true, true], 0),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn checking_an_item_clears_the_all_row() {
        assert_eq!(exclusive_after_toggle(&[true, true, false], 1), vec![1]);
    }

    #[test]
    fn checking_another_item_keeps_the_first() {
        assert_eq!(exclusive_after_toggle(&[false, true, true], 2), vec![1, 2]);
    }

    #[test]
    fn unchecking_one_item_keeps_the_rest() {
        assert_eq!(exclusive_after_toggle(&[false, false, true], 1), vec![2]);
    }

    #[test]
    fn unchecking_the_last_item_restores_all() {
        assert_eq!(
            exclusive_after_toggle(&[false, false], 1),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn unchecking_all_does_not_stick() {
        assert_eq!(
            exclusive_after_toggle(&[false, false], 0),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn folder_search_matches_label_case_insensitively() {
        assert!(folder_matches_query("cats", ""));
        assert!(folder_matches_query("cats", "  CAT "));
        assert!(folder_matches_query("", "root"));
        assert!(!folder_matches_query("cats", "dog"));
    }

    #[test]
    fn filtered_toggle_keeps_hidden_selection() {
        let ids = [None, Some("dogs".into())];
        let (on, next) =
            exclusive_selection_after_toggle(&[false, true], 1, &ids, &["cats".into()]);
        assert_eq!(on, vec![false, true]);
        assert_eq!(next, vec!["cats".to_owned(), "dogs".to_owned()]);
    }

    #[test]
    fn filtered_all_clears_hidden_selection() {
        let ids = [None, Some("dogs".into())];
        let (on, next) = exclusive_selection_after_toggle(&[true, true], 0, &ids, &["cats".into()]);
        assert_eq!(on, vec![true, false]);
        assert!(next.is_empty());
    }

    #[test]
    fn unchecking_last_visible_keeps_hidden_without_all() {
        let ids = [None, Some("dogs".into())];
        let (on, next) =
            exclusive_selection_after_toggle(&[false, false], 1, &ids, &["cats".into()]);
        assert_eq!(on, vec![false, false]);
        assert_eq!(next, vec!["cats".to_owned()]);
    }
}
