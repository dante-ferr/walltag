use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    gio, glib, Align, Box, Label, Orientation, Overlay, PolicyType, ScrolledWindow, SearchEntry,
    Spinner,
};

use crate::backends::ApplyOptions;
use crate::metadata::MetadataStore;
use crate::scan::scan_wallpapers;
use crate::scan::WallpaperEntry;
use crate::ui::color::probe_colors;
use crate::ui::color_cache::{persist_color_samples, plan_and_prune, ColorCache};
use crate::ui::dim_cache::{resolve_dimensions, DimensionCache};
use crate::ui::filter::{build_rows, GalleryFilter, GalleryRow, SortBy, SortData};
use crate::ui::gallery_scroll::{install_gallery_scroller, GalleryScroller};
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
        let filter = {
            let cfg = config.borrow();
            GalleryFilter {
                query: cfg.search_query.clone(),
                group_by: cfg.group_by.clone(),
                sort_by: cfg.sort_by,
                tag: cfg.filter_tag.clone(),
                folders: cfg.filter_folders.clone(),
            }
        };
        Self {
            entries: Rc::new(RefCell::new(Vec::new())),
            metadata: Rc::new(RefCell::new(metadata)),
            config,
            filter: Rc::new(RefCell::new(filter)),
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

    /// Fills hue colors from the disk cache, then samples whatever is still missing.
    /// Returns `true` when the in-memory colors changed. Results are dropped when
    /// `generation` no longer matches `current` (the folder was rescanned meanwhile).
    async fn load_colors(&self, current: &Cell<u64>, generation: u64) -> bool {
        if self.filter.borrow().sort_by != SortBy::Hue {
            return false;
        }
        if self.sort_data.borrow().stats.is_empty() {
            return false;
        }
        let root = self.config.borrow().wallpaper_root.clone();
        let items: Vec<_> = self
            .entries
            .borrow()
            .iter()
            .map(|e| (e.relative_key.clone(), e.absolute.clone()))
            .collect();
        let stats = self.sort_data.borrow().stats.clone();
        let root_for_plan = root.clone();
        let Ok(plan) =
            gio::spawn_blocking(move || plan_and_prune(&root_for_plan, &items, &stats)).await
        else {
            return false;
        };
        if current.get() != generation {
            return false;
        }
        let misses = plan.misses.clone();
        let changed = {
            let data = self.sort_data.borrow();
            data.colors != plan.colors || data.unreadable != plan.unreadable
        };
        {
            let mut data = self.sort_data.borrow_mut();
            data.colors = plan.colors;
            data.unreadable = plan.unreadable;
        }
        if misses.is_empty() {
            return changed;
        }
        let input = misses.clone();
        let Ok(found) = gio::spawn_blocking(move || probe_colors(input)).await else {
            return changed;
        };
        if current.get() != generation {
            return false;
        }
        self.sort_data
            .borrow_mut()
            .record_colors(&misses, found.clone());
        let stats = self.sort_data.borrow().stats.clone();
        let found_for_save = found;
        let requested = misses;
        let _ = gio::spawn_blocking(move || {
            persist_color_samples(&root, &stats, &requested, &found_for_save);
        })
        .await;
        true
    }
}

pub struct GalleryView {
    pub root: Box,
    /// Not placed in the gallery; the sidebar hosts it.
    pub search: SearchEntry,
    pub grid: MasonryGrid,
    scroller: GalleryScroller,
    /// Centered over the gallery while scans or hue sampling run.
    spinner: Spinner,
    model: GalleryModel,
    dimensions: Dimensions,
    generation: Rc<Cell<u64>>,
    busy: Rc<Cell<u32>>,
    /// Invoked after the visible set changes so the sidebar can refresh filter lists.
    on_repopulated: Rc<RefCell<Vec<Rc<dyn Fn()>>>>,
}

