//! The color editor (v0.6.0): the panel the pen on the middle shade opens.
//! It edits a color, and with Compare on a second color shown on the right
//! half of the card, and changes nothing until Use.

use crate::color::{contrast_ratio, format_bare, from_hsv, parse_color, to_hsv, wants_dark_text, ColorField, Rgb};
use crate::config::{persist_config, HistoryStore};
use crate::ui_state::{committed_rgb, copy_text_async, formats, push_history, show_input_error, sync_history_model, update_ui_colors};
use crate::{AppWindow, FieldRow};
use serde::{Deserialize, Serialize};
use slint::{Color, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

/// Hue in degrees, saturation and brightness 0 to 1. Kept like this (not as
/// RGB) so dragging through grays doesn't lose the hue.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Hsv {
    h: f32,
    s: f32,
    v: f32,
}

impl Hsv {
    fn of(rgb: Rgb) -> Self {
        let (h, s, v) = to_hsv(rgb);
        Self { h, s, v }
    }
    fn rgb(self) -> Rgb {
        from_hsv(self.h, self.s, self.v)
    }
}

#[derive(Default)]
struct Editor {
    /// The edited color (the whole card while Compare is off). None: the
    /// selected color, nothing edited yet.
    edit: Option<Hsv>,
    /// The selected color the edit started from.
    base: Rgb,
    /// The second color (the right half while Compare is on).
    compare: Option<Hsv>,
    /// The next pick goes to the compare half (decided when the pick starts).
    picking: bool,
    revision: i32,
}

thread_local! {
    static EDITOR: RefCell<Editor> = RefCell::new(Editor::default());
    /// The panel's format fields; kept (not recreated) so typing isn't interrupted.
    static FIELDS: RefCell<Option<Rc<VecModel<FieldRow>>>> = const { RefCell::new(None) };
}

/// The editor as saved in config.json ("Keep the edit after a restart").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedEdit {
    pub edit: Option<[u8; 3]>,
    pub base: [u8; 3],
    pub compare: Option<[u8; 3]>,
}

pub fn install(ui: &AppWindow) {
    let fields = Rc::new(VecModel::<FieldRow>::default());
    ui.set_editor_fields(ModelRc::from(fields.clone()));
    FIELDS.with(|f| *f.borrow_mut() = Some(fields));
}

fn color(rgb: Rgb) -> Color {
    Color::from_rgb_u8(rgb.0, rgb.1, rgb.2)
}

fn ink(rgb: Rgb) -> Color {
    if wants_dark_text(rgb) {
        Color::from_rgb_u8(0x14, 0x14, 0x18)
    } else {
        Color::from_rgb_u8(0xff, 0xff, 0xff)
    }
}

fn name(ui: &AppWindow, rgb: Rgb) -> slint::SharedString {
    if ui.get_setting_show_name() {
        crate::names::display_name(rgb).into()
    } else {
        "".into()
    }
}

/// The color the panel works on: the second color with Compare on, else the edit.
fn target(ed: &Editor, compare_on: bool) -> Hsv {
    let edit = ed.edit.unwrap_or_else(|| Hsv::of(committed_rgb()));
    if compare_on {
        ed.compare.unwrap_or(edit)
    } else {
        edit
    }
}

fn set_target(ed: &mut Editor, compare_on: bool, hsv: Hsv) {
    if compare_on {
        ed.compare = Some(hsv);
    } else {
        ed.edit = Some(hsv);
        ed.base = committed_rgb();
    }
}

/// Opens the panel. A remembered edit comes back, unless the selected color
/// changed since and "Start fresh when the color changes" is on.
pub fn open(ui: &AppWindow) {
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        if ed.edit.is_some() && ed.base != committed_rgb() && ui.get_setting_editor_reset() {
            ed.edit = None;
        }
    });
    refresh(ui, None);
    ui.set_editor_open(true);
}

/// Esc, the close button or a click outside: closes and keeps the edit.
pub fn close(ui: &AppWindow) {
    ui.set_editor_open(false);
}

/// Cancel: closes; forgets the edit unless "Cancel forgets the edit" is off.
pub fn cancel(ui: &AppWindow, history: &HistoryStore) {
    if ui.get_setting_editor_cancel_forgets() {
        EDITOR.with(|e| e.borrow_mut().edit = None);
        persist_config(ui, history);
    }
    close(ui);
}

