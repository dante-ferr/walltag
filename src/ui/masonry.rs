//! Aspect-preserving column masonry with a uniform gap between tiles and virtualized tiles.
//!
//! Layout is pure integer math (`compute_layout`). The widget only keeps `Picture`
//! children for tiles near the viewport and recycles them while scrolling.

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
    let next = if zoom_in { zoom * ZOOM_STEP } else { zoom / ZOOM_STEP };
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

#[derive(Debug, Clone)]
pub struct GridItem {
    pub key: String,
    pub path: PathBuf,
    /// Width divided by height.
    pub aspect: f32,
    /// Source width in pixels, or 0 if unknown. Caps thumbnail size.
    pub natural_width: i32,
    pub tooltip: String,
    /// Shown under the tile when file names are enabled.
    pub file_name: String,
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
    /// Image area of each tile.
    pub rects: Vec<Rect>,
    /// Height of the file name strip directly under each image; 0 when names are hidden.
    pub label_height: i32,
}

impl Layout {
    /// Image plus label strip. Hit tests, selection, and virtualization use the cell.
    pub fn cell(&self, idx: usize) -> Option<Rect> {
        self.rects.get(idx).map(|r| Rect {
            h: r.h + self.label_height,
            ..*r
        })
    }

    pub fn hit(&self, x: f64, y: f64) -> Option<usize> {
        (0..self.rects.len()).find(|&i| self.cell(i).is_some_and(|c| c.contains(x, y)))
    }