impl GalleryView {
    pub fn new(model: GalleryModel) -> Self {
        let root = Box::new(Orientation::Vertical, 0);
        root.set_margin_top(8);
        root.set_margin_bottom(8);
        root.set_margin_start(8);
        root.set_margin_end(8);
        root.set_vexpand(true);
        root.add_css_class("walltag-gallery-root");

        let search = SearchEntry::new();
        search.set_placeholder_text(Some("Search by filename…"));
        search.set_hexpand(true);

        let spinner = Spinner::new();
        spinner.set_tooltip_text(Some("Loading…"));
        spinner.add_css_class("walltag-loading");
        spinner.set_halign(Align::Center);
        spinner.set_valign(Align::Center);
        // Clicks pass through so blank-space clear still works under the wheel.
        spinner.set_can_target(false);
        spinner.set_visible(false);

        let scroll = ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(PolicyType::Never)
            .kinetic_scrolling(true)
            .build();
        let grid = MasonryGrid::new(&scroll.vadjustment());
        scroll.set_child(Some(&grid));
        let scroller = install_gallery_scroller(&scroll);

        let overlay = Overlay::new();
        overlay.set_vexpand(true);
        overlay.set_hexpand(true);
        overlay.set_child(Some(&scroll));
        overlay.add_overlay(&spinner);
        root.append(&overlay);

        let view = Self {
            root,
            search,
            grid,
            scroller,
            spinner,
            model,
            dimensions: Rc::new(RefCell::new(HashMap::new())),
            generation: Rc::new(Cell::new(0)),
            busy: Rc::new(Cell::new(0)),
            on_repopulated: Rc::new(RefCell::new(Vec::new())),
        };
        view.wire_filters();
        view
    }

    fn set_busy(&self, busy: bool) {
        let n = self.busy.get();
        let next = if busy {
            n.saturating_add(1)
        } else {
            n.saturating_sub(1)
        };
        self.busy.set(next);
        if next > 0 {
            self.spinner.set_visible(true);
            self.spinner.start();
        } else {
            self.spinner.stop();
            self.spinner.set_visible(false);
        }
    }

