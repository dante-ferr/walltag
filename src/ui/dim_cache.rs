//! On-disk image sizes, keyed by wallpaper root plus relative path, mtime, and size.
//!
//! A hit skips `Pixbuf::file_info` on the next launch. Entries are dropped when the
//! file's mtime or size no longer matches.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ui::cache_paths::{cache_dir, stable_hash};
use crate::ui::filter::{probe_stats, FileStat};
use crate::ui::thumbs::probe_dimensions;

const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct DimEntry {
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
    w: i32,
    h: i32,
}

#[derive(Serialize, Deserialize)]
struct DimFile {
    version: u32,
    entries: HashMap<String, DimEntry>,
}

/// Sizes remembered for one wallpaper root.
pub struct DimensionCache {
    path: PathBuf,
    entries: HashMap<String, DimEntry>,
    dirty: bool,
}

impl DimensionCache {
    /// Loads the cache for `wallpaper_root`. A missing or unreadable file yields an empty cache.
    pub fn load(wallpaper_root: &Path) -> Self {
        match dimensions_path(wallpaper_root) {
            Some(path) => Self::open(path),
            None => Self {
                path: PathBuf::new(),
                entries: HashMap::new(),
                dirty: false,
            },
        }
    }

    pub(crate) fn open(path: PathBuf) -> Self {
        let entries = std::fs::read(path.as_path())
            .ok()
            .and_then(|raw| serde_json::from_slice::<DimFile>(&raw).ok())
            .filter(|file| file.version == VERSION)
            .map(|file| file.entries)
            .unwrap_or_default();
        Self {
            path,
            entries,
            dirty: false,
        }
    }

    /// Every stored size, without checking mtime. The gallery paints these immediately
    /// and replaces them if the worker finds a mismatch.
    pub fn unvalidated(&self) -> HashMap<String, (i32, i32)> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.w > 0 && entry.h > 0)
            .map(|(key, entry)| (key.clone(), (entry.w, entry.h)))
            .collect()
    }

    pub fn lookup(&self, key: &str, stat: &FileStat) -> Option<(i32, i32)> {
        let entry = self.entries.get(key)?;
        let (secs, nanos) = mtime_parts(stat.mtime?)?;
        (entry.size == stat.size
            && entry.mtime_secs == secs
            && entry.mtime_nanos == nanos
            && entry.w > 0
            && entry.h > 0)
            .then_some((entry.w, entry.h))
    }

    pub fn insert(&mut self, key: &str, stat: &FileStat, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        let Some((secs, nanos)) = stat.mtime.and_then(mtime_parts) else {
            return;
        };
        let entry = DimEntry {
            size: stat.size,
            mtime_secs: secs,
            mtime_nanos: nanos,
            w,
            h,
        };
        if self.entries.get(key) != Some(&entry) {
            self.entries.insert(key.to_owned(), entry);
            self.dirty = true;
        }
    }

    /// Drops sizes for wallpapers that are no longer in the folder.
    pub fn retain_present(&mut self, keys: &HashSet<&str>) {
        let before = self.entries.len();
        self.entries.retain(|key, _| keys.contains(key.as_str()));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(parent) = self.path.parent() else {
            return;
        };
        if let Err(err) = std::fs::create_dir_all(parent) {
            tracing::warn!("Could not create dimension cache directory: {err}");
            return;
        }
        let file = DimFile {
            version: VERSION,
            entries: self.entries.clone(),
        };
        match serde_json::to_vec(&file) {
            Ok(bytes) => match std::fs::write(&self.path, bytes) {
                Ok(()) => self.dirty = false,
                Err(err) => tracing::warn!(
                    "Could not write dimension cache {}: {err}",
                    self.path.display()
                ),
            },
            Err(err) => tracing::warn!("Could not serialize dimension cache: {err}"),
        }
    }
}

