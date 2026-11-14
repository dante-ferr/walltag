//! Scales sidebar and top-bar chrome (gallery tile zoom is separate).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk::Key;
use gtk4::gdk::{Display, ModifierType};
use gtk4::glib;
use gtk4::prelude::DeviceExt;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CssProvider, EventControllerKey, EventControllerScroll,
    EventControllerScrollFlags, Paned, PickFlags, Widget, STYLE_PROVIDER_PRIORITY_USER,
};
use libadwaita::ApplicationWindow;

use crate::config::{sidebar_limits, AppConfig, ConfigPaths};
use crate::ui::masonry::MasonryGrid;

pub const MIN_GUI_SCALE: f32 = 0.75;
pub const MAX_GUI_SCALE: f32 = 1.75;
const GUI_SCALE_STEP: f32 = 1.1;
pub const DEFAULT_GUI_SCALE: f32 = 1.0;
const SIDEBAR_SECTION_SPACING: i32 = 10;

/// Returns the GUI scale after one step in or out, clamped and snapped near 100%.
pub fn step_gui_scale(scale: f32, scale_up: bool) -> f32 {
    let next = if scale_up {
        scale * GUI_SCALE_STEP
    } else {
        scale / GUI_SCALE_STEP
    };
    let next = next.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    if (next - DEFAULT_GUI_SCALE).abs() < 0.01 {
        DEFAULT_GUI_SCALE
    } else {
        next
    }
}

pub fn install_scale_provider(display: &Display) -> CssProvider {
    let provider = CssProvider::new();
    gtk4::style_context_add_provider_for_display(
        display,
        &provider,
        STYLE_PROVIDER_PRIORITY_USER + 1,
    );
    provider
}

fn px(value_at_100: f32, scale: f32) -> String {
    format!("{:.2}px", value_at_100 * scale)
}