/// Use: applies the color on show, adds it to history and closes. The edit is
/// done, so it is forgotten; the compare color stays.
pub fn use_color(ui: &AppWindow, history: &HistoryStore) {
    let rgb = EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        let rgb = target(&ed, ui.get_setting_editor_compare()).rgb();
        ed.edit = None;
        rgb
    });
    close(ui);
    push_history(history, rgb);
    sync_history_model(ui, history);
    update_ui_colors(ui, rgb);
    persist_config(ui, history);
}

/// A drag on the saturation and brightness square.
pub fn set_sv(ui: &AppWindow, s: f32, v: f32) {
    update(ui, |hsv| Hsv { s: s.clamp(0.0, 1.0), v: v.clamp(0.0, 1.0), ..hsv });
}

/// A drag on the hue slider.
pub fn set_hue(ui: &AppWindow, h: f32) {
    update(ui, |hsv| Hsv { h: h.clamp(0.0, 360.0), ..hsv });
}

fn update(ui: &AppWindow, change: impl FnOnce(Hsv) -> Hsv) {
    let compare_on = ui.get_setting_editor_compare();
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        let hsv = change(target(&ed, compare_on));
        set_target(&mut ed, compare_on, hsv);
    });
    refresh(ui, None);
}

/// Typing in one of the panel's format fields: the color follows when it reads.
pub fn field_edited(ui: &AppWindow, key: &str, text: &str) {
    let Some(field) = ColorField::from_ui_label(key) else { return };
    if let Some(rgb) = parse_color(field, text) {
        set_from(ui, rgb, Some(field));
    }
}

/// Enter in a field: takes the value (or says it can't be read).
pub fn field_accepted(ui: &AppWindow, key: &str, text: &str) {
    let Some(field) = ColorField::from_ui_label(key) else { return };
    match parse_color(field, text) {
        Some(rgb) => set_from(ui, rgb, None),
        None => {
            show_input_error(ui, field);
            refresh(ui, None);
        }
    }
}

fn set_from(ui: &AppWindow, rgb: Rgb, typing_in: Option<ColorField>) {
    let compare_on = ui.get_setting_editor_compare();
    EDITOR.with(|e| set_target(&mut e.borrow_mut(), compare_on, Hsv::of(rgb)));
    refresh(ui, typing_in);
}

/// The copy icon of a field: the full form, like the boxes.
pub fn copy(ui: &AppWindow, key: &str) {
    let Some(field) = ColorField::from_ui_label(key) else { return };
    let rgb = EDITOR.with(|e| target(&e.borrow(), ui.get_setting_editor_compare()).rgb());
    copy_text_async(crate::color::copy_text(field, rgb));
}

/// The Compare button. The first time, the second color starts as the edit.
pub fn toggle_compare(ui: &AppWindow, history: &HistoryStore) {
    let on = !ui.get_setting_editor_compare();
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        if on && ed.compare.is_none() {
            ed.compare = Some(target(&ed, false));
        }
    });
    ui.set_setting_editor_compare(on);
    refresh(ui, None);
    persist_config(ui, history);
}

/// Called when a pick starts: true if it picks the compare color (the panel is
/// open with Compare on), so it must not touch the selected color or history.
pub fn begin_pick(ui: &AppWindow) -> bool {
    let to_editor = ui.get_editor_open() && ui.get_setting_editor_compare();
    EDITOR.with(|e| e.borrow_mut().picking = to_editor);
    to_editor
}

/// While picking for the compare half: the color under the cursor shows there.
/// False when the pick is a normal one.
pub fn preview_pick(ui: &AppWindow, rgb: Rgb) -> bool {
    if !EDITOR.with(|e| e.borrow().picking) {
        return false;
    }
    show_compare(ui, color(rgb), ink(rgb), name(ui, rgb), format_bare(formats().auto_copy_field(), rgb).into());
    true
}

/// The pick for the compare half ended: keep the color (None: cancelled).
pub fn end_pick(ui: &AppWindow, picked: Option<Rgb>) {
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        ed.picking = false;
        if let Some(rgb) = picked {
            ed.compare = Some(Hsv::of(rgb));
        }
    });
    refresh(ui, None);
}

/// The selected color changed (a pick, history, search, Use): with "Start
/// fresh when the color changes" on, an edit made on the old color goes.
/// Ignored during a pick (its hover previews; a cancelled pick changes
/// nothing); the picker calls `settle` once the pick lands.
pub fn selected_changed(ui: &AppWindow, rgb: Rgb) {
    if crate::picker::PICKER_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    settle(ui, rgb);
}

/// See `selected_changed`.
pub fn settle(ui: &AppWindow, rgb: Rgb) {
    let reset = EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        let reset = ed.edit.is_some() && ed.base != rgb && ui.get_setting_editor_reset();
        if reset {
            ed.edit = None;
        }
        reset
    });
    if reset || ui.get_editor_open() {
        refresh(ui, None);
    }
}

