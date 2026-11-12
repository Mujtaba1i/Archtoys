//! The Color Picker's page: what its buttons, search, editor, format boxes,
//! shades and history do (wired once at startup, main.rs).

use crate::color::{copy_text, parse_color, scale_rgb, ColorField};
use crate::config::{persist_config, HistoryStore};
use crate::picker::{self, start_picker, PickerSource};
use crate::ui_state::{
    self, clear_input_error, committed_rgb, copy_text_async, current_rgb, push_history, show_input_error, show_message, sync_history_model,
    update_ui_colors, update_ui_preview_except_field,
};
use crate::{editor, AppWindow};
use slint::ComponentHandle;

pub(crate) fn wire(ui: &AppWindow, history_store: &HistoryStore) {
    let ui_handle = ui.as_weak();
    ui.on_cancel_pick(picker::cancel_picker);
    ui.on_pick_nudge(picker::nudge_picker);
    ui.on_pick_point(picker::pick_point);

    let pick_ui = ui_handle.clone();
    let pick_history = history_store.clone();
    ui.on_pick_color(move || {
        if let Some(ui) = pick_ui.upgrade() {
            start_picker(&ui, pick_history.clone(), PickerSource::Button);
        }
    });

    // The boxes show values without the wrapper; copying gives the full form.
    let copy_ui = ui_handle.clone();
    ui.on_copy_value(move |key| {
        if let (Some(ui), Some(field)) = (copy_ui.upgrade(), ColorField::from_ui_label(&key)) {
            copy_text_async(copy_text(field, current_rgb(&ui)));
        }
    });

    // --- Color search ---
    let search_ui = ui_handle.clone();
    ui.on_search_changed(move |text| {
        if let Some(ui) = search_ui.upgrade() {
            ui_state::show_search(&ui, &text);
        }
    });
    ui.on_search_load_more(ui_state::load_more_search);

    // Hovering a result shows its color; leaving the results puts the old one back.
    let hover_ui = ui_handle.clone();
    ui.on_search_hovered(move |index| {
        if let Some(ui) = hover_ui.upgrade() {
            match ui_state::search_hit_color(index) {
                Some(rgb) => ui_state::update_ui_preview(&ui, rgb),
                None => update_ui_colors(&ui, committed_rgb()),
            }
        }
    });

    let applied_ui = ui_handle.clone();
    let applied_history = history_store.clone();
    ui.on_search_applied(move |index| {
        let (Some(ui), Some(rgb)) = (applied_ui.upgrade(), ui_state::search_hit_color(index)) else {
            return;
        };
        push_history(&applied_history, rgb);
        sync_history_model(&ui, &applied_history);
        update_ui_colors(&ui, rgb);
        persist_config(&ui, &applied_history);
    });

    // Closed without choosing: the color from before the search comes back.
    let closed_ui = ui_handle.clone();
    ui.on_search_closed(move || {
        if let Some(ui) = closed_ui.upgrade() {
            update_ui_colors(&ui, committed_rgb());
            ui_state::release_search(&ui);
        }
    });

    let toggle_ui = ui_handle.clone();
    let toggle_history = history_store.clone();
    ui.on_format_toggled(move |key, on| {
        let (Some(ui), Some(field)) = (toggle_ui.upgrade(), ColorField::from_ui_label(&key)) else {
            return;
        };
        match ui_state::toggle_format(&ui, field, on) {
            Ok(()) => persist_config(&ui, &toggle_history),
            Err(message) => show_message(&ui, message),
        }
    });

    let move_ui = ui_handle.clone();
    let move_history = history_store.clone();
    ui.on_format_moved(move |key, delta| {
        let (Some(ui), Some(field)) = (move_ui.upgrade(), ColorField::from_ui_label(&key)) else {
            return;
        };
        ui_state::move_format(&ui, field, delta);
        persist_config(&ui, &move_history);
    });

    let history_click_ui = ui_handle.clone();
    let history_click_store = history_store.clone();
    ui.on_history_clicked(move |index| {
        if let Some(ui) = history_click_ui.upgrade() {
            let picked = history_click_store.lock().unwrap().get(index as usize).copied();
            if let Some(rgb) = picked {
                update_ui_colors(&ui, rgb);
                persist_config(&ui, &history_click_store); // remember it for next start
            }
        }
    });

    let clear_ui = ui_handle.clone();
    let clear_history = history_store.clone();
    ui.on_clear_history(move || {
        let Some(ui) = clear_ui.upgrade() else {
            return;
        };
        let keep_rgb = current_rgb(&ui);
        {
            let mut guard = clear_history.lock().unwrap();
            guard.clear();
            guard.push(keep_rgb);
        }
        sync_history_model(&ui, &clear_history);
        persist_config(&ui, &clear_history);
    });

    let shade_ui = ui_handle.clone();
    let shade_history = history_store.clone();
    ui.on_shade_clicked(move |factor| {
        if let Some(ui) = shade_ui.upgrade() {
            let rgb = scale_rgb(current_rgb(&ui), factor);
            push_history(&shade_history, rgb);
            sync_history_model(&ui, &shade_history);
            update_ui_colors(&ui, rgb);
            persist_config(&ui, &shade_history);
        }
    });

    // --- The color editor (the pen on the middle shade) ---
    let ed_ui = ui_handle.clone();
    ui.on_editor_open_requested(move || {
        if let Some(ui) = ed_ui.upgrade() {
            editor::open(&ui);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_close(move || {
        if let Some(ui) = ed_ui.upgrade() {
            editor::close(&ui);
        }
    });
    let (ed_ui, ed_history) = (ui_handle.clone(), history_store.clone());
    ui.on_editor_cancel(move || {
        if let Some(ui) = ed_ui.upgrade() {
            editor::cancel(&ui, &ed_history);
        }
    });
    let (ed_ui, ed_history) = (ui_handle.clone(), history_store.clone());
    ui.on_editor_use(move || {
        if let Some(ui) = ed_ui.upgrade() {
            editor::use_color(&ui, &ed_history);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_sv(move |s, v| {
        if let Some(ui) = ed_ui.upgrade() {
            editor::set_sv(&ui, s, v);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_hue(move |h| {
        if let Some(ui) = ed_ui.upgrade() {
            editor::set_hue(&ui, h);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_field_edited(move |key, text| {
        if let Some(ui) = ed_ui.upgrade() {
            editor::field_edited(&ui, &key, &text);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_field_accepted(move |key, text| {
        if let Some(ui) = ed_ui.upgrade() {
            editor::field_accepted(&ui, &key, &text);
        }
    });
    let ed_ui = ui_handle.clone();
    ui.on_editor_copy(move |key| {
        if let Some(ui) = ed_ui.upgrade() {
            editor::copy(&ui, &key);
        }
    });
    let (ed_ui, ed_history) = (ui_handle.clone(), history_store.clone());
    ui.on_editor_compare_toggled(move || {
        if let Some(ui) = ed_ui.upgrade() {
            editor::toggle_compare(&ui, &ed_history);
        }
    });
    // Pick on the card's right half: the same as the hotkey while Compare is on.
    let (ed_ui, ed_history) = (ui_handle.clone(), history_store.clone());
    ui.on_editor_pick(move || {
        if let Some(ui) = ed_ui.upgrade() {
            start_picker(&ui, ed_history.clone(), PickerSource::Button);
        }
    });

    let edited_ui = ui_handle.clone();
    ui.on_value_edited(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str) else {
            return;
        };
        if let Some(ui) = edited_ui.upgrade() {
            if let Some(rgb) = parse_color(field, &value) {
                update_ui_preview_except_field(&ui, field, rgb);
            }
        }
    });

    let accepted_ui = ui_handle.clone();
    let accepted_history = history_store.clone();
    ui.on_value_accepted(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str) else {
            return;
        };
        if let Some(ui) = accepted_ui.upgrade() {
            if let Some(rgb) = parse_color(field, &value) {
                clear_input_error(&ui);
                push_history(&accepted_history, rgb);
                sync_history_model(&ui, &accepted_history);
                update_ui_colors(&ui, rgb);
                persist_config(&ui, &accepted_history);
            } else {
                // Put back the last good color instead of turning black.
                show_input_error(&ui, field);
                update_ui_colors(&ui, committed_rgb());
            }
        }
    });

    let blurred_ui = ui_handle.clone();
    ui.on_value_blurred(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str) else {
            return;
        };
        if let Some(ui) = blurred_ui.upgrade() {
            match parse_color(field, &value) {
                Some(rgb) => update_ui_colors(&ui, rgb),
                None => {
                    show_input_error(&ui, field);
                    update_ui_colors(&ui, committed_rgb());
                }
            }
        }
    });
}