/// Overrides Adwaita theme metrics for chrome only (Libadwaita ignores parent `font-size`).
fn chrome_scale_css(scale: f32) -> String {
    let s = scale;
    format!(
        r#"
        .walltag-main-shell {{
            --wt-s: {s:.4};
        }}
        .walltag-topbar {{
            font-size: {font};
        }}
        .walltag-topbar windowhandle,
        .walltag-topbar headerbar {{
            min-height: {top_h};
            padding-left: {m4};
            padding-right: {m4};
        }}
        .walltag-topbar-btn,
        .walltag-topbar windowcontrols button {{
            min-width: {btn};
            min-height: {btn};
            padding: {btn_pad};
        }}
        .walltag-topbar-btn image,
        .walltag-topbar windowcontrols button image {{
            -gtk-icon-size: {icon};
            min-width: {icon};
            min-height: {icon};
        }}
        .walltag-topbar windowcontrols {{
            border-spacing: {m4};
        }}
        .walltag-zoom-label {{
            min-width: {zoom_w};
            min-height: {btn};
            font-size: {zoom_font};
        }}
        .walltag-loading {{
            margin-end: {m4};
        }}
        .walltag-sidebar-shell {{
            font-size: {font};
        }}
        .walltag-sidebar {{
            margin-top: {m8};
            margin-bottom: {m8};
            margin-start: {m12};
            margin-end: {m10};
            transition: margin 200ms ease;
        }}
        .walltag-sidebar-edit {{
            transition: opacity 200ms ease, margin 200ms ease;
        }}
        .walltag-sidebar preferencesgroup > box {{
            border-spacing: {sp6};
            transition: border-spacing 200ms ease;
        }}
        .walltag-sidebar preferencesgroup > box > box.header,
        .walltag-sidebar preferencesgroup > box > box:first-child {{
            margin-left: {m4};
            margin-right: {m4};
            margin-bottom: {m1};
        }}
        .walltag-sidebar preferencesgroup > box > box > label.heading {{
            font-size: {heading};
        }}
        .walltag-sidebar preferencesgroup > box > box > label.subtitle,
        .walltag-sidebar preferencesgroup > box label.subtitle {{
            font-size: {subtitle};
        }}
        .walltag-sidebar preferencesgroup > box > list,
        .walltag-sidebar preferencesgroup list.boxed-list {{
            border-radius: {r12};
            padding-top: {m4};
            padding-bottom: {m4};
            padding-left: {m1};
            padding-right: {m1};
            margin-top: {m4};
            min-height: {row_h};
            transition: margin 200ms ease, padding 200ms ease;
        }}
        .walltag-sidebar row {{
            min-height: {row_h};
            border-radius: {r8};
            margin-top: {m0};
            margin-bottom: {m0};
            margin-start: {m4};
            margin-end: {m4};
            transition: opacity 180ms ease, margin 180ms ease;
        }}
        .walltag-sidebar row > box.header {{
            min-height: {row_h};
            padding-top: {m3};
            padding-bottom: {m3};
            padding-left: {pad10};
            padding-right: {pad10};
            border-spacing: {sp8};
        }}
        .walltag-sidebar row.spin > box.header {{
            min-height: {row_h};
        }}
        .walltag-sidebar row > box.header > box.title {{
            margin-top: {m3};
            margin-bottom: {m3};
        }}
        .walltag-sidebar row > box.header > box.title > label.title {{
            font-size: {body};
        }}
        .walltag-sidebar row > box.header > box.suffixes button,
        .walltag-sidebar row > box.header > box.suffixes button.pill {{
            min-height: {ctrl_h};
            min-width: {ctrl_h};
            padding-top: {m3};
            padding-bottom: {m3};
            padding-start: {p12};
            padding-end: {p12};
        }}
        .walltag-sidebar row > box.header > box.suffixes entry,
        .walltag-sidebar entry,
        .walltag-sidebar spinbutton {{
            min-height: {ctrl_h};
            padding-top: {p3};
            padding-bottom: {p3};
        }}
        .walltag-sidebar switch {{
            min-width: {sw_w};
            min-height: {sw_h};
        }}
        .walltag-sidebar checkbutton,
        .walltag-sidebar check {{
            min-width: {check};
            min-height: {check};
        }}
        .walltag-sidebar row.combo > box.header > box.suffixes image.dropdown-arrow {{
            -gtk-icon-size: {icon20};
            min-width: {icon20};
            min-height: {icon20};
        }}
        .walltag-overlay-body {{
            margin: 0;
            padding: {m6};
            min-width: {dialog_w};
        }}
        row.combo popover.menu > contents {{
            padding: {m4};
        }}
        row.combo popover.menu > contents listview > row,
        row.combo popover.menu > contents listview > row > box {{
            min-height: {menu_row};
            padding-top: {m3};
            padding-bottom: {m3};
            padding-left: {pad10};
            padding-right: {pad10};
        }}
        "#,
        font = px(14.0, s),
        top_h = px(28.0, s),
        btn = px(22.0, s),
        btn_pad = px(1.0, s),
        icon = px(16.0, s),
        zoom_w = px(24.0, s),
        zoom_font = px(9.8, s),
        m0 = px(0.0, s),
        m1 = px(1.0, s),
        m3 = px(3.0, s),
        m4 = px(4.0, s),
        m6 = px(6.0, s),
        m8 = px(8.0, s),
        m10 = px(10.0, s),
        m12 = px(12.0, s),
        sp6 = px(6.0, s),
        sp8 = px(8.0, s),
        heading = px(10.08, s),
        subtitle = px(11.9, s),
        r12 = px(12.0, s),
        r8 = px(8.0, s),
        row_h = px(32.0, s),
        pad10 = px(10.0, s),
        body = px(14.0, s),
        p12 = px(12.0, s),
        p3 = px(3.0, s),
        icon20 = px(20.0, s),
        dialog_w = px(260.0, s),
        ctrl_h = px(28.0, s),
        sw_w = px(46.0, s),
        sw_h = px(26.0, s),
        check = px(18.0, s),
        menu_row = px(32.0, s),
    )
}

pub fn apply_gui_scale(
    provider: &CssProvider,
    scale: f32,
    previous: f32,
    sidebar_contents: &[GtkBox],
    sidebar_widget: Option<&Widget>,
    split: Option<&Paned>,
) {
    let scale = scale.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    let previous = previous.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    provider.load_from_string(&chrome_scale_css(scale));
    let spacing = (SIDEBAR_SECTION_SPACING as f32 * scale).round() as i32;
    for content in sidebar_contents {
        content.set_spacing(spacing);
    }
    if let (Some(sidebar), Some(split)) = (sidebar_widget, split) {
        sync_sidebar_paned_limits(scale, previous, sidebar, split);
    }
}

