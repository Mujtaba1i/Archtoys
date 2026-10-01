//! Freeze-frame picker: one screenshot, shown fullscreen in our own window.
//! Inside our own window we may read the mouse position, even on Wayland,
//! so we can show the same live preview and magnifier as on X11.
//!
//! The screenshot file is deleted as soon as it has been loaded into memory.

use super::{center_rgb, crop_rgba, finish_picker, magnifier_image, show_hover, PickOutcome, PickerContext, MAGNIFIER_RADIUS};
use crate::color::{format_hex, Rgb};
use crate::config::HistoryStore;
use crate::{AppWindow, FreezePickerWindow};
use slint::{Color, ComponentHandle, SharedPixelBuffer};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct Screenshot {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

impl Screenshot {
    fn pixel_area(&self, x: i32, y: i32) -> Vec<u8> {
        crop_rgba(&self.rgba, self.width, self.height, x, y, MAGNIFIER_RADIUS)
    }
}

thread_local! {
    static FREEZE_WINDOW: RefCell<Option<FreezePickerWindow>> = const { RefCell::new(None) };
    /// Cancels the running freeze pick (used by Esc in the main window).
    static CANCEL: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Cancels the freeze pick, if one is running. UI thread only.
pub(crate) fn cancel_active() {
    let cancel = CANCEL.with(|c| c.borrow().clone());
    if let Some(cancel) = cancel {
        cancel();
    }
}

/// Puts the main window above the freeze window, without moving it, so the
/// user sees the live values while picking ("Minimize on Pick" off).
fn raise_main_window(ui: &AppWindow) {
    use slint::winit_030::WinitWindowAccessor;
    ui.set_keep_on_top(true); // X11: the freeze window is always-on-top too
    ui.set_picking(true);
    ui.window().with_winit_window(|window| window.focus_window());
    ui.invoke_focus_pick_scope();
}

/// Undoes `raise_main_window`. UI thread only.
pub(super) fn end_main_window_raise(ui: &AppWindow) {
    ui.set_keep_on_top(false);
    ui.set_picking(false);
    CANCEL.with(|c| *c.borrow_mut() = None);
}

/// Hides and drops the freeze window. UI thread only.
pub(super) fn release_window() {
    FREEZE_WINDOW.with(|slot| {
        if let Some(window) = slot.borrow_mut().take() {
            window.hide().ok();
        }
    });
}

/// Loads a PNG screenshot and deletes the file right away.
pub(crate) fn load_png(path: &Path) -> Result<Screenshot, String> {
    let loaded = image::open(path);
    if let Err(err) = std::fs::remove_file(path) {
        eprintln!("freeze picker: could not delete {}: {err}", path.display());
    }
    let img = loaded
        .map_err(|err| format!("reading screenshot {}: {err}", path.display()))?
        .into_rgba8();
    let (width, height) = img.dimensions();
    Ok(Screenshot {
        rgba: img.into_raw(),
        width,
        height,
    })
}

/// Logical window position → screenshot pixel.
fn to_image_pixel(x: f32, y: f32, window_w: f32, window_h: f32, img_w: u32, img_h: u32) -> (i32, i32) {
    if window_w <= 0.0 || window_h <= 0.0 {
        return (0, 0);
    }
    let ix = (x / window_w * img_w as f32).floor() as i32;
    let iy = (y / window_h * img_h as f32).floor() as i32;
    (ix.clamp(0, img_w as i32 - 1), iy.clamp(0, img_h as i32 - 1))
}

/// The screenshot shows only this window's monitor if the shapes match.
/// (With several monitors it shows all of them, and we can't map it yet.)
fn same_shape(img_w: u32, img_h: u32, win_w: f32, win_h: f32) -> bool {
    if win_w < 2.0 || win_h < 2.0 || img_h == 0 {
        return false;
    }
    let img_ratio = img_w as f32 / img_h as f32;
    let win_ratio = win_w / win_h;
    (img_ratio - win_ratio).abs() / win_ratio < 0.03
}

/// Checks every 100 ms (up to 3 s) until the window has its fullscreen size,
/// then falls back if the screenshot's shape doesn't match the window.
fn check_fit(
    weak: slint::Weak<FreezePickerWindow>,
    done: Rc<Cell<bool>>,
    fallback: Rc<RefCell<Option<Box<dyn FnOnce()>>>>,
    img_w: u32,
    img_h: u32,
    attempt: u32,
) {
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let Some(w) = weak.upgrade() else { return };
        if done.get() {
            return;
        }
        let size = w.window().size();
        let scale = w.window().scale_factor();
        let (win_w, win_h) = (size.width as f32 / scale, size.height as f32 / scale);
        let sized = win_w > 100.0 && win_h > 100.0;
        if !sized && attempt < 30 {
            check_fit(weak, done, fallback, img_w, img_h, attempt + 1);
            return;
        }
        if !same_shape(img_w, img_h, win_w, win_h) {
            eprintln!(
                "freeze picker: screenshot {img_w}x{img_h} doesn't match the window \
                 {win_w:.0}x{win_h:.0} (several monitors?); using the other picker"
            );
            done.set(true);
            release_window();
            if let Some(fallback) = fallback.borrow_mut().take() {
                fallback();
            }
        }
    });
}

