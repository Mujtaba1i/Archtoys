//! Pushes colors, the format rows and history into the main window.

use crate::color::{calculate_shades, copy_text, format_canonical, ColorField, Rgb};
use crate::config::{persist_config, HistoryStore};
use crate::formats::FormatList;
use crate::{AppWindow, FieldRow, FormatOption};
use arboard::{Clipboard, SetExtLinux};
use slint::{Color, ComponentHandle, Model, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::thread;
use std::time::Duration;

thread_local! {
    /// The last color fully shown in all rows. Typing a half-finished value
    /// only previews; this is what we go back to if the input is invalid.
    static COMMITTED: Cell<Rgb> = const { Cell::new((203, 182, 172)) };
    /// Bumped for every error, so an old timer doesn't hide a newer message.
    static ERROR_GENERATION: Cell<u64> = const { Cell::new(0) };
    /// Which formats are shown, and in which order.
    static FORMATS: RefCell<FormatList> = RefCell::new(FormatList::default());
    /// The rows on screen. Kept (not recreated) so typing isn't interrupted.
    static ROWS: RefCell<Option<Rc<VecModel<FieldRow>>>> = const { RefCell::new(None) };
    /// The list in the "Customize formats" panel.
    static OPTIONS: RefCell<Option<Rc<VecModel<FormatOption>>>> = const { RefCell::new(None) };
    /// Bumped on every update; a row copies its value into its text field
    /// whenever this changes (see ValueRow in app.slint).
    static REVISION: Cell<i32> = const { Cell::new(0) };
}

/// The last color that was fully applied (see `COMMITTED`).
pub fn committed_rgb() -> Rgb {
    COMMITTED.with(Cell::get)
}

/// A short message at the bottom of the window, for 3 seconds.
pub fn show_message(ui: &AppWindow, text: &str) {
    ui.set_input_error(text.into());
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

/// Shows "Invalid HEX — e.g. #FF46A2" for 3 seconds.
pub fn show_input_error(ui: &AppWindow, field: ColorField) {
    show_message(ui, &format!("Invalid {} — e.g. {}", field.label(), field.example()));
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

// ---------------------------------------------------------------- formats

pub fn formats() -> FormatList {
    FORMATS.with(|f| f.borrow().clone())
}

/// Sets the format list (e.g. from config.json) and rebuilds the rows.
pub fn set_formats(ui: &AppWindow, list: FormatList) {
    FORMATS.with(|f| *f.borrow_mut() = list);
    rebuild_rows(ui);
}

/// Creates the row and option models. Call once at startup.
pub fn install_models(ui: &AppWindow) {
    let rows = Rc::new(VecModel::<FieldRow>::default());
    ui.set_fields(ModelRc::from(rows.clone()));
    ROWS.with(|r| *r.borrow_mut() = Some(rows));
    let options = Rc::new(VecModel::<FormatOption>::default());
    ui.set_format_options(ModelRc::from(options.clone()));
    OPTIONS.with(|o| *o.borrow_mut() = Some(options));
    rebuild_rows(ui);
}

fn next_revision() -> i32 {
    REVISION.with(|r| {
        r.set(r.get().wrapping_add(1));
        r.get()
    })
}

fn make_row(ui: &AppWindow, field: ColorField, rgb: Rgb, auto_field: ColorField, revision: i32) -> FieldRow {
    FieldRow {
        key: field.label().into(),
        label: field.label().into(),
        value: format_canonical(field, rgb).into(),
        rev: revision,
        auto_copy: ui.get_setting_autocopy() && field == auto_field,
        editable: field.editable(),
    }
}

/// Rebuilds the rows and the options list after the format list changed.
fn rebuild_rows(ui: &AppWindow) {
    let list = formats();
    let rgb = committed_rgb();
    let auto_field = list.auto_copy_field();
    let revision = next_revision();
    let rows: Vec<FieldRow> = list
        .enabled()
        .into_iter()
        .map(|f| make_row(ui, f, rgb, auto_field, revision))
        .collect();
    ROWS.with(|r| {
        if let Some(model) = r.borrow().as_ref() {
            model.set_vec(rows);
        }
    });
    let options: Vec<FormatOption> = list
        .items()
        .iter()
        .map(|(f, on)| FormatOption {
            key: f.label().into(),
            label: f.label().into(),
            enabled: *on,
        })
        .collect();
    OPTIONS.with(|o| {
        if let Some(model) = o.borrow().as_ref() {
            model.set_vec(options);
        }
    });
}

/// Updates every row's value, except the one the user is typing in.
fn refresh_values(ui: &AppWindow, rgb: Rgb, skip: Option<ColorField>) {
    let auto_field = formats().auto_copy_field();
    let revision = next_revision();
    ROWS.with(|r| {
        let rows = r.borrow();
        let Some(model) = rows.as_ref() else { return };
        for i in 0..model.row_count() {
            let Some(old) = model.row_data(i) else { continue };
            let Some(field) = ColorField::from_ui_label(&old.key) else { continue };
            if Some(field) == skip {
                continue;
            }
            model.set_row_data(i, make_row(ui, field, rgb, auto_field, revision));
        }
    });
}

/// Turns a format on or off; refuses to turn off the last one.
pub fn toggle_format(ui: &AppWindow, field: ColorField, on: bool) -> Result<(), &'static str> {
    let result = FORMATS.with(|f| f.borrow_mut().set_enabled(field, on));
    rebuild_rows(ui); // also resets the checkbox if it was refused
    result
}

pub fn move_format(ui: &AppWindow, field: ColorField, delta: i32) {
    let moved = FORMATS.with(|f| f.borrow_mut().move_by(field, delta));
    if moved {
        rebuild_rows(ui);
    }
}

/// Re-draws the rows (e.g. after Auto Copy was switched, which moves the highlight).
pub fn refresh_rows(ui: &AppWindow) {
    refresh_values(ui, committed_rgb(), None);
}

// ------------------------------------------------------------ the color

/// Main swatch + the light/dark shades bar.
fn update_preview_color(ui: &AppWindow, rgb: Rgb) {
    ui.set_current_color(to_color(rgb));
    let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(rgb);
    ui.set_shade_lighter_2(to_color(lighter_2));
    ui.set_shade_lighter_1(to_color(lighter_1));
    ui.set_shade_darker_1(to_color(darker_1));
    ui.set_shade_darker_2(to_color(darker_2));
}

/// Swatch, shades and every row.
pub fn update_ui_colors(ui: &AppWindow, rgb: Rgb) {
    COMMITTED.with(|c| c.set(rgb));
    update_preview_color(ui, rgb);
    refresh_values(ui, rgb, None);
}

/// Like `update_ui_colors`, but leaves the row the user is typing in alone.
pub fn update_ui_preview_except_field(ui: &AppWindow, editing_field: ColorField, rgb: Rgb) {
    update_preview_color(ui, rgb);
    refresh_values(ui, rgb, Some(editing_field));
}

// ------------------------------------------------------------ history etc.

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
        // the first format in the list (the name is skipped)
        copy_text_async(copy_text(formats().auto_copy_field(), rgb));
    } else {
        ui.window().show().ok();
    }

    persist_config(ui, history_store);
}
