//! Pushes colors and history into the main window.

use crate::color::{calculate_shades, format_canonical, format_hex, ColorField, Rgb};
use crate::config::{persist_config, HistoryStore};
use crate::AppWindow;
use arboard::{Clipboard, SetExtLinux};
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use std::cell::Cell;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

thread_local! {
    /// The last color fully shown in all four rows. Typing a half-finished
    /// value only previews; this is what we go back to if the input is invalid.
    static COMMITTED: Cell<Rgb> = const { Cell::new((203, 182, 172)) };
    /// Bumped for every error, so an old timer doesn't hide a newer message.
    static ERROR_GENERATION: Cell<u64> = const { Cell::new(0) };
}

/// The last color that was fully applied (see `COMMITTED`).
pub fn committed_rgb() -> Rgb {
    COMMITTED.with(Cell::get)
}

fn example_for(field: ColorField) -> &'static str {
    match field {
        ColorField::Hex => "e.g. #FF46A2",
        ColorField::Rgb => "e.g. rgb(255, 70, 162)",
        ColorField::Hsl => "e.g. hsl(330, 100%, 64%)",
        ColorField::Hsv => "e.g. hsv(330, 73%, 100%)",
    }
}

fn field_name(field: ColorField) -> &'static str {
    match field {
        ColorField::Hex => "HEX",
        ColorField::Rgb => "RGB",
        ColorField::Hsl => "HSL",
        ColorField::Hsv => "HSV",
    }
}

/// Shows "Invalid HEX — e.g. #FF46A2" for 3 seconds.
pub fn show_input_error(ui: &AppWindow, field: ColorField) {
    ui.set_input_error(format!("Invalid {} — {}", field_name(field), example_for(field)).into());
    let generation = ERROR_GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    let weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_secs(3), move || {
        if ERROR_GENERATION.with(Cell::get) == generation {
            if let Some(ui) = weak.upgrade() {
                ui.set_input_error("".into());
            }
        }
    });
}

pub fn clear_input_error(ui: &AppWindow) {
    ui.set_input_error("".into());
}

fn to_color(rgb: Rgb) -> Color {
    Color::from_rgb_u8(rgb.0, rgb.1, rgb.2)
}

/// The color currently shown in the main window.
pub fn current_rgb(ui: &AppWindow) -> Rgb {
    let c = ui.get_current_color();
    (c.red(), c.green(), c.blue())
}

/// Main swatch + the light/dark shades bar.
fn update_preview_color(ui: &AppWindow, rgb: Rgb) {
    ui.set_current_color(to_color(rgb));
    let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(rgb);
    ui.set_shade_lighter_2(to_color(lighter_2));
    ui.set_shade_lighter_1(to_color(lighter_1));
    ui.set_shade_darker_1(to_color(darker_1));
    ui.set_shade_darker_2(to_color(darker_2));
}

/// Swatch, shades and all four value rows.
pub fn update_ui_colors(ui: &AppWindow, rgb: Rgb) {
    COMMITTED.with(|c| c.set(rgb));
    update_preview_color(ui, rgb);
    ui.set_val_hex(format_canonical(ColorField::Hex, rgb).into());
    ui.set_val_rgb(format_canonical(ColorField::Rgb, rgb).into());
    ui.set_val_hsl(format_canonical(ColorField::Hsl, rgb).into());
    ui.set_val_hsv(format_canonical(ColorField::Hsv, rgb).into());
}

/// Like `update_ui_colors`, but leaves the row the user is typing in alone.
pub fn update_ui_preview_except_field(ui: &AppWindow, editing_field: ColorField, rgb: Rgb) {
    update_preview_color(ui, rgb);
    if editing_field != ColorField::Hex {
        ui.set_val_hex(format_canonical(ColorField::Hex, rgb).into());
    }
    if editing_field != ColorField::Rgb {
        ui.set_val_rgb(format_canonical(ColorField::Rgb, rgb).into());
    }
    if editing_field != ColorField::Hsl {
        ui.set_val_hsl(format_canonical(ColorField::Hsl, rgb).into());
    }
    if editing_field != ColorField::Hsv {
        ui.set_val_hsv(format_canonical(ColorField::Hsv, rgb).into());
    }
}

pub fn sync_history_model(ui: &AppWindow, history_store: &HistoryStore) {
    let colors: Vec<Color> = {
        let guard = history_store.lock().unwrap();
        guard.iter().map(|rgb| to_color(*rgb)).collect()
    };
    ui.set_history_model(ModelRc::from(Rc::new(VecModel::from(colors))));
}

pub fn push_history(history_store: &HistoryStore, rgb: Rgb) {
    history_store.lock().unwrap().insert(0, rgb);
}

pub fn copy_text_async(text: String) {
    thread::spawn(move || match Clipboard::new() {
        Ok(mut clipboard) => {
            let _ = clipboard.set().wait().text(text);
        }
        Err(err) => eprintln!("Clipboard error: {err}"),
    });
}

/// A color was picked: add it to history, show it, copy it if enabled, save.
pub fn apply_selected_color(ui: &AppWindow, history_store: &HistoryStore, rgb: Rgb) {
    push_history(history_store, rgb);
    sync_history_model(ui, history_store);
    update_ui_colors(ui, rgb);

    if ui.get_setting_autocopy() {
        copy_text_async(format_hex(rgb.0, rgb.1, rgb.2));
    } else {
        ui.window().show().ok();
    }

    persist_config(ui, history_store);
}
