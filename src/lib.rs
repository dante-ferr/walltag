pub mod backends;
pub mod cli;
pub mod config;
pub mod metadata;
pub mod scan;
pub mod ui;

pub use cli::Cli;
pub use config::{AppConfig, BackendKind, ConfigPaths, GroupBy, SortBy};
pub use metadata::{MetadataStore, WallpaperRecord};
pub use scan::{scan_wallpapers, WallpaperEntry};
