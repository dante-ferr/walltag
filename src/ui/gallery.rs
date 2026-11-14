use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gio, glib, Box, Label, Orientation, PolicyType, ScrolledWindow, SearchEntry};

use crate::backends::ApplyOptions;
use crate::metadata::MetadataStore;
use crate::scan::WallpaperEntry;
use crate::ui::color::probe_colors;
use crate::ui::dim_cache::{resolve_dimensions, DimensionCache};
use crate::ui::filter::{build_rows, GalleryFilter, GalleryRow, SortBy, SortData};
use crate::ui::masonry::{GridItem, MasonryGrid, FALLBACK_ASPECT};
use crate::AppConfig;
use crate::Cli;

type Dimensions = Rc<RefCell<HashMap<String, (i32, i32)>>>;

#[derive(Clone)]
pub struct GalleryModel {
    entries: Rc<RefCell<Vec<WallpaperEntry>>>,
    metadata: Rc<RefCell<MetadataStore>>,
    config: Rc<RefCell<AppConfig>>,
    filter: Rc<RefCell<GalleryFilter>>,
    /// File stats and sampled colors that the sort modes read.
    sort_data: Rc<RefCell<SortData>>,
}

impl GalleryModel {
    /// `config` is shared with the rest of the UI so sidebar changes apply immediately.
    pub fn new(config: Rc<RefCell<AppConfig>>, metadata: MetadataStore) -> Self {
        let (group_by, sort_by) = {
            let cfg = config.borrow();
            (cfg.group_by, cfg.sort_by)
        };
        Self {
            entries: Rc::new(RefCell::new(Vec::new())),
            metadata: Rc::new(RefCell::new(metadata)),
            config,
            filter: Rc::new(RefCell::new(GalleryFilter {
                group_by,
                sort_by,
                ..Default::default()
            })),
            sort_data: Rc::new(RefCell::new(SortData::default())),
        }
    }

    pub fn set_entries(&self, entries: Vec<WallpaperEntry>) {
        *self.entries.borrow_mut() = entries;
    }

    pub fn config(&self) -> Rc<RefCell<AppConfig>> {
        self.config.clone()
    }

    pub fn set_metadata(&self, metadata: MetadataStore) {
        *self.metadata.borrow_mut() = metadata;
    }

    pub fn metadata(&self) -> Rc<RefCell<MetadataStore>> {
        self.metadata.clone()
    }

    pub fn entries(&self) -> Rc<RefCell<Vec<WallpaperEntry>>> {
        self.entries.clone()
    }

    pub fn filter(&self) -> Rc<RefCell<GalleryFilter>> {
        self.filter.clone()
    }

    pub fn sort_data(&self) -> Rc<RefCell<SortData>> {
        self.sort_data.clone()
    }

    /// Samples average colors for entries that have none, when the hue sort is active.
    /// Returns `true` if new colors were stored. Results are dropped when `generation`
    /// no longer matches `current` (the folder was rescanned meanwhile).
    async fn load_colors(&self, current: &Cell<u64>, generation: u64) -> bool {
        if self.filter.borrow().sort_by != SortBy::Hue {
            return false;
        }
        let missing = self.sort_data.borrow().missing_colors(&self.entries.borrow());
        if missing.is_empty() {
            return false;
        }
        let input = missing.clone();
        let Ok(found) = gio::spawn_blocking(move || probe_colors(input)).await else {
            return false;
        };
        if current.get() != generation {
            return false;
        }
        self.sort_data.borrow_mut().record_colors(&missing, found);
        true
    }
}

pub struct GalleryView {
    pub root: Box,
    /// Not placed in the gallery; the sidebar hosts it.
    pub search: SearchEntry,
    pub grid: MasonryGrid,
    model: GalleryModel,
    dimensions: Dimensions,
    generation: Rc<Cell<u64>>,
    /// Invoked after the visible set changes so the sidebar can refresh filter lists.
    on_repopulated: Rc<RefCell<Vec<Rc<dyn Fn()>>>>,
}

