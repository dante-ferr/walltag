# Agent rules

## Clear selection and focus (Esc and blank-space click)

Esc and blank-space click always:

1. **Drop keyboard focus** from any input (`window.set_focus(None)`), so search and tag fields unfocus.
2. **Clear every selection** in the app (today: the gallery edit batch).

Blank space means empty gallery space, gallery margins, the sidebar background, and other non-interactive areas.

Controls do not count as blank space for clearing: buttons, entries, rows, scrollbars, popovers, and the sidebar divider keep their normal behavior. Clicks on those do not clear focus or selection through this path.

Implementation:

- `clear_focus_and_selection` in `src/ui/app.rs` is the single Esc path. `wire_clear_selection` hooks Esc and blank-space presses on the window in the capture phase and never claims the event.
- On blank clicks inside the gallery, the window handler still unfocuses; the masonry grid clears the edit batch on empty presses (`click_action` in `src/ui/masonry.rs`).
- When you add a new kind of selection, clear it from `clear_focus_and_selection` (and from the blank-click path that calls `grid.clear_selection()`).
- When you add a new interactive widget type, add it to `is_interactive` in `src/ui/app.rs` if a click on it must not clear focus or selection.

## Persist chosen options

Every user-chosen Browse/Output/Gallery setting that is not ephemeral must live in `~/.config/walltag/config.json` via `AppConfig` and restore on startup. That includes filters, sort, group-by, zoom, sidebar width/visibility, monitor, backend, and awww options. Changing the wallpaper root may reset Browse filters; persist that reset too.
