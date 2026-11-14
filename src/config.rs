use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Swww,
    Awww,
}

impl BackendKind {
    pub const ALL: [Self; 2] = [Self::Swww, Self::Awww];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Swww => "swww",
            Self::Awww => "awww",
        }
    }
}

/// Transition effects accepted by `awww img --transition-type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AwwwTransitionKind {
    None,
    #[default]
    Simple,
    Fade,
    Left,
    Right,
    Top,
    Bottom,
    Wipe,
    Wave,
    Grow,
    Center,
    Any,
    Outer,
    Random,
}

impl AwwwTransitionKind {
    pub const ALL: [Self; 14] = [
        Self::None,
        Self::Simple,
        Self::Fade,
        Self::Left,
        Self::Right,
        Self::Top,
        Self::Bottom,
        Self::Wipe,
        Self::Wave,
        Self::Grow,
        Self::Center,
        Self::Any,
        Self::Outer,
        Self::Random,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Simple => "simple",
            Self::Fade => "fade",
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Wipe => "wipe",
            Self::Wave => "wave",
            Self::Grow => "grow",
            Self::Center => "center",
            Self::Any => "any",
            Self::Outer => "outer",
            Self::Random => "random",
        }
    }

    /// Whether `--transition-angle` has an effect for this transition.
    pub fn uses_angle(self) -> bool {
        matches!(self, Self::Wipe | Self::Wave)
    }
}

/// awww-specific animation settings. The duration lives in `AppConfig::transition_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AwwwOptions {
    #[serde(default)]
    pub transition: AwwwTransitionKind,
    #[serde(default = "default_awww_fps")]
    pub fps: u32,
    #[serde(default = "default_awww_angle")]
    pub angle: u32,
}

fn default_awww_fps() -> u32 {
    30
}

fn default_awww_angle() -> u32 {
    45
}

impl Default for AwwwOptions {
    fn default() -> Self {
        Self {
            transition: AwwwTransitionKind::default(),
            fps: default_awww_fps(),
            angle: default_awww_angle(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub wallpaper_root: PathBuf,
    #[serde(default)]
    pub backend: BackendKind,
    #[serde(default = "default_transition_secs")]
    pub transition_secs: f32,
    /// Output name (for example `DP-1`). `None` applies to all monitors.
    #[serde(default)]
    pub monitor: Option<String>,
    #[serde(default)]
    pub awww: AwwwOptions,
    #[serde(default)]
    pub custom_css: Option<PathBuf>,
    /// Sidebar width in logical pixels, set by dragging the divider.
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: i32,
    /// Show each wallpaper's file name under its tile.
    #[serde(default)]
    pub show_filenames: bool,
    /// Grouping dimensions. Empty leaves the gallery flat.
    ///
    /// Folder is the outer group when both are set. A legacy config may store one
    /// string: `"none"`, `"folder"`, or `"tag"`.
    #[serde(default, deserialize_with = "deserialize_group_by")]
    pub group_by: Vec<GroupBy>,
    /// How the gallery orders groups and the wallpapers inside them.
    #[serde(default)]
    pub sort_by: SortBy,
    /// Filename search text in the Browse section.
    #[serde(default)]
    pub search_query: String,
    /// Tag filter (`None` = all tags).
    #[serde(default)]
    pub filter_tag: Option<String>,
    /// Folders to show, as `parent_relative` (`""` is the wallpaper root).
    /// Empty shows every folder. A legacy config may store one string or `null`.
    #[serde(
        default,
        rename = "filter_folder",
        deserialize_with = "deserialize_filter_folders"
    )]
    pub filter_folders: Vec<String>,
    /// Gallery zoom factor (1.0 = 100%).
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    /// Sidebar and top bar scale (1.0 = 100%). Gallery tile zoom is separate (`zoom`).
    #[serde(default = "default_gui_scale")]
    pub gui_scale: f32,
    /// Whether the sidebar is visible.
    #[serde(default = "default_true")]
    pub sidebar_visible: bool,
}

/// Ordering for groups (section headers) and for the wallpapers inside each group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    /// File name, A to Z.
    #[default]
    NameAsc,
    /// File name, Z to A.
    NameDesc,
    /// Path relative to the wallpaper root.
    Path,
    /// Filesystem modification time, newest first.
    DateNewest,
    /// Filesystem modification time, oldest first.
    DateOldest,
    /// File size, largest first.
    SizeLargest,
    /// File size, smallest first.
    SizeSmallest,
    /// Average color hue around the color wheel. Low-saturation images come last.
    Hue,
}

