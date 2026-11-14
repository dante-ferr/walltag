//! Centered floating sheet overlays shared across the app.
//!
//! Wraps [`libadwaita::Dialog`] in floating presentation with translucent dimming
//! and a single sheet surface (no padded double frame). Style hooks use the
//! `walltag-overlay` / `walltag-overlay-body` CSS classes from [`overlay_css`].

use gtk4::glib::object::IsA;
use gtk4::prelude::GtkWindowExt;
use gtk4::prelude::*;
use gtk4::{Box, Button, Orientation, Widget};
use libadwaita::prelude::*;
use libadwaita::{Dialog, DialogPresentationMode};

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

        FloatingOverlay { dialog, body }
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

    /// Dismisses this overlay when Esc is pressed inside [`Self::body`].
    ///
    /// Capture phase runs before a focused [`gtk4::SearchEntry`] handles Escape
    /// (that binding emits `stop-search` and consumes the key, so the dialog's
    /// own Escape shortcut never runs).
    pub fn wire_escape_to_close(&self) {
        let overlay = self.clone();
        let key = gtk4::EventControllerKey::new();
        key.set_propagation_phase(gtk4::PropagationPhase::Capture);
        key.connect_key_pressed(move |_, keyval, _, _| {
            if keyval == gtk4::gdk::Key::Escape {
                overlay.force_close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.body.add_controller(key);
    }
}

/// Closes the [`Dialog`] that contains keyboard focus, if any.
///
/// Call from the main window's Esc handler (capture phase) so a focused
/// [`gtk4::SearchEntry`] inside a floating sheet dismisses the overlay instead of
/// only clearing focus.
pub fn close_focused_dialog(window: &impl IsA<gtk4::Window>) -> bool {
    let Some(focus) = window.focus() else {
        return false;
    };
    let mut current = Some(focus);
    while let Some(widget) = current {
        if let Ok(dialog) = widget.clone().downcast::<Dialog>() {
            return dialog.close();
        }
        current = widget.parent();
    }
    false
}

/// Base CSS for every [`FloatingOverlay`]. Installed with the app chrome CSS.
pub fn overlay_css() -> &'static str {
    r#"
    floating-sheet:has(.walltag-overlay) > dimming {
      background-color: alpha(black, 0.32);
    }
    floating-sheet:has(.walltag-overlay) > sheet {
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
