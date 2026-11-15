## Walltag 0.1.2

### Fixes

- Load `custom_css` from memory and poll the file so Wallust theme updates apply while the app stays open

### Packaging

- Self-hosted Arch pacman repository on GitHub Pages (`pacman -Syu walltag` after one-time repo setup)
- Release workflow automation for crates.io (when `CRATES_IO_TOKEN` is set) and pacman repo publishing

### Install

- **Arch (pacman repo):** see https://dante-ferr.github.io/walltag/
- **crates.io:** `cargo install walltag`
- **Source:** build with Rust 1.75+, GTK 4, and Libadwaita

### Requirements

A Wayland session and either `swww` or `awww` on the host.