/// Shows the freeze window. UI thread only. `fallback` runs instead if the
/// screenshot cannot be used (for example with several monitors).
pub(crate) fn show(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    shot: Screenshot,
    fallback: Box<dyn FnOnce()>,
) {
    let window = match FreezePickerWindow::new() {
        Ok(window) => window,
        Err(err) => {
            eprintln!("freeze picker: window failed: {err:?}");
            fallback();
            return;
        }
    };

    let buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&shot.rgba, shot.width, shot.height);
    window.set_screenshot_image(slint::Image::from_rgba8(buffer));

    let shot = Rc::new(shot);
    let done = Rc::new(Cell::new(false));
    let fallback = Rc::new(RefCell::new(Some(fallback)));

    // Ends picking exactly once.
    let finish = {
        let done = done.clone();
        let ui_weak = ui_weak.clone();
        let history_store = history_store.clone();
        Rc::new(move |outcome: PickOutcome| {
            if !done.replace(true) {
                finish_picker(ui_weak.clone(), history_store.clone(), context, outcome);
            }
        })
    };

    let logical_size = |w: &FreezePickerWindow| {
        let size = w.window().size();
        let scale = w.window().scale_factor();
        (size.width as f32 / scale, size.height as f32 / scale)
    };

    let pixel_at = {
        let shot = shot.clone();
        let weak = window.as_weak();
        move |x: f32, y: f32| -> Option<(Vec<u8>, Rgb)> {
            let w = weak.upgrade()?;
            let (win_w, win_h) = logical_size(&w);
            let (ix, iy) = to_image_pixel(x, y, win_w, win_h, shot.width, shot.height);
            let area = shot.pixel_area(ix, iy);
            let rgb = center_rgb(&area, MAGNIFIER_RADIUS);
            Some((area, rgb))
        }
    };
    let pixel_at = Rc::new(pixel_at);

    {
        let weak = window.as_weak();
        let ui_weak = ui_weak.clone();
        let pixel_at = pixel_at.clone();
        window.on_pointer_moved(move |x, y| {
            let (Some(w), Some((area, rgb))) = (weak.upgrade(), pixel_at(x, y)) else {
                return;
            };
            w.set_cursor_x(x);
            w.set_cursor_y(y);
            w.set_magnifier(magnifier_image(&area, MAGNIFIER_RADIUS));
            w.set_preview_color(Color::from_rgb_u8(rgb.0, rgb.1, rgb.2));
            w.set_preview_hex(format_hex(rgb.0, rgb.1, rgb.2).into());
            if let Some(ui) = ui_weak.upgrade() {
                show_hover(&ui, rgb);
            }
        });
    }
    {
        let finish = finish.clone();
        let pixel_at = pixel_at.clone();
        window.on_pick_at(move |x, y| {
            if let Some((_, rgb)) = pixel_at(x, y) {
                finish(PickOutcome::Picked(rgb));
            }
        });
    }
    {
        let finish = finish.clone();
        window.on_cancel_pick(move || finish(PickOutcome::Cancelled));
    }
    {
        let finish = finish.clone();
        window.window().on_close_requested(move || {
            finish(PickOutcome::Cancelled);
            slint::CloseRequestResponse::HideWindow
        });
    }

    {
        let finish = finish.clone();
        CANCEL.with(|c| *c.borrow_mut() = Some(Rc::new(move || finish(PickOutcome::Cancelled))));
    }

    if let Err(err) = window.show() {
        eprintln!("freeze picker: show failed: {err:?}");
        if let Some(fallback) = fallback.borrow_mut().take() {
            done.set(true);
            fallback();
        }
        return;
    }

    // Once the window has its fullscreen size, check the screenshot fits it.
    check_fit(window.as_weak(), done.clone(), fallback.clone(), shot.width, shot.height, 0);

    // If the main window is open (and not minimized), bring it above the
    // freeze window once that is up, so it isn't hidden behind the screenshot.
    {
        let ui_weak = ui_weak.clone();
        let done = done.clone();
        slint::Timer::single_shot(Duration::from_millis(150), move || {
            if done.get() {
                return;
            }
            if let Some(ui) = ui_weak.upgrade() {
                if ui.window().is_visible() && !ui.window().is_minimized() {
                    raise_main_window(&ui);
                }
            }
        });
    }

    FREEZE_WINDOW.with(|slot| *slot.borrow_mut() = Some(window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_window_position_to_image_pixel() {
        // 1920x1080 logical window, 3840x2160 screenshot (200% scaling)
        assert_eq!(to_image_pixel(0.0, 0.0, 1920.0, 1080.0, 3840, 2160), (0, 0));
        assert_eq!(to_image_pixel(960.0, 540.0, 1920.0, 1080.0, 3840, 2160), (1920, 1080));
        assert_eq!(to_image_pixel(5000.0, -3.0, 1920.0, 1080.0, 3840, 2160), (3839, 0));
    }

    #[test]
    fn detects_multi_monitor_screenshots() {
        assert!(same_shape(3840, 2160, 1920.0, 1080.0)); // one HiDPI monitor
        assert!(same_shape(2560, 1440, 1706.7, 960.0)); // 150% scaling
        assert!(!same_shape(3840, 1080, 1920.0, 1080.0)); // two monitors side by side
        assert!(!same_shape(1920, 1080, 1.0, 1.0)); // window not sized yet
    }
}
