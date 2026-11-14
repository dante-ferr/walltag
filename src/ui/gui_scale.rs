//! Scales sidebar and top-bar chrome (gallery tile zoom is separate).

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk::{Display, ModifierType};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, CssProvider, EventControllerScroll, EventControllerScrollFlags, Paned, Widget,
    STYLE_PROVIDER_PRIORITY_USER,
};
use libadwaita::{gio, Application, ApplicationWindow};

use crate::config::{sidebar_limits, AppConfig, ConfigPaths};

pub const MIN_GUI_SCALE: f32 = 0.75;
pub const MAX_GUI_SCALE: f32 = 1.75;
const GUI_SCALE_STEP: f32 = 1.1;
pub const DEFAULT_GUI_SCALE: f32 = 1.0;
const SIDEBAR_SECTION_SPACING: i32 = 16;

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
        }}
        .walltag-topbar-btn {{
            min-width: {btn};
            min-height: {btn};
            padding: {btn_pad};
        }}
        .walltag-topbar-btn image {{
            -gtk-icon-size: {icon};
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
            margin-top: {m12};
            margin-bottom: {m12};
            margin-start: {m16};
            margin-end: {m14};
        }}
        .walltag-sidebar preferencesgroup > box {{
            border-spacing: {sp8};
        }}
        .walltag-sidebar preferencesgroup > box > box.header,
        .walltag-sidebar preferencesgroup > box > box:first-child {{
            margin-left: {m4};
            margin-right: {m4};
            margin-bottom: {m2};
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
            padding-top: {m2};
            padding-bottom: {m2};
            margin-top: {m2};
        }}
        .walltag-sidebar row {{
            border-radius: {r8};
            margin-top: {m1};
            margin-bottom: {m1};
            margin-start: {m4};
            margin-end: {m4};
        }}
        .walltag-sidebar row > box.header {{
            min-height: {row_h};
            padding-left: {pad12};
            padding-right: {pad12};
            border-spacing: {sp10};
        }}
        .walltag-sidebar row.spin > box.header {{
            min-height: {row_h};
        }}
        .walltag-sidebar row > box.header > box.title {{
            margin-top: {m4};
            margin-bottom: {m4};
        }}
        .walltag-sidebar row > box.header > box.title > label.title {{
            font-size: {body};
        }}
        .walltag-sidebar row > box.header > box.suffixes button.pill {{
            padding-top: {m4};
            padding-bottom: {m4};
            padding-start: {p14};
            padding-end: {p14};
        }}
        .walltag-sidebar row > box.header > box.suffixes entry {{
            padding-top: {p3};
            padding-bottom: {p3};
        }}
        .walltag-sidebar row.combo > box.header > box.suffixes image.dropdown-arrow {{
            -gtk-icon-size: {icon20};
            min-width: {icon20};
            min-height: {icon20};
        }}
        "#
        ,
        font = px(14.0, s),
        top_h = px(28.0, s),
        btn = px(22.0, s),
        btn_pad = px(1.0, s),
        icon = px(16.0, s),
        zoom_w = px(24.0, s),
        zoom_font = px(9.8, s),
        m4 = px(4.0, s),
        m12 = px(12.0, s),
        m14 = px(14.0, s),
        m16 = px(16.0, s),
        m2 = px(2.0, s),
        m1 = px(1.0, s),
        sp8 = px(8.0, s),
        sp10 = px(10.0, s),
        heading = px(10.08, s),
        subtitle = px(11.9, s),
        r12 = px(12.0, s),
        r8 = px(8.0, s),
        row_h = px(36.0, s),
        pad12 = px(12.0, s),
        body = px(14.0, s),
        p14 = px(14.0, s),
        p3 = px(3.0, s),
        icon20 = px(20.0, s),
    )
}

pub fn apply_gui_scale(
    provider: &CssProvider,
    scale: f32,
    sidebar_content: Option<&GtkBox>,
    sidebar_widget: Option<&Widget>,
    split: Option<&Paned>,
) {
    let scale = scale.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    provider.load_from_string(&chrome_scale_css(scale));
    if let Some(content) = sidebar_content {
        content.set_spacing((SIDEBAR_SECTION_SPACING as f32 * scale).round() as i32);
    }
    if let (Some(sidebar), Some(split)) = (sidebar_widget, split) {
        sync_sidebar_paned_limits(scale, sidebar, split);
    }
}