    /// Indices of tiles that overlap the rectangle spanned by two corner points.
    pub fn hit_rect(&self, x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<usize> {
        let (left, right) = (x0.min(x1), x0.max(x1));
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        (0..self.rects.len())
            .filter(|&i| {
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
    /// Empty the edit batch.
    ClearSelection,
    Ignore,
}

/// Decides the effect of a click. The wallpaper is set only by a primary-button click on a
/// tile while the edit batch is empty.
pub fn click_action(button: u32, on_tile: bool, batch_empty: bool) -> ClickAction {
    match (button, on_tile) {
        (BUTTON_SECONDARY, true) => ClickAction::Toggle,
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
pub fn compute_layout_zoomed(
    aspects: &[f32],
    width: i32,
    zoom: f32,
    label_height: i32,
) -> Layout {
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

    for aspect in aspects {
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
        label_height,
    }
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

    struct Tile {
        picture: Picture,
        label: Option<Label>,
        cancel: Arc<AtomicBool>,
    }

    #[derive(Default)]
    pub struct MasonryGrid {
        pub items: RefCell<Rc<Vec<GridItem>>>,
        pub layout: RefCell<Option<Rc<Layout>>>,
        tiles: RefCell<HashMap<usize, Tile>>,
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
                Orientation::Horizontal => (MIN_WIDGET_WIDTH, MIN_WIDGET_WIDTH, -1, -1),
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
            let layout = self.layout_for(width);
            for (idx, tile) in self.tiles.borrow().iter() {
                if let Some(r) = layout.rects.get(*idx) {
                    tile.picture
                        .size_allocate(&gtk4::Allocation::new(r.x, r.y, r.w, r.h), -1);
                    if let Some(label) = &tile.label {
                        label.size_allocate(
                            &gtk4::Allocation::new(r.x, r.y + r.h, r.w, layout.label_height),
                            -1,
                        );
                    }
                }
            }
            // Realize/recycle after allocation so we never reparent during allocate.
            self.queue_sync();
        }

        fn snapshot(&self, snapshot: &gtk4::Snapshot) {
            self.parent_snapshot(snapshot);
            self.snapshot_overlays(snapshot);
        }
    }

    impl MasonryGrid {
        /// Draws selection tint and border over the tiles (inside each tile's bounds, so
        /// the layout stays gapless), then the drag rectangle.
        fn snapshot_overlays(&self, snapshot: &gtk4::Snapshot) {
            let Some(layout) = self.layout.borrow().clone() else {
                return;
            };
            let drag = self.drag.get().filter(|d| d.active);
            let preview: HashSet<usize> = drag
                .map(|d| {
                    layout
                        .hit_rect(d.start.0, d.start.1, d.current.0, d.current.1)
                        .into_iter()
                        .collect()
                })
                .unwrap_or_default();
            let accent = self.accent();
            let with_alpha = |alpha: f32| gdk::RGBA::new(accent.red(), accent.green(), accent.blue(), alpha);

            if !self.selected.borrow().is_empty() || !preview.is_empty() {
                let items = self.items.borrow().clone();
                let selected = self.selected.borrow();
                for idx in self.tiles.borrow().keys() {
                    let (Some(r), Some(item)) = (layout.cell(*idx), items.get(*idx)) else {
                        continue;
                    };
                    let is_selected = selected.contains(&item.key);
                    if !is_selected && !preview.contains(idx) {
                        continue;
                    }
                    let rect = graphene::Rect::new(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
                    snapshot.append_color(&with_alpha(if is_selected { 0.30 } else { 0.18 }), &rect);
                    if is_selected {
                        let border = [with_alpha(1.0); 4];
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
            let aspects: Vec<f32> = self.items.borrow().iter().map(|i| i.aspect).collect();
            let layout = Rc::new(compute_layout_zoomed(
                &aspects,
                width,
                self.zoom(),
                self.label_height(),
            ));
            *self.layout.borrow_mut() = Some(layout.clone());
            layout
        }

        pub fn set_items(&self, items: Vec<GridItem>, reset_scroll: bool) {
            self.release_all();
            *self.items.borrow_mut() = Rc::new(items);
            *self.layout.borrow_mut() = None;
            if reset_scroll {
                if let Some(adj) = self.vadj.borrow().as_ref() {
                    adj.set_value(0.0);
                }
            }
            self.obj().queue_resize();
            self.queue_sync();
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

        /// Realizes tiles near the viewport and recycles those far away.
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

            let near = (value - page * REALIZE_PAD, value + page + page * REALIZE_PAD);
            let far = (value - page * RECYCLE_PAD, value + page + page * RECYCLE_PAD);
            let overlaps = |r: &Rect, (top, bottom): (f64, f64)| {
                (r.bottom() as f64) > top && (r.y as f64) < bottom
            };

            let stale: Vec<usize> = self
                .tiles
                .borrow()
                .keys()
                .copied()
                .filter(|i| layout.cell(*i).map_or(true, |r| !overlaps(&r, far)))
                .collect();
            for idx in stale {
                self.release(idx);
            }

            let items = self.items.borrow().clone();
            let mut added = false;
            for idx in 0..layout.rects.len() {
                let Some(cell) = layout.cell(idx) else { continue };
                if overlaps(&cell, near) && !self.tiles.borrow().contains_key(&idx) {
                    if let Some(item) = items.get(idx) {
                        self.acquire(idx, item, &layout.rects[idx]);
                        added = true;
                    }
                }
            }
            if added {
                obj.queue_allocate();
            }
        }

        fn acquire(&self, idx: usize, item: &GridItem, rect: &Rect) {
            let picture = self.pool.borrow_mut().pop().unwrap_or_else(|| {
                let p = Picture::new();
                p.set_can_shrink(true);
                p.set_content_fit(ContentFit::Fill);
                p.add_css_class("walltag-thumb");
                p
            });
            picture.set_parent(&*self.obj());

            let cancel = Arc::new(AtomicBool::new(false));
            let target = picture.clone();
            let px = rect.w * self.obj().scale_factor();
            self.thumbs
                .request(&item.path, px, item.natural_width, cancel.clone(), move |tex| {
                    target.set_paintable(Some(tex));
                });
            let label = self.show_names.get().then(|| {
                let label = self.label_pool.borrow_mut().pop().unwrap_or_else(|| {
                    let l = Label::new(None);
                    l.set_xalign(0.0);
                    l.set_single_line_mode(true);
                    l.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                    l.set_valign(gtk4::Align::Center);
                    l.add_css_class("walltag-tile-name");
                    l
                });
                label.set_text(&item.file_name);
                label.set_parent(&*self.obj());
                label
            });
            self.tiles
                .borrow_mut()
                .insert(idx, Tile { picture, label, cancel });
        }

        fn release(&self, idx: usize) {
            let Some(tile) = self.tiles.borrow_mut().remove(&idx) else {
                return;
            };
            tile.cancel.store(true, Ordering::Relaxed);
            tile.picture.set_paintable(None::<&gtk4::gdk::Paintable>);
            tile.picture.unparent();
            let mut pool = self.pool.borrow_mut();
            if pool.len() < MAX_POOL {
                pool.push(tile.picture);
            }
            if let Some(label) = tile.label {
                label.unparent();
                let mut pool = self.label_pool.borrow_mut();
                if pool.len() < MAX_POOL {
                    pool.push(label);
                }
            }
        }

        pub fn release_all(&self) {
            let indices: Vec<usize> = self.tiles.borrow().keys().copied().collect();
            for idx in indices {
                self.release(idx);
            }
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
            grid.imp().drag.set(Some(Drag {
                start: (x, y),
                current: (x, y),
                button,
                active: false,
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
            let Some(mut d) = grid.imp().drag.take() else { return };
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
        *imp.layout.borrow_mut() = None;
        imp.release_all();
        self.queue_resize();
        // `upper` only changes after the next allocation; restore the offset then.
        if let Some(relative) = relative {
            let weak = self.downgrade();
            self.add_tick_callback(move |_, _| {
                if let Some(grid) = weak.upgrade() {
                    if let Some(adj) = grid.imp().vadj.borrow().as_ref() {
                        let max = (adj.upper() - adj.page_size()).max(0.0);
                        adj.set_value((relative * adj.upper()).min(max));
                    }
                }
                glib::ControlFlow::Break
            });
        }
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
        *imp.layout.borrow_mut() = None;
        imp.release_all();
        self.queue_resize();
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

    /// Replaces all tiles. Pass `reset_scroll` to jump back to the top.
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

    /// Keys of the wallpapers in the edit batch, in gallery order.
    pub fn selected_keys(&self) -> Vec<String> {
        let imp = self.imp();
        let selected = imp.selected.borrow();
        imp.items
            .borrow()
            .iter()
            .filter(|i| selected.contains(&i.key))
            .map(|i| i.key.clone())
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
        match click_action(d.button, item.is_some(), batch_empty) {
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
                let apart = a.x + a.w <= b.x
                    || b.x + b.w <= a.x
                    || a.bottom() <= b.y
                    || b.bottom() <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
        // The label strip counts as part of the tile for hit testing.
        let r = labeled.rects[0];
        assert_eq!(labeled.hit(r.x as f64 + 1.0, (r.y + r.h + 5) as f64), Some(0));
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
                let apart = a.x + a.w <= b.x || b.x + b.w <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
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
        let all = layout.hit_rect(
            layout.width as f64,
            layout.height as f64,
            0.0,
            0.0,
        );
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
        assert_eq!(click_action(1, true, true), ClickAction::Activate);
        assert_eq!(click_action(1, true, false), ClickAction::Toggle);
        assert_eq!(click_action(3, true, true), ClickAction::Toggle);
        assert_eq!(click_action(3, true, false), ClickAction::Toggle);
        assert_eq!(click_action(1, false, false), ClickAction::ClearSelection);
        assert_eq!(click_action(1, false, true), ClickAction::Ignore);
        assert_eq!(click_action(3, false, false), ClickAction::Ignore);
        assert_eq!(click_action(2, true, true), ClickAction::Ignore);
    }

    #[test]
    fn press_becomes_drag_after_threshold() {
        let mut d = Drag {
            start: (10.0, 10.0),
            current: (12.0, 12.0),
            button: 1,
            active: false,
        };
        assert!(!d.moved_past_threshold());
        d.current = (10.0, 17.0);
        assert!(d.moved_past_threshold());
    }
}