    pub fn connect_repopulated<F: Fn() + 'static>(&self, f: F) {
        self.on_repopulated.borrow_mut().push(Rc::new(f));
    }

    pub fn scroller(&self) -> &GalleryScroller {
        &self.scroller
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

        let needs_colors = self.model.filter.borrow().sort_by == SortBy::Hue
            && !self.model.sort_data.borrow().stats.is_empty()
            && !self
                .model
                .sort_data
                .borrow()
                .missing_colors(&self.model.entries.borrow())
                .is_empty();
        if !needs_colors {
            return;
        }

        self.set_busy(true);
        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let current = self.generation.clone();
        let generation = current.get();
        let on_repopulated = self.on_repopulated.clone();
        let busy = self.busy.clone();
        let spinner = self.spinner.clone();
        glib::spawn_future_local(async move {
            let updated = model.load_colors(&current, generation).await;
            end_busy(&busy, &spinner);
            if !updated {
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
        // Restore the saved query without firing a redundant first refresh.
        let saved = model.filter.borrow().query.clone();
        if !saved.is_empty() {
            self.search.set_text(&saved);
        }
        self.search.connect_search_changed(move |entry| {
            let query = entry.text().to_string();
            model.filter.borrow_mut().query = query.clone();
            {
                let config = model.config();
                let mut cfg = config.borrow_mut();
                if cfg.search_query != query {
                    cfg.search_query = query;
                    if let Err(err) = crate::ConfigPaths::save(&cfg) {
                        tracing::error!("Failed to save config: {err:#}");
                    }
                }
            }
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
    /// Replaces gallery contents. When `reset_browse` is true (new wallpaper root), clears
    /// and persists the search/tag/folder filters; otherwise keeps the saved Browse state.
    pub fn reload_from_scan(&self, entries: Vec<WallpaperEntry>, reset_browse: bool) {
        self.generation.set(self.generation.get() + 1);
        let generation = self.generation.get();

        let root = self.model.config().borrow().wallpaper_root.clone();
        let probe_input: Vec<_> = entries
            .iter()
            .map(|e| (e.relative_key.clone(), e.absolute.clone()))
            .collect();
        if reset_browse {
            self.search.set_text("");
            {
                let mut filter = self.model.filter.borrow_mut();
                filter.query.clear();
                filter.tag = None;
                filter.folders.clear();
            }
            {
                let config = self.model.config();
                let mut cfg = config.borrow_mut();
                cfg.search_query.clear();
                cfg.filter_tag = None;
                cfg.filter_folders.clear();
                if let Err(err) = crate::ConfigPaths::save(&cfg) {
                    tracing::error!("Failed to save config: {err:#}");
                }
            }
        }
        self.model.set_entries(entries);
        *self.dimensions.borrow_mut() = DimensionCache::load(&root).unvalidated();
        if self.model.filter.borrow().sort_by == SortBy::Hue {
            let (colors, unreadable) = ColorCache::load(&root).unvalidated();
            let mut data = self.model.sort_data.borrow_mut();
            data.colors = colors;
            data.unreadable = unreadable;
        }
        populate(&self.grid, &self.model, &self.dimensions.borrow(), true);
        self.notify_repopulated();

        self.set_busy(true);
        let grid = self.grid.clone();
        let model = self.model.clone();
        let dimensions = self.dimensions.clone();
        let current = self.generation.clone();
        let on_repopulated = self.on_repopulated.clone();
        let busy = self.busy.clone();
        let spinner = self.spinner.clone();
        glib::spawn_future_local(async move {
            let root_for_probe = root.clone();
            let Ok((found, stats)) =
                gio::spawn_blocking(move || resolve_dimensions(&root_for_probe, &probe_input))
                    .await
            else {
                end_busy(&busy, &spinner);
                return;
            };
            if current.get() != generation {
                end_busy(&busy, &spinner);
                return;
            }
            *dimensions.borrow_mut() = found;
            model.sort_data().borrow_mut().replace_stats(stats);
            if populate(&grid, &model, &dimensions.borrow(), false) {
                for cb in on_repopulated.borrow().iter() {
                    cb();
                }
            }
            let colors_updated = model.load_colors(&current, generation).await;
            end_busy(&busy, &spinner);
            if !colors_updated || current.get() != generation {
                return;
            }
            if populate(&grid, &model, &dimensions.borrow(), false) {
                for cb in on_repopulated.borrow().iter() {
                    cb();
                }
            }
        });
    }

    /// Rescans the wallpaper root and reloads the gallery, keeping Browse filters.
    /// New files are sampled for the hue sort; unchanged files reuse the color cache.
    pub fn rescan(view: &Rc<Self>) {
        view.clear_empty_banner();
        view.set_busy(true);
        let view = Rc::clone(view);
        let busy = view.busy.clone();
        let spinner = view.spinner.clone();
        let root = view.model.config().borrow().wallpaper_root.clone();
        glib::spawn_future_local(async move {
            let scanned = gio::spawn_blocking(move || scan_wallpapers(&root)).await;
            let entries = match scanned {
                Ok(Ok(entries)) => entries,
                Ok(Err(err)) => {
                    tracing::warn!("Scan failed: {err:#}");
                    Vec::new()
                }
                Err(_) => {
                    end_busy(&busy, &spinner);
                    return;
                }
            };
            view.reload_from_scan(entries, false);
            end_busy(&busy, &spinner);
            view.show_empty_banner(&view.model.config().borrow());
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
                let tooltip = item_tooltip(&wp.file_name, &record.tags, record.favorite);
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

fn end_busy(busy: &Cell<u32>, spinner: &Spinner) {
    let next = busy.get().saturating_sub(1);
    busy.set(next);
    if next == 0 {
        spinner.stop();
        spinner.set_visible(false);
    }
}

fn tooltip_text(file_name: &str, tags: &[String]) -> String {
    if tags.is_empty() {
        file_name.to_owned()
    } else {
        format!("{}\n{}", file_name, tags.join(" · "))
    }
}

fn item_tooltip(file_name: &str, tags: &[String], favorite: bool) -> String {
    let base = tooltip_text(file_name, tags);
    if favorite {
        format!("{base}\n★ Favorite")
    } else {
        base
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
    refresh_tooltips(grid, model, &metadata, keys);
    saved
}

/// Removes `tags` from the wallpapers in `keys`, saves metadata, and refreshes tooltips.
pub fn remove_tags(
    grid: &MasonryGrid,
    model: &GalleryModel,
    keys: &[String],
    tags: &[String],
) -> anyhow::Result<()> {
    let metadata = model.metadata();
    let mut metadata = metadata.borrow_mut();
    metadata.remove_tags(keys, tags);
    let saved = metadata.save();
    refresh_tooltips(grid, model, &metadata, keys);
    saved
}

/// Sets the favorite flag on `keys`, saves metadata, and refreshes tooltips.
pub fn set_favorite(
    grid: &MasonryGrid,
    model: &GalleryModel,
    keys: &[String],
    favorite: bool,
) -> anyhow::Result<()> {
    let metadata = model.metadata();
    let mut metadata = metadata.borrow_mut();
    metadata.set_favorite(keys, favorite);
    let saved = metadata.save();
    refresh_tooltips(grid, model, &metadata, keys);
    saved
}

fn refresh_tooltips(
    grid: &MasonryGrid,
    model: &GalleryModel,
    metadata: &crate::metadata::MetadataStore,
    keys: &[String],
) {
    let entries = model.entries();
    for entry in entries
        .borrow()
        .iter()
        .filter(|e| keys.contains(&e.relative_key))
    {
        let record = metadata.record_for_key(&entry.relative_key);
        grid.update_tooltip(
            &entry.relative_key,
            item_tooltip(&entry.file_name, &record.tags, record.favorite),
        );
    }
}

/// Applies a random wallpaper from the currently filtered set (ignoring section headers).
///
/// Selection is synchronous; the backend call and `post_command` run off the UI thread.
pub fn set_random_wallpaper(model: &GalleryModel) -> Option<String> {
    let metadata = model.metadata();
    let metadata = metadata.borrow();
    let filter = model.filter.borrow().clone();
    let rows = build_rows(
        &model.entries().borrow(),
        &metadata,
        &filter,
        &model.sort_data().borrow(),
    );
    // Deduplicate: multi-tag grouping repeats the same wallpaper under each tag.
    let mut seen = std::collections::HashSet::new();
    let candidates: Vec<_> = rows
        .into_iter()
        .filter_map(|r| match r {
            GalleryRow::Wallpaper(wp) => seen.insert(wp.relative_key.clone()).then_some(wp),
            GalleryRow::Header(_) => None,
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    let entry = candidates[seed % candidates.len()].clone();
    let cfg = model.config().borrow().clone();
    let options = ApplyOptions::from_config(&cfg);
    apply_wallpaper_async(
        cfg,
        entry.absolute.clone(),
        options,
        entry.relative_key.clone(),
    );
    Some(entry.relative_key)
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
        let cfg = config.borrow().clone();
        let options = ApplyOptions::from_config(&cfg);
        apply_wallpaper_async(cfg, entry.absolute, options, entry.relative_key);
    });
}

/// Runs the wallpaper backend, `post_command`, and state persist off the GTK main thread.
fn apply_wallpaper_async(
    cfg: AppConfig,
    image: std::path::PathBuf,
    options: ApplyOptions,
    relative_key: String,
) {
    let backend = cfg.backend;
    let monitor = options
        .monitor
        .clone()
        .unwrap_or_else(|| "all monitors".into());
    glib::spawn_future_local(async move {
        let result = gio::spawn_blocking(move || {
            crate::backends::apply_wallpaper(backend, &image, &options)?;
            Cli::persist_last_wallpaper(&cfg, &image)?;
            Ok::<_, anyhow::Error>(())
        })
        .await;
        match result {
            Ok(Ok(())) => tracing::info!(
                "Set wallpaper via {} on {}: {}",
                backend.as_str(),
                monitor,
                relative_key
            ),
            Ok(Err(err)) => tracing::error!("Failed to set wallpaper: {err:#}"),
            Err(_) => tracing::error!("Wallpaper apply task was cancelled"),
        }
    });
}

/// Returns CSS bundled with the app for gallery layout tweaks (Libadwaita remains primary).
pub fn default_app_css() -> &'static str {
    r#"
    .walltag-gallery-root {
      background: transparent;
    }
    .walltag-main-shell {
      --wt-s: 1;
    }
    .walltag-topbar {
      font-size: 14px;
      min-height: 0;
      padding: 0 0.143em;
    }
    .walltag-topbar windowhandle,
    .walltag-topbar headerbar {
      min-height: 2em;
      padding-top: 0;
      padding-bottom: 0;
    }
    .walltag-topbar-btn {
      min-width: 1.571em;
      min-height: 1.571em;
      padding: 0.071em;
    }
    .walltag-zoom-controls {
      border-spacing: 0;
    }
    .walltag-zoom-label {
      min-width: 1.714em;
      min-height: 1.571em;
      padding: 0 0.071em;
      font-size: 0.7em;
      font-weight: 600;
      opacity: 0.65;
      letter-spacing: 0.02em;
    }
    .walltag-loading {
      min-width: 2.5em;
      min-height: 2.5em;
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
    /* Sidebar shell: slight plane separation from the gallery. */
    .walltag-sidebar-shell {
      font-size: 14px;
      background-color: alpha(@window_bg_color, 1.0);
      background-image: linear-gradient(
        180deg,
        alpha(@window_fg_color, 0.035),
        alpha(@window_fg_color, 0.012) 40%,
        transparent 100%
      );
    }
    /* Sidebar chrome: inset groups, compact section titles and rows. */
    .walltag-sidebar {
      background: transparent;
      margin: 0.571em 0.714em 0.571em 0.857em;
      transition: margin 200ms ease;
    }
    .walltag-sidebar-edit {
      transition: opacity 200ms ease, margin 200ms ease;
    }
    .walltag-sidebar preferencesgroup > box {
      border-spacing: 0.429em;
      transition: border-spacing 200ms ease;
    }
    /* Section titles sit inset from the pane edge instead of flush left. */
    .walltag-sidebar preferencesgroup > box > box.header,
    .walltag-sidebar preferencesgroup > box > box:first-child {
      margin-left: 0.286em;
      margin-right: 0.286em;
      margin-bottom: 0.071em;
    }
    .walltag-sidebar preferencesgroup > box > box > label.heading {
      font-size: 0.72em;
      font-weight: 700;
      letter-spacing: 0.06em;
      text-transform: uppercase;
      color: alpha(@window_fg_color, 0.55);
      padding: 0.071em 0.286em 0 0.286em;
    }
    .walltag-sidebar preferencesgroup > box > box > label.subtitle,
    .walltag-sidebar preferencesgroup > box label.subtitle {
      font-size: 0.85em;
      color: alpha(@window_fg_color, 0.5);
      padding: 0 0.286em;
    }
    /* Soft card behind each preferences list. */
    .walltag-sidebar preferencesgroup > box > list,
    .walltag-sidebar preferencesgroup list.boxed-list {
      background-color: alpha(@window_fg_color, 0.045);
      border: 1px solid alpha(@window_fg_color, 0.06);
      border-radius: 0.857em;
      padding: 0.071em 0;
      margin-top: 0.071em;
      transition: margin 200ms ease, padding 200ms ease;
    }
    .walltag-sidebar row {
      min-height: 0;
      background: transparent;
      border-radius: 0.571em;
      margin: 0 0.286em;
      transition: opacity 180ms ease, margin 180ms ease;
    }
    .walltag-sidebar row:hover {
      background-color: alpha(@window_fg_color, 0.05);
    }
    .walltag-sidebar row > box.header {
      min-height: 2.286em;
      padding: 0 0.714em;
      border-spacing: 0.571em;
    }
    .walltag-sidebar row > box.header > box.title {
      margin: 0.214em 0;
    }
    .walltag-sidebar row > box.header > box.title > label.title {
      font-weight: 500;
    }
    .walltag-sidebar row.entry {
      min-height: 0;
    }
    .walltag-sidebar row.entry > box.header {
      padding: 0 0.714em;
    }
    .walltag-sidebar row.spin > box.header {
      min-height: 2.286em;
    }
    .walltag-sidebar row > box.header > box.suffixes button.pill {
      min-height: 0;
      padding: 0.214em 0.857em;
    }
    .walltag-sidebar row > box.header > box.suffixes entry {
      min-height: 0;
      padding-top: 0.214em;
      padding-bottom: 0.214em;
    }
    .walltag-sidebar preferencesgroup.walltag-shared-tags > box {
      border-spacing: 0.071em;
    }
    .walltag-sidebar preferencesgroup.walltag-shared-tags row {
      min-height: 0;
    }
    .walltag-sidebar preferencesgroup.walltag-shared-tags row > box.header {
      min-height: 1.714em;
      padding-top: 0;
      padding-bottom: 0;
    }
    .walltag-sidebar preferencesgroup.walltag-shared-tags row > box.header > box.title {
      margin-top: 0;
      margin-bottom: 0;
    }
    .walltag-sidebar preferencesgroup.walltag-shared-tags row > box.header > box.suffixes button {
      min-height: 0;
      min-width: 0;
      padding: 0.143em;
    }
    /* Closed combo value: the inline list, not the menu, never gets a hover chip.
       The menu listview is also under box.suffixes, so this must stay on .inline. */
    row.combo > box.header > box.suffixes listview.inline,
    row.combo > box.header > box.suffixes listview.inline > row,
    row.combo > box.header > box.suffixes listview.inline > row:hover,
    row.combo > box.header > box.suffixes listview.inline > row:active,
    row.combo > box.header > box.suffixes listview.inline > row:focus {
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      outline: none;
      min-height: 0;
      padding: 0 0.143em;
    }
    row.combo > box.header > box.suffixes image.dropdown-arrow,
    row.combo > box.header > box.suffixes image.dropdown-arrow:hover {
      -gtk-icon-size: 1.429em;
      background: none;
      background-color: transparent;
      box-shadow: none;
      border: none;
      padding: 0;
      margin: 0 0.143em;
      min-width: 1.429em;
      min-height: 1.429em;
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
      padding: 0.214em;
      border-radius: 0.714em;
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
      border-radius: 0.429em;
    }
    row.combo popover.menu > contents listview > row > box {
      padding: 0.357em 0.714em;
      min-height: 0;
    }
    row.combo popover.menu > contents listview > row:hover,
    row.combo popover.menu > contents listview > row:selected {
      background-color: color-mix(in srgb, currentColor 10%, transparent);
    }
    row.combo popover.menu > contents listview > row:hover:selected,
    row.combo popover.menu > contents listview > row:active {
      background-color: color-mix(in srgb, currentColor 19%, transparent);
    }
    /* walltag-css-end */
    "#
}