impl GalleryView {
    pub fn new(model: GalleryModel) -> Self {
        let root = Box::new(Orientation::Vertical, 0);
        root.set_margin_top(12);
        root.set_margin_bottom(12);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.set_vexpand(true);
        root.add_css_class("walltag-gallery-root");

        let search = SearchEntry::new();
        search.set_placeholder_text(Some("Search by filename…"));
        search.set_hexpand(true);

        let scroll = ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(PolicyType::Never)
            .build();
        let grid = MasonryGrid::new(&scroll.vadjustment());
        scroll.set_child(Some(&grid));
        root.append(&scroll);

        let view = Self {
            root,
            search,
            grid,
            model,
            dimensions: Rc::new(RefCell::new(HashMap::new())),
            generation: Rc::new(Cell::new(0)),
            on_repopulated: Rc::new(RefCell::new(Vec::new())),
        };
        view.wire_filters();
        view
    }

    pub fn connect_repopulated<F: Fn() + 'static>(&self, f: F) {
        self.on_repopulated.borrow_mut().push(Rc::new(f));
    }

    /// Rebuilds the grid from the current entries, metadata, filter, and sort. With the
    /// hue sort, colors that are not cached yet are sampled in the background and the grid
    /// is rebuilt once they arrive.
    pub fn refresh(&self, reset_scroll: bool) {
        populate(
            &self.grid,
            &self.model,
            &self.dimensions.borrow(),
            reset_scroll,
        );
        self.notify_repopulated();

        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let current = self.generation.clone();
        let generation = current.get();
        let on_repopulated = self.on_repopulated.clone();
        glib::spawn_future_local(async move {
            if !model.load_colors(&current, generation).await {
                return;
            }
            if populate(&grid, &model, &dimensions.borrow(), false) {
                for cb in on_repopulated.borrow().iter() {
                    cb();
                }
            }
        });
    }

    fn notify_repopulated(&self) {
        for cb in self.on_repopulated.borrow().iter() {
            cb();
        }
    }

    fn wire_filters(&self) {
        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let on_repopulated = self.on_repopulated.clone();
        self.search.connect_search_changed(move |entry| {
            model.filter.borrow_mut().query = entry.text().to_string();
            populate(&grid, &model, &dimensions.borrow(), true);
            for cb in on_repopulated.borrow().iter() {
                cb();
            }
        });
    }

    /// Replaces the gallery contents and paints immediately.
    ///
    /// Saved sizes are applied up front so a warm cache lays out with the real aspect
    /// ratios. Stats, cache misses, and hue samples finish on a worker and update the
    /// grid only when the order or the sizes actually change.
    pub fn reload_from_scan(&self, entries: Vec<WallpaperEntry>) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();

        let root = self.model.config().borrow().wallpaper_root.clone();
        let probe_input: Vec<_> = entries
            .iter()
            .map(|e| (e.relative_key.clone(), e.absolute.clone()))
            .collect();
        self.search.set_text("");
        self.model.filter.borrow_mut().query.clear();
        self.model.filter.borrow_mut().tag = None;
        self.model.filter.borrow_mut().folder = None;
        self.model.set_entries(entries);
        *self.dimensions.borrow_mut() = DimensionCache::load(&root).unvalidated();
        populate(&self.grid, &self.model, &self.dimensions.borrow(), true);
        self.notify_repopulated();

        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let current = self.generation.clone();
        let on_repopulated = self.on_repopulated.clone();
        glib::spawn_future_local(async move {
            let root_for_probe = root.clone();
            let Ok((found, stats)) =
                gio::spawn_blocking(move || resolve_dimensions(&root_for_probe, &probe_input))
                    .await
            else {
                return;
            };
            if current.get() != generation {
                return;
            }
            *dimensions.borrow_mut() = found;
            model.sort_data().borrow_mut().replace_stats(stats);
            if populate(&grid, &model, &dimensions.borrow(), false) {
                for cb in on_repopulated.borrow().iter() {
                    cb();
                }
            }
            if !model.load_colors(&current, generation).await {
                return;
            }
            if current.get() != generation {
                return;
            }
            if populate(&grid, &model, &dimensions.borrow(), false) {
                for cb in on_repopulated.borrow().iter() {
                    cb();
                }
            }
        });
    }

    pub fn clear_empty_banner(&self) {
        let mut child = self.root.first_child();
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if widget.has_css_class("walltag-empty-banner") {
                self.root.remove(&widget);
            }
            child = next;
        }
    }

    pub fn show_empty_banner(&self, config: &AppConfig) {
        self.clear_empty_banner();
        if self.model.entries().borrow().is_empty() {
            let banner = Box::new(Orientation::Vertical, 6);
            banner.add_css_class("walltag-empty-banner");
            let msg = Label::new(Some(&format!(
                "No images found under {}.\nAdd wallpapers or choose a different folder.",
                config.wallpaper_root.display()
            )));
            msg.set_wrap(true);
            banner.append(&msg);
            self.root.prepend(&banner);
        }
    }
}

