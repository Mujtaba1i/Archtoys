//! The main window's size (v0.7.0): the pinned sidebar takes its width beside
//! the toy, so the window and its limits grow by it.

use crate::AppWindow;
use slint::winit_030::{winit, WinitWindowAccessor};
use slint::{ComponentHandle, LogicalSize};

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

/// The window's size limits; the pinned sidebar adds its width.
pub(crate) fn apply_constraints(ui: &AppWindow) {
    let extra = pinned_width(ui.get_sidebar_mode());
    ui.window().with_winit_window(|window| {
        window.set_min_inner_size(Some(winit::dpi::LogicalSize::new(crate::WINDOW_MIN_WIDTH + extra, crate::WINDOW_MIN_HEIGHT)));
        window.set_max_inner_size(Some(winit::dpi::LogicalSize::new(crate::WINDOW_MAX_WIDTH + extra, crate::WINDOW_MAX_HEIGHT)));
        window.set_resizable(true);
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
    let size = window.size().to_logical(window.scale_factor());
    let extra = pinned_width(new);
    let width = (size.width as f64 + extra - pinned_width(old))
        .clamp(crate::WINDOW_MIN_WIDTH + extra, crate::WINDOW_MAX_WIDTH + extra);
    window.set_size(LogicalSize::new(width as f32, size.height));
}
