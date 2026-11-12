//! The Color Picker on the frozen screen (`freeze.rs`): the magnifier and the
//! value next to the cursor, the arrow keys move the pick point one screen
//! pixel, a click or Enter picks.

use super::{
    center_rgb, crop_rgba, finish_picker, magnifier_image, preview_text, show_hover, PickOutcome, PickerContext, MAGNIFIER_RADIUS,
};
use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::freeze::{self, Screenshot, Session, Toy};
use crate::{AppWindow, FreezePickerWindow};
use slint::winit_030::WinitWindowAccessor;
use slint::{Color, ComponentHandle};
use std::cell::Cell;
use std::rc::Rc;

struct ColorPick {
    history_store: HistoryStore,
    context: PickerContext,
    /// The pick point, in screenshot pixels.
    point: Cell<Option<(i32, i32)>>,
    /// The window showing the preview card.
    shown_on: Cell<Option<usize>>,
}

/// Shows the frozen screen for picking a color. UI thread only. `fallback`
/// runs instead if the screenshot cannot be used.
pub(crate) fn show(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    shot: Screenshot,
    fallback: Box<dyn FnOnce()>,
) {
    let toy = Rc::new(ColorPick { history_store, context, point: Cell::new(None), shown_on: Cell::new(None) });
    freeze::show(ui_weak, toy, shot, fallback);
}

/// Puts the main window above the freeze window, without moving it, so the
/// user sees the live values while picking ("Minimize on Pick" off).
fn raise_main_window(ui: &AppWindow) {
    ui.set_keep_on_top(true); // X11: the freeze window is always-on-top too
    ui.set_picking(true);
    ui.window().with_winit_window(|window| window.focus_window());
    ui.invoke_focus_pick_scope();
}

/// Undoes `raise_main_window`. UI thread only.
pub(super) fn end_main_window_raise(ui: &AppWindow) {
    ui.set_keep_on_top(false);
    ui.set_picking(false);
}

fn rgb_at(s: &Session, (ix, iy): (i32, i32)) -> (Vec<u8>, Rgb) {
    let area = crop_rgba(&s.shot.rgba, s.shot.width, s.shot.height, ix, iy, MAGNIFIER_RADIUS);
    let rgb = center_rgb(&area, MAGNIFIER_RADIUS);
    (area, rgb)
}

impl ColorPick {
    fn finish(&self, s: &Session, outcome: PickOutcome) {
        if s.end() {
            finish_picker(s.ui_weak.clone(), self.history_store.clone(), self.context, outcome);
        }
    }

    /// Shows the preview at the pick point, on the monitor it is on.
    fn show_point(&self, s: &Session, marker: bool) {
        let Some(point) = self.point.get() else { return };
        let Some((i, x, y)) = s.layout.borrow().as_ref().and_then(|l| l.place(point.0, point.1)) else {
            return;
        };
        let (area, rgb) = rgb_at(s, point);
        let windows = s.windows.borrow();
        if let Some(old) = self.shown_on.replace(Some(i)).filter(|&old| old != i) {
            if let Some((w, _)) = windows.get(old) {
                w.set_preview_hex("".into());
                w.set_marker(false);
            }
        }
        let Some((w, _)) = windows.get(i) else { return };
        w.set_cursor_x(x);
        w.set_cursor_y(y);
        w.set_marker(marker);
        w.set_magnifier(magnifier_image(&area, MAGNIFIER_RADIUS));
        w.set_preview_color(Color::from_rgb_u8(rgb.0, rgb.1, rgb.2));
        w.set_preview_hex(preview_text(rgb));
        drop(windows);
        if let Some(ui) = s.ui_weak.upgrade() {
            show_hover(&ui, rgb);
        }
    }
}

impl Toy for ColorPick {
    fn window_created(&self, window: &FreezePickerWindow, _i: usize) {
        window.set_text_mode(false);
    }

    fn pointer(&self, s: &Rc<Session>, point: (i32, i32)) {
        self.point.set(Some(point));
        self.show_point(s, false);
    }

    fn click(&self, s: &Rc<Session>, i: usize, x: f32, y: f32) {
        if let Some(point) = s.pixel(i, x, y) {
            let (_, rgb) = rgb_at(s, point);
            self.finish(s, PickOutcome::Picked(rgb));
        }
    }

    fn nudge(&self, s: &Rc<Session>, dx: i32, dy: i32) {
        let (w, h) = (s.shot.width, s.shot.height);
        let moved = {
            let layout = s.layout.borrow();
            let Some(layout) = layout.as_ref() else { return };
            // no point yet (the mouse hasn't moved): start in the middle of the first monitor
            let start = self.point.get().unwrap_or_else(|| {
                let r = layout.screens[0].rect;
                let (x, y) = ((r.w / layout.screens[0].unit) / 2.0, (r.h / layout.screens[0].unit) / 2.0);
                layout.pixel(0, x, y, w, h)
            });
            layout.nudge(start, dx, dy, w, h).or(Some(start))
        };
        self.point.set(moved);
        self.show_point(s, true);
    }

    fn enter(&self, s: &Rc<Session>) {
        if let Some(point) = self.point.get() {
            let (_, rgb) = rgb_at(s, point);
            self.finish(s, PickOutcome::Picked(rgb));
        }
    }

    fn cancelled(&self, s: &Rc<Session>) {
        finish_picker(s.ui_weak.clone(), self.history_store.clone(), self.context, PickOutcome::Cancelled);
    }

    /// If the main window is open (and not minimized), bring it above the
    /// freeze windows once they are up, so it isn't hidden behind the screenshot.
    fn shown(&self, s: &Rc<Session>) {
        if let Some(ui) = s.ui_weak.upgrade() {
            if ui.window().is_visible() && !ui.window().is_minimized() {
                raise_main_window(&ui);
            }
        }
    }
}