/// Rebuilds the visible rows. Returns `false` when `reset_scroll` is false and the
/// rows already match, so thumbnails that are already decoding are left alone.
fn populate(
    grid: &MasonryGrid,
    model: &GalleryModel,
    dimensions: &HashMap<String, (i32, i32)>,
    reset_scroll: bool,
) -> bool {
    let metadata = model.metadata();
    let metadata = metadata.borrow();
    let filter = model.filter.borrow().clone();
    let rows = build_rows(
        &model.entries().borrow(),
        &metadata,
        &filter,
        &model.sort_data().borrow(),
    );
    let items: Vec<GridItem> = rows
        .into_iter()
        .map(|row| match row {
            GalleryRow::Header(title) => GridItem::header(title),
            GalleryRow::Wallpaper(wp) => {
                let (aspect, natural_width) = match dimensions.get(&wp.relative_key) {
                    Some(&(w, h)) => (w as f32 / h as f32, w),
                    None => (FALLBACK_ASPECT, 0),
                };
                let record = metadata.record_for_key(&wp.relative_key);
                let tooltip = tooltip_text(&wp.file_name, &record.tags);
                GridItem {
                    key: wp.relative_key.clone(),
                    path: wp.absolute.clone(),
                    aspect,
                    natural_width,
                    tooltip,
                    file_name: wp.file_name.clone(),
                    section_title: None,
                }
            }
        })
        .collect();
    if reset_scroll {
        grid.set_items(items, true);
        true
    } else {
        grid.replace_items_if_changed(items, false)
    }
}

fn tooltip_text(file_name: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        file_name.to_owned()
    } else {
        format!("{}\n{}", file_name, tags.join(" · "))
    }
}

/// Adds `tags` to the wallpapers in `keys`, saves the metadata file, and refreshes the
/// tile tooltips.
pub fn assign_tags(
    grid: &MasonryGrid,
    model: &GalleryModel,
    keys: &[String],
    tags: &[String],
) -> anyhow::Result<()> {
    let metadata = model.metadata();
    let mut metadata = metadata.borrow_mut();
    metadata.add_tags(keys, tags);
    let saved = metadata.save();

    let entries = model.entries();
    for entry in entries.borrow().iter().filter(|e| keys.contains(&e.relative_key)) {
        let record = metadata.record_for_key(&entry.relative_key);
        grid.update_tooltip(&entry.relative_key, tooltip_text(&entry.file_name, &record.tags));
    }
    saved
}

/// Applies a random wallpaper from the currently filtered set (ignoring section headers).
pub fn set_random_wallpaper(model: &GalleryModel) -> anyhow::Result<Option<String>> {
    let metadata = model.metadata();
    let metadata = metadata.borrow();
    let filter = model.filter.borrow().clone();
    let rows = build_rows(
        &model.entries().borrow(),
        &metadata,
        &filter,
        &model.sort_data().borrow(),
    );
    let candidates: Vec<_> = rows
        .into_iter()
        .filter_map(|r| match r {
            GalleryRow::Wallpaper(wp) => Some(wp),
            GalleryRow::Header(_) => None,
        })
        .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    let entry = candidates[seed % candidates.len()].clone();
    let cfg = model.config().borrow().clone();
    let options = ApplyOptions::from_config(&cfg);
    crate::backends::apply_wallpaper(cfg.backend, &entry.absolute, &options)?;
    Cli::persist_last_wallpaper(&cfg, &entry.absolute)?;
    Ok(Some(entry.relative_key))
}

