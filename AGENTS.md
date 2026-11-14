# Agent rules

## Clear selection (Esc and blank-space click)

Any selection in the app clears on:

1. **Esc**, pressed anywhere in the window.
2. **A click on blank space**: empty gallery space, gallery margins, the sidebar background, and other non-interactive areas.

Controls do not count as blank space: buttons, entries, rows, scrollbars, popovers, and the sidebar divider keep their normal behavior and do not clear the selection.

Implementation:

- `clear_all_selection` in `src/ui/app.rs` is the single clear path. `wire_clear_selection` hooks Esc and blank-space presses to it on the window in the capture phase and never claims the event.
- The masonry grid also clears the edit batch when you press empty space inside it (`click_action` in `src/ui/masonry.rs`).
- When you add a new kind of selection, clear it from `clear_all_selection`. Do not add a separate Esc or click handler.
- When you add a new interactive widget type, add it to `is_interactive` in `src/ui/app.rs` if it must not clear the selection on click.
