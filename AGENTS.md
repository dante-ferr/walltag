# Agent rules

## Clear selection and focus (Esc and blank-space click)

Esc and blank-space click always:

1. **Close any open floating overlay** (`close_visible_dialog`), even when focus never entered the sheet.
2. **Drop keyboard focus** from any input (`window.set_focus(None)`), so search and tag fields unfocus.
3. **Clear every selection** in the app (today: the gallery edit batch).

Blank space means empty gallery space, gallery margins, the sidebar background, and other non-interactive areas. A click on the overlay dimming (outside the sheet) closes the overlay; it does not count as blank-space selection clear.

Controls do not count as blank space for clearing: buttons, entries, rows, scrollbars, popovers, and the sidebar divider keep their normal behavior. Clicks on those do not clear focus or selection through this path.

Implementation:

- `wire_clear_selection` in `src/ui/app.rs` is the Esc / blank-space path. Esc closes `ApplicationWindow::visible_dialog` first, then calls `clear_focus_and_selection`. Floating overlays also dismiss on dimming click (`FloatingOverlay` in `src/ui/overlay.rs` and the window capture click handler).
- On blank clicks inside the gallery, the window handler still unfocuses; the masonry grid clears the edit batch on empty presses (`click_action` in `src/ui/masonry.rs`).
- When you add a new kind of selection, clear it from `clear_focus_and_selection` (and from the blank-click path that calls `grid.clear_selection()`).
- When you add a new interactive widget type, add it to `is_interactive` in `src/ui/app.rs` if a click on it must not clear focus or selection.

## Persist chosen options

Every user-chosen Browse/Output/Gallery setting that is not ephemeral must live in `~/.config/walltag/config.json` via `AppConfig` and restore on startup. That includes filters, sort, group-by, zoom, gui scale, sidebar width/visibility, monitor, backend, and awww options. Changing the wallpaper root may reset Browse filters; persist that reset too.