/// GtkPaned size request and position clamps follow scaled chrome width.
fn sync_sidebar_paned_limits(gui_scale: f32, sidebar: &Widget, split: &Paned) {
    let (min, max) = sidebar_limits(gui_scale);
    sidebar.set_size_request(min, -1);
    if split.shrinks_start_child() {
        return;
    }
    split.set_shrink_start_child(false);
    let pos = split.position().clamp(min, max);
    if split.position() != pos {
        split.set_position(pos);
    }
}

fn set_gui_scale(
    config: &Rc<RefCell<AppConfig>>,
    provider: &CssProvider,
    scale: f32,
    sidebar_content: Option<&GtkBox>,
    sidebar_widget: Option<&Widget>,
    split: Option<&Paned>,
    persist: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    let scale = scale.clamp(MIN_GUI_SCALE, MAX_GUI_SCALE);
    if (config.borrow().gui_scale - scale).abs() < f32::EPSILON {
        apply_gui_scale(provider, scale, sidebar_content, sidebar_widget, split);
        return;
    }
    config.borrow_mut().gui_scale = scale;
    apply_gui_scale(provider, scale, sidebar_content, sidebar_widget, split);
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

fn wire_super_wheel(widget: &Widget, bump: Rc<dyn Fn(bool)>) {
    let scroll = EventControllerScroll::new(EventControllerScrollFlags::VERTICAL);
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    widget.add_controller(scroll.clone());
    scroll.connect_scroll(move |controller, _dx, dy| {
        let super_held = controller.current_event().is_some_and(|event| {
            event
                .modifier_state()
                .contains(ModifierType::SUPER_MASK)
        });
        if !super_held {
            return glib::Propagation::Proceed;
        }
        if dy < 0.0 {
            bump(true);
        } else if dy > 0.0 {
            bump(false);
        }
        glib::Propagation::Stop
    });
}

/// Super+wheel on the sidebar or top bar scales chrome; Super+`+` / Super+`-` work globally.
pub fn wire_gui_scale(
    app: &Application,
    window: &ApplicationWindow,
    wheel_targets: &[Widget],
    sidebar_content: Option<&GtkBox>,
    sidebar_widget: &Widget,
    split: &Paned,
    config: Rc<RefCell<AppConfig>>,
    provider: &CssProvider,
) {
    let initial = config.borrow().gui_scale;
    apply_gui_scale(
        provider,
        initial,
        sidebar_content,
        Some(sidebar_widget),
        Some(split),
    );

    let persist: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    let bump = {
        let config = config.clone();
        let provider = provider.clone();
        let persist = persist.clone();
        let sidebar = sidebar_content.cloned();
        let sidebar_widget = sidebar_widget.clone();
        let split = split.clone();
        Rc::new(move |up: bool| {
            let next = step_gui_scale(config.borrow().gui_scale, up);
            set_gui_scale(
                &config,
                &provider,
                next,
                sidebar.as_ref(),
                Some(&sidebar_widget),
                Some(&split),
                &persist,
            );
        })
    };

    for target in wheel_targets {
        wire_super_wheel(target, bump.clone());
    }

    let zoom_in = gio::SimpleAction::new("gui-scale-in", None);
    {
        let bump = bump.clone();
        zoom_in.connect_activate(move |_, _| bump(true));
    }
    let zoom_out = gio::SimpleAction::new("gui-scale-out", None);
    {
        let bump = bump.clone();
        zoom_out.connect_activate(move |_, _| bump(false));
    };
    window.add_action(&zoom_in);
    window.add_action(&zoom_out);
    app.set_accels_for_action(
        "win.gui-scale-in",
        &[
            "<Meta>plus",
            "<Meta>equal",
            "<Meta><Shift>equal",
            "<Meta>KP_Add",
        ],
    );
    app.set_accels_for_action(
        "win.gui-scale-out",
        &["<Meta>minus", "<Meta>KP_Subtract"],
    );
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
        assert!(css.contains("16.80px")); // 14 * 1.2
    }
}