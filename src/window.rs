//! The main window's size (v0.7.0): the pinned sidebar takes its width beside
//! the toy, so the window and its limits grow by it.

use crate::ui_state::{to_logical, to_physical};
use crate::AppWindow;
use slint::winit_030::{winit, WinitWindowAccessor};
use slint::ComponentHandle;

/// The pinned sidebar's width (`side-w` in app.slint).
pub(crate) const SIDEBAR_WIDTH: f64 = 178.0;
/// Its width with only the icons pinned (`rail-w` in app.slint).
const RAIL_WIDTH: f64 = 60.0;

/// How much room the pinned sidebar takes (mode 0 none, 1 icons, 2 all).
fn pinned_width(mode: i32) -> f64 {
    match mode {
        1 => RAIL_WIDTH,
        2 => SIDEBAR_WIDTH,
        _ => 0.0,
    }
}

/// The window's size limits; the pinned sidebar adds its width. In physical
/// pixels with Slint's scale factor (see `ui_state::to_physical`).
pub(crate) fn apply_constraints(ui: &AppWindow) {
    let extra = pinned_width(ui.get_sidebar_mode());
    let scale = ui.window().scale_factor();
    let limit = |w: f64, h: f64| {
        let size = to_physical((w as f32, h as f32), scale);
        winit::dpi::PhysicalSize::new(size.width, size.height)
    };
    ui.window().with_winit_window(|window| {
        window.set_min_inner_size(Some(limit(crate::WINDOW_MIN_WIDTH + extra, crate::WINDOW_MIN_HEIGHT)));
        window.set_max_inner_size(Some(limit(crate::WINDOW_MAX_WIDTH + extra, crate::WINDOW_MAX_HEIGHT)));
        window.set_resizable(true);
    });
}

/// The bar at the top is being dragged: the desktop moves the window (and
/// still snaps it).
pub(crate) fn start_move(ui: &AppWindow) {
    ui.window().with_winit_window(|window| {
        if let Err(err) = window.drag_window() {
            eprintln!("window: can't move it: {err}");
        }
    });
    // The desktop takes the mouse for the move, so Archtoys never hears the
    // button go up: the bar would keep the mouse and treat the next press
    // anywhere in the window as a drag of the bar. Let go of it (after this
    // event, as Slint is still handling it).
    let ui = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || {
        if let Some(ui) = ui.upgrade() {
            let _ = ui.window().dispatch_event_with_result(slint::platform::WindowEvent::PointerExited);
        }
    });
}

/// The sidebar was pinned or unpinned (or switched between icons and all of
/// it): it takes its width beside the toy, so the window grows by the
/// difference (or gives it back), and the toy keeps its size.
pub(crate) fn sidebar_mode_changed(ui: &AppWindow, old: i32, new: i32) {
    apply_constraints(ui);
    if ui.window().is_maximized() || ui.window().is_fullscreen() {
        return;
    }
    let window = ui.window();
    let scale = window.scale_factor();
    let (w, h) = to_logical(window.size(), scale);
    let extra = pinned_width(new);
    let width = (w as f64 + extra - pinned_width(old)).clamp(crate::WINDOW_MIN_WIDTH + extra, crate::WINDOW_MAX_WIDTH + extra);
    window.set_size(to_physical((width as f32, h), scale));
}
