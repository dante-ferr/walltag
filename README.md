# Walltag

Minimalist wallpaper manager and visual gallery for Linux on Wayland. Walltag stores tag metadata as JSON inside your wallpaper root directory and can drive `swww` or `awww` for setting wallpapers.

## Requirements

- Rust 1.75 or newer
- GTK 4 and Libadwaita development packages (for example on Arch: `gtk4`, `libadwaita`)
- A Wayland session and an installed wallpaper daemon (`swww` or `awww`)

## Configuration

Walltag reads `config.json` from the XDG config directory:

- Path: `~/.config/walltag/config.json` (or `$XDG_CONFIG_HOME/walltag/config.json`)

Example:

```json
{
  "wallpaper_root": "/home/you/Pictures/Wallpapers",
  "backend": "swww",
  "transition_secs": 1.0,
  "custom_css": null
}
```

Tag metadata lives at `{wallpaper_root}/.walltag/metadata.json`.

## Run

```bash
cargo run
```

Headless restore (for compositor autostart):

```bash
cargo run -- --restore
```

Set log verbosity with `RUST_LOG`, for example `RUST_LOG=walltag=debug`.

## Sidebar

**Browse** has the wallpaper folder button, filename search, folder filter, and random pick. **Gallery** has group-by, sort, tag filter, and file-name labels. **Edit** assigns tags to the selected tiles (right-click or drag).

## Status

Working MVP: masonry gallery, portable metadata, swww/awww backends, `--restore`, multi-select tagging, and folder/tag grouping.
