//! Gallery search, filter, and group-by helpers.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::metadata::MetadataStore;
use crate::scan::WallpaperEntry;
use crate::ui::color::{group_hue_key, AvgColor, HueKey};

pub use crate::config::{GroupBy, SortBy};

/// Label for the root folder group (files directly under the wallpaper root).
pub const ROOT_FOLDER_LABEL: &str = "Root";
/// Label for wallpapers with no tags when grouping by tag.
pub const UNTAGGED_LABEL: &str = "Untagged";

/// Active search / filter / group / sort state (persisted via `AppConfig`).
#[derive(Debug, Clone, Default)]
pub struct GalleryFilter {
    pub query: String,
    pub group_by: GroupBy,
    pub sort_by: SortBy,
    /// When set, only wallpapers that have this tag.
    pub tag: Option<String>,
    /// When set, only wallpapers in this folder (`parent_relative`, empty = root).
    pub folder: Option<String>,
}

#[derive(Debug, Clone)]
pub enum GalleryRow {
    Header(String),
    Wallpaper(WallpaperEntry),
}

/// Folder path as shown in the UI (`Root` for the wallpaper root).
pub fn folder_label(parent_relative: &str) -> String {
    if parent_relative.is_empty() {
        ROOT_FOLDER_LABEL.to_owned()
    } else {
        parent_relative.to_owned()
    }
}

/// Sorted unique folder keys (`parent_relative`) present in `entries`.
pub fn collect_folders(entries: &[WallpaperEntry]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for e in entries {
        set.insert(e.parent_relative.clone());
    }
    set.into_iter().collect()
}

/// Sorted unique tags across metadata for the given entries.
pub fn collect_tags(entries: &[WallpaperEntry], metadata: &MetadataStore) -> Vec<String> {
    let mut set = BTreeSet::new();
    for e in entries {
        for tag in metadata.record_for_key(&e.relative_key).tags {
            set.insert(tag);
        }
    }
    set.into_iter().collect()
}

/// Filesystem facts used by the date and size sorts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileStat {
    pub mtime: Option<SystemTime>,
    pub size: u64,
}

/// Per-wallpaper data the sort modes need, keyed by `relative_key`. Stats come from one
/// `stat` per file at scan time. Average colors are sampled lazily the first time the
/// hue sort is used and stay cached until the file changes or the folder is rescanned.
#[derive(Debug, Clone, Default)]
pub struct SortData {
    pub stats: HashMap<String, FileStat>,
    pub colors: HashMap<String, AvgColor>,
    /// Keys whose image could not be decoded. They are not sampled again until rescan.
    pub unreadable: HashSet<String>,
}

impl SortData {
    /// Entries with no cached average color that are still worth sampling.
    pub fn missing_colors(&self, entries: &[WallpaperEntry]) -> Vec<(String, PathBuf)> {
        entries
            .iter()
            .filter(|e| {
                !self.colors.contains_key(&e.relative_key)
                    && !self.unreadable.contains(&e.relative_key)
            })
            .map(|e| (e.relative_key.clone(), e.absolute.clone()))
            .collect()
    }

    /// Stores sampled colors. Requested keys with no result are marked unreadable.
    pub fn record_colors(
        &mut self,
        requested: &[(String, PathBuf)],
        found: HashMap<String, AvgColor>,
    ) {
        for (key, _) in requested {
            if !found.contains_key(key) {
                self.unreadable.insert(key.clone());
            }
        }
        self.colors.extend(found);
    }

    /// Replaces the stats and keeps cached colors only for files whose stat is unchanged.
    pub fn replace_stats(&mut self, stats: HashMap<String, FileStat>) {
        let old = std::mem::take(&mut self.stats);
        self.unreadable.clear();
        self.colors
            .retain(|key, _| matches!((old.get(key), stats.get(key)), (Some(a), Some(b)) if a == b));
        self.stats = stats;
    }

    fn stat(&self, key: &str) -> Option<&FileStat> {
        self.stats.get(key)
    }
}

/// Reads mtime and size for each `(key, path)`. Files that cannot be read are omitted.
pub fn probe_stats(items: &[(String, PathBuf)]) -> HashMap<String, FileStat> {
    items
        .iter()
        .filter_map(|(key, path)| {
            let meta = std::fs::metadata(path).ok()?;
            Some((
                key.clone(),
                FileStat {
                    mtime: meta.modified().ok(),
                    size: meta.len(),
                },
            ))
        })
        .collect()
}

