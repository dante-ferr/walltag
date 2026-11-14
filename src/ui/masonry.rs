//! Aspect-preserving column masonry with a uniform gap between tiles and virtualized tiles.
//!
//! Layout is pure integer math (`compute_layout`). The widget only keeps `Picture`
//! children for tiles near the viewport and recycles them while scrolling.
//!
//! Layout changes (items, zoom, file names, column count) animate for `ANIM_DURATION_MS`
//! with ease-out cubic. Tiles are keyed by item key. Kept tiles interpolate their rectangle,
//! new tiles fade in with widget opacity, and removed tiles fade out and are then recycled.
//! Hit testing, tooltips, and drag selection always use the final layout; only drawing
//! (tiles and selection tint) uses the interpolated rectangles.

use std::path::PathBuf;
use std::rc::Rc;

/// Preferred column width in logical pixels at 100% zoom; the column count follows the
/// widget width.
const TARGET_COLUMN_WIDTH: f32 = 420.0;
/// Zoom limits and step. Zoom multiplies `TARGET_COLUMN_WIDTH`: larger zoom, larger tiles.
pub const MIN_ZOOM: f32 = 0.4;
pub const MAX_ZOOM: f32 = 3.0;
const ZOOM_STEP: f32 = 1.25;
pub const DEFAULT_ZOOM: f32 = 1.0;

/// Returns the zoom after one step in or out, clamped to `MIN_ZOOM..=MAX_ZOOM`. A result
/// within 1% of 100% snaps to exactly 100% so zooming in and out returns to the default.
pub fn step_zoom(zoom: f32, zoom_in: bool) -> f32 {
    let next = if zoom_in {
        zoom * ZOOM_STEP
    } else {
        zoom / ZOOM_STEP
    };
    let next = next.clamp(MIN_ZOOM, MAX_ZOOM);
    if (next - DEFAULT_ZOOM).abs() < 0.01 {
        DEFAULT_ZOOM
    } else {
        next
    }
}
/// Gap in logical pixels between neighboring tiles, horizontally and vertically.
pub const TILE_GAP: i32 = 10;
const MIN_WIDGET_WIDTH: i32 = 120;
/// Aspect ratio (width / height) used when a file's size could not be read.
pub const FALLBACK_ASPECT: f32 = 16.0 / 9.0;

/// Logical height of a group section header row.
pub const SECTION_HEADER_HEIGHT: i32 = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct GridItem {
    pub key: String,
    pub path: PathBuf,
    /// Width divided by height. Ignored when [`Self::section_title`] is set.
    pub aspect: f32,
    /// Source width in pixels, or 0 if unknown. Caps thumbnail size.
    pub natural_width: i32,
    pub tooltip: String,
    /// Shown under the tile when file names are enabled.
    pub file_name: String,
    /// When set, this row is a full-width group header (not a wallpaper tile).
    pub section_title: Option<String>,
}

impl GridItem {
    pub fn is_header(&self) -> bool {
        self.section_title.is_some()
    }

    pub fn header(title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            key: format!("header:{title}"),
            path: PathBuf::new(),
            aspect: 1.0,
            natural_width: 0,
            tooltip: String::new(),
            file_name: String::new(),
            section_title: Some(title),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    fn bottom(&self) -> i32 {
        self.y + self.h
    }

    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x as f64
            && px < (self.x + self.w) as f64
            && py >= self.y as f64
            && py < self.bottom() as f64
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    /// Image area of each tile (or header band).
    pub rects: Vec<Rect>,
    /// Parallel to `rects`: true when the row is a section header.
    pub is_header: Vec<bool>,
    /// Height of the file name strip directly under each image; 0 when names are hidden.
    pub label_height: i32,
    /// Number of masonry columns.
    pub columns: usize,
}

impl Layout {
    /// Image plus label strip (headers have no label strip). Hit tests, selection, and
    /// virtualization use the cell.
    pub fn cell(&self, idx: usize) -> Option<Rect> {
        self.rects.get(idx).map(|r| {
            let extra = if self.is_header.get(idx).copied().unwrap_or(false) {
                0
            } else {
                self.label_height
            };
            Rect {
                h: r.h + extra,
                ..*r
            }
        })
    }

    pub fn is_header(&self, idx: usize) -> bool {
        self.is_header.get(idx).copied().unwrap_or(false)
    }

    /// Hit-test wallpaper tiles only (section headers are ignored).
    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        (0..self.rects.len())
            .find(|&i| !self.is_header(i) && self.cell(i).is_some_and(|c| c.contains(x, y)))
    }

    /// Indices of wallpaper tiles that overlap the rectangle spanned by two corner points.
    pub fn hit_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<usize> {
        let (left, right) = (x0.min(x1), x0.max(x1));
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        (0..self.rects.len())
            .filter(|&i| {
                if self.is_header(i) {
                    return false;
                }
                let r = self.cell(i).expect("index in range");
                (r.x as f64) < right
                    && ((r.x + r.w) as f64) > left
                    && (r.y as f64) < bottom
                    && (r.bottom() as f64) > top
            })
            .collect()
    }
}

/// Pointer travel (logical pixels) after which a press becomes a drag selection.
const DRAG_THRESHOLD: f64 = 6.0;
const BUTTON_PRIMARY: u32 = 1;
const BUTTON_SECONDARY: u32 = 3;

/// What a press-and-release without a drag does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    /// Set the wallpaper.
    Activate,
    /// Add the tile to the edit batch, or remove it if it is already there.
    Toggle,
    /// Select only this tile, clearing any other selected tiles.
    SelectOnly,
    /// Empty the edit batch.
    ClearSelection,
    Ignore,
}

/// Decides the effect of a click. The wallpaper is set only by a primary-button click on a
/// tile while the edit batch is empty. Secondary click selects only the tile unless Shift or
/// Control is held, in which case the tile is toggled in the existing batch.
pub fn click_action(
    button: u32,
    on_tile: bool,
    batch_empty: bool,
    extend_selection: bool,
) -> ClickAction {
    match (button, on_tile) {
        (BUTTON_SECONDARY, true) if extend_selection => ClickAction::Toggle,
        (BUTTON_SECONDARY, true) => ClickAction::SelectOnly,
        (BUTTON_PRIMARY, true) if batch_empty => ClickAction::Activate,
        (BUTTON_PRIMARY, true) => ClickAction::Toggle,
        (BUTTON_PRIMARY, false) if !batch_empty => ClickAction::ClearSelection,
        _ => ClickAction::Ignore,
    }
}

/// In-progress press on the grid.
#[derive(Debug, Clone, Copy)]
struct Drag {
    start: (f64, f64),
    current: (f64, f64),
    button: u32,
    active: bool,
    /// Shift or Control held at press time (secondary click adds to the batch).
    extend_selection: bool,
}

impl Drag {
    fn moved_past_threshold(&self) -> bool {
        let (dx, dy) = (self.current.0 - self.start.0, self.current.1 - self.start.1);
        dx * dx + dy * dy >= DRAG_THRESHOLD * DRAG_THRESHOLD
    }
}

/// Column count at 100% zoom.
#[cfg(test)]
pub fn columns_for_width(width: i32) -> usize {
    columns_for_width_zoomed(width, DEFAULT_ZOOM)
}

pub fn columns_for_width_zoomed(width: i32, zoom: f32) -> usize {
    let target = TARGET_COLUMN_WIDTH * zoom.clamp(MIN_ZOOM, MAX_ZOOM);
    ((width as f32 / target).round() as usize).max(1)
}

/// Places each item in the currently shortest column. Tile widths are whole pixels that,
/// together with `TILE_GAP` between columns, fill `width` exactly. Tiles in a column are
/// `TILE_GAP` apart.
#[cfg(test)]
pub fn compute_layout(aspects: &[f32], width: i32) -> Layout {
    compute_layout_zoomed(aspects, width, DEFAULT_ZOOM, 0)
}

