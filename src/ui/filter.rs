//! Gallery search, filter, and group-by helpers.

use std::collections::{BTreeMap, BTreeSet};

use crate::metadata::MetadataStore;
use crate::scan::WallpaperEntry;

pub use crate::config::GroupBy;

/// Label for the root folder group (files directly under the wallpaper root).
pub const ROOT_FOLDER_LABEL: &str = "Root";
/// Label for wallpapers with no tags when grouping by tag.
pub const UNTAGGED_LABEL: &str = "Untagged";

/// Active search / filter / group state (tag/folder filters are session-only).
#[derive(Debug, Clone, Default)]
pub struct GalleryFilter {
    pub query: String,
    pub group_by: GroupBy,
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

/// Filters and optionally groups wallpapers for the gallery.
pub fn build_rows(
    entries: &[WallpaperEntry],
    metadata: &MetadataStore,
    filter: &GalleryFilter,
) -> Vec<GalleryRow> {
    let query = filter.query.to_lowercase();
    let filtered: Vec<&WallpaperEntry> = entries
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

    match filter.group_by {
        GroupBy::None => filtered
            .into_iter()
            .cloned()
            .map(GalleryRow::Wallpaper)
            .collect(),
        GroupBy::Folder => {
            let mut groups: BTreeMap<String, Vec<WallpaperEntry>> = BTreeMap::new();
            for wp in filtered {
                groups
                    .entry(folder_label(&wp.parent_relative))
                    .or_default()
                    .push(wp.clone());
            }
            flatten_groups(groups)
        }
        GroupBy::Tag => {
            let mut groups: BTreeMap<String, Vec<WallpaperEntry>> = BTreeMap::new();
            for wp in filtered {
                let tags = metadata.record_for_key(&wp.relative_key).tags;
                let label = tags
                    .first()
                    .cloned()
                    .unwrap_or_else(|| UNTAGGED_LABEL.to_owned());
                groups.entry(label).or_default().push(wp.clone());
            }
            flatten_groups_untagged_last(groups)
        }
    }
}

fn flatten_groups(groups: BTreeMap<String, Vec<WallpaperEntry>>) -> Vec<GalleryRow> {
    let mut rows = Vec::new();
    for (title, items) in groups {
        append_group(&mut rows, title, items);
    }
    rows
}

/// Like [`flatten_groups`], but the Untagged section always comes last.
fn flatten_groups_untagged_last(mut groups: BTreeMap<String, Vec<WallpaperEntry>>) -> Vec<GalleryRow> {
    let untagged = groups.remove(UNTAGGED_LABEL);
    let mut rows = flatten_groups(groups);
    if let Some(items) = untagged {
        append_group(&mut rows, UNTAGGED_LABEL.to_owned(), items);
    }
    rows
}

fn append_group(rows: &mut Vec<GalleryRow>, title: String, items: Vec<WallpaperEntry>) {
    if items.is_empty() {
        return;
    }
    rows.push(GalleryRow::Header(title));
    rows.extend(items.into_iter().map(GalleryRow::Wallpaper));
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
        );
        assert_eq!(rows.len(), 1);
        assert!(matches!(&rows[0], GalleryRow::Wallpaper(w) if w.relative_key == "forest.png"));
    }
}
