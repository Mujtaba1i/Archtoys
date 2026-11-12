//! Text Extractor's page: what its buttons do (wired once at startup, main.rs).

use crate::ocr;
use crate::AppWindow;
use slint::ComponentHandle;

pub(crate) fn wire(ui: &AppWindow) {
    let ui_handle = ui.as_weak();
    let t_ui = ui_handle.clone();
    ui.on_text_turn_on(move || {
        if let Some(ui) = t_ui.upgrade() {
            ocr::turn_on(&ui);
        }
    });
    ui.on_text_cancel_install(ocr::cancel_install);
    let t_ui = ui_handle.clone();
    ui.on_text_check_again(move || {
        if let Some(ui) = t_ui.upgrade() {
            ocr::check_again(&ui);
        }
    });
    let t_ui = ui_handle.clone();
    ui.on_text_extract(move || {
        if let Some(ui) = t_ui.upgrade() {
            ocr::start_capture(&ui);
        }
    });
    ui.on_text_copy(|text| ocr::copy(&text));
    let t_ui = ui_handle.clone();
    ui.on_text_delete(move |index| {
        if let Some(ui) = t_ui.upgrade() {
            ocr::delete(&ui, index);
        }
    });
    let t_ui = ui_handle.clone();
    ui.on_text_clear(move || {
        if let Some(ui) = t_ui.upgrade() {
            ocr::clear(&ui);
        }
    });
    let t_ui = ui_handle.clone();
    ui.on_text_delete_languages(move || {
        if let Some(ui) = t_ui.upgrade() {
            ocr::delete_languages(&ui);
        }
    });
}