/// Like `compute_layout`, with the target column width scaled by `zoom` and a
/// `label_height` strip reserved under every image (0 for none).
#[allow(dead_code)]
pub fn compute_layout_zoomed(aspects: &[f32], width: i32, zoom: f32, label_height: i32) -> Layout {
    let flags: Vec<bool> = aspects.iter().map(|_| false).collect();
    compute_layout_with_headers(aspects, &flags, width, zoom, label_height)
}

/// Masonry layout that supports full-width section headers. `is_header[i]` marks
/// item `i` as a header (aspect ignored); headers flush columns and span `width`.
pub fn compute_layout_with_headers(
    aspects: &[f32],
    is_header: &[bool],
    width: i32,
    zoom: f32,
    label_height: i32,
) -> Layout {
    assert_eq!(aspects.len(), is_header.len());
    let label_height = label_height.max(0);
    let width = width.max(1);
    let mut columns = columns_for_width_zoomed(width, zoom);
    // Keep every column at least one pixel wide.
    while columns > 1 && width - TILE_GAP * (columns as i32 - 1) < columns as i32 {
        columns -= 1;
    }
    let usable = width - TILE_GAP * (columns as i32 - 1);
    let edges: Vec<i32> = (0..=columns)
        .map(|i| (usable as i64 * i as i64 / columns as i64) as i32)
        .collect();
    let mut heights = vec![0i32; columns];
    let mut rects = Vec::with_capacity(aspects.len());

    for (i, aspect) in aspects.iter().enumerate() {
        if is_header.get(i).copied().unwrap_or(false) {
            // Flush columns so the header sits below the tallest masonry column.
            let y = heights.iter().copied().max().unwrap_or(0);
            // Drop the trailing gap above the header when columns already have content.
            let y = if y > TILE_GAP { y - TILE_GAP } else { y };
            let y = if y > 0 { y + TILE_GAP } else { y };
            rects.push(Rect {
                x: 0,
                y,
                w: width,
                h: SECTION_HEADER_HEIGHT,
            });
            let next_y = y + SECTION_HEADER_HEIGHT + TILE_GAP;
            for h in &mut heights {
                *h = next_y;
            }
            continue;
        }
        let col = heights
            .iter()
            .enumerate()
            .min_by_key(|(_, h)| **h)
            .map(|(i, _)| i)
            .unwrap_or(0);
        let w = edges[col + 1] - edges[col];
        let h = ((w as f32 / aspect.max(0.01)).round() as i32).max(1);
        rects.push(Rect {
            x: edges[col] + col as i32 * TILE_GAP,
            y: heights[col],
            w,
            h,
        });
        heights[col] += h + label_height + TILE_GAP;
    }

    // Each column carries one trailing gap; the widget has no bottom gap.
    let height = heights
        .into_iter()
        .max()
        .map_or(0, |h| (h - TILE_GAP).max(0));
    Layout {
        width,
        height,
        rects,
        is_header: is_header.to_vec(),
        label_height,
        columns,
    }
}

/// Duration of layout transitions (moves, fades) in milliseconds.
const ANIM_DURATION_MS: f64 = 280.0;

/// Ease-out cubic: fast start, gentle landing. Maps `0..=1` to `0..=1`.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Linear animation progress (`0..=1`) after `elapsed_us` microseconds.
fn progress(elapsed_us: i64) -> f32 {
    ((elapsed_us.max(0) as f64) / (ANIM_DURATION_MS * 1000.0)).min(1.0) as f32
}

/// Animated visual state of one tile: image rectangle, label strip height, tile opacity,
/// and label opacity (relative to the tile).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vis {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub lh: f32,
    pub a: f32,
    pub la: f32,
}

impl Vis {
    pub fn lerp(self, to: Vis, t: f32) -> Vis {
        let mix = |a: f32, b: f32| a + (b - a) * t;
        Vis {
            x: mix(self.x, to.x),
            y: mix(self.y, to.y),
            w: mix(self.w, to.w),
            h: mix(self.h, to.h),
            lh: mix(self.lh, to.lh),
            a: mix(self.a, to.a),
            la: mix(self.la, to.la),
        }
    }
}

/// How the scroll offset changes together with the content.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScrollMode {
    /// Keep the offset (clamped to the new content height).
    Keep,
    /// Jump to the top.
    Top,
    /// Keep the offset as a fraction of the content height.
    Relative(f64),
}

/// New scroll value for content of `height` pixels in a viewport of `page` pixels.
fn scroll_target(mode: ScrollMode, old_value: f64, _old_upper: f64, page: f64, height: i32) -> f64 {
    let upper = (height as f64).max(page);
    let max = (upper - page).max(0.0);
    let value = match mode {
        ScrollMode::Keep => old_value,
        ScrollMode::Top => 0.0,
        ScrollMode::Relative(r) => r * upper,
    };
    value.clamp(0.0, max)
}

/// Unique tile id per item, parallel to `items`. The first item with a key uses the key;
/// repeats (a wallpaper listed under several tag groups) get a numeric suffix.
pub fn item_ids(items: &[GridItem]) -> Vec<String> {
    let mut seen: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    items
        .iter()
        .map(|item| {
            let n = seen.entry(item.key.as_str()).or_insert(0);
            let id = if *n == 0 {
                item.key.clone()
            } else {
                format!("{}\u{1}{}", item.key, n)
            };
            *n += 1;
            id
        })
        .collect()
}

mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use gtk4::prelude::*;
    use gtk4::subclass::prelude::*;
    use gtk4::{
        gdk, glib, graphene, gsk, Adjustment, ContentFit, Label, Orientation, Picture,
        SizeRequestMode,
    };

    use super::*;
    use crate::ui::thumbs::Thumbnailer;

    /// Tiles within this many viewport heights above/below the view are realized.
    const REALIZE_PAD: f64 = 0.75;
    /// Tiles beyond this many viewport heights are recycled (hysteresis avoids thrash).
    const RECYCLE_PAD: f64 = 1.5;
    const MAX_POOL: usize = 64;

    /// Vertical padding added to the text height of a file name label.
    const LABEL_PAD: i32 = 6;

    /// Cached layout and ids that the tiles currently target.
    type Shown = (Rc<Vec<String>>, Rc<Layout>);

    /// One realized tile (or a tile that is fading out after it left the item set).
    struct Tile {
        /// Item key, used to look up the selection.
        key: String,
        picture: Option<Picture>,
        label: Option<Label>,
        cancel: Arc<AtomicBool>,
        /// Pixel width of the last thumbnail request.
        thumb_px: i32,
        /// Index in the final layout; `None` while the tile fades out.
        idx: Option<usize>,
        /// Animation start, end, and current (drawn) state.
        from: Vis,
        to: Vis,
        cur: Vis,
    }

    impl Tile {
        fn is_header(&self) -> bool {
            self.picture.is_none()
        }

        fn step(&mut self, eased: f32) {
            self.cur = self.from.lerp(self.to, eased);
        }

        /// Pushes the current opacity to the widgets.
        fn apply(&self) {
            let c = self.cur;
            if let Some(picture) = &self.picture {
                picture.set_opacity(c.a as f64);
            }
            if let Some(label) = &self.label {
                let factor = if self.is_header() { 1.0 } else { c.la };
                label.set_opacity((c.a * factor) as f64);
            }
        }
    }

    /// Data that tiles realized mid-animation need to join the running transition.
    struct Transition {
        /// Visual state of every item in the previous layout, already shifted by the
        /// scroll change so the screen position is continuous.
        prev: HashMap<String, Vis>,
    }

    #[derive(Default)]
    pub struct MasonryGrid {
        pub items: RefCell<Rc<Vec<GridItem>>>,
        /// Parallel to `items`: unique tile ids (duplicate keys get a suffix).
        ids: RefCell<Rc<Vec<String>>>,
        /// Final layout. Hit testing, tooltips, and drag selection use this layout, not
        /// the interpolated tile positions, so a click lands where the tile settles.
        pub layout: RefCell<Option<Rc<Layout>>>,
        tiles: RefCell<HashMap<String, Tile>>,
        leaving: RefCell<Vec<Tile>>,
        shown: RefCell<Option<Shown>>,
        placed_width: Cell<i32>,
        transition: RefCell<Option<Transition>>,
        anim_start: Cell<Option<i64>>,
        anim_e: Cell<f32>,
        ticking: Cell<bool>,
        pool: RefCell<Vec<Picture>>,
        label_pool: RefCell<Vec<Label>>,
        pub show_names: Cell<bool>,
        pub vadj: RefCell<Option<Adjustment>>,
        pub thumbs: Thumbnailer,
        pub on_activate: RefCell<Option<Rc<dyn Fn(&GridItem)>>>,
        /// Keys of the wallpapers in the edit batch.
        pub selected: RefCell<HashSet<String>>,
        pub on_selection: RefCell<Vec<Rc<dyn Fn()>>>,
        pub on_zoom: RefCell<Vec<Rc<dyn Fn(f32)>>>,
        pub(super) drag: Cell<Option<Drag>>,
        sync_queued: Cell<bool>,
        pub zoom: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MasonryGrid {
        const NAME: &'static str = "WalltagMasonryGrid";
        type Type = super::MasonryGrid;
        type ParentType = gtk4::Widget;
    }

    impl ObjectImpl for MasonryGrid {
        fn dispose(&self) {
            let obj = self.obj();
            while let Some(child) = obj.first_child() {
                child.unparent();
            }
            self.tiles.borrow_mut().clear();
            self.leaving.borrow_mut().clear();
            self.pool.borrow_mut().clear();
            self.label_pool.borrow_mut().clear();
        }
    }

    impl WidgetImpl for MasonryGrid {
        fn request_mode(&self) -> SizeRequestMode {
            SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            match orientation {
                // Minimum is 0 so a narrow window cannot force this width onto the
                // sidebar. Natural width stays at the comfortable column size.
                Orientation::Horizontal => (0, MIN_WIDGET_WIDTH, -1, -1),
                _ => {
                    let width = if for_size > 0 {
                        for_size
                    } else {
                        self.obj().width().max(MIN_WIDGET_WIDTH)
                    };
                    let height = self.layout_for(width).height;
                    (height, height, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            self.layout_for(width);
            if self.placed_width.get() != width {
                self.on_width_changed(width);
            }
            for tile in self.tiles.borrow().values() {
                place(tile);
            }
            for tile in self.leaving.borrow().iter() {
                place(tile);
            }
            // Realize/recycle after allocation so we never reparent during allocate.
            self.queue_sync();
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            self.parent_snapshot(snapshot);
            self.snapshot_overlays(snapshot);
        }
    }

    /// Allocates a tile at its current (interpolated) state.
    fn place(tile: &Tile) {
        let c = tile.cur;
        let (x0, x1) = (c.x.round() as i32, (c.x + c.w).round() as i32);
        let (y0, y1) = (c.y.round() as i32, (c.y + c.h).round() as i32);
        let (w, h) = ((x1 - x0).max(1), (y1 - y0).max(1));
        match (&tile.picture, &tile.label) {
            (None, Some(label)) => {
                label.size_allocate(&gtk4::Allocation::new(x0, y0, w, h), -1);
            }
            (picture, label) => {
                if let Some(picture) = picture {
                    picture.size_allocate(&gtk4::Allocation::new(x0, y0, w, h), -1);
                }
                if let Some(label) = label {
                    // Keep the strip at its larger size while it grows or shrinks.
                    let lh = (tile.from.lh.max(tile.to.lh).round() as i32).max(1);
                    label.size_allocate(&gtk4::Allocation::new(x0, y1, w, lh), -1);
                }
            }
        }
    }

    fn overlaps(top: f64, bottom: f64, (a, b): (f64, f64)) -> bool {
        bottom > a && top < b
    }

    impl MasonryGrid {
        /// Draws selection tint and border over the tiles (inside each tile's bounds, so
        /// the layout stays gapless), then the drag rectangle. Tints follow the animated
        /// tile positions; the drag preview is computed on the final layout.
        fn snapshot_overlays(&self, snapshot: &gtk4::Snapshot) {
            let drag = self.drag.get().filter(|d| d.active);
            let preview: HashSet<usize> = match (drag, self.layout.borrow().as_ref()) {
                (Some(d), Some(layout)) => layout
                    .hit_rect(d.start.0, d.start.1, d.current.0, d.current.1)
                    .into_iter()
                    .collect(),
                _ => HashSet::new(),
            };
            let accent = self.accent();
            let with_alpha =
                |alpha: f32| gdk::RGBA::new(accent.red(), accent.green(), accent.blue(), alpha);

            if !self.selected.borrow().is_empty() || !preview.is_empty() {
                let selected = self.selected.borrow();
                for tile in self.tiles.borrow().values() {
                    let Some(idx) = tile.idx else { continue };
                    if tile.is_header() {
                        continue;
                    }
                    let is_selected = selected.contains(&tile.key);
                    if !is_selected && !preview.contains(&idx) {
                        continue;
                    }
                    let c = tile.cur;
                    let fade = c.a;
                    let rect = graphene::Rect::new(c.x, c.y, c.w, c.h + c.lh);
                    snapshot.append_color(
                        &with_alpha(if is_selected { 0.30 } else { 0.18 } * fade),
                        &rect,
                    );
                    if is_selected {
                        let border = [with_alpha(fade); 4];
                        snapshot.append_border(
                            &gsk::RoundedRect::from_rect(rect, 0.0),
                            &[3.0; 4],
                            &border,
                        );
                    }
                }
            }

            if let Some(d) = drag {
                let (x, y) = (d.start.0.min(d.current.0), d.start.1.min(d.current.1));
                let (w, h) = (
                    (d.start.0 - d.current.0).abs(),
                    (d.start.1 - d.current.1).abs(),
                );
                let rect = graphene::Rect::new(x as f32, y as f32, w as f32, h as f32);
                snapshot.append_color(&with_alpha(0.15), &rect);
                snapshot.append_border(
                    &gsk::RoundedRect::from_rect(rect, 0.0),
                    &[1.5; 4],
                    &[with_alpha(0.9); 4],
                );
            }
        }

        #[allow(deprecated)]
        fn accent(&self) -> gdk::RGBA {
            self.obj()
                .style_context()
                .lookup_color("accent_bg_color")
                .unwrap_or_else(|| gdk::RGBA::new(0.21, 0.52, 0.89, 1.0))
        }

        pub fn zoom(&self) -> f32 {
            // `Cell` default is 0.0 before `MasonryGrid::new` sets it.
            if self.zoom.get() > 0.0 {
                self.zoom.get()
            } else {
                DEFAULT_ZOOM
            }
        }

        /// Height of the file name strip, from the current font; 0 when names are hidden.
        pub fn label_height(&self) -> i32 {
            if !self.show_names.get() {
                return 0;
            }
            let (_, text_height) = self.obj().create_pango_layout(Some("Ag")).pixel_size();
            text_height + LABEL_PAD
        }

        pub fn layout_for(&self, width: i32) -> Rc<Layout> {
            if let Some(layout) = self.layout.borrow().as_ref() {
                if layout.width == width.max(1) {
                    return layout.clone();
                }
            }
            let items = self.items.borrow();
            let aspects: Vec<f32> = items.iter().map(|i| i.aspect).collect();
            let headers: Vec<bool> = items.iter().map(|i| i.is_header()).collect();
            let layout = Rc::new(compute_layout_with_headers(
                &aspects,
                &headers,
                width,
                self.zoom(),
                self.label_height(),
            ));
            drop(items);
            *self.layout.borrow_mut() = Some(layout.clone());
            layout
        }

        /// Replaces the items. Tiles whose key survives move to their new place, new keys
        /// fade in, and removed keys fade out.
        pub fn set_items(&self, items: Vec<GridItem>, reset_scroll: bool) {
            *self.ids.borrow_mut() = Rc::new(item_ids(&items));
            *self.items.borrow_mut() = Rc::new(items);
            self.transition(if reset_scroll {
                ScrollMode::Top
            } else {
                ScrollMode::Keep
            });
        }

        /// Recomputes the layout after items, zoom, or label visibility changed and animates
        /// the tiles to it. The scroll offset follows `mode` and is applied together with
        /// the new content height, so tile positions stay continuous on screen.
        ///
        /// A jump to the top from a scrolled position cross-fades instead of sliding, since
        /// the old and new positions would be unrelated on screen.
        pub fn transition(&self, mode: ScrollMode) {
            let obj = self.obj();
            let width = obj.width();
            *self.layout.borrow_mut() = None;
            let adj = self.vadj.borrow().clone();
            if width <= 0 {
                // Not allocated yet: the first allocation places everything instantly.
                self.release_all();
                *self.shown.borrow_mut() = None;
                if let (ScrollMode::Top, Some(adj)) = (mode, &adj) {
                    adj.set_value(0.0);
                }
                obj.queue_resize();
                self.queue_sync();
                return;
            }
            let layout = self.layout_for(width);
            let (old_value, old_upper, page) = adj.as_ref().map_or((0.0, 0.0, 1.0), |a| {
                (a.value(), a.upper(), a.page_size().max(1.0))
            });
            let upper = (layout.height as f64).max(page);
            let value = scroll_target(mode, old_value, old_upper, page, layout.height);
            let moves = !(matches!(mode, ScrollMode::Top) && old_value > 0.5);
            self.retarget(width, true, moves, (value - old_value) as f32);
            if let Some(adj) = adj {
                // Set the new range ourselves; the viewport only does so after the next
                // allocation, which would show one frame at the wrong offset.
                adj.configure(
                    value,
                    adj.lower(),
                    upper,
                    adj.step_increment(),
                    adj.page_increment(),
                    page,
                );
            }
            self.sync();
            obj.queue_resize();
        }

        /// Called from `size_allocate` when the widget width changed. Only a column count
        /// change (or an animation already running) animates; plain resizing follows the
        /// window directly.
        fn on_width_changed(&self, width: i32) {
            let layout = self.layout_for(width);
            let columns_changed = self
                .shown
                .borrow()
                .as_ref()
                .is_some_and(|(_, old)| old.columns != layout.columns);
            let animate = columns_changed || self.transition.borrow().is_some();
            self.retarget(width, animate, true, 0.0);
        }

        fn animations_enabled(&self) -> bool {
            gtk4::Settings::default().map_or(true, |s| s.is_gtk_enable_animations())
        }

        /// Visual end state of item `idx` in `layout`.
        fn target_vis(&self, layout: &Layout, idx: usize) -> Vis {
            let r = layout.rects[idx];
            let header = layout.is_header(idx);
            Vis {
                x: r.x as f32,
                y: r.y as f32,
                w: r.w as f32,
                h: r.h as f32,
                lh: if header {
                    0.0
                } else {
                    layout.label_height as f32
                },
                a: 1.0,
                la: if header || self.show_names.get() {
                    1.0
                } else {
                    0.0
                },
            }
        }

        /// Points every realized tile at its place in the current layout. With `animate`
        /// the tiles start from where they are drawn now (plus `shift`, the scroll change);
        /// otherwise they jump. With `moves` false every tile fades out and the new ones
        /// fade in instead of sliding.
        fn retarget(&self, width: i32, animate: bool, moves: bool, shift: f32) {
            let layout = self.layout_for(width);
            let ids = self.ids.borrow().clone();
            let items = self.items.borrow().clone();
            let old = self.shown.borrow().clone();
            self.placed_width.set(width);

            let unchanged = old.as_ref().is_some_and(|(old_ids, old_layout)| {
                **old_ids == *ids
                    && old_layout.rects == layout.rects
                    && old_layout.is_header == layout.is_header
                    && old_layout.label_height == layout.label_height
            });
            *self.shown.borrow_mut() = Some((ids.clone(), layout.clone()));
            if unchanged {
                return;
            }

            let animate = animate
                && self.animations_enabled()
                && old.as_ref().is_some_and(|(old_ids, _)| !old_ids.is_empty());
            if !animate {
                self.finish_animation();
            }

            let show = self.show_names.get();
            let index: HashMap<&str, usize> = if animate && !moves {
                HashMap::new()
            } else {
                ids.iter()
                    .enumerate()
                    .map(|(i, id)| (id.as_str(), i))
                    .collect()
            };

            // Tiles whose item is gone.
            let gone: Vec<String> = self
                .tiles
                .borrow()
                .keys()
                .filter(|id| !index.contains_key(id.as_str()))
                .cloned()
                .collect();
            for id in gone {
                let Some(mut tile) = self.tiles.borrow_mut().remove(&id) else {
                    continue;
                };
                if animate {
                    tile.idx = None;
                    tile.cur.y += shift;
                    tile.from = tile.cur;
                    tile.to = Vis { a: 0.0, ..tile.cur };
                    self.leaving.borrow_mut().push(tile);
                } else {
                    self.release_tile(tile);
                }
            }
            for tile in self.leaving.borrow_mut().iter_mut() {
                tile.cur.y += shift;
                tile.from = tile.cur;
                tile.to = Vis { a: 0.0, ..tile.cur };
            }

            // Tiles that stay.
            let scale = self.obj().scale_factor();
            for (id, tile) in self.tiles.borrow_mut().iter_mut() {
                let idx = index[id.as_str()];
                let item = &items[idx];
                tile.idx = Some(idx);
                let mut to = self.target_vis(&layout, idx);
                if !tile.is_header() {
                    if show && tile.label.is_none() {
                        tile.label = Some(self.new_label(&item.file_name));
                        tile.cur.la = if animate { 0.0 } else { 1.0 };
                    }
                    if !show {
                        to.la = 0.0;
                        if !animate {
                            self.drop_label(tile);
                        }
                    }
                    // Keep the old thumbnail until a sharper one arrives.
                    let needed = to.w as i32 * scale;
                    if needed > tile.thumb_px {
                        self.request_thumb(tile, item, needed);
                    }
                }
                if animate {
                    tile.cur.y += shift;
                    tile.from = tile.cur;
                } else {
                    tile.from = to;
                    tile.cur = to;
                }
                tile.to = to;
                tile.apply();
            }

            if animate {
                let prev = if moves {
                    old.map_or_else(HashMap::new, |(old_ids, old_layout)| {
                        old_ids
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| *i < old_layout.rects.len())
                            .map(|(i, id)| {
                                let mut v = self.target_vis(&old_layout, i);
                                v.y += shift;
                                v.la = if old_layout.label_height > 0 || old_layout.is_header(i) {
                                    1.0
                                } else {
                                    0.0
                                };
                                (id.clone(), v)
                            })
                            .collect()
                    })
                } else {
                    HashMap::new()
                };
                *self.transition.borrow_mut() = Some(Transition { prev });
                self.start_animation();
            }
        }

        fn start_animation(&self) {
            self.anim_start.set(None);
            self.anim_e.set(0.0);
            if self.ticking.replace(true) {
                return;
            }
            let weak = self.obj().downgrade();
            self.obj()
                .add_tick_callback(move |_, clock| match weak.upgrade() {
                    Some(obj) => obj.imp().tick(clock.frame_time()),
                    None => glib::ControlFlow::Break,
                });
        }

        fn tick(&self, now: i64) -> glib::ControlFlow {
            if self.transition.borrow().is_none() {
                self.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            let start = self.anim_start.get().unwrap_or_else(|| {
                self.anim_start.set(Some(now));
                now
            });
            let t = progress(now - start);
            let eased = ease_out_cubic(t);
            self.anim_e.set(eased);
            for tile in self.tiles.borrow_mut().values_mut() {
                tile.step(eased);
                tile.apply();
            }
            for tile in self.leaving.borrow_mut().iter_mut() {
                tile.step(eased);
                tile.apply();
            }
            let obj = self.obj();
            obj.queue_allocate();
            obj.queue_draw();
            if t >= 1.0 {
                self.finish_animation();
                self.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        }

        /// Snaps every tile to its end state, removes tiles that faded out, and drops
        /// labels that faded out.
        fn finish_animation(&self) {
            let leaving: Vec<Tile> = self.leaving.borrow_mut().drain(..).collect();
            for tile in leaving {
                self.release_tile(tile);
            }
            for tile in self.tiles.borrow_mut().values_mut() {
                tile.from = tile.to;
                tile.cur = tile.to;
                if tile.to.la <= 0.0 && !tile.is_header() {
                    self.drop_label(tile);
                }
                tile.apply();
            }
            *self.transition.borrow_mut() = None;
            self.anim_e.set(1.0);
        }

        fn queue_sync(&self) {
            if self.sync_queued.replace(true) {
                return;
            }
            let obj = self.obj().clone();
            glib::idle_add_local_full(glib::Priority::HIGH, move || {
                let imp = obj.imp();
                imp.sync_queued.set(false);
                imp.sync();
                glib::ControlFlow::Break
            });
        }

        /// Realizes tiles near the viewport and recycles those far away. A tile realized
        /// while a transition runs joins it: it slides from its old position when the item
        /// existed before, and fades in otherwise.
        pub fn sync(&self) {
            let obj = self.obj();
            let width = obj.width();
            if width <= 0 {
                return;
            }
            let layout = self.layout_for(width);
            let (value, page) = match self.vadj.borrow().as_ref() {
                Some(adj) => (adj.value(), adj.page_size().max(1.0)),
                None => (0.0, 1.0),
            };

            let near = (
                value - page * REALIZE_PAD,
                value + page + page * REALIZE_PAD,
            );
            let far = (
                value - page * RECYCLE_PAD,
                value + page + page * RECYCLE_PAD,
            );

            // A tile stays while either its drawn or its final position is near the view.
            let stale: Vec<String> = self
                .tiles
                .borrow()
                .iter()
                .filter(|(_, t)| {
                    let drawn =
                        overlaps(t.cur.y as f64, (t.cur.y + t.cur.h + t.cur.lh) as f64, far);
                    let target = t
                        .idx
                        .and_then(|i| layout.cell(i))
                        .is_some_and(|r| overlaps(r.y as f64, r.bottom() as f64, far));
                    !(drawn || target)
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in stale {
                if let Some(tile) = self.tiles.borrow_mut().remove(&id) {
                    self.release_tile(tile);
                }
            }

            let items = self.items.borrow().clone();
            let ids = self.ids.borrow().clone();
            let mut added = false;
            for idx in 0..layout.rects.len() {
                let Some(cell) = layout.cell(idx) else {
                    continue;
                };
                if !overlaps(cell.y as f64, cell.bottom() as f64, near) {
                    continue;
                }
                let (Some(item), Some(id)) = (items.get(idx), ids.get(idx)) else {
                    continue;
                };
                if !self.tiles.borrow().contains_key(id) {
                    self.acquire(id, idx, item, &layout);
                    added = true;
                }
            }
            if added {
                obj.queue_allocate();
            }
        }

        fn new_label(&self, text: &str) -> Label {
            let label = self.label_pool.borrow_mut().pop().unwrap_or_else(|| {
                let l = Label::new(None);
                l.set_xalign(0.0);
                l.set_single_line_mode(true);
                l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                l.set_valign(gtk4::Align::Center);
                l
            });
            label.remove_css_class("walltag-section-header");
            label.add_css_class("walltag-tile-name");
            label.set_text(text);
            label.set_parent(&*self.obj());
            label
        }

        fn request_thumb(&self, tile: &mut Tile, item: &GridItem, px: i32) {
            let Some(picture) = tile.picture.clone() else {
                return;
            };
            tile.cancel.store(true, Ordering::Relaxed);
            tile.cancel = Arc::new(AtomicBool::new(false));
            tile.thumb_px = px;
            self.thumbs.request(
                &item.path,
                px,
                item.natural_width,
                tile.cancel.clone(),
                move |tex| {
                    picture.set_paintable(Some(tex));
                },
            );
        }

        fn acquire(&self, id: &str, idx: usize, item: &GridItem, layout: &Layout) {
            let to = self.target_vis(layout, idx);
            let cur_from = match self.transition.borrow().as_ref() {
                Some(tr) => {
                    let from = tr.prev.get(id).copied().unwrap_or(Vis { a: 0.0, ..to });
                    Some(from)
                }
                None => None,
            };
            let from = cur_from.unwrap_or(to);
            let cur = if cur_from.is_some() {
                from.lerp(to, self.anim_e.get())
            } else {
                to
            };

            let mut tile = if let Some(title) = item.section_title.as_deref() {
                let label = self.label_pool.borrow_mut().pop().unwrap_or_else(|| {
                    let l = Label::new(None);
                    l.set_xalign(0.0);
                    l.set_single_line_mode(true);
                    l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    l.set_valign(gtk4::Align::Center);
                    l
                });
                label.set_text(title);
                label.remove_css_class("walltag-tile-name");
                label.add_css_class("walltag-section-header");
                label.set_parent(&*self.obj());
                Tile {
                    key: item.key.clone(),
                    picture: None,
                    label: Some(label),
                    cancel: Arc::new(AtomicBool::new(false)),
                    thumb_px: 0,
                    idx: Some(idx),
                    from,
                    to,
                    cur,
                }
            } else {
                let picture = self.pool.borrow_mut().pop().unwrap_or_else(|| {
                    let p = Picture::new();
                    p.set_can_shrink(true);
                    p.set_content_fit(ContentFit::Fill);
                    p.add_css_class("walltag-thumb");
                    p
                });
                picture.set_parent(&*self.obj());
                let label = self
                    .show_names
                    .get()
                    .then(|| self.new_label(&item.file_name));
                Tile {
                    key: item.key.clone(),
                    picture: Some(picture),
                    label,
                    cancel: Arc::new(AtomicBool::new(false)),
                    thumb_px: 0,
                    idx: Some(idx),
                    from,
                    to,
                    cur,
                }
            };
            if !tile.is_header() {
                let px = layout.rects[idx].w * self.obj().scale_factor();
                self.request_thumb(&mut tile, item, px);
            }
            tile.apply();
            self.tiles.borrow_mut().insert(id.to_owned(), tile);
        }

        fn drop_label(&self, tile: &mut Tile) {
            if let Some(label) = tile.label.take() {
                self.recycle_label(label);
            }
        }

        fn recycle_label(&self, label: Label) {
            label.unparent();
            label.set_opacity(1.0);
            label.remove_css_class("walltag-section-header");
            label.remove_css_class("walltag-tile-name");
            let mut pool = self.label_pool.borrow_mut();
            if pool.len() < MAX_POOL {
                pool.push(label);
            }
        }

        fn release_tile(&self, tile: Tile) {
            tile.cancel.store(true, Ordering::Relaxed);
            if let Some(picture) = tile.picture {
                picture.set_paintable(None::<&gtk4::gdk::Paintable>);
                picture.unparent();
                picture.set_opacity(1.0);
                let mut pool = self.pool.borrow_mut();
                if pool.len() < MAX_POOL {
                    pool.push(picture);
                }
            }
            if let Some(label) = tile.label {
                self.recycle_label(label);
            }
        }

        pub fn release_all(&self) {
            let tiles: Vec<Tile> = self.tiles.borrow_mut().drain().map(|(_, t)| t).collect();
            for tile in tiles {
                self.release_tile(tile);
            }
            let leaving: Vec<Tile> = self.leaving.borrow_mut().drain(..).collect();
            for tile in leaving {
                self.release_tile(tile);
            }
            *self.transition.borrow_mut() = None;
        }
    }
}

use gtk4::prelude::*;
use gtk4::subclass::prelude::ObjectSubclassIsExt;
use std::collections::HashSet;

use gtk4::{glib, Adjustment, GestureDrag};

glib::wrapper! {
    /// Vertically scrolling masonry of image tiles with zero spacing.
    pub struct MasonryGrid(ObjectSubclass<imp::MasonryGrid>)
        @extends gtk4::Widget,
        @implements gtk4::Accessible, gtk4::Buildable, gtk4::ConstraintTarget;
}

impl MasonryGrid {
    /// `vadjustment` is the vertical adjustment of the `ScrolledWindow` that hosts the grid.
    pub fn new(vadjustment: &Adjustment) -> Self {
        let grid: Self = glib::Object::new();
        grid.imp().zoom.set(DEFAULT_ZOOM);
        *grid.imp().vadj.borrow_mut() = Some(vadjustment.clone());
        grid.set_cursor_from_name(Some("pointer"));
        grid.set_hexpand(true);
        grid.add_css_class("walltag-masonry");

        let weak = grid.downgrade();
        vadjustment.connect_value_changed(move |_| {
            if let Some(grid) = weak.upgrade() {
                grid.imp().sync();
            }
        });
        let weak = grid.downgrade();
        vadjustment.connect_changed(move |_| {
            if let Some(grid) = weak.upgrade() {
                grid.imp().sync();
            }
        });

        let drag = GestureDrag::builder().button(0).build();
        let weak = grid.downgrade();
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(grid) = weak.upgrade() else { return };
            let button = gesture.current_button();
            if button != BUTTON_PRIMARY && button != BUTTON_SECONDARY {
                gesture.set_state(gtk4::EventSequenceState::Denied);
                return;
            }
            let state = gesture.current_event_state();
            let extend_selection = state.contains(gtk4::gdk::ModifierType::CONTROL_MASK)
                || state.contains(gtk4::gdk::ModifierType::SHIFT_MASK);
            grid.imp().drag.set(Some(Drag {
                start: (x, y),
                current: (x, y),
                button,
                active: false,
                extend_selection,
            }));
        });
        let weak = grid.downgrade();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some(grid) = weak.upgrade() else { return };
            let imp = grid.imp();
            let Some(mut d) = imp.drag.get() else { return };
            d.current = (d.start.0 + dx, d.start.1 + dy);
            if !d.active && d.moved_past_threshold() {
                d.active = true;
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            }
            imp.drag.set(Some(d));
            if d.active {
                grid.queue_draw();
            }
        });
        let weak = grid.downgrade();
        drag.connect_drag_end(move |_, dx, dy| {
            let Some(grid) = weak.upgrade() else { return };
            let Some(mut d) = grid.imp().drag.take() else {
                return;
            };
            d.current = (d.start.0 + dx, d.start.1 + dy);
            grid.finish_press(d);
        });
        let weak = grid.downgrade();
        drag.connect_cancel(move |_, _| {
            if let Some(grid) = weak.upgrade() {
                grid.imp().drag.set(None);
                grid.queue_draw();
            }
        });
        grid.add_controller(drag);

        grid.set_has_tooltip(true);
        grid.connect_query_tooltip(|grid, x, y, _keyboard, tooltip| {
            match grid.item_at(x as f64, y as f64) {
                Some(item) if !item.tooltip.is_empty() => {
                    tooltip.set_text(Some(&item.tooltip));
                    true
                }
                _ => false,
            }
        });

        grid
    }

    /// Current zoom factor (1.0 = 100%).
    pub fn zoom(&self) -> f32 {
        self.imp().zoom()
    }

    /// Sets the zoom (clamped), drops the cached layout, and relays out. Tiles are
    /// re-requested at the new size and the scroll position keeps its relative offset.
    pub fn set_zoom(&self, zoom: f32) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        let imp = self.imp();
        if (imp.zoom() - zoom).abs() < f32::EPSILON {
            return;
        }
        let relative = imp.vadj.borrow().as_ref().and_then(|adj| {
            let upper = adj.upper();
            (upper > 0.0).then(|| adj.value() / upper)
        });
        imp.zoom.set(zoom);
        // Tiles slide and resize to the new layout; the scroll offset keeps its fraction.
        imp.transition(relative.map_or(ScrollMode::Keep, ScrollMode::Relative));
        for callback in imp.on_zoom.borrow().clone() {
            callback(zoom);
        }
    }

    /// Shows or hides the file name under each tile and relays out.
    pub fn set_show_names(&self, show: bool) {
        let imp = self.imp();
        if imp.show_names.replace(show) == show {
            return;
        }
        imp.transition(ScrollMode::Keep);
    }

    pub fn zoom_in(&self) {
        self.set_zoom(step_zoom(self.zoom(), true));
    }

    pub fn zoom_out(&self) {
        self.set_zoom(step_zoom(self.zoom(), false));
    }

    pub fn reset_zoom(&self) {
        self.set_zoom(DEFAULT_ZOOM);
    }

    /// Runs `f` with the new zoom whenever it changes.
    pub fn connect_zoom_changed(&self, f: impl Fn(f32) + 'static) {
        self.imp().on_zoom.borrow_mut().push(Rc::new(f));
    }

    /// Replaces the items and animates the tiles to the new layout: kept keys slide and
    /// resize, new keys fade in, removed keys fade out. The first fill from an empty grid
    /// is instant. Pass `reset_scroll` to jump back to the top (cross-fade when scrolled).
    /// Selected wallpapers that are no longer present are dropped from the edit batch.
    pub fn set_items(&self, items: Vec<GridItem>, reset_scroll: bool) {
        let present: HashSet<&str> = items.iter().map(|i| i.key.as_str()).collect();
        let pruned = {
            let mut selected = self.imp().selected.borrow_mut();
            let before = selected.len();
            selected.retain(|k| present.contains(k.as_str()));
            selected.len() != before
        };
        self.imp().set_items(items, reset_scroll);
        if pruned {
            self.notify_selection();
        }
    }

    /// Replaces the tiles when `items` differ from the current list.
    /// Returns `false` when the list is unchanged so in-flight thumbnails stay put.
    /// `reset_scroll` always replaces, including when the lists match.
    pub fn replace_items_if_changed(&self, items: Vec<GridItem>, reset_scroll: bool) -> bool {
        if !reset_scroll && self.imp().items.borrow().as_ref() == items.as_slice() {
            return false;
        }
        self.set_items(items, reset_scroll);
        true
    }

    /// Keys of the wallpapers in the edit batch, in gallery order.
    pub fn selected_keys(&self) -> Vec<String> {
        let imp = self.imp();
        let selected = imp.selected.borrow();
        let mut seen = HashSet::new();
        imp.items
            .borrow()
            .iter()
            .filter(|i| !i.is_header() && selected.contains(&i.key))
            .filter_map(|i| seen.insert(i.key.clone()).then(|| i.key.clone()))
            .collect()
    }

    pub fn clear_selection(&self) {
        let cleared = {
            let mut selected = self.imp().selected.borrow_mut();
            let had = !selected.is_empty();
            selected.clear();
            had
        };
        if cleared {
            self.queue_draw();
            self.notify_selection();
        }
    }

    /// Runs `f` whenever the edit batch changes.
    pub fn connect_selection_changed(&self, f: impl Fn() + 'static) {
        self.imp().on_selection.borrow_mut().push(Rc::new(f));
    }

    /// Replaces the tooltip of one item without relaying out the grid.
    pub fn update_tooltip(&self, key: &str, tooltip: String) {
        let mut items = self.imp().items.borrow_mut();
        if let Some(item) = Rc::make_mut(&mut items).iter_mut().find(|i| i.key == key) {
            item.tooltip = tooltip;
        }
    }

    fn notify_selection(&self) {
        let callbacks = self.imp().on_selection.borrow().clone();
        for callback in callbacks {
            callback();
        }
    }

    fn toggle_selected(&self, key: &str) {
        {
            let mut selected = self.imp().selected.borrow_mut();
            if !selected.remove(key) {
                selected.insert(key.to_owned());
            }
        }
        self.queue_draw();
        self.notify_selection();
    }

    fn select_only(&self, key: &str) {
        let changed = {
            let mut selected = self.imp().selected.borrow_mut();
            let already = selected.len() == 1 && selected.contains(key);
            if already {
                false
            } else {
                selected.clear();
                selected.insert(key.to_owned());
                true
            }
        };
        if changed {
            self.queue_draw();
            self.notify_selection();
        }
    }

    /// Handles the end of a press: a drag adds the enclosed tiles to the edit batch, a
    /// click follows `click_action`.
    fn finish_press(&self, d: Drag) {
        let imp = self.imp();
        if d.active {
            let added: Vec<String> = match imp.layout.borrow().clone() {
                Some(layout) => {
                    let items = imp.items.borrow();
                    layout
                        .hit_rect(d.start.0, d.start.1, d.current.0, d.current.1)
                        .into_iter()
                        .filter_map(|i| items.get(i).map(|item| item.key.clone()))
                        .collect()
                }
                None => Vec::new(),
            };
            let changed = {
                let mut selected = imp.selected.borrow_mut();
                let before = selected.len();
                selected.extend(added);
                selected.len() != before
            };
            self.queue_draw();
            if changed {
                self.notify_selection();
            }
            return;
        }

        let item = self.item_at(d.current.0, d.current.1);
        let batch_empty = imp.selected.borrow().is_empty();
        match click_action(d.button, item.is_some(), batch_empty, d.extend_selection) {
            ClickAction::Activate => {
                let callback = imp.on_activate.borrow().clone();
                if let (Some(callback), Some(item)) = (callback, item) {
                    callback(&item);
                }
            }
            ClickAction::Toggle => {
                if let Some(item) = item {
                    self.toggle_selected(&item.key);
                }
            }
            ClickAction::SelectOnly => {
                if let Some(item) = item {
                    self.select_only(&item.key);
                }
            }
            ClickAction::ClearSelection => self.clear_selection(),
            ClickAction::Ignore => {}
        }
    }

    pub fn connect_item_activated(&self, f: impl Fn(&GridItem) + 'static) {
        *self.imp().on_activate.borrow_mut() = Some(Rc::new(f));
    }

    fn item_at(&self, x: f64, y: f64) -> Option<GridItem> {
        let imp = self.imp();
        let layout = imp.layout.borrow().clone()?;
        let idx = layout.hit(x, y)?;
        imp.items.borrow().get(idx).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aspects() -> Vec<f32> {
        vec![
            16.0 / 9.0,
            4.0 / 3.0,
            1.0,
            9.0 / 16.0,
            21.0 / 9.0,
            3.0 / 2.0,
            16.0 / 10.0,
            2.0,
            0.75,
            1.5,
        ]
    }

    #[test]
    fn column_count_follows_width() {
        assert_eq!(columns_for_width(100), 1);
        assert_eq!(columns_for_width(560), 1);
        assert_eq!(columns_for_width(700), 2);
        assert_eq!(columns_for_width(1100), 3);
    }

    #[test]
    fn zoom_scales_column_count() {
        assert_eq!(columns_for_width_zoomed(1100, 1.0), 3);
        assert_eq!(columns_for_width_zoomed(1100, 0.5), 5);
        assert_eq!(columns_for_width_zoomed(1100, 2.0), 1);
        // Out-of-range zoom is clamped instead of dividing by ~0.
        assert_eq!(
            columns_for_width_zoomed(1100, 0.0),
            columns_for_width_zoomed(1100, MIN_ZOOM)
        );
    }

    #[test]
    fn zoomed_layout_still_fills_width() {
        for zoom in [MIN_ZOOM, 0.7, 1.0, 1.6, MAX_ZOOM] {
            let layout = compute_layout_zoomed(&aspects(), 1100, zoom, 0);
            let right = layout.rects.iter().map(|r| r.x + r.w).max().unwrap();
            assert_eq!(right, 1100, "zoom {zoom}");
        }
    }

    #[test]
    fn label_strip_is_reserved_under_every_tile() {
        let plain = compute_layout_zoomed(&aspects(), 1100, 1.0, 0);
        let labeled = compute_layout_zoomed(&aspects(), 1100, 1.0, 24);
        assert_eq!(labeled.label_height, 24);
        assert!(labeled.height > plain.height);
        // Image rectangles keep their size; only the vertical position shifts.
        for (a, b) in plain.rects.iter().zip(&labeled.rects) {
            assert_eq!((a.x, a.w, a.h), (b.x, b.w, b.h));
        }
        // Cells (image + label) never overlap and stay one gap apart in a column.
        for i in 0..labeled.rects.len() {
            let a = labeled.cell(i).unwrap();
            assert_eq!(a.h, labeled.rects[i].h + 24);
            assert!(a.bottom() <= labeled.height);
            for j in i + 1..labeled.rects.len() {
                let b = labeled.cell(j).unwrap();
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        // The label strip counts as part of the tile for hit testing.
        let r = labeled.rects[0];
        assert_eq!(
            labeled.hit(r.x as f64 + 1.0, (r.y + r.h + 5) as f64),
            Some(0)
        );
        assert_eq!(plain.hit(r.x as f64 + 1.0, (r.y + r.h + 5) as f64), None);
    }

    #[test]
    fn zoom_steps_are_clamped_and_return_to_default() {
        assert_eq!(step_zoom(MAX_ZOOM, true), MAX_ZOOM);
        assert_eq!(step_zoom(MIN_ZOOM, false), MIN_ZOOM);
        assert!(step_zoom(1.0, true) > 1.0);
        assert!(step_zoom(1.0, false) < 1.0);
        let out_in = step_zoom(step_zoom(1.0, false), true);
        assert_eq!(out_in, DEFAULT_ZOOM);
        let mut z = 1.0;
        for _ in 0..50 {
            z = step_zoom(z, true);
        }
        assert_eq!(z, MAX_ZOOM);
    }

    #[test]
    fn columns_span_full_width_with_uniform_gaps() {
        for width in [300, 777, 1100, 1921] {
            let layout = compute_layout(&aspects(), width);
            let mut xs: Vec<(i32, i32)> = layout.rects.iter().map(|r| (r.x, r.w)).collect();
            xs.sort();
            xs.dedup();
            let cols = xs.len();
            assert_eq!(cols, columns_for_width(width), "width {width}");
            let mut cursor = 0;
            for (i, (x, w)) in xs.into_iter().enumerate() {
                if i > 0 {
                    cursor += TILE_GAP;
                }
                assert_eq!(x, cursor, "width {width}");
                cursor += w;
            }
            assert_eq!(cursor, width);
        }
    }

    #[test]
    fn tiles_in_a_column_are_one_gap_apart_and_never_overlap() {
        let layout = compute_layout(&aspects(), 1100);
        let mut by_col: std::collections::BTreeMap<i32, Vec<Rect>> = Default::default();
        for r in &layout.rects {
            by_col.entry(r.x).or_default().push(*r);
        }
        for rects in by_col.values() {
            let mut y = 0;
            for r in rects {
                assert_eq!(r.y, y);
                y = r.bottom() + TILE_GAP;
            }
            assert!(y - TILE_GAP <= layout.height);
        }
    }

    #[test]
    fn no_two_tiles_overlap() {
        let layout = compute_layout(&aspects(), 1100);
        for (i, a) in layout.rects.iter().enumerate() {
            for b in &layout.rects[i + 1..] {
                let apart =
                    a.x + a.w <= b.x || b.x + b.w <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn gap_between_tiles_is_not_a_hit() {
        let layout = compute_layout(&aspects(), 1100);
        let r = layout.rects[0];
        assert_eq!(layout.hit((r.x + r.w) as f64 + 1.0, r.y as f64 + 1.0), None);
    }

    #[test]
    fn aspect_ratio_is_preserved_within_a_pixel() {
        let a = aspects();
        let layout = compute_layout(&a, 1100);
        for (r, aspect) in layout.rects.iter().zip(&a) {
            let expected_h = r.w as f32 / aspect;
            assert!((r.h as f32 - expected_h).abs() <= 0.5 + f32::EPSILON);
        }
    }

    #[test]
    fn height_is_tallest_column() {
        let layout = compute_layout(&aspects(), 1100);
        let max_bottom = layout.rects.iter().map(Rect::bottom).max().unwrap();
        assert_eq!(layout.height, max_bottom);
    }

    #[test]
    fn empty_input_has_zero_height() {
        let layout = compute_layout(&[], 800);
        assert_eq!(layout.height, 0);
        assert!(layout.rects.is_empty());
    }

    #[test]
    fn hit_test_finds_tile() {
        let layout = compute_layout(&aspects(), 1100);
        let r = layout.rects[3];
        assert_eq!(layout.hit(r.x as f64 + 1.0, r.y as f64 + 1.0), Some(3));
        assert_eq!(layout.hit(-1.0, 0.0), None);
    }

    #[test]
    fn rect_hit_selects_overlapping_tiles_only() {
        let layout = compute_layout(&aspects(), 1100);
        let r = layout.rects[0];
        let hits = layout.hit_rect(
            r.x as f64 + 2.0,
            r.y as f64 + 2.0,
            r.x as f64 + 4.0,
            r.y as f64 + 4.0,
        );
        assert_eq!(hits, vec![0]);
        // Corner order does not matter.
        let all = layout.hit_rect(layout.width as f64, layout.height as f64, 0.0, 0.0);
        assert_eq!(all.len(), layout.rects.len());
        assert!(layout.hit_rect(-10.0, -10.0, -1.0, -1.0).is_empty());
    }

    #[test]
    fn rect_hit_excludes_touching_edges() {
        let layout = compute_layout(&aspects(), 1100);
        let r = layout.rects[0];
        let hits = layout.hit_rect(r.x as f64 - 5.0, r.y as f64, r.x as f64, r.y as f64 + 5.0);
        assert!(!hits.contains(&0));
    }

    #[test]
    fn wallpaper_applies_only_on_primary_click_with_empty_batch() {
        assert_eq!(click_action(1, true, true, false), ClickAction::Activate);
        assert_eq!(click_action(1, true, false, false), ClickAction::Toggle);
        assert_eq!(click_action(3, true, true, false), ClickAction::SelectOnly);
        assert_eq!(click_action(3, true, false, false), ClickAction::SelectOnly);
        assert_eq!(click_action(3, true, false, true), ClickAction::Toggle);
        assert_eq!(
            click_action(1, false, false, false),
            ClickAction::ClearSelection
        );
        assert_eq!(click_action(1, false, true, false), ClickAction::Ignore);
        assert_eq!(click_action(3, false, false, false), ClickAction::Ignore);
        assert_eq!(click_action(2, true, true, false), ClickAction::Ignore);
    }

    #[test]
    fn press_becomes_drag_after_threshold() {
        let mut d = Drag {
            start: (10.0, 10.0),
            current: (12.0, 12.0),
            button: 1,
            active: false,
            extend_selection: false,
        };
        assert!(!d.moved_past_threshold());
        d.current = (10.0, 17.0);
        assert!(d.moved_past_threshold());
    }

    #[test]
    fn ease_out_cubic_is_monotonic_and_bounded() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(-1.0), 0.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
        let mut last = 0.0;
        for i in 1..=100 {
            let v = ease_out_cubic(i as f32 / 100.0);
            assert!(v >= last);
            last = v;
        }
        // Ease-out: more than half done at half time.
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn progress_reaches_one_after_the_duration() {
        assert_eq!(progress(-5), 0.0);
        assert_eq!(progress(0), 0.0);
        assert!((progress((ANIM_DURATION_MS * 500.0) as i64) - 0.5).abs() < 1e-3);
        assert_eq!(progress((ANIM_DURATION_MS * 1000.0) as i64), 1.0);
        assert_eq!(progress(i64::MAX), 1.0);
    }

    #[test]
    fn vis_lerp_interpolates_every_field() {
        let a = Vis {
            x: 0.0,
            y: 10.0,
            w: 100.0,
            h: 50.0,
            lh: 0.0,
            a: 0.0,
            la: 0.0,
        };
        let b = Vis {
            x: 20.0,
            y: 30.0,
            w: 200.0,
            h: 150.0,
            lh: 20.0,
            a: 1.0,
            la: 1.0,
        };
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        let m = a.lerp(b, 0.5);
        assert_eq!(
            (m.x, m.y, m.w, m.h, m.lh, m.a, m.la),
            (10.0, 20.0, 150.0, 100.0, 10.0, 0.5, 0.5)
        );
    }

    #[test]
    fn layout_reports_column_count() {
        let layout = compute_layout(&aspects(), 1100);
        assert_eq!(layout.columns, columns_for_width(1100));
    }

    #[test]
    fn tile_ids_are_unique_even_for_repeated_keys() {
        let mk = |k: &str| GridItem {
            key: k.into(),
            path: PathBuf::new(),
            aspect: 1.0,
            natural_width: 0,
            tooltip: String::new(),
            file_name: String::new(),
            section_title: None,
        };
        let items = vec![mk("a"), mk("b"), mk("a"), GridItem::header("g"), mk("a")];
        let ids = item_ids(&items);
        assert_eq!(ids[0], "a");
        assert_eq!(ids[1], "b");
        assert_eq!(ids[3], "header:g");
        let unique: HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), items.len());
    }

    #[test]
    fn scroll_target_clamps_to_content() {
        assert_eq!(
            scroll_target(ScrollMode::Top, 500.0, 2000.0, 600.0, 3000),
            0.0
        );
        assert_eq!(
            scroll_target(ScrollMode::Keep, 500.0, 2000.0, 600.0, 3000),
            500.0
        );
        // Content shrinks below the offset: clamp to the new maximum.
        assert_eq!(
            scroll_target(ScrollMode::Keep, 500.0, 2000.0, 600.0, 800),
            200.0
        );
        // Content shorter than the viewport: no scrolling.
        assert_eq!(
            scroll_target(ScrollMode::Keep, 500.0, 2000.0, 600.0, 300),
            0.0
        );
        assert_eq!(
            scroll_target(ScrollMode::Relative(0.5), 0.0, 2000.0, 600.0, 3000),
            1500.0
        );
        assert_eq!(
            scroll_target(ScrollMode::Relative(0.9), 0.0, 2000.0, 600.0, 1000),
            400.0
        );
    }
}
