use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gtk4::gdk::{self, MemoryFormat, Texture};
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::{gio, glib};

/// Maximum number of decoded thumbnails kept in memory.
const CACHE_CAPACITY: usize = 256;
/// Thumbnail widths are rounded up to this step so small resizes reuse cached textures.
const WIDTH_STEP: i32 = 64;
const MAX_THUMB_WIDTH: i32 = 1024;
const PROBE_THREADS: usize = 4;

/// Reads image dimensions from file headers only (no full decode).
/// Returns `(width, height)` per key. Files the loaders cannot read are omitted.
pub fn probe_dimensions(items: Vec<(String, PathBuf)>) -> HashMap<String, (i32, i32)> {
    if items.is_empty() {
        return HashMap::new();
    }
    let chunk = items.len().div_ceil(PROBE_THREADS).max(1);
    let mut out = HashMap::with_capacity(items.len());
    std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|(key, path)| {
                            let (_, w, h) = Pixbuf::file_info(path)?;
                            (w > 0 && h > 0).then(|| (key.clone(), (w, h)))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for handle in handles {
            if let Ok(found) = handle.join() {
                out.extend(found);
            }
        }
    });
    out
}

struct RawThumb {
    width: i32,
    height: i32,
    rowstride: usize,
    has_alpha: bool,
    pixels: Vec<u8>,
}

impl RawThumb {
    fn into_texture(self) -> Texture {
        let format = if self.has_alpha {
            MemoryFormat::R8g8b8a8
        } else {
            MemoryFormat::R8g8b8
        };
        let bytes = glib::Bytes::from_owned(self.pixels);
        gdk::MemoryTexture::new(self.width, self.height, format, &bytes, self.rowstride).upcast()
    }
}

use gtk4::prelude::Cast;

fn decode(path: &Path, width: i32, cancel: &AtomicBool) -> Option<RawThumb> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    // Height -1 keeps the aspect ratio. JPEG loaders can decode at reduced size directly.
    let pixbuf = Pixbuf::from_file_at_scale(path, width, -1, true).ok()?;
    Some(RawThumb {
        width: pixbuf.width(),
        height: pixbuf.height(),
        rowstride: pixbuf.rowstride() as usize,
        has_alpha: pixbuf.has_alpha(),
        pixels: pixbuf.read_pixel_bytes().to_vec(),
    })
}

type Key = (PathBuf, i32);

#[derive(Default)]
struct Cache {
    map: HashMap<Key, Texture>,
    order: VecDeque<Key>,
}

impl Cache {
    fn get(&mut self, key: &Key) -> Option<Texture> {
        let tex = self.map.get(key)?.clone();
        self.order.retain(|k| k != key);
        self.order.push_back(key.clone());
        Some(tex)
    }

    fn insert(&mut self, key: Key, tex: Texture) {
        if self.map.insert(key.clone(), tex).is_some() {
            self.order.retain(|k| k != &key);
        }
        self.order.push_back(key);
        while self.order.len() > CACHE_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Decodes downscaled thumbnails off the main thread and keeps an LRU of textures.
#[derive(Clone, Default)]
pub struct Thumbnailer {
    cache: Rc<RefCell<Cache>>,
}

impl Thumbnailer {
    /// Requests a thumbnail about `width_px` wide. `done` runs on the main thread
    /// unless `cancel` is set before the decode finishes.
    pub fn request(
        &self,
        path: &Path,
        width_px: i32,
        natural_width: i32,
        cancel: Arc<AtomicBool>,
        done: impl FnOnce(&Texture) + 'static,
    ) {
        let mut width = (width_px.max(1) + WIDTH_STEP - 1) / WIDTH_STEP * WIDTH_STEP;
        width = width.min(MAX_THUMB_WIDTH);
        if natural_width > 0 {
            width = width.min(natural_width);
        }
        let key = (path.to_path_buf(), width);

        if let Some(tex) = self.cache.borrow_mut().get(&key) {
            done(&tex);
            return;
        }

        let cache = self.cache.clone();
        let path = key.0.clone();
        glib::spawn_future_local(async move {
            let job_cancel = cancel.clone();
            let job_path = path.clone();
            let raw = gio::spawn_blocking(move || decode(&job_path, width, &job_cancel)).await;
            let Ok(Some(raw)) = raw else {
                return;
            };
            let texture = raw.into_texture();
            cache.borrow_mut().insert((path, width), texture.clone());
            if !cancel.load(Ordering::Relaxed) {
                done(&texture);
            }
        });
    }
}