/// Compares options so that `None` always sorts last, whatever the direction.
fn cmp_opt_last<T: Ord>(a: Option<T>, b: Option<T>, descending: bool) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => {
            if descending {
                b.cmp(&a)
            } else {
                a.cmp(&b)
            }
        }
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn name_cmp(a: &WallpaperEntry, b: &WallpaperEntry) -> Ordering {
    a.file_name
        .to_lowercase()
        .cmp(&b.file_name.to_lowercase())
        .then_with(|| a.relative_key.cmp(&b.relative_key))
}

/// Orders two wallpapers under `sort`. Ties fall back to name A to Z.
pub fn compare_entries(
    a: &WallpaperEntry,
    b: &WallpaperEntry,
    sort: SortBy,
    data: &SortData,
) -> Ordering {
    let primary = match sort {
        SortBy::NameAsc => return name_cmp(a, b),
        SortBy::NameDesc => return name_cmp(b, a),
        SortBy::Path => return a.relative_key.cmp(&b.relative_key),
        SortBy::DateNewest | SortBy::DateOldest => cmp_opt_last(
            data.stat(&a.relative_key).and_then(|s| s.mtime),
            data.stat(&b.relative_key).and_then(|s| s.mtime),
            sort == SortBy::DateNewest,
        ),
        SortBy::SizeLargest | SortBy::SizeSmallest => cmp_opt_last(
            data.stat(&a.relative_key).map(|s| s.size),
            data.stat(&b.relative_key).map(|s| s.size),
            sort == SortBy::SizeLargest,
        ),
        SortBy::Hue => {
            let ka = HueKey::new(data.colors.get(&a.relative_key));
            let kb = HueKey::new(data.colors.get(&b.relative_key));
            ka.partial_cmp(&kb).unwrap_or(Ordering::Equal)
        }
    };
    primary.then_with(|| name_cmp(a, b))
}

/// What a group sorts by: its label for name and path sorts, otherwise an aggregate of
/// its members (newest/oldest mtime, largest/smallest file, mean hue).
enum GroupKey {
    Label(String),
    Time(Option<SystemTime>),
    Size(Option<u64>),
    Hue(HueKey),
}

struct Group<'a> {
    label: String,
    items: Vec<&'a WallpaperEntry>,
}

/// Group label as a sort key. The root group sorts before every named folder.
fn label_key(label: &str) -> String {
    if label == ROOT_FOLDER_LABEL {
        String::new()
    } else {
        label.to_lowercase()
    }
}

fn group_key(group: &Group, sort: SortBy, data: &SortData) -> GroupKey {
    let mtimes = || {
        group
            .items
            .iter()
            .filter_map(|e| data.stat(&e.relative_key).and_then(|s| s.mtime))
    };
    let sizes = || {
        group
            .items
            .iter()
            .filter_map(|e| data.stat(&e.relative_key).map(|s| s.size))
    };
    match sort {
        SortBy::NameAsc | SortBy::NameDesc | SortBy::Path => {
            GroupKey::Label(label_key(&group.label))
        }
        SortBy::DateNewest => GroupKey::Time(mtimes().max()),
        SortBy::DateOldest => GroupKey::Time(mtimes().min()),
        SortBy::SizeLargest => GroupKey::Size(sizes().max()),
        SortBy::SizeSmallest => GroupKey::Size(sizes().min()),
        SortBy::Hue => GroupKey::Hue(group_hue_key(
            group.items.iter().map(|e| data.colors.get(&e.relative_key)),
        )),
    }
}