/// What to save in config.json, when "Keep the edit after a restart" is on.
pub fn saved(ui: &AppWindow) -> Option<SavedEdit> {
    if !ui.get_setting_editor_keep() {
        return None;
    }
    let arr = |rgb: Rgb| [rgb.0, rgb.1, rgb.2];
    EDITOR.with(|e| {
        let ed = e.borrow();
        Some(SavedEdit { edit: ed.edit.map(|h| arr(h.rgb())), base: arr(ed.base), compare: ed.compare.map(|h| arr(h.rgb())) })
    })
}

/// Restores what `saved` wrote (at startup).
pub fn restore(saved: &SavedEdit) {
    let rgb = |a: [u8; 3]| (a[0], a[1], a[2]);
    EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        ed.edit = saved.edit.map(|a| Hsv::of(rgb(a)));
        ed.base = rgb(saved.base);
        ed.compare = saved.compare.map(|a| Hsv::of(rgb(a)));
    });
}

fn show_compare(ui: &AppWindow, c: Color, i: Color, n: slint::SharedString, value: slint::SharedString) {
    ui.set_editor_compare_color(c);
    ui.set_editor_compare_ink(i);
    ui.set_editor_compare_name(n);
    ui.set_editor_compare_value(value);
}

/// Pushes the editor's state into the window. `typing_in` is the field the
/// user is typing in: its text is left alone.
pub fn refresh(ui: &AppWindow, typing_in: Option<ColorField>) {
    let compare_on = ui.get_setting_editor_compare();
    let first = formats().auto_copy_field();
    let (edit, compare, work, revision) = EDITOR.with(|e| {
        let mut ed = e.borrow_mut();
        ed.revision = ed.revision.wrapping_add(1);
        let edit = target(&ed, false);
        (edit, ed.compare.unwrap_or(edit), target(&ed, compare_on), ed.revision)
    });
    let (edit_rgb, compare_rgb, work_rgb) = (edit.rgb(), compare.rgb(), work.rgb());

    ui.set_editor_color(color(edit_rgb));
    ui.set_editor_ink(ink(edit_rgb));
    ui.set_editor_name(name(ui, edit_rgb));
    ui.set_editor_value(format_bare(first, edit_rgb).into());
    show_compare(ui, color(compare_rgb), ink(compare_rgb), name(ui, compare_rgb), format_bare(first, compare_rgb).into());
    ui.set_editor_ratio(format!("{:.2} : 1", contrast_ratio(committed_rgb(), compare_rgb)).into());

    ui.set_editor_h(work.h);
    ui.set_editor_s(work.s);
    ui.set_editor_v(work.v);
    ui.set_editor_hue_color(color(from_hsv(work.h, 1.0, 1.0)));
    ui.set_editor_use_color(color(work_rgb));
    ui.set_editor_use_ink(ink(work_rgb));
    ui.set_editor_use_label(format!("Use {}", format_bare(first, work_rgb)).into());

    // the first two formats, or one when only one is on
    let rows: Vec<FieldRow> = formats()
        .enabled()
        .into_iter()
        .take(2)
        .map(|f| FieldRow { key: f.label().into(), label: f.label().into(), value: format_bare(f, work_rgb).into(), rev: revision })
        .collect();
    FIELDS.with(|m| {
        let model = m.borrow();
        let Some(model) = model.as_ref() else { return };
        if model.row_count() != rows.len() {
            model.set_vec(rows);
            return;
        }
        for (i, row) in rows.into_iter().enumerate() {
            if ColorField::from_ui_label(&row.key) != typing_in {
                model.set_row_data(i, row);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compare_color_starts_as_the_edit_and_stays_on_its_own() {
        let mut ed = Editor { edit: Some(Hsv::of((200, 40, 40))), base: (255, 0, 0), ..Default::default() };
        assert_eq!(target(&ed, true).rgb(), (200, 40, 40)); // no compare color yet: the edit
        set_target(&mut ed, true, Hsv::of((30, 58, 138)));
        assert_eq!(target(&ed, true).rgb(), (30, 58, 138));
        assert_eq!(target(&ed, false).rgb(), (200, 40, 40)); // the edit is untouched
    }

    #[test]
    fn a_saved_edit_reads_back() {
        let saved = SavedEdit { edit: Some([1, 2, 3]), base: [4, 5, 6], compare: None };
        let json = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<SavedEdit>(&json).unwrap(), saved);
    }
}