/// GtkPaned size request follows the scaled minimum. An open sidebar's width scales
/// with the chrome so the column grows and shrinks, not only its text.
fn sync_sidebar_paned_limits(gui_scale: f32, previous: f32, sidebar: &Widget, split: &Paned) {
    let (min, max) = sidebar_limits(gui_scale);
    sidebar.set_size_request(min, -1);
    if split.shrinks_start_child() {
        return;
    }
    split.set_shrink_start_child(false);
    let ratio = if previous > f32::EPSILON {
        gui_scale / previous
    } else {
        1.0
    };
    let pos = (split.position() as f32 * ratio).round() as i32;
    let pos = pos.clamp(min, max);
    if split.position() != pos {
        split.set_position(pos);
    }
}

fn set_gui_scale(
    config: &Rc<RefCell<AppConfig>>,
    provider: &CssProvider,
    scale: f32,
    sidebar_contents: &[GtkBox],
    sidebar_widget: Option<&Widget>,
    split: Option<&Paned>,
    persist: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    let scale = scale.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    let previous = config.borrow().gui_scale;
    if (previous - scale).abs() < f32::EPSILON {
        apply_gui_scale(
            provider,
            scale,
            previous,
            sidebar_contents,
            sidebar_widget,
            split,
        );
        return;
    }
    config.borrow_mut().gui_scale = scale;
    apply_gui_scale(
        provider,
        scale,
        previous,
        sidebar_contents,
        sidebar_widget,
        split,
    );
    schedule_persist_gui_scale(config.clone(), persist);
}

fn schedule_persist_gui_scale(
    config: Rc<RefCell<AppConfig>>,
    pending: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    if let Some(id) = pending.borrow_mut().take() {
        id.remove();
    }
    let slot = pending.clone();
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        slot.borrow_mut().take();
        if let Err(err) = ConfigPaths::save(&config.borrow()) {
            tracing::error!("Failed to save config: {err:#}");
        }
    });
    *pending.borrow_mut() = Some(id);
}

fn held_modifiers(controller: &EventControllerScroll) -> ModifierType {
    let mut state = controller.current_event_state();
    if let Some(event) = controller.current_event() {
        state |= event.modifier_state();
    }
    if let Some(device) = controller.current_event_device() {
        state |= device.modifier_state();
    }
    state
}

/// Widget under the pointer, in window coordinates. Scroll events can be grabbed by the
/// focused gallery, so the event target is not the hover target.
fn widget_under_pointer(window: &impl WidgetExt) -> Option<Widget> {
    let pointer = window
        .display()
        .default_seat()
        .and_then(|seat| seat.pointer())?;
    let (surface, x, y) = pointer.surface_at_position();
    let surface = surface?;
    if window.native()?.surface().as_ref() != Some(&surface) {
        return None;
    }
    window.pick(x, y, PickFlags::DEFAULT)
}

fn pointer_over(widget: &Widget, hovered: &Widget) -> bool {
    hovered == widget || hovered.is_ancestor(widget)
}

fn step_by_wheel(dy: f64, bump: &Rc<dyn Fn(bool)>) {
    if dy < 0.0 {
        bump(true);
    } else if dy > 0.0 {
        bump(false);
    }
}

fn pointer_over_gallery(window: &ApplicationWindow, gallery: &Widget) -> bool {
    widget_under_pointer(window).is_some_and(|hovered| pointer_over(gallery, &hovered))
}

fn is_zoom_in_key(key: Key) -> bool {
    matches!(key, Key::plus | Key::equal | Key::KP_Add)
}

fn is_zoom_out_key(key: Key) -> bool {
    matches!(key, Key::minus | Key::KP_Subtract)
}

fn is_zoom_reset_key(key: Key) -> bool {
    matches!(key, Key::_0 | Key::KP_0)
}

