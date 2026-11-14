## Walltag 0.1.0

First public release of Walltag, a minimalist wallpaper manager and visual gallery for Linux on Wayland.

### Highlights

- Masonry wallpaper gallery with zoom, smooth scrolling, and folder/tag grouping
- Tag editing with multi-select
- `swww` and `awww` backends
- `--restore` for compositor autostart (optional `--no-post-command`)
- Configurable `post_command` after a successful set
- Settings persisted under `~/.config/walltag/config.json`

### Install

- **crates.io:** `cargo install walltag`
- **Flatpak:** see `packaging/flatpak/` (Flathub submission after review)
- **AUR:** packaging is ready in `packaging/aur/` (upload when AUR registration reopens)
- **Source:** build with Rust 1.75+, GTK 4, and Libadwaita

### Requirements

A Wayland session and either `swww` or `awww` on the host.
