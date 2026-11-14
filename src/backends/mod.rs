mod awww;
mod swww;
mod traits;

pub use traits::{ApplyOptions, WallpaperBackend, WallpaperState};

use crate::config::BackendKind;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

pub fn backend_for(kind: BackendKind) -> Arc<dyn WallpaperBackend> {
    match kind {
        BackendKind::Swww => Arc::new(swww::SwwwBackend),
        BackendKind::Awww => Arc::new(awww::AwwwBackend),
    }
}

pub fn apply_wallpaper(kind: BackendKind, image: &Path, options: &ApplyOptions) -> Result<()> {
    backend_for(kind).set_wallpaper(image, options)
}

pub fn restore_wallpaper(kind: BackendKind, image: &Path, options: &ApplyOptions) -> Result<()> {
    backend_for(kind).restore(image, options)
}