impl SortBy {
    pub const ALL: [Self; 8] = [
        Self::NameAsc,
        Self::NameDesc,
        Self::Path,
        Self::DateNewest,
        Self::DateOldest,
        Self::SizeLargest,
        Self::SizeSmallest,
        Self::Hue,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::NameAsc => "Name (A–Z)",
            Self::NameDesc => "Name (Z–A)",
            Self::Path => "Path",
            Self::DateNewest => "Date modified (newest)",
            Self::DateOldest => "Date modified (oldest)",
            Self::SizeLargest => "Size (largest)",
            Self::SizeSmallest => "Size (smallest)",
            Self::Hue => "Color (hue)",
        }
    }
}

/// One way to group wallpapers under section headers.
///
/// The gallery stores a list of these. `None` means the list is empty. Folder and Tag
/// can both be active; Folder is the outer group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GroupBy {
    #[default]
    None,
    /// Group by subfolder relative to the wallpaper root ("workspace").
    Folder,
    /// Group by tag. Each wallpaper appears under every tag it has, or Untagged.
    Tag,
}

impl GroupBy {
    pub const ALL: [Self; 3] = [Self::None, Self::Folder, Self::Tag];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Folder => "folder",
            Self::Tag => "tag",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Folder => "Folder",
            Self::Tag => "Tag",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|kind| kind.as_str() == name)
    }

    /// Keeps Folder and Tag, in that order. `None` and duplicates are dropped.
    pub fn normalize(kinds: &[Self]) -> Vec<Self> {
        let mut out = Vec::new();
        if kinds.contains(&Self::Folder) {
            out.push(Self::Folder);
        }
        if kinds.contains(&Self::Tag) {
            out.push(Self::Tag);
        }
        out
    }
}

/// Sidebar paned limits at GUI scale 1.0 (logical px).
pub const BASE_MIN_SIDEBAR_WIDTH: i32 = 300;
pub const BASE_MAX_SIDEBAR_WIDTH: i32 = 640;

/// Unscaled limits; prefer [`sidebar_limits`] when `gui_scale` is known.
pub const MIN_SIDEBAR_WIDTH: i32 = BASE_MIN_SIDEBAR_WIDTH;
pub const MAX_SIDEBAR_WIDTH: i32 = BASE_MAX_SIDEBAR_WIDTH;

/// Minimum and maximum sidebar width for the current GUI scale.
pub fn sidebar_limits(gui_scale: f32) -> (i32, i32) {
    const MIN_SCALE: f32 = 0.75;
    const MAX_SCALE: f32 = 1.75;
    let s = gui_scale.clamp(MIN_SCALE, MAX_SCALE);
    let min = (BASE_MIN_SIDEBAR_WIDTH as f32 * s).round() as i32;
    let max = (BASE_MAX_SIDEBAR_WIDTH as f32 * s).round() as i32;
    (min, max.max(min))
}

fn default_sidebar_width() -> i32 {
    BASE_MIN_SIDEBAR_WIDTH
}

fn default_transition_secs() -> f32 {
    1.0
}

fn default_zoom() -> f32 {
    1.0
}

fn default_gui_scale() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            wallpaper_root: PathBuf::from("~/Pictures/Wallpapers"),
            backend: BackendKind::default(),
            transition_secs: default_transition_secs(),
            monitor: None,
            awww: AwwwOptions::default(),
            custom_css: None,
            sidebar_width: default_sidebar_width(),
            show_filenames: false,
            group_by: Vec::new(),
            sort_by: SortBy::default(),
            search_query: String::new(),
            filter_tag: None,
            filter_folders: Vec::new(),
            zoom: default_zoom(),
            gui_scale: default_gui_scale(),
            sidebar_visible: true,
        }
    }
}

impl AppConfig {
    pub fn expand_paths(&mut self) -> Result<()> {
        self.wallpaper_root = expand_home(&self.wallpaper_root)?;
        if let Some(css) = &self.custom_css {
            self.custom_css = Some(expand_home(css)?);
        }
        let (min, max) = sidebar_limits(self.gui_scale);
        self.sidebar_width = self.sidebar_width.clamp(min, max);
        self.zoom = self.zoom.clamp(0.4, 3.0);
        self.gui_scale = self.gui_scale.clamp(0.75, 1.75);
        self.group_by = GroupBy::normalize(&self.group_by);
        self.filter_folders = dedupe_strings(&self.filter_folders);
        Ok(())
    }

    pub fn metadata_path(&self) -> PathBuf {
        self.wallpaper_root.join(".walltag").join("metadata.json")
    }

