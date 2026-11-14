//! On-disk average colors for the hue sort, keyed by wallpaper root, relative path,
//! mtime, and size.
//!
//! A hit skips decoding on the next launch. A new file, or a file whose mtime or size
//! changed, is sampled again. Entries for wallpapers that left the folder are dropped
//! the next time the cache is planned.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ui::cache_paths::{cache_dir, stable_hash};
use crate::ui::color::AvgColor;
use crate::ui::filter::FileStat;

const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CachedColor {
    Sampled(AvgColor),
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ColorEntry {
    size: u64,
    mtime_secs: i64,
    mtime_nanos: u32,
    hue: f32,
    saturation: f32,
    lightness: f32,
    unreadable: bool,
}

#[derive(Serialize, Deserialize)]
struct ColorFile {
    version: u32,
    entries: HashMap<String, ColorEntry>,
}

/// Colors remembered for one wallpaper root.
pub struct ColorCache {
    path: PathBuf,
    entries: HashMap<String, ColorEntry>,
    dirty: bool,
}

/// Cache hits plus the files that still need sampling.
pub struct ColorPlan {
    pub colors: HashMap<String, AvgColor>,
    pub unreadable: HashSet<String>,
    pub misses: Vec<(String, PathBuf)>,
}

impl ColorCache {
    /// Loads the cache for `wallpaper_root`. A missing or unreadable file yields an empty cache.
    pub fn load(wallpaper_root: &Path) -> Self {
        match colors_path(wallpaper_root) {
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
            .and_then(|raw| serde_json::from_slice::<ColorFile>(&raw).ok())
            .filter(|file| file.version == VERSION)
            .map(|file| file.entries)
            .unwrap_or_default();
        Self {
            path,
            entries,
            dirty: false,
        }
    }

    /// Every stored color, without checking mtime. The gallery paints these immediately
    /// and replaces them if the worker finds a mismatch.
    pub fn unvalidated(&self) -> (HashMap<String, AvgColor>, HashSet<String>) {
        let mut colors = HashMap::new();
        let mut unreadable = HashSet::new();
        for (key, entry) in &self.entries {
            if entry.unreadable {
                unreadable.insert(key.clone());
            } else {
                colors.insert(key.clone(), entry.color());
            }
        }
        (colors, unreadable)
    }

    pub fn lookup(&self, key: &str, stat: &FileStat) -> Option<CachedColor> {
        let entry = self.entries.get(key)?;
        let (secs, nanos) = mtime_parts(stat.mtime?)?;
        if entry.size != stat.size || entry.mtime_secs != secs || entry.mtime_nanos != nanos {
            return None;
        }
        Some(if entry.unreadable {
            CachedColor::Unreadable
        } else {
            CachedColor::Sampled(entry.color())
        })
    }

    pub fn insert(&mut self, key: &str, stat: &FileStat, color: Option<AvgColor>) {
        let Some((secs, nanos)) = stat.mtime.and_then(mtime_parts) else {
            return;
        };
        let entry = match color {
            Some(color) => ColorEntry {
                size: stat.size,
                mtime_secs: secs,
                mtime_nanos: nanos,
                hue: color.hue,
                saturation: color.saturation,
                lightness: color.lightness,
                unreadable: false,
            },
            None => ColorEntry {
                size: stat.size,
                mtime_secs: secs,
                mtime_nanos: nanos,
                hue: 0.0,
                saturation: 0.0,
                lightness: 0.0,
                unreadable: true,
            },
        };
        if self.entries.get(key) != Some(&entry) {
            self.entries.insert(key.to_owned(), entry);
            self.dirty = true;
        }
    }

    /// Drops colors for wallpapers that are no longer in the folder.
    pub fn retain_present(&mut self, keys: &HashSet<&str>) {
        let before = self.entries.len();
        self.entries.retain(|key, _| keys.contains(key.as_str()));
        if self.entries.len() != before {
            self.dirty = true;
        }
    }

    /// Splits `items` into cache hits and files that still need a sample.
    /// Files with no stat are omitted from both: they cannot be invalidated later.
    pub fn plan(
        &self,
        items: &[(String, PathBuf)],
        stats: &HashMap<String, FileStat>,
    ) -> ColorPlan {
        let mut colors = HashMap::new();
        let mut unreadable = HashSet::new();
        let mut misses = Vec::new();
        for (key, path) in items {
            let Some(stat) = stats.get(key) else {
                continue;
            };
            match self.lookup(key, stat) {
                Some(CachedColor::Sampled(color)) => {
                    colors.insert(key.clone(), color);
                }
                Some(CachedColor::Unreadable) => {
                    unreadable.insert(key.clone());
                }
                None => misses.push((key.clone(), path.clone())),
            }
        }
        ColorPlan {
            colors,
            unreadable,
            misses,
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
            tracing::warn!("Could not create color cache directory: {err}");
            return;
        }
        let file = ColorFile {
            version: VERSION,
            entries: self.entries.clone(),
        };
        match serde_json::to_vec(&file) {
            Ok(bytes) => match std::fs::write(&self.path, bytes) {
                Ok(()) => self.dirty = false,
                Err(err) => {
                    tracing::warn!("Could not write color cache {}: {err}", self.path.display())
                }
            },
            Err(err) => tracing::warn!("Could not serialize color cache: {err}"),
        }
    }
}

impl ColorEntry {
    fn color(&self) -> AvgColor {
        AvgColor {
            hue: self.hue,
            saturation: self.saturation,
            lightness: self.lightness,
        }
    }
}

fn mtime_parts(mtime: SystemTime) -> Option<(i64, u32)> {
    let dur = mtime.duration_since(UNIX_EPOCH).ok()?;
    Some((dur.as_secs() as i64, dur.subsec_nanos()))
}

fn colors_path(wallpaper_root: &Path) -> Option<PathBuf> {
    let hash = stable_hash(&[
        b"walltag-colors-v1",
        wallpaper_root.as_os_str().as_encoded_bytes(),
    ]);
    Some(
        cache_dir()?
            .join("colors")
            .join(format!("{hash:032x}.json")),
    )
}

/// Drops entries that left the folder, then returns hits and files still to sample.
pub fn plan_and_prune(
    wallpaper_root: &Path,
    items: &[(String, PathBuf)],
    stats: &HashMap<String, FileStat>,
) -> ColorPlan {
    let mut cache = ColorCache::load(wallpaper_root);
    let present: HashSet<&str> = items.iter().map(|(key, _)| key.as_str()).collect();
    cache.retain_present(&present);
    let plan = cache.plan(items, stats);
    let misses = plan.misses.len();
    let hits = plan.colors.len() + plan.unreadable.len();
    cache.save();
    if misses > 0 {
        tracing::debug!("Wallpaper colors: {hits} from disk cache, {misses} to sample");
    }
    plan
}

/// Stores newly sampled colors. `None` in `found` is recorded as unreadable.
/// Does not drop other keys, so a newer scan's entries survive if this write lands late.
pub fn persist_color_samples(
    wallpaper_root: &Path,
    stats: &HashMap<String, FileStat>,
    requested: &[(String, PathBuf)],
    found: &HashMap<String, AvgColor>,
) {
    let mut cache = ColorCache::load(wallpaper_root);
    for (key, _) in requested {
        let Some(stat) = stats.get(key) else {
            continue;
        };
        cache.insert(key, stat, found.get(key).copied());
    }
    cache.save();
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

    fn red() -> AvgColor {
        AvgColor {
            hue: 0.0,
            saturation: 1.0,
            lightness: 0.5,
        }
    }

    #[test]
    fn lookup_requires_matching_size_and_mtime() {
        let dir = TempDir::new().unwrap();
        let mut cache = ColorCache::open(dir.path().join("colors.json"));
        let stat = stat_at(20, 10);
        cache.insert("a.png", &stat, Some(red()));
        assert!(matches!(
            cache.lookup("a.png", &stat),
            Some(CachedColor::Sampled(_))
        ));
        assert_eq!(cache.lookup("a.png", &stat_at(21, 10)), None);
        assert_eq!(cache.lookup("a.png", &stat_at(20, 11)), None);
        assert_eq!(cache.lookup("missing.png", &stat), None);
    }

    #[test]
    fn unreadable_stays_cached_until_the_file_changes() {
        let dir = TempDir::new().unwrap();
        let mut cache = ColorCache::open(dir.path().join("colors.json"));
        let stat = stat_at(4, 2);
        cache.insert("bad.png", &stat, None);
        assert_eq!(
            cache.lookup("bad.png", &stat),
            Some(CachedColor::Unreadable)
        );
        assert_eq!(cache.lookup("bad.png", &stat_at(5, 2)), None);
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("colors.json");
        let mut cache = ColorCache::open(path.clone());
        cache.insert("cats/nap.jpg", &stat_at(4, 3), Some(red()));
        cache.retain_present(&HashSet::from(["cats/nap.jpg"]));
        cache.save();

        let loaded = ColorCache::open(path);
        match loaded.lookup("cats/nap.jpg", &stat_at(4, 3)) {
            Some(CachedColor::Sampled(color)) => assert_eq!(color, red()),
            other => panic!("expected sampled color, got {other:?}"),
        }
        assert!(loaded.unvalidated().0.contains_key("cats/nap.jpg"));
    }

    #[test]
    fn plan_reuses_unchanged_files_and_misses_new_or_edited_ones() {
        let dir = TempDir::new().unwrap();
        let mut cache = ColorCache::open(dir.path().join("colors.json"));
        cache.insert("old.png", &stat_at(10, 1), Some(red()));
        cache.insert("edited.png", &stat_at(10, 1), Some(red()));
        cache.insert("gone.png", &stat_at(10, 1), Some(red()));

        let mut stats = HashMap::new();
        stats.insert("old.png".into(), stat_at(10, 1));
        stats.insert("edited.png".into(), stat_at(10, 2));
        stats.insert("new.png".into(), stat_at(8, 4));
        let items: Vec<(String, PathBuf)> = vec![
            ("old.png".into(), PathBuf::from("/w/old.png")),
            ("edited.png".into(), PathBuf::from("/w/edited.png")),
            ("new.png".into(), PathBuf::from("/w/new.png")),
        ];
        let present: HashSet<&str> = items.iter().map(|(key, _)| key.as_str()).collect();
        cache.retain_present(&present);
        let plan = cache.plan(&items, &stats);

        assert!(plan.colors.contains_key("old.png"));
        assert!(!plan.colors.contains_key("edited.png"));
        assert!(!plan.colors.contains_key("new.png"));
        assert!(cache.lookup("gone.png", &stat_at(10, 1)).is_none());
        let miss_keys: HashSet<&str> = plan.misses.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(miss_keys, HashSet::from(["edited.png", "new.png"]));
    }
}
