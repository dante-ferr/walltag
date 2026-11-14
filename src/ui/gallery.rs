use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{gio, glib, Box, Label, Orientation, PolicyType, ScrolledWindow, SearchEntry};

use crate::backends::ApplyOptions;
use crate::metadata::MetadataStore;
use crate::scan::WallpaperEntry;
use crate::ui::masonry::{GridItem, MasonryGrid, FALLBACK_ASPECT};
use crate::ui::thumbs::probe_dimensions;
use crate::AppConfig;
use crate::Cli;

type Dimensions = Rc<RefCell<HashMap<String, (i32, i32)>>>;

#[derive(Clone)]
pub struct GalleryModel {
    entries: Rc<RefCell<Vec<WallpaperEntry>>>,
    metadata: Rc<RefCell<MetadataStore>>,
    config: Rc<RefCell<AppConfig>>,
}

impl GalleryModel {
    /// `config` is shared with the rest of the UI so sidebar changes apply immediately.
    pub fn new(config: Rc<RefCell<AppConfig>>, metadata: MetadataStore) -> Self {
        Self {
            entries: Rc::new(RefCell::new(Vec::new())),
            metadata: Rc::new(RefCell::new(metadata)),
            config,
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
}

pub struct GalleryView {
    pub root: Box,
    /// Not placed in the gallery; the sidebar hosts it.
    pub search: SearchEntry,
    pub grid: MasonryGrid,
    model: GalleryModel,
    dimensions: Dimensions,
    generation: Rc<Cell<u64>>,
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
        };
        view.wire_filters();
        view
    }

    fn wire_filters(&self) {
        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        self.search.connect_search_changed(move |entry| {
            populate(&grid, &model, &dimensions.borrow(), &entry.text(), true);
        });
    }

    /// Replaces the gallery contents. Image sizes are read on a worker thread first so the
    /// masonry lays out once with the correct aspect ratios.
    pub fn reload_from_scan(&self, entries: Vec<WallpaperEntry>) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();

        let probe_input: Vec<_> = entries
            .iter()
            .map(|e| (e.relative_key.clone(), e.absolute.clone()))
            .collect();
        self.search.set_text("");
        self.model.set_entries(entries);
        self.dimensions.borrow_mut().clear();
        self.grid.set_items(Vec::new(), true);

        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let current = self.generation.clone();
        let search = self.search.clone();
        glib::spawn_future_local(async move {
            let Ok(found) = gio::spawn_blocking(move || probe_dimensions(probe_input)).await
            else {
                return;
            };
            if current.get() != generation {
                return;
            }
            *dimensions.borrow_mut() = found;
            populate(&grid, &model, &dimensions.borrow(), &search.text(), true);
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

fn populate(
    grid: &MasonryGrid,
    model: &GalleryModel,
    dimensions: &HashMap<String, (i32, i32)>,
    query: &str,
    reset_scroll: bool,
) {
    let query = query.to_lowercase();
    let metadata = model.metadata();
    let metadata = metadata.borrow();
    let items: Vec<GridItem> = model
        .entries()
        .borrow()
        .iter()
        .filter(|wp| query.is_empty() || wp.file_name.to_lowercase().contains(&query))
        .map(|wp| {
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
            }
        })
        .collect();
    grid.set_items(items, reset_scroll);
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

pub fn wire_activate(grid: &MasonryGrid, model: &GalleryModel) {
    let config = model.config();
    let entries = model.entries();
    grid.connect_item_activated(move |item| {
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
    /* Flat dropdown indicator: no chip, border, or extra padding around the arrow. */
    row.combo image.dropdown-arrow,
    row.combo .suffixes image,
    dropdown arrow,
    dropdown > box > button.toggle,
    dropdown > box > button.toggle:hover,
    dropdown > box > button.toggle:active,
    dropdown > box > button.toggle:checked {
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      outline: none;
      padding: 0;
      min-height: 0;
      min-width: 0;
    }
    row.combo image.dropdown-arrow {
      margin: 0 2px;
    }
    "#
}