    pub fn state_path(&self) -> PathBuf {
        self.wallpaper_root.join(".walltag").join("state.json")
    }
}

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub config_file: PathBuf,
    pub config_dir: PathBuf,
}

impl ConfigPaths {
    pub fn discover() -> Result<Self> {
        let proj = ProjectDirs::from("com", "walltag", "walltag")
            .context("could not resolve XDG config directory for walltag")?;
        let config_dir = proj.config_dir().to_path_buf();
        Ok(Self {
            config_file: config_dir.join("config.json"),
            config_dir,
        })
    }

    pub fn load() -> Result<AppConfig> {
        let paths = Self::discover()?;
        let mut config = if paths.config_file.exists() {
            let raw = fs::read_to_string(&paths.config_file)
                .with_context(|| format!("read {}", paths.config_file.display()))?;
            serde_json::from_str(&raw).context("parse walltag config.json")?
        } else {
            fs::create_dir_all(&paths.config_dir).context("create walltag config directory")?;
            let mut default = AppConfig::default();
            default.expand_paths()?;
            let pretty =
                serde_json::to_string_pretty(&default).context("serialize default config")?;
            fs::write(&paths.config_file, format!("{pretty}\n"))
                .with_context(|| format!("write {}", paths.config_file.display()))?;
            tracing::info!("Wrote default config to {}", paths.config_file.display());
            default
        };
        config.expand_paths()?;
        Ok(config)
    }

    pub fn save(config: &AppConfig) -> Result<()> {
        let paths = Self::discover()?;
        fs::create_dir_all(&paths.config_dir).context("create walltag config directory")?;
        let pretty = serde_json::to_string_pretty(config).context("serialize walltag config")?;
        fs::write(&paths.config_file, format!("{pretty}\n"))
            .with_context(|| format!("write {}", paths.config_file.display()))?;
        Ok(())
    }
}

fn dedupe_strings(items: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for item in items {
        if !out.contains(item) {
            out.push(item.clone());
        }
    }
    out
}

fn group_by_from_str<E: de::Error>(text: &str) -> Result<Vec<GroupBy>, E> {
    let kind =
        GroupBy::parse(text).ok_or_else(|| E::unknown_variant(text, &["none", "folder", "tag"]))?;
    Ok(GroupBy::normalize(&[kind]))
}

fn deserialize_group_by<'de, D>(deserializer: D) -> Result<Vec<GroupBy>, D::Error>
where
    D: Deserializer<'de>,
{
    struct GroupByVisitor;

    impl<'de> Visitor<'de> for GroupByVisitor {
        type Value = Vec<GroupBy>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a group-by name or a list of them")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
            group_by_from_str(value)
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
            group_by_from_str(&value)
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut items = Vec::new();
            while let Some(kind) = seq.next_element::<GroupBy>()? {
                items.push(kind);
            }
            Ok(GroupBy::normalize(&items))
        }
    }

    deserializer.deserialize_any(GroupByVisitor)
}

fn deserialize_filter_folders<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct FolderVisitor;

    impl<'de> Visitor<'de> for FolderVisitor {
        type Value = Vec<String>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a folder, null, or a list of folders")
        }

        fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
            Ok(vec![value.to_owned()])
        }

        fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
            Ok(vec![value])
        }

        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut items = Vec::new();
            while let Some(folder) = seq.next_element::<String>()? {
                if !items.contains(&folder) {
                    items.push(folder);
                }
            }
            Ok(items)
        }
    }

    deserializer.deserialize_any(FolderVisitor)
}

