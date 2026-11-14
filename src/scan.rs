use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "webp", "gif", "bmp", "tiff", "tif", "avif", "jxl",
];

#[derive(Debug, Clone)]
pub struct WallpaperEntry {
    pub absolute: PathBuf,
    /// Path relative to the wallpaper root, POSIX-style for stable metadata keys.
    pub relative_key: String,
    pub file_name: String,
    pub parent_relative: String,
}

pub fn scan_wallpapers(root: &Path) -> Result<Vec<WallpaperEntry>> {
    if !root.exists() {
        anyhow::bail!(
            "wallpaper root does not exist: {}",
            root.display()
        );
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize {}", root.display()))?;

    let mut out = Vec::new();
    for entry in WalkDir::new(&root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if !is_image(path) {
            continue;
        }
        let absolute = path.to_path_buf();
        let relative = absolute
            .strip_prefix(&root)
            .context("path outside wallpaper root")?;
        let relative_key = relative
            .to_string_lossy()
            .replace('\\', "/");
        let file_name = relative
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative_key.clone());
        let parent_relative = relative
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();

        out.push(WallpaperEntry {
            absolute,
            relative_key,
            file_name,
            parent_relative,
        });
    }

    out.sort_by(|a, b| a.relative_key.cmp(&b.relative_key));
    Ok(out)
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(|ext| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn finds_images_recursively() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(root.join("top.png"), b"x").unwrap();
        let nested = root.join("cats");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("nap.jpg"), b"x").unwrap();
        fs::write(nested.join("readme.txt"), b"nope").unwrap();

        let found = scan_wallpapers(root).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.iter().any(|e| e.relative_key == "top.png"));
        assert!(found.iter().any(|e| e.relative_key == "cats/nap.jpg"));
    }
}