pub fn wire_activate(grid: &MasonryGrid, model: &GalleryModel) {
    let config = model.config();
    let entries = model.entries();
    grid.connect_item_activated(move |item| {
        if item.is_header() {
            return;
        }
        let Some(entry) = entries
            .borrow()
            .iter()
            .find(|e| e.relative_key == item.key)
            .cloned()
        else {
            return;
        };
        let cfg = config.borrow();
        let options = ApplyOptions::from_config(&cfg);
        if let Err(err) = crate::backends::apply_wallpaper(cfg.backend, &entry.absolute, &options)
        {
            tracing::error!("Failed to set wallpaper: {err:#}");
            return;
        }
        if let Err(err) = Cli::persist_last_wallpaper(&cfg, &entry.absolute) {
            tracing::error!("Failed to save wallpaper state: {err:#}");
        }
        tracing::info!(
            "Set wallpaper via {} on {}: {}",
            cfg.backend.as_str(),
            options.monitor.as_deref().unwrap_or("all monitors"),
            entry.relative_key
        );
    });
}

/// Returns CSS bundled with the app for gallery layout tweaks (Libadwaita remains primary).
pub fn default_app_css() -> &'static str {
    r#"
    .walltag-gallery-root {
      background: transparent;
    }
    .walltag-thumb {
      background: alpha(@window_fg_color, 0.06);
    }
    .walltag-tile-name {
      font-size: 0.9em;
      padding: 0 2px;
      color: alpha(@window_fg_color, 0.8);
    }
    .walltag-section-header {
      font-weight: 600;
      font-size: 1.05em;
      padding: 4px 2px;
      color: @window_fg_color;
      opacity: 0.85;
    }
    /* walltag-css-start */
    /* Compact sidebar: tighter rows, headers, and gaps between groups. */
    .walltag-sidebar preferencesgroup > box {
      border-spacing: 4px;
    }
    .walltag-sidebar preferencesgroup > box > box > label.heading {
      font-size: 0.95em;
    }
    .walltag-sidebar row {
      min-height: 0;
    }
    .walltag-sidebar row > box.header {
      min-height: 34px;
      padding: 0 8px;
      border-spacing: 8px;
    }
    .walltag-sidebar row > box.header > box.title {
      margin: 3px 0;
    }
    .walltag-sidebar row.entry {
      min-height: 0;
    }
    .walltag-sidebar row.entry > box.header {
      padding: 0 8px;
    }
    .walltag-sidebar row.spin > box.header {
      min-height: 34px;
    }
    .walltag-sidebar row > box.header > box.suffixes button.pill {
      min-height: 0;
      padding: 3px 14px;
    }
    .walltag-sidebar row > box.header > box.suffixes entry {
      min-height: 0;
      padding-top: 3px;
      padding-bottom: 3px;
    }
    /* Dropdown rows: the selected value and arrow never get a hover chip. */
    row.combo > box.header > box.suffixes listview,
    row.combo > box.header > box.suffixes listview > row,
    row.combo > box.header > box.suffixes listview > row:hover,
    row.combo > box.header > box.suffixes listview > row:active,
    row.combo > box.header > box.suffixes listview > row:focus {
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      outline: none;
      min-height: 0;
      padding: 0 2px;
    }
    row.combo > box.header > box.suffixes image.dropdown-arrow,
    row.combo > box.header > box.suffixes image.dropdown-arrow:hover {
      -gtk-icon-size: 20px;
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      padding: 0;
      margin: 0 2px;
      min-width: 20px;
      min-height: 20px;
    }
    row.combo > box.header > box.suffixes > box,
    row.combo > box.header > box.suffixes > box:hover {
      background: none;
      background-color: transparent;
    }
    /* Dropdown popover: no gray frame around the list. */
    row.combo popover.menu,
    row.combo popover.menu.background {
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      padding: 0;
    }
    row.combo popover.menu > contents {
      padding: 3px;
      border-radius: 10px;
    }
    row.combo popover.menu > contents box,
    row.combo popover.menu > contents scrolledwindow,
    row.combo popover.menu > contents viewport,
    row.combo popover.menu > contents listview {
      background: none;
      background-color: transparent;
      padding: 0;
    }
    row.combo popover.menu > contents listview > row {
      background-color: transparent;
      min-height: 0;
      padding: 0;
      border-radius: 6px;
    }
    row.combo popover.menu > contents listview > row > box {
      padding: 5px 10px;
      min-height: 0;
    }
    row.combo popover.menu > contents listview > row:hover,
    row.combo popover.menu > contents listview > row:selected {
      background-color: alpha(currentColor, 0.1);
    }
    /* walltag-css-end */
    "#
}