fn cmp_group_keys(a: &GroupKey, b: &GroupKey, sort: SortBy) -> Ordering {
    match (a, b) {
        (GroupKey::Label(a), GroupKey::Label(b)) => {
            if sort == SortBy::NameDesc {
                b.cmp(a)
            } else {
                a.cmp(b)
            }
        }
        (GroupKey::Time(a), GroupKey::Time(b)) => cmp_opt_last(*a, *b, sort == SortBy::DateNewest),
        (GroupKey::Size(a), GroupKey::Size(b)) => cmp_opt_last(*a, *b, sort == SortBy::SizeLargest),
        (GroupKey::Hue(a), GroupKey::Hue(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        _ => Ordering::Equal,
    }
}

/// Filters, sorts, and optionally groups wallpapers for the gallery. `filter.sort_by`
/// orders the wallpapers inside each group (or the flat list) and the groups themselves.
pub fn build_rows(
    entries: &[WallpaperEntry],
    metadata: &MetadataStore,
    filter: &GalleryFilter,
    sort_data: &SortData,
) -> Vec<GalleryRow> {
    let query = filter.query.to_lowercase();
    let sort = filter.sort_by;
    let mut filtered: Vec<&WallpaperEntry> = entries
        .iter()
        .filter(|wp| {
            if !query.is_empty() && !wp.file_name.to_lowercase().contains(&query) {
                return false;
            }
            if let Some(folder) = &filter.folder {
                if &wp.parent_relative != folder {
                    return false;
                }
            }
            if let Some(tag) = &filter.tag {
                let tags = metadata.record_for_key(&wp.relative_key).tags;
                if !tags.iter().any(|t| t == tag) {
                    return false;
                }
            }
            true
        })
        .collect();
    filtered.sort_by(|a, b| compare_entries(a, b, sort, sort_data));

    match filter.group_by {
        GroupBy::None => filtered
            .into_iter()
            .cloned()
            .map(GalleryRow::Wallpaper)
            .collect(),
        GroupBy::Folder => {
            let mut groups: BTreeMap<String, Vec<&WallpaperEntry>> = BTreeMap::new();
            for wp in filtered {
                groups
                    .entry(folder_label(&wp.parent_relative))
                    .or_default()
                    .push(wp);
            }
            flatten_groups(groups, None, sort, sort_data)
        }
        GroupBy::Tag => {
            let mut groups: BTreeMap<String, Vec<&WallpaperEntry>> = BTreeMap::new();
            for wp in filtered {
                let tags = metadata.record_for_key(&wp.relative_key).tags;
                let label = tags
                    .first()
                    .cloned()
                    .unwrap_or_else(|| UNTAGGED_LABEL.to_owned());
                groups.entry(label).or_default().push(wp);
            }
            flatten_groups(groups, Some(UNTAGGED_LABEL), sort, sort_data)
        }
    }
}

/// Orders `groups` under `sort` (items inside are already sorted). The `last` group, if
/// any, always comes last.
fn flatten_groups(
    mut groups: BTreeMap<String, Vec<&WallpaperEntry>>,
    last: Option<&str>,
    sort: SortBy,
    data: &SortData,
) -> Vec<GalleryRow> {
    let tail = last.and_then(|label| groups.remove(label).map(|items| (label.to_owned(), items)));
    let mut keyed: Vec<(GroupKey, Group)> = groups
        .into_iter()
        .map(|(label, items)| {
            let group = Group { label, items };
            (group_key(&group, sort, data), group)
        })
        .collect();
    // The map iterates by label, and the sort is stable, so equal keys keep label order.
    keyed.sort_by(|(a, _), (b, _)| cmp_group_keys(a, b, sort));

    let mut rows = Vec::new();
    for (_, group) in keyed {
        append_group(&mut rows, group.label, group.items);
    }
    if let Some((label, items)) = tail {
        append_group(&mut rows, label, items);
    }
    rows
}

fn append_group(rows: &mut Vec<GalleryRow>, title: String, items: Vec<&WallpaperEntry>) {
    if items.is_empty() {
        return;
    }
    rows.push(GalleryRow::Header(title));
    rows.extend(items.into_iter().cloned().map(GalleryRow::Wallpaper));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::MetadataStore;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn entry(rel: &str, parent: &str, name: &str) -> WallpaperEntry {
        WallpaperEntry {
            absolute: PathBuf::from("/w").join(rel),
            relative_key: rel.into(),
            file_name: name.into(),
            parent_relative: parent.into(),
        }
    }

    fn gray(l: f32) -> AvgColor {
        AvgColor { hue: 0.0, saturation: 0.0, lightness: l }
    }

    fn hued(h: f32) -> AvgColor {
        AvgColor { hue: h, saturation: 0.8, lightness: 0.5 }
    }

    fn keys(rows: &[GalleryRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                GalleryRow::Header(t) => format!("#{t}"),
                GalleryRow::Wallpaper(w) => w.relative_key.clone(),
            })
            .collect()
    }

    fn sorted(
        entries: &[WallpaperEntry],
        group_by: GroupBy,
        sort_by: SortBy,
        data: &SortData,
    ) -> Vec<String> {
        let dir = TempDir::new().unwrap();
        let meta = MetadataStore::open(dir.path().join("m.json")).unwrap();
        keys(&build_rows(
            entries,
            &meta,
            &GalleryFilter { group_by, sort_by, ..Default::default() },
            data,
        ))
    }

    fn stat(secs: u64, size: u64) -> FileStat {
        FileStat {
            mtime: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs)),
            size,
        }
    }

    #[test]
    fn name_and_path_sorts() {
        let entries = vec![
            entry("b/zed.png", "b", "zed.png"),
            entry("a/Yak.png", "a", "Yak.png"),
            entry("c/apple.png", "c", "apple.png"),
        ];
        let d = SortData::default();
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::NameAsc, &d),
            ["c/apple.png", "a/Yak.png", "b/zed.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::NameDesc, &d),
            ["b/zed.png", "a/Yak.png", "c/apple.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::Path, &d),
            ["a/Yak.png", "b/zed.png", "c/apple.png"]
        );
        // Groups follow the same direction as the images.
        assert_eq!(
            sorted(&entries, GroupBy::Folder, SortBy::NameDesc, &d),
            ["#c", "c/apple.png", "#b", "b/zed.png", "#a", "a/Yak.png"]
        );
    }

    #[test]
    fn date_and_size_sorts_put_missing_last() {
        let entries = vec![
            entry("old.png", "", "old.png"),
            entry("new.png", "", "new.png"),
            entry("none.png", "", "none.png"),
        ];
        let mut d = SortData::default();
        d.stats.insert("old.png".into(), stat(100, 5_000));
        d.stats.insert("new.png".into(), stat(900, 1_000));
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::DateNewest, &d),
            ["new.png", "old.png", "none.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::DateOldest, &d),
            ["old.png", "new.png", "none.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::SizeLargest, &d),
            ["old.png", "new.png", "none.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::SizeSmallest, &d),
            ["new.png", "old.png", "none.png"]
        );
    }

    #[test]
    fn groups_sort_by_newest_member_for_date() {
        let entries = vec![
            entry("a/1.png", "a", "1.png"),
            entry("a/2.png", "a", "2.png"),
            entry("b/3.png", "b", "3.png"),
        ];
        let mut d = SortData::default();
        d.stats.insert("a/1.png".into(), stat(100, 1));
        d.stats.insert("a/2.png".into(), stat(200, 1));
        d.stats.insert("b/3.png".into(), stat(500, 1));
        assert_eq!(
            sorted(&entries, GroupBy::Folder, SortBy::DateNewest, &d),
            ["#b", "b/3.png", "#a", "a/2.png", "a/1.png"]
        );
        assert_eq!(
            sorted(&entries, GroupBy::Folder, SortBy::DateOldest, &d),
            ["#a", "a/1.png", "a/2.png", "#b", "b/3.png"]
        );
    }

    #[test]
    fn hue_sort_orders_wheel_then_grayscale_then_unsampled() {
        let entries = vec![
            entry("unsampled.png", "", "unsampled.png"),
            entry("white.png", "", "white.png"),
            entry("blue.png", "", "blue.png"),
            entry("black.png", "", "black.png"),
            entry("red.png", "", "red.png"),
            entry("green.png", "", "green.png"),
        ];
        let mut d = SortData::default();
        d.colors.insert("white.png".into(), gray(0.95));
        d.colors.insert("blue.png".into(), hued(240.0));
        d.colors.insert("black.png".into(), gray(0.05));
        d.colors.insert("red.png".into(), hued(5.0));
        d.colors.insert("green.png".into(), hued(120.0));
        assert_eq!(
            sorted(&entries, GroupBy::None, SortBy::Hue, &d),
            ["red.png", "green.png", "blue.png", "black.png", "white.png", "unsampled.png"]
        );
    }

    #[test]
    fn hue_groups_sort_by_mean_hue_and_neutral_groups_last() {
        let entries = vec![
            entry("gray/g.png", "gray", "g.png"),
            entry("blue/b.png", "blue", "b.png"),
            entry("red/r.png", "red", "r.png"),
        ];
        let mut d = SortData::default();
        d.colors.insert("gray/g.png".into(), gray(0.5));
        d.colors.insert("blue/b.png".into(), hued(230.0));
        d.colors.insert("red/r.png".into(), hued(10.0));
        assert_eq!(
            sorted(&entries, GroupBy::Folder, SortBy::Hue, &d),
            ["#red", "red/r.png", "#blue", "blue/b.png", "#gray", "gray/g.png"]
        );
    }

    #[test]
    fn untagged_group_stays_last_under_every_sort() {
        let entries = vec![
            entry("a.png", "", "a.png"),
            entry("z.png", "", "z.png"),
        ];
        let dir = TempDir::new().unwrap();
        let mut meta = MetadataStore::open(dir.path().join("m.json")).unwrap();
        meta.set_tags("z.png", vec!["alpha".into()]);
        for sort_by in SortBy::ALL {
            let rows = build_rows(
                &entries,
                &meta,
                &GalleryFilter { group_by: GroupBy::Tag, sort_by, ..Default::default() },
                &SortData::default(),
            );
            assert_eq!(keys(&rows), ["#alpha", "z.png", "#Untagged", "a.png"], "{sort_by:?}");
        }
    }

    #[test]
    fn replace_stats_keeps_colors_only_for_unchanged_files() {
        let mut d = SortData::default();
        d.stats.insert("same.png".into(), stat(1, 10));
        d.stats.insert("edited.png".into(), stat(1, 10));
        d.colors.insert("same.png".into(), hued(10.0));
        d.colors.insert("edited.png".into(), hued(20.0));
        let mut next = HashMap::new();
        next.insert("same.png".to_owned(), stat(1, 10));
        next.insert("edited.png".to_owned(), stat(2, 10));
        d.replace_stats(next);
        assert!(d.colors.contains_key("same.png"));
        assert!(!d.colors.contains_key("edited.png"));
    }

    #[test]
    fn groups_by_folder_with_root_label() {
        let entries = vec![
            entry("a.png", "", "a.png"),
            entry("cats/b.png", "cats", "b.png"),
        ];
        let dir = TempDir::new().unwrap();
        let meta = MetadataStore::open(dir.path().join("m.json")).unwrap();
        let rows = build_rows(
            &entries,
            &meta,
            &GalleryFilter {
                group_by: GroupBy::Folder,
                ..Default::default()
            },
            &SortData::default(),
        );
        assert!(matches!(&rows[0], GalleryRow::Header(t) if t == "Root"));
        assert!(matches!(&rows[1], GalleryRow::Wallpaper(w) if w.relative_key == "a.png"));
        assert!(matches!(&rows[2], GalleryRow::Header(t) if t == "cats"));
        assert!(matches!(&rows[3], GalleryRow::Wallpaper(w) if w.relative_key == "cats/b.png"));
    }

    #[test]
    fn groups_by_first_tag_and_untagged() {
        let entries = vec![
            entry("a.png", "", "a.png"),
            entry("b.png", "", "b.png"),
        ];
        let dir = TempDir::new().unwrap();
        let mut meta = MetadataStore::open(dir.path().join("m.json")).unwrap();
        meta.set_tags("a.png", vec!["night".into(), "dark".into()]);
        let rows = build_rows(
            &entries,
            &meta,
            &GalleryFilter {
                group_by: GroupBy::Tag,
                ..Default::default()
            },
            &SortData::default(),
        );
        // Tags are stored sorted, so the first tag is "dark".
        assert!(matches!(&rows[0], GalleryRow::Header(t) if t == "dark"));
        assert!(matches!(&rows[1], GalleryRow::Wallpaper(w) if w.relative_key == "a.png"));
        assert!(matches!(&rows[2], GalleryRow::Header(t) if t == UNTAGGED_LABEL));
        assert!(matches!(&rows[3], GalleryRow::Wallpaper(w) if w.relative_key == "b.png"));
    }

    #[test]
    fn filters_by_tag_and_query() {
        let entries = vec![
            entry("forest.png", "", "forest.png"),
            entry("city.png", "", "city.png"),
        ];
        let dir = TempDir::new().unwrap();
        let mut meta = MetadataStore::open(dir.path().join("m.json")).unwrap();
        meta.set_tags("forest.png", vec!["nature".into()]);
        let rows = build_rows(
            &entries,
            &meta,
            &GalleryFilter {
                query: "for".into(),
                tag: Some("nature".into()),
                ..Default::default()
            },
            &SortData::default(),
        );
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0], GalleryRow::Wallpaper(w) if w.relative_key == "forest.png"));
    }
}
