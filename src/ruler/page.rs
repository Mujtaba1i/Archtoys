//! The Screen Ruler's page: what its buttons do (wired once at startup, main.rs).

use crate::config::{persist_config, HistoryStore};
use crate::AppWindow;
use slint::ComponentHandle;

pub(crate) fn wire(ui: &AppWindow, history_store: &HistoryStore) {
    let ui_handle = ui.as_weak();
    let r_ui = ui_handle.clone();
    ui.on_ruler_measure(move || {
        if let Some(ui) = r_ui.upgrade() {
            super::start(&ui);
        }
    });
    let (r_ui, r_history) = (ui_handle.clone(), history_store.clone());
    ui.on_ruler_mode_picked(move |mode| {
        if let Some(ui) = r_ui.upgrade() {
            ui.set_setting_ruler_mode(mode);
            persist_config(&ui, &r_history);
        }
    });
    let r_ui = ui_handle.clone();
    ui.on_ruler_copy_latest(move || {
        if let Some(text) = r_ui.upgrade().and_then(|ui| super::latest_text(&ui)) {
            crate::ui_state::copy_text_async(text);
        }
    });
    let r_ui = ui_handle.clone();
    ui.on_ruler_delete(move |index| {
        if let Some(ui) = r_ui.upgrade() {
            super::delete(&ui, index);
        }
    });
    let r_ui = ui_handle.clone();
    ui.on_ruler_select(move |index| {
        if let Some(ui) = r_ui.upgrade() {
            super::select(&ui, index);
        }
    });
    let r_ui = ui_handle.clone();
    ui.on_ruler_clear(move || {
        if let Some(ui) = r_ui.upgrade() {
            super::clear(&ui);
        }
    });
}
