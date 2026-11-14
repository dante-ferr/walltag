use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gtk4::gdk::{self, MemoryFormat, Texture};
use gtk4::gdk_pixbuf::Pixbuf;
use gtk4::{gio, glib};

use crate::ui::cache_paths::{cache_dir, stable_hash};

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

/// Identity of a source file. A change in size or mtime misses both caches.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FileStamp {
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    let dur = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(FileStamp {
        size: meta.len(),
        mtime_secs: dur.as_secs() as i64,
        mtime_nanos: dur.subsec_nanos(),
    })
}

fn raw_from_pixbuf(pixbuf: &Pixbuf) -> RawThumb {
    RawThumb {
        width: pixbuf.width(),
        height: pixbuf.height(),
        rowstride: pixbuf.rowstride() as usize,
        has_alpha: pixbuf.has_alpha(),
        pixels: pixbuf.read_pixel_bytes().to_vec(),
    }
}

/// `{cache}/thumbs/{shard}/{hash}.png`, or `base` when the caller supplies one (tests).
fn thumb_file(base: &Path, source: &Path, width: i32, stamp: FileStamp) -> PathBuf {
    let hash = stable_hash(&[
        b"walltag-thumb-v1",
        source.as_os_str().as_encoded_bytes(),
        &width.to_le_bytes(),
        &stamp.size.to_le_bytes(),
        &stamp.mtime_secs.to_le_bytes(),
        &stamp.mtime_nanos.to_le_bytes(),
    ]);
    let hex = format!("{hash:032x}");
    base.join("thumbs")
        .join(&hex[..2])
        .join(format!("{hex}.png"))
}

fn read_cached_thumb(path: &Path, width: i32, stamp: FileStamp) -> Option<RawThumb> {
    let dest = thumb_file(&cache_dir()?, path, width, stamp);
    match Pixbuf::from_file(&dest) {
        Ok(pixbuf) => Some(raw_from_pixbuf(&pixbuf)),
        Err(_) => {
            if dest.is_file() {
                let _ = std::fs::remove_file(&dest);
            }
            None
        }
    }
}

fn write_cached_thumb(path: &Path, width: i32, stamp: FileStamp, pixbuf: &Pixbuf) {
    let Some(base) = cache_dir() else {
        return;
    };
    let dest = thumb_file(&base, path, width, stamp);
    let Some(parent) = dest.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = dest.with_extension("png.part");
    if pixbuf.savev(&tmp, "png", &[]).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if std::fs::rename(&tmp, &dest).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn decode(
    path: &Path,
    width: i32,
    stamp: Option<FileStamp>,
    cancel: &AtomicBool,
) -> Option<RawThumb> {
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    if let Some(stamp) = stamp {
        if let Some(raw) = read_cached_thumb(path, width, stamp) {
            return Some(raw);
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    // Height -1 keeps the aspect ratio. JPEG loaders can decode at reduced size directly.
    let pixbuf = Pixbuf::from_file_at_scale(path, width, -1, true).ok()?;
    if let Some(stamp) = stamp {
        write_cached_thumb(path, width, stamp, &pixbuf);
    }
    Some(raw_from_pixbuf(&pixbuf))
}

type Key = (PathBuf, i32, u64, i64, u32);

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
        let stamp = file_stamp(path);
        let (size, mtime_secs, mtime_nanos) = stamp
            .map(|stamp| (stamp.size, stamp.mtime_secs, stamp.mtime_nanos))
            .unwrap_or((0, 0, 0));
        let key = (path.to_path_buf(), width, size, mtime_secs, mtime_nanos);

        if let Some(tex) = self.cache.borrow_mut().get(&key) {
            done(&tex);
            return;
        }

        let cache = self.cache.clone();
        let path = key.0.clone();
        glib::spawn_future_local(async move {
            let job_cancel = cancel.clone();
            let job_path = path.clone();
            let raw =
                gio::spawn_blocking(move || decode(&job_path, width, stamp, &job_cancel)).await;
            let Ok(Some(raw)) = raw else {
                return;
            };
            let texture = raw.into_texture();
            cache.borrow_mut().insert(
                (path, width, size, mtime_secs, mtime_nanos),
                texture.clone(),
            );
            if !cancel.load(Ordering::Relaxed) {
                done(&texture);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_file_changes_when_the_source_changes() {
        let base = Path::new("/tmp/walltag-cache");
        let source = Path::new("/pics/a.png");
        let stamp = FileStamp {
            size: 10,
            mtime_secs: 1,
            mtime_nanos: 0,
        };
        let first = thumb_file(base, source, 256, stamp);
        let replaced = thumb_file(base, source, 256, FileStamp { size: 11, ..stamp });
        assert_ne!(first, replaced);
        let name = first.file_name().unwrap().to_string_lossy();
        assert!(name.ends_with(".png"));
        assert_eq!(
            first
                .parent()
                .and_then(|p| p.file_name())
                .unwrap()
                .to_string_lossy(),
            &name[..2]
        );
    }
}
