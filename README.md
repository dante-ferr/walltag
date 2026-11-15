# Walltag

Minimalist wallpaper manager and visual gallery for Linux on Wayland. Walltag stores tag metadata as JSON inside your wallpaper root directory and can drive `swww` or `awww` for setting wallpapers.

## Requirements

- Rust 1.75 or newer
- GTK 4 and Libadwaita development packages (for example on Arch: `gtk4`, `libadwaita`)
- A Wayland session and an installed wallpaper daemon (`swww` or `awww`)

## Install

### From source

```bash
cargo install --path .
# or, after the crate is on crates.io:
cargo install walltag
```

The first GUI launch installs a FreeDesktop `.desktop` entry and icon under
`~/.local/share/` so Walltag appears in app launchers (rofi, GNOME, etc.).
System packages (AUR, Flatpak) already ship those files under `/usr/share`.

### Flatpak

Packaging lives in [`packaging/flatpak/`](packaging/flatpak/). Build locally with `flatpak-builder`, or install from Flathub after the submission is accepted. The Flatpak needs host access so it can run `swww` / `awww` and your `post_command`.

### Arch Linux (AUR)

Packaging lives in [`packaging/aur/`](packaging/aur/). Upload when AUR account registration reopens. See [packaging/aur/README.md](packaging/aur/README.md).

```bash
git clone https://aur.archlinux.org/walltag.git
cd walltag
makepkg -si
```

## Configuration

Walltag reads `config.json` from the XDG config directory:

- Path: `~/.config/walltag/config.json` (or `$XDG_CONFIG_HOME/walltag/config.json`)

Example:

```json
{
  "wallpaper_root": "/home/you/Pictures/Wallpapers",
  "backend": "swww",
  "transition_secs": 1.0,
  "post_command": "bash ~/.config/hypr/UserScripts/combined_wallust.sh \"$wallpaper\"",
  "custom_css": null
}
```

`post_command` runs through `sh -c` after a successful set. It expands `$wallpaper` (image path) and `$monitor` (output name, or empty when applying to all monitors). Leave it empty to skip the hook.

Set `custom_css` to a GTK CSS file path to override Libadwaita colors. Walltag reloads that file when it changes. Leave it unset to keep the built-in stylesheet.

Tag metadata lives at `{wallpaper_root}/.walltag/metadata.json`. Last wallpaper state lives at `{wallpaper_root}/.walltag/state.json`.

## Run

```bash
cargo run
```

Headless restore (for compositor autostart):

```bash
cargo run -- --restore
```

Skip the post command during restore (for example when the compositor runs the palette hook once afterward):

```bash
cargo run -- --restore --no-post-command
```

Set log verbosity with `RUST_LOG`, for example `RUST_LOG=walltag=debug`.

## Sidebar

**Browse** has the wallpaper folder button, filename search, folder filter, and random pick. **Output** has monitor, backend, post command, and awww animation controls. **Gallery** has group-by, sort, tag filter, and file-name labels. **Edit** assigns tags to the selected tiles (right-click or drag).

## Status

Working MVP: masonry gallery, portable metadata, swww/awww backends, `--restore`, `post_command`, multi-select tagging, and folder/tag grouping.
