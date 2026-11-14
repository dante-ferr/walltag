//! Centered floating sheet overlays shared across the app.
//!
//! Wraps [`libadwaita::Dialog`] in floating presentation with translucent dimming
//! and a single sheet surface (no padded double frame). Style hooks use the
//! `walltag-overlay` / `walltag-overlay-body` CSS classes from [`overlay_css`].
//!
//! Esc and a click on the dimming (outside the sheet) always dismiss the overlay.
//! The main window Esc handler also closes [`ApplicationWindow::visible_dialog`]
//! so Esc works even when focus never entered the sheet.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::glib::object::IsA;
use gtk4::prelude::*;
use gtk4::{Box, Button, EventControllerKey, GestureClick, Orientation, PropagationPhase, Widget};
use libadwaita::prelude::*;
use libadwaita::{ApplicationWindow, Dialog, DialogPresentationMode};

const DEFAULT_CONTENT_WIDTH: i32 = 300;

/// Centered floating overlay with translucent dimming and a compact sheet.
#[derive(Clone)]
#[allow(dead_code)] // `body` is read by the public accessor API below.
pub struct FloatingOverlay {
    dialog: Dialog,
    body: Box,
}

// Accessors stay available for future overlays even when unused by the first caller.
#[allow(dead_code)]
impl FloatingOverlay {
    /// Dialog title shown in the sheet header.
    pub fn set_title(&self, title: &str) {
        self.dialog.set_title(title);
    }

    /// Replaces the sheet body contents.
    pub fn set_child(&self, child: Option<&impl IsA<Widget>>) {
        while let Some(existing) = self.body.first_child() {
            self.body.remove(&existing);
        }
        if let Some(child) = child {
            self.body.append(child);
        }
    }

    pub fn body(&self) -> &Box {
        &self.body
    }

    pub fn dialog(&self) -> &Dialog {
        &self.dialog
    }

    /// Requests a close (honors `can-close`).
    pub fn close(&self) -> bool {
        self.dialog.close()
    }
}

/// Builds a [`FloatingOverlay`].
pub struct FloatingOverlayBuilder {
    title: String,
    content_width: i32,
    body_classes: Vec<&'static str>,
    child: Option<Widget>,
}

impl FloatingOverlayBuilder {
    fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            content_width: DEFAULT_CONTENT_WIDTH,
            body_classes: Vec::new(),
            child: None,
        }
    }

    /// Preferred content width in pixels (scaled chrome may still grow with content).
    pub fn content_width(mut self, width: i32) -> Self {
        self.content_width = width.max(1);
        self
    }

    /// Extra CSS class on the body wrapper (for example `walltag-sidebar` for preference rows).
    pub fn body_class(mut self, class: &'static str) -> Self {
        self.body_classes.push(class);
        self
    }

    /// Widget shown inside the sheet.
    pub fn child(mut self, child: &impl IsA<Widget>) -> Self {
        self.child = Some(child.as_ref().clone());
        self
    }

    pub fn build(self) -> FloatingOverlay {
        let body = Box::new(Orientation::Vertical, 0);
        body.add_css_class("walltag-overlay-body");
        for class in &self.body_classes {
            body.add_css_class(class);
        }
        if let Some(child) = self.child {
            body.append(&child);
        }

        let dialog = Dialog::builder()
            .title(&self.title)
            .content_width(self.content_width)
            .follows_content_size(true)
            .presentation_mode(DialogPresentationMode::Floating)
            .child(&body)
            .build();
        dialog.add_css_class("walltag-overlay");

        let overlay = FloatingOverlay { dialog, body };
        overlay.wire_dismiss();
        overlay
    }
}

impl FloatingOverlay {
    pub fn builder(title: impl Into<String>) -> FloatingOverlayBuilder {
        FloatingOverlayBuilder::new(title)
    }

    /// Opens the overlay centered over the widget's window.
    pub fn present(&self, parent: &impl IsA<Widget>) {
        self.dialog.present(Some(parent));
    }

    /// Closes immediately.
    pub fn force_close(&self) {
        self.dialog.force_close();
    }

    /// Presents this overlay when `button` is clicked.
    pub fn bind_trigger(&self, button: &impl IsA<Button>) {
        let overlay = self.clone();
        button.as_ref().connect_clicked(move |btn| {
            overlay.present(btn);
        });
    }

    /// Esc inside the sheet, and a click on the dimming outside it, dismiss this overlay.
    ///
    /// Esc on the body uses capture so a focused [`gtk4::SearchEntry`] cannot consume
    /// the key before the sheet closes. Dimming click covers floating mode, which does
    /// not dismiss on outside press by default (only bottom sheets do).
    fn wire_dismiss(&self) {
        let overlay = self.clone();
        let key = EventControllerKey::new();
        key.set_propagation_phase(PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                overlay.force_close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.body.add_controller(key);

        let dialog = self.dialog.clone();
        let wired = Rc::new(Cell::new(false));
        self.dialog.connect_map(move |mapped| {
            if wired.get() {
                return;
            }
            let Some(dimming) = find_css_named(mapped.upcast_ref(), "dimming") else {
                return;
            };
            wired.set(true);
            let click = GestureClick::new();
            click.set_button(1);
            let dialog = dialog.clone();
            click.connect_released(move |gesture, _, _, _| {
                dialog.close();
                gesture.set_state(gtk4::EventSequenceState::Claimed);
            });
            dimming.add_controller(click);
        });
    }
}

/// Closes the window's topmost visible [`Dialog`], if any.
///
/// Prefer this over walking keyboard focus: floating sheets stay open when Esc is
/// pressed before anything inside them is focused.
pub fn close_visible_dialog(window: &impl IsA<ApplicationWindow>) -> bool {
    window.visible_dialog().is_some_and(|dialog| dialog.close())
}

/// Whether `target` is the floating-sheet dimming (outside the sheet content).
pub fn is_dialog_dimming(target: &Widget) -> bool {
    let mut current = Some(target.clone());
    while let Some(widget) = current {
        if widget.css_name() == "dimming" {
            return true;
        }
        if widget.is::<Dialog>() {
            return false;
        }
        current = widget.parent();
    }
    false
}

fn find_css_named(root: &Widget, name: &str) -> Option<Widget> {
    if root.css_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(node) = child {
        if let Some(found) = find_css_named(&node, name) {
            return Some(found);
        }
        child = node.next_sibling();
    }
    None
}

/// Base CSS for every [`FloatingOverlay`]. Installed with the app chrome CSS.
pub fn overlay_css() -> &'static str {
    r#"
    /* Avoid :has(); GTK theme CSS does not accept that pseudoclass here. */
    floating-sheet > dimming {
      background-color: alpha(black, 0.32);
    }
    floating-sheet > sheet {
      background-color: @window_bg_color;
      color: @window_fg_color;
    }
    .walltag-overlay {
      background: none;
      background-color: transparent;
    }
    .walltag-overlay-body {
      margin: 0;
      padding: 0.429em;
      background: transparent;
    }
    .walltag-overlay-body preferencesgroup > box {
      border-spacing: 0;
    }
    .walltag-overlay-body preferencesgroup > box > list,
    .walltag-overlay-body preferencesgroup list.boxed-list {
      background: none;
      background-color: transparent;
      border: none;
      border-radius: 0;
      margin: 0;
      padding: 0;
      box-shadow: none;
    }
    "#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_defaults_width() {
        let b = FloatingOverlayBuilder::new("Test");
        assert_eq!(b.content_width, DEFAULT_CONTENT_WIDTH);
        assert_eq!(b.title, "Test");
        assert!(b.child.is_none());
    }
}