fn expand_home(path: &Path) -> Result<PathBuf> {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/") {
        let home = std::env::var("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home).join(rest));
    }
    if s == "~" {
        let home = std::env::var("HOME").context("HOME is not set")?;
        return Ok(PathBuf::from(home));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_config_without_new_fields_loads() {
        let cfg: AppConfig =
            serde_json::from_str(r#"{"wallpaper_root":"/x","backend":"awww"}"#).unwrap();
        assert_eq!(cfg.monitor, None);
        assert_eq!(cfg.awww, AwwwOptions::default());
        assert_eq!(cfg.sidebar_width, BASE_MIN_SIDEBAR_WIDTH);
        assert!(!cfg.show_filenames);
        assert_eq!(cfg.sort_by, SortBy::NameAsc);
        assert!(cfg.search_query.is_empty());
        assert_eq!(cfg.filter_tag, None);
        assert!(cfg.group_by.is_empty());
        assert!(cfg.filter_folders.is_empty());
        assert_eq!(cfg.zoom, 1.0);
        assert_eq!(cfg.gui_scale, 1.0);
        assert!(cfg.sidebar_visible);
    }

    #[test]
    fn sort_by_round_trips_as_snake_case() {
        let mut cfg = AppConfig::default();
        cfg.sort_by = SortBy::DateNewest;
        let raw = serde_json::to_string(&cfg).unwrap();
        assert!(raw.contains(r#""sort_by":"date_newest""#));
        let back: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.sort_by, SortBy::DateNewest);
        for sort in SortBy::ALL {
            let json = serde_json::to_string(&sort).unwrap();
            assert_eq!(serde_json::from_str::<SortBy>(&json).unwrap(), sort);
        }
    }

    #[test]
    fn sidebar_width_is_clamped_on_load() {
        let mut cfg: AppConfig =
            serde_json::from_str(r#"{"wallpaper_root":"/x","sidebar_width":5000}"#).unwrap();
        cfg.expand_paths().unwrap();
        let (min, max) = sidebar_limits(1.0);
        assert_eq!(cfg.sidebar_width, max);
        cfg.sidebar_width = 1;
        cfg.expand_paths().unwrap();
        assert_eq!(cfg.sidebar_width, min);
    }

    #[test]
    fn sidebar_limits_follow_gui_scale() {
        let (min, max) = sidebar_limits(1.2);
        assert_eq!(min, 360);
        assert_eq!(max, 768);
    }

    #[test]
    fn legacy_group_by_and_folder_load_as_lists() {
        let folder: AppConfig = serde_json::from_str(
            r#"{"wallpaper_root":"/x","group_by":"folder","filter_folder":"cats"}"#,
        )
        .unwrap();
        assert_eq!(folder.group_by, vec![GroupBy::Folder]);
        assert_eq!(folder.filter_folders, vec!["cats".to_owned()]);

        let none: AppConfig = serde_json::from_str(
            r#"{"wallpaper_root":"/x","group_by":"none","filter_folder":null}"#,
        )
        .unwrap();
        assert!(none.group_by.is_empty());
        assert!(none.filter_folders.is_empty());

        let root: AppConfig =
            serde_json::from_str(r#"{"wallpaper_root":"/x","filter_folder":""}"#).unwrap();
        assert_eq!(root.filter_folders, vec!["".to_owned()]);

        let both: AppConfig = serde_json::from_str(
            r#"{"wallpaper_root":"/x","group_by":["tag","none","folder"],"filter_folder":["cats","","cats"]}"#,
        )
        .unwrap();
        assert_eq!(both.group_by, vec![GroupBy::Folder, GroupBy::Tag]);
        assert_eq!(both.filter_folders, vec!["cats".to_owned(), "".to_owned()]);
    }

    #[test]
    fn group_by_and_folders_round_trip_as_lists() {
        let mut cfg = AppConfig::default();
        cfg.group_by = vec![GroupBy::Tag, GroupBy::Folder, GroupBy::None];
        cfg.filter_folders = vec!["cats".into(), "".into(), "cats".into()];
        cfg.expand_paths().unwrap();
        assert_eq!(cfg.group_by, vec![GroupBy::Folder, GroupBy::Tag]);
        assert_eq!(cfg.filter_folders, vec!["cats".to_owned(), "".to_owned()]);

        let raw = serde_json::to_string(&cfg).unwrap();
        assert!(raw.contains(r#""group_by":["folder","tag"]"#));
        assert!(raw.contains(r#""filter_folder":["cats",""]"#));
        let back: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.group_by, cfg.group_by);
        assert_eq!(back.filter_folders, cfg.filter_folders);
    }

    #[test]
    fn unknown_group_by_is_rejected() {
        let err =
            serde_json::from_str::<AppConfig>(r#"{"wallpaper_root":"/x","group_by":"color"}"#);
        assert!(err.is_err());
    }

    #[test]
    fn awww_options_round_trip() {
        let mut cfg = AppConfig::default();
        cfg.monitor = Some("DP-1".into());
        cfg.sidebar_width = 411;
        cfg.show_filenames = true;
        cfg.awww.transition = AwwwTransitionKind::Wipe;
        let raw = serde_json::to_string(&cfg).unwrap();
        let back: AppConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.monitor.as_deref(), Some("DP-1"));
        assert_eq!(back.sidebar_width, 411);
        assert!(back.show_filenames);
        assert_eq!(back.awww.transition, AwwwTransitionKind::Wipe);
    }
}
