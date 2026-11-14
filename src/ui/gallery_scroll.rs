//! Smooth vertical scroll for the gallery `ScrolledWindow`.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::gdk::{Key, ModifierType, ScrollUnit};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Adjustment, Editable, EventControllerKey, EventControllerScroll, EventControllerScrollFlags,
    PropagationPhase, ScrolledWindow, Widget, Window,
};

use crate::ui::masonry::ease_out_cubic;

/// Viewport height moved per mouse-wheel notch.
pub const WHEEL_PAGE_FRACTION: f64 = 0.22;
/// Viewport height moved per Up/Down key press.
const KEY_LINE_FRACTION: f64 = 0.11;
/// Share of each scroll step applied immediately so input feels responsive.
const SCROLL_LEAD_FRACTION: f64 = 0.22;
const SCROLL_ANIM_MS: f64 = 260.0;

struct SmoothScroll {
    scroll: ScrolledWindow,
    adj: Adjustment,
    from: Cell<f64>,
    target: Cell<f64>,
    start_us: Cell<Option<i64>>,
    ticking: Cell<bool>,
}

impl SmoothScroll {
    fn clamped(&self, value: f64) -> f64 {
        let page = self.adj.page_size().max(1.0);
        let upper = (self.adj.upper() - page).max(self.adj.lower());
        value.clamp(self.adj.lower(), upper)
    }

    fn line_delta(&self) -> f64 {
        self.adj.page_size().max(1.0) * KEY_LINE_FRACTION
    }

    fn bump(self: &Rc<Self>, delta: f64) {
        if delta == 0.0 {
            return;
        }
        let base = if self.ticking.get() {
            self.target.get()
        } else {
            self.adj.value()
        };
        let target = self.clamped(base + delta);
        let lead = self.clamped(self.adj.value() + delta * SCROLL_LEAD_FRACTION);
        self.adj.set_value(lead);
        self.from.set(lead);
        self.target.set(target);
        self.start_us.set(None);
        self.ensure_tick();
    }

    fn tick(&self, now: i64) -> bool {
        let start = self.start_us.get().unwrap_or_else(|| {
            self.start_us.set(Some(now));
            now
        });
        let t = scroll_progress(now - start);
        let eased = ease_out_cubic(t);
        let from = self.from.get();
        let target = self.target.get();
        if t >= 1.0 {
            self.adj.set_value(target);
            return true;
        }
        self.adj
            .set_value(from + (target - from) * f64::from(eased));
        false
    }

    fn ensure_tick(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let state = self.clone();
        let weak = self.scroll.downgrade();
        self.scroll.add_tick_callback(move |_, clock| {
            let Some(_scroll) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.tick(clock.frame_time()) {
                state.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }
}

fn scroll_progress(elapsed_us: i64) -> f32 {
    ((elapsed_us.max(0) as f64) / (SCROLL_ANIM_MS * 1000.0)).min(1.0) as f32
}

/// Wheel and keyboard scrolling for one gallery `ScrolledWindow`.
#[derive(Clone)]
pub struct GalleryScroller(Rc<SmoothScroll>);

impl GalleryScroller {
    pub fn bump(&self, delta: f64) {
        self.0.bump(delta);
    }

    fn line_delta(&self) -> f64 {
        self.0.line_delta()
    }
}

/// Mouse wheel animates vertically; trackpad keeps GTK kinetic scrolling.
pub fn install_gallery_scroller(scroll: &ScrolledWindow) -> GalleryScroller {
    let state = Rc::new(SmoothScroll {
        scroll: scroll.clone(),
        adj: scroll.vadjustment(),
        from: Cell::new(0.0),
        target: Cell::new(0.0),
        start_us: Cell::new(None),
        ticking: Cell::new(false),
    });

    let controller = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    let scroll_state = state.clone();
    controller.connect_scroll(move |ctrl, _dx, dy| {
        if ctrl
            .current_event_state()
            .contains(ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        match ctrl.unit() {
            ScrollUnit::Wheel => {
                let page = scroll_state.adj.page_size().max(1.0);
                let delta = dy.signum() * page * WHEEL_PAGE_FRACTION * dy.abs().max(1.0);
                scroll_state.bump(delta);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    scroll.add_controller(controller);

    GalleryScroller(state)
}

fn arrow_key_delta(key: Key) -> Option<f64> {
    match key {
        Key::Up | Key::KP_Up => Some(-1.0),
        Key::Down | Key::KP_Down => Some(1.0),
        _ => None,
    }
}

fn typing_in_field(window: &Window) -> bool {
    let Some(focus) = GtkWindowExt::focus(window) else {
        return false;
    };
    let mut current: Option<Widget> = Some(focus);
    while let Some(widget) = current {
        if widget.is::<Editable>() {
            return true;
        }
        current = widget.parent();
    }
    false
}

/// **Up** / **Down** scroll the gallery when focus is not in a text field.
pub fn wire_gallery_arrow_keys(window: &Window, scroller: &GalleryScroller) {
    let key = EventControllerKey::new();
    key.set_propagation_phase(PropagationPhase::Capture);
    let window_for_keys = window.clone();
    let scroller = scroller.clone();
    key.connect_key_pressed(move |_, keyval, _, modifiers| {
        if modifiers.intersects(
            ModifierType::CONTROL_MASK
                | ModifierType::ALT_MASK
                | ModifierType::SHIFT_MASK
                | ModifierType::SUPER_MASK,
        ) {
            return glib::Propagation::Proceed;
        }
        let Some(sign) = arrow_key_delta(keyval) else {
            return glib::Propagation::Proceed;
        };
        if typing_in_field(&window_for_keys) {
            return glib::Propagation::Proceed;
        }
        scroller.bump(sign * scroller.line_delta());
        glib::Propagation::Stop
    });
    window.add_controller(key);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_progress_reaches_one_after_duration() {
        assert_eq!(scroll_progress(0), 0.0);
        assert_eq!(scroll_progress((SCROLL_ANIM_MS * 1000.0) as i64), 1.0);
    }

    #[test]
    fn ease_out_starts_fast_for_responsive_scroll() {
        let early = ease_out_cubic(0.15);
        assert!(early > 0.35, "expected snappy start, got {early}");
    }

    #[test]
    fn arrow_keys_map_to_vertical_signs() {
        assert_eq!(arrow_key_delta(Key::Up), Some(-1.0));
        assert_eq!(arrow_key_delta(Key::Down), Some(1.0));
        assert_eq!(arrow_key_delta(Key::a), None);
    }
}