fn mtime_parts(mtime: SystemTime) -> Option<(i64, u32)> {
    let dur = mtime.duration_since(UNIX_EPOCH).ok()?;
    Some((dur.as_secs() as i64, dur.subsec_nanos()))
}

fn dimensions_path(wallpaper_root: &Path) -> Option<PathBuf> {
    let hash = stable_hash(&[
        b"walltag-dims-v1",
        wallpaper_root.as_os_str().as_encoded_bytes(),
    ]);
    Some(
        cache_dir()?
            .join("dimensions")
            .join(format!("{hash:032x}.json")),
    )
}

/// Stats every file, reuses cached sizes when mtime and size match, and reads headers
/// only for the rest. Writes the updated cache before returning.
pub fn resolve_dimensions(
    wallpaper_root: &Path,
    items: &[(String, PathBuf)],
) -> (HashMap<String, (i32, i32)>, HashMap<String, FileStat>) {
    let stats = probe_stats(items);
    let mut cache = DimensionCache::load(wallpaper_root);
    let present: HashSet<&str> = items.iter().map(|(key, _)| key.as_str()).collect();
    cache.retain_present(&present);

    let mut dims = HashMap::with_capacity(items.len());
    let mut misses = Vec::new();
    for (key, path) in items {
        if let Some(dim) = stats.get(key).and_then(|stat| cache.lookup(key, stat)) {
            dims.insert(key.clone(), dim);
            continue;
        }
        misses.push((key.clone(), path.clone()));
    }

    let probed = probe_dimensions(misses);
    for (key, wh) in &probed {
        if let Some(stat) = stats.get(key) {
            cache.insert(key, stat, wh.0, wh.1);
        }
        dims.insert(key.clone(), *wh);
    }
    let probed_len = probed.len();
    cache.save();
    if probed_len > 0 {
        let cached = dims.len().saturating_sub(probed_len);
        tracing::debug!("Wallpaper sizes: {cached} from disk cache, {probed_len} read from files");
    }
    (dims, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    fn stat_at(size: u64, secs: u64) -> FileStat {
        FileStat {
            mtime: Some(UNIX_EPOCH + Duration::from_secs(secs)),
            size,
        }
    }

    #[test]
    fn lookup_requires_matching_size_and_mtime() {
        let dir = TempDir::new().unwrap();
        let mut cache = DimensionCache::open(dir.path().join("dims.json"));
        let stat = stat_at(20, 10);
        cache.insert("a.png", &stat, 1920, 1080);
        assert_eq!(cache.lookup("a.png", &stat), Some((1920, 1080)));
        assert_eq!(cache.lookup("a.png", &stat_at(21, 10)), None);
        assert_eq!(cache.lookup("a.png", &stat_at(20, 11)), None);
        assert_eq!(cache.lookup("missing.png", &stat), None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("dims.json");
        let mut cache = DimensionCache::open(path.clone());
        cache.insert("cats/nap.jpg", &stat_at(4, 3), 800, 600);
        cache.retain_present(&HashSet::from(["cats/nap.jpg"]));
        cache.save();

        let loaded = DimensionCache::open(path);
        assert_eq!(
            loaded.lookup("cats/nap.jpg", &stat_at(4, 3)),
            Some((800, 600))
        );
        assert_eq!(loaded.unvalidated().get("cats/nap.jpg"), Some(&(800, 600)));
    }

    #[test]
    fn retain_drops_wallpapers_that_left_the_folder() {
        let dir = TempDir::new().unwrap();
        let mut cache = DimensionCache::open(dir.path().join("dims.json"));
        cache.insert("keep.png", &stat_at(1, 1), 10, 10);
        cache.insert("gone.png", &stat_at(1, 1), 10, 10);
        cache.retain_present(&HashSet::from(["keep.png"]));
        assert!(cache.lookup("keep.png", &stat_at(1, 1)).is_some());
        assert!(cache.lookup("gone.png", &stat_at(1, 1)).is_none());
    }
}
