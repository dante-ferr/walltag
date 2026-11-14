Build a minimalist wallpaper manager and visual gallery in **Rust** for Linux/Wayland using **GTK4** and **Libadwaita**.

Write all code, UI strings, and minimal comments strictly in **English**.

### Core Concept & Features

- **Gallery & Tagging:** Display wallpapers in a clean, responsive grid where users can assign, edit, and filter by tags, as well as search by filename. The grid should fit the images in columns without whitespaces, similarly to google images.
- **Waypaper Feature Parity:** Include full support for recursive subfolder scanning, viewing subfolder contents, grouping wallpapers dynamically by either subfolders or tags, transition/display controls, random selection, and a headless `--restore` CLI flag for compositor autostart.
- **Portable Metadata:** Work with a fixed user-configured root wallpaper directory and store all wallpaper tag metadata in a JSON file directly inside that directory to facilitate backups and version control (e.g., Git).
- **Backends:** Support `swww` and `awww` as the initial wallpaper daemons, designed cleanly so additional backends can be supported in the future.
- **Global Theming:** Support system themes out of the box via GTK4/Libadwaita, plus custom user theming via an external CSS file (`@define-color` variables) with live hot-reload (e.g., via `notify` and `CssProvider`) for seamless integration with tools like `wallust` or `matugen`.

There are lots of things not yet described here, so ask for direction every time decisions have to be made.