/// Ctrl+wheel on the gallery zooms tiles; elsewhere it scales chrome.
fn wire_pointer_scroll(
    window: &ApplicationWindow,
    gallery: &Widget,
    grid: &MasonryGrid,
    bump_gui: Rc<dyn Fn(bool)>,
) {
    let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    window.add_controller(scroll.clone());
    let window = window.clone();
    let gallery = gallery.clone();
    let grid = grid.clone();
    scroll.connect_scroll(move |controller, _dx, dy| {
        if !held_modifiers(controller).contains(ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        if pointer_over_gallery(&window, &gallery) {
            if dy < 0.0 {
                grid.zoom_in();
            } else if dy > 0.0 {
                grid.zoom_out();
            }
        } else {
            step_by_wheel(dy, &bump_gui);
        }
        glib::Propagation::Stop
    });
}

/// Ctrl+`+` / Ctrl+`-` / Ctrl+`0` follow the same gallery-vs-chrome split as the wheel.
fn wire_ctrl_scale_keys(
    window: &ApplicationWindow,
    gallery: &Widget,
    grid: &MasonryGrid,
    bump_gui: Rc<dyn Fn(bool)>,
    reset_gui: Rc<dyn Fn()>,
) {
    let key = EventControllerKey::new();
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    window.add_controller(key.clone());
    let window = window.clone();
    let gallery = gallery.clone();
    let grid = grid.clone();
    key.connect_key_pressed(move |_, keyval, _, modifiers| {
        if !modifiers.contains(ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        let over_gallery = pointer_over_gallery(&window, &gallery);
        if is_zoom_in_key(keyval) {
            if over_gallery {
                grid.zoom_in();
            } else {
                bump_gui(true);
            }
            return glib::Propagation::Stop;
        }
        if is_zoom_out_key(keyval) {
            if over_gallery {
                grid.zoom_out();
            } else {
                bump_gui(false);
            }
            return glib::Propagation::Stop;
        }
        if is_zoom_reset_key(keyval) {
            if over_gallery {
                grid.reset_zoom();
            } else {
                reset_gui();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
}

/// Ctrl+wheel and Ctrl+`+` / `-` / `0`: gallery zoom over the gallery, chrome scale elsewhere.
pub fn wire_gui_scale(
    window: &ApplicationWindow,
    gallery: &Widget,
    grid: &MasonryGrid,
    sidebar_contents: &[GtkBox],
    sidebar_widget: &Widget,
    split: &Paned,
    config: Rc<RefCell<AppConfig>>,
    provider: &CssProvider,
) {
    let initial = config.borrow().gui_scale;
    apply_gui_scale(
        provider,
        initial,
        initial,
        sidebar_contents,
        Some(sidebar_widget),
        Some(split),
    );

    let persist: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    let bump = {
        let config = config.clone();
        let provider = provider.clone();
        let persist = persist.clone();
        let sidebar_contents: Vec<GtkBox> = sidebar_contents.to_vec();
        let sidebar_widget = sidebar_widget.clone();
        let split = split.clone();
        Rc::new(move |up: bool| {
            let next = step_gui_scale(config.borrow().gui_scale, up);
            set_gui_scale(
                &config,
                &provider,
                next,
                &sidebar_contents,
                Some(&sidebar_widget),
                Some(&split),
                &persist,
            );
        })
    };

    let reset_gui = {
        let config = config.clone();
        let provider = provider.clone();
        let persist = persist.clone();
        let sidebar_contents: Vec<GtkBox> = sidebar_contents.to_vec();
        let sidebar_widget = sidebar_widget.clone();
        let split = split.clone();
        Rc::new(move || {
            set_gui_scale(
                &config,
                &provider,
                DEFAULT_GUI_SCALE,
                &sidebar_contents,
                Some(&sidebar_widget),
                Some(&split),
                &persist,
            );
        })
    };

    wire_pointer_scroll(window, gallery, grid, bump.clone());
    wire_ctrl_scale_keys(window, gallery, grid, bump, reset_gui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_scale_snaps_near_default() {
        let up = step_gui_scale(DEFAULT_GUI_SCALE, true);
        let back = step_gui_scale(up, false);
        assert_eq!(back, DEFAULT_GUI_SCALE);
    }

    #[test]
    fn gui_scale_clamps_at_max() {
        assert_eq!(step_gui_scale(MAX_GUI_SCALE, true), MAX_GUI_SCALE);
    }

    #[test]
    fn chrome_css_uses_scale_factor() {
        let css = chrome_scale_css(1.2);
        assert!(css.contains("--wt-s: 1.2"));
        assert!(css.contains("16.80px")); // 14 * 1.2 body font
        assert!(css.contains("38.40px")); // 32 * 1.2 row height
    }
}
