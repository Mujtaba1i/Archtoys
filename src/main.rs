slint::include_modules!();

mod color;
mod config;
mod hotkey;
mod picker;
mod portal;
mod tray;
mod ui_state;

use color::{parse_color, scale_rgb, ColorField};
use config::{apply_config, load_config, persist_config, sync_autostart_entry, HistoryStore};
use hotkey::{build_hotkey_from_capture, HotkeyService, HotkeyStatus, DEFAULT_HOTKEY_TEXT};
use ksni::blocking::TrayMethods;
use picker::{start_picker, PickerSource};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use ui_state::{
    clear_input_error, committed_rgb, copy_text_async, current_rgb, push_history, show_input_error,
    sync_history_model, update_ui_colors, update_ui_preview_except_field,
};

const WINDOW_MIN_WIDTH: f64 = 480.0;
const WINDOW_MIN_HEIGHT: f64 = 320.0;
const WINDOW_MAX_WIDTH: f64 = 900.0;
const WINDOW_MAX_HEIGHT: f64 = 620.0;

fn apply_native_window_constraints(ui: &AppWindow) {
    use slint::winit_030::{winit, WinitWindowAccessor};

    ui.window().with_winit_window(|window| {
        window.set_min_inner_size(Some(winit::dpi::LogicalSize::new(WINDOW_MIN_WIDTH, WINDOW_MIN_HEIGHT)));
        window.set_max_inner_size(Some(winit::dpi::LogicalSize::new(WINDOW_MAX_WIDTH, WINDOW_MAX_HEIGHT)));
        window.set_resizable(true);
    });
}

fn apply_hidden_startup_state(ui: &AppWindow) {
    use slint::winit_030::WinitWindowAccessor;

    // Keep startup fully tray-only: no visible top-level window/task entry.
    ui.window().hide().ok();
    ui.window().with_winit_window(|window| {
        window.set_visible(false);
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let start_hidden = std::env::args().any(|arg| arg == "--start-hidden" || arg == "--minimized");

    // Set the Wayland app_id / X11 WM_CLASS to "archtoys" so that the
    // compositor can match the running window to archtoys.desktop and
    // display the correct icon in the dock/taskbar.
    std::env::set_var("SLINT_APP_ID", "archtoys");

    let ui = AppWindow::new()?;
    apply_native_window_constraints(&ui);
    if start_hidden {
        apply_hidden_startup_state(&ui);
    }
    let ui_handle = ui.as_weak();

    let tray = tray::AppTray { ui: ui_handle.clone() };
    let _tray_handle = match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("tray: failed to spawn: {err:?}");
            None
        }
    };

    let history_store: HistoryStore = Arc::new(Mutex::new(vec![(203, 182, 172), (85, 85, 85)]));

    if let Some(cfg) = load_config() {
        apply_config(&ui, &history_store, &cfg);
    }
    if ui.get_setting_hotkey().trim().is_empty() {
        ui.set_setting_hotkey(DEFAULT_HOTKEY_TEXT.into());
    }
    sync_autostart_entry(ui.get_setting_autostart());

    // --- Global hotkey ---
    let trigger: hotkey::Trigger = {
        let ui_weak = Mutex::new(ui_handle.clone());
        let history = history_store.clone();
        Arc::new(move || {
            let history = history.clone();
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(move |ui| {
                start_picker(&ui, history, PickerSource::Hotkey);
            });
        })
    };
    // Shows who owns the shortcut. When the desktop does, the settings show
    // the desktop's real shortcut and a button to change it there.
    let on_status: hotkey::StatusCallback = {
        let ui_weak = Mutex::new(ui_handle.clone());
        Arc::new(move |status: HotkeyStatus| {
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(move |ui| match status {
                HotkeyStatus::SystemManaged(description) => {
                    ui.set_hotkey_system_managed(true);
                    ui.set_hotkey_display(description.into());
                }
                HotkeyStatus::AppManaged => ui.set_hotkey_system_managed(false),
            });
        })
    };
    let (hotkey_service, hotkey_text) = HotkeyService::start(&ui.get_setting_hotkey(), trigger, on_status);
    let hotkey_service = Rc::new(hotkey_service);
    let active_hotkey_text = Rc::new(RefCell::new(hotkey_text.clone()));
    ui.set_setting_hotkey(hotkey_text.into());

    sync_history_model(&ui, &history_store);
    update_ui_colors(&ui, (203, 182, 172));

    let settings_ui = ui_handle.clone();
    let settings_history = history_store.clone();
    ui.on_settings_changed(move || {
        if let Some(ui) = settings_ui.upgrade() {
            persist_config(&ui, &settings_history);
            sync_autostart_entry(ui.get_setting_autostart());
        }
    });

    let hotkey_ui = ui_handle.clone();
    let hotkey_history = history_store.clone();
    let hotkey_service_apply = hotkey_service.clone();
    ui.on_hotkey_captured(move |key_text, ctrl, alt, shift, meta| {
        let Some(ui) = hotkey_ui.upgrade() else {
            return;
        };
        let result = build_hotkey_from_capture(&key_text, ctrl, alt, shift, meta)
            .and_then(|candidate| hotkey_service_apply.change(&candidate));
        match result {
            Ok(text) => {
                *active_hotkey_text.borrow_mut() = text.clone();
                ui.set_setting_hotkey(text.into());
                persist_config(&ui, &hotkey_history);
            }
            Err(err) => {
                eprintln!("hotkey: {err}");
                ui.set_setting_hotkey(active_hotkey_text.borrow().clone().into());
            }
        }
    });

    let open_settings_service = hotkey_service.clone();
    ui.on_open_hotkey_settings(move || open_settings_service.open_system_settings());

    let refresh_service = hotkey_service.clone();
    ui.on_settings_opened(move || refresh_service.refresh());

    ui.on_cancel_pick(picker::cancel_picker);

    let pick_ui = ui_handle.clone();
    let pick_history = history_store.clone();
    ui.on_pick_color(move || {
        if let Some(ui) = pick_ui.upgrade() {
            start_picker(&ui, pick_history.clone(), PickerSource::Button);
        }
    });

    ui.on_copy_to_clipboard(move |text| {
        copy_text_async(text.to_string());
    });

    let history_click_ui = ui_handle.clone();
    let history_click_store = history_store.clone();
    ui.on_history_clicked(move |index| {
        if let Some(ui) = history_click_ui.upgrade() {
            let picked = history_click_store.lock().unwrap().get(index as usize).copied();
            if let Some(rgb) = picked {
                update_ui_colors(&ui, rgb);
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

    // If "Minimize to tray on close" is enabled, hide the window and keep the
    // event loop alive (tray stays). Otherwise, persist config and quit.
    let ui_close = ui_handle.clone();
    let close_history = history_store.clone();
    ui.window().on_close_requested(move || {
        if let Some(ui) = ui_close.upgrade() {
            if ui.get_setting_minimize_tray() {
                ui.window().hide().ok();
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            persist_config(&ui, &close_history);
        }
        slint::quit_event_loop().ok();
        slint::CloseRequestResponse::HideWindow
    });

    if !start_hidden {
        ui.show()?;
    }

    let result = slint::run_event_loop_until_quit();
    drop(hotkey_service);
    result
}
