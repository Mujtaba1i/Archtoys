//! Picking a color from the screen.
//!
//! * **X11**: live. We read only the few pixels under the cursor (`x11.rs`).
//! * **Wayland**: "freeze frame". We take one screenshot through the portal,
//!   show it fullscreen, and the user picks from it (`freeze.rs`). If that is
//!   not possible we fall back to KDE's / the portal's own picker (`wayland.rs`).
//!
//! While hovering, the main window shows the hovered color. Esc (or right
//! click) puts back the color that was shown before picking started.

mod freeze;
mod wayland;
mod x11;

use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::ui_state::{apply_selected_color, current_rgb, update_ui_colors};
use crate::AppWindow;
use slint::ComponentHandle;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) static PICKER_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(crate) static PICKER_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Half the magnifier size: 5 → an 11×11 pixel area around the cursor.
pub(crate) const MAGNIFIER_RADIUS: i32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerSource {
    Hotkey,
    Button,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PickerContext {
    source: PickerSource,
    was_visible_before_trigger: bool,
    /// Shown again if the user cancels.
    previous: Rgb,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum PickOutcome {
    Picked(Rgb),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionType {
    X11,
    Wayland,
    Unknown,
}

fn detect_session_type() -> SessionType {
    match std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "x11" => SessionType::X11,
        "wayland" => SessionType::Wayland,
        _ => SessionType::Unknown,
    }
}

/// Starts picking. Call on the UI thread.
pub fn start_picker(ui: &AppWindow, history_store: HistoryStore, source: PickerSource) {
    if PICKER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }
    PICKER_CANCELLED.store(false, Ordering::SeqCst);

    let was_visible = ui.window().is_visible();
    if ui.get_setting_minimize() {
        ui.window().hide().ok();
    }

    let context = PickerContext {
        source,
        was_visible_before_trigger: was_visible,
        previous: current_rgb(ui),
    };
    let ui_weak = ui.as_weak();
    // Give the compositor a moment to remove our window before a screenshot.
    let hid_window = was_visible && ui.get_setting_minimize();

    // ARCHTOYS_PICKER=freeze uses the freeze frame on X11 too (for testing);
    // ARCHTOYS_PICKER=system uses the desktop's own picker on Wayland.
    let mode = std::env::var("ARCHTOYS_PICKER").unwrap_or_default().to_ascii_lowercase();
    let force_freeze = mode == "freeze";
    let force_system = mode == "system";

    match detect_session_type() {
        SessionType::Wayland if force_system => wayland::start_system(ui_weak, history_store, context),
        SessionType::Wayland => wayland::start(ui_weak, history_store, context, hid_window),
        SessionType::X11 | SessionType::Unknown if force_freeze => {
            x11::start_freeze(ui_weak, history_store, context, hid_window)
        }
        SessionType::X11 | SessionType::Unknown => x11::start(ui_weak, history_store, context),
    }
}

/// Cancels the current pick (Esc pressed in the main window).
pub fn cancel_picker() {
    PICKER_CANCELLED.store(true, Ordering::SeqCst); // X11 live picker
    freeze::cancel_active(); // freeze frame
}

/// Live preview while hovering: swatch, shades and all four values.
pub(crate) fn show_hover(ui: &AppWindow, rgb: Rgb) {
    update_ui_colors(ui, rgb);
}

/// Ends picking. Safe to call from any thread.
pub(crate) fn finish_picker(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    outcome: PickOutcome,
) {
    let result = slint::invoke_from_event_loop(move || {
        x11::release_windows();
        freeze::release_window();
        if let Some(ui) = ui_weak.upgrade() {
            freeze::end_main_window_raise(&ui);
            let selected = match outcome {
                PickOutcome::Picked(rgb) => {
                    apply_selected_color(&ui, &history_store, rgb);
                    true
                }
                PickOutcome::Cancelled => {
                    update_ui_colors(&ui, context.previous);
                    false
                }
            };

            let stealth = selected
                && context.source == PickerSource::Hotkey
                && !context.was_visible_before_trigger
                && ui.get_setting_autocopy();

            if ui.get_setting_minimize() && !stealth {
                ui.window().show().ok();
            }
        }

        PICKER_ACTIVE.store(false, Ordering::SeqCst);
    });
    if let Err(err) = result {
        eprintln!("finish_picker: invoke_from_event_loop error: {err:?}");
        PICKER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Copies an `(2r+1)²` area centred on (cx, cy) out of an RGBA image, as RGBA.
/// Pixels outside the image are black.
pub(crate) fn crop_rgba(rgba: &[u8], width: u32, height: u32, cx: i32, cy: i32, radius: i32) -> Vec<u8> {
    let size = (2 * radius + 1) as usize;
    let mut out = vec![0u8; size * size * 4];
    for dy in 0..size as i32 {
        for dx in 0..size as i32 {
            let x = cx - radius + dx;
            let y = cy - radius + dy;
            let o = (dy as usize * size + dx as usize) * 4;
            if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                let i = (y as usize * width as usize + x as usize) * 4;
                out[o..o + 3].copy_from_slice(&rgba[i..i + 3]);
            }
            out[o + 3] = 255;
        }
    }
    out
}

/// The center pixel of a square RGBA area made by `crop_rgba`.
pub(crate) fn center_rgb(area: &[u8], radius: i32) -> Rgb {
    let size = (2 * radius + 1) as usize;
    let i = (radius as usize * size + radius as usize) * 4;
    (area[i], area[i + 1], area[i + 2])
}

pub(crate) fn magnifier_image(area: &[u8], radius: i32) -> slint::Image {
    let size = (2 * radius + 1) as u32;
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(area, size, size);
    slint::Image::from_rgba8(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_fills_outside_with_black_and_finds_center() {
        // 2x1 image: red, green
        let img = [255, 0, 0, 255, 0, 255, 0, 255];
        let area = crop_rgba(&img, 2, 1, 1, 0, 1);
        assert_eq!(area.len(), 3 * 3 * 4);
        assert_eq!(center_rgb(&area, 1), (0, 255, 0));
        // left neighbour of the center is red, top row is outside → black
        assert_eq!(&area[12..16], &[255, 0, 0, 255]);
        assert_eq!(&area[0..4], &[0, 0, 0, 255]);
    }
}
