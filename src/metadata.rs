use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const METADATA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetadataFile {
    pub version: u32,
    #[serde(default)]
    pub wallpapers: BTreeMap<String, WallpaperRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WallpaperRecord {
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub favorite: bool,
}

impl MetadataFile {
    pub fn new() -> Self {
        Self {
            version: METADATA_VERSION,
            wallpapers: BTreeMap::new(),
        }
    }
}

pub struct MetadataStore {
    path: PathBuf,
    data: MetadataFile,
}

impl MetadataStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data = if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&raw).context("parse wallpaper metadata.json")?
        } else {
            MetadataFile::new()
        };
        Ok(Self { path, data })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data(&self) -> &MetadataFile {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut MetadataFile {
        &mut self.data
    }

    pub fn record_for_key(&self, key: &str) -> WallpaperRecord {
        self.data
            .wallpapers
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_tags(&mut self, key: impl Into<String>, tags: Vec<String>) {
        let key = key.into();
        let entry = self.data.wallpapers.entry(key).or_default();
        let mut normalized: Vec<String> = tags
            .into_iter()
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        normalized.sort();
        normalized.dedup();
        entry.tags = normalized;
    }

    /// Adds `tags` to every wallpaper in `keys`, keeping their existing tags. Tags are
    /// normalized like `set_tags`. Does nothing when `tags` has no usable entries.
    pub fn add_tags(&mut self, keys: &[String], tags: &[String]) {
        if tags.iter().all(|t| t.trim().is_empty()) {
            return;
        }
        for key in keys {
            let mut merged = self.record_for_key(key).tags;
            merged.extend(tags.iter().cloned());
            self.set_tags(key.clone(), merged);
        }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        let pretty =
            serde_json::to_string_pretty(&self.data).context("serialize wallpaper metadata")?;
        fs::write(&self.path, format!("{pretty}\n"))
            .with_context(|| format!("write {}", self.path.display()))?;
        Ok(())
    }
}

/// Splits user input such as `forest, dark #night` into tags. Commas and whitespace
/// separate tags; a leading `#` is dropped. Tags are lowercased and deduplicated.
pub fn parse_tag_input(input: &str) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for raw in input.split(|c: char| c == ',' || c.is_whitespace()) {
        let tag = raw.trim().trim_start_matches('#').to_lowercase();
        if !tag.is_empty() && !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commas_spaces_and_hashes() {
        assert_eq!(
            parse_tag_input(" Forest, dark  #Night ,,forest"),
            vec!["forest", "dark", "night"]
        );
        assert!(parse_tag_input("  , # ").is_empty());
    }

    #[test]
    fn add_tags_merges_into_each_wallpaper_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".walltag/metadata.json");
        let mut store = MetadataStore::open(&path).unwrap();
        store.set_tags("a.png", vec!["dark".into()]);

        let keys = vec!["a.png".to_string(), "sub/b.png".to_string()];
        store.add_tags(&keys, &["Forest".into(), "dark".into()]);
        store.save().unwrap();

        let reopened = MetadataStore::open(&path).unwrap();
        assert_eq!(reopened.record_for_key("a.png").tags, vec!["dark", "forest"]);
        assert_eq!(reopened.record_for_key("sub/b.png").tags, vec!["dark", "forest"]);
    }

    #[test]
    fn add_tags_with_blank_input_creates_no_records() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = MetadataStore::open(dir.path().join("m.json")).unwrap();
        store.add_tags(&["a.png".to_string()], &["  ".to_string()]);
        assert!(store.data().wallpapers.is_empty());
    }
}
