// The UI is compiled by the `slint!` macro, not by a build script: the build
// script route also compiles every image format Slint's compiler can encode (an
// AV1 video encoder among them), about 60 crates the app never uses.
slint::slint! { export * from "ui/app.slint"; }

mod color;
mod config;
mod diagnostics;
mod editor;
mod formats;
mod hotkey;
mod kwin;
mod launch;
mod name_index;
mod names;
mod ocr;
mod picker;
mod portal;
mod search;
mod single_instance;
mod tray;
mod ui_state;
mod window;

use color::{copy_text, parse_color, scale_rgb, ColorField};
use config::{apply_config, load_config, persist_config, sync_autostart_entry, HistoryStore};
use hotkey::{build_hotkey_from_capture, HotkeyService, HotkeyStatus, Shortcut, DEFAULT_HOTKEY_TEXT, DEFAULT_TEXT_HOTKEY_TEXT};
use ksni::blocking::TrayMethods;
use picker::{start_picker, PickerSource};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ui_state::{
    clear_input_error, committed_rgb, copy_text_async, current_rgb, push_history, show_input_error,
    show_message, sync_history_model, update_ui_colors, update_ui_preview_except_field,
};

pub(crate) const WINDOW_MIN_WIDTH: f64 = 480.0;
pub(crate) const WINDOW_MIN_HEIGHT: f64 = 420.0;
pub(crate) const WINDOW_MAX_WIDTH: f64 = 900.0;
pub(crate) const WINDOW_MAX_HEIGHT: f64 = 1000.0;

fn apply_hidden_startup_state(ui: &AppWindow) {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;

    // Keep startup fully tray-only: no visible top-level window/task entry.
    ui.window().hide().ok();
    ui.window().with_winit_window(|window| {
        window.set_visible(false);
    });

    // Slint still creates the window when the event loop starts, and on Wayland
    // a window can't exist hidden: it sat in the taskbar, never drawn, and
    // showing it later didn't draw it either (issue #7). Once it exists, show
    // and hide it in the same turn: on Wayland that makes Slint destroy it, and
    // the next show creates a fresh window. (On X11 it is created invisible.)
    let weak = ui.as_weak();
    let started = slint::spawn_local(async move {
        let Some(ui) = weak.upgrade() else { return };
        let Ok(window) = ui.window().winit_window().await else { return };
        let wayland = window
            .window_handle()
            .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(..)));
        drop(window); // Slint only destroys a window nothing else holds
        if wayland {
            ui.window().show().ok();
            ui.window().hide().ok();
        }
    });
    if let Err(err) = started {
        eprintln!("startup: couldn't keep the window hidden: {err:?}");
    }
}

/// Shows the window in any state (tray, minimized, behind others, another
/// desktop). Where the desktop refuses to raise it, the taskbar entry flashes.
pub(crate) fn bring_to_front(ui: &AppWindow) {
    use slint::winit_030::{winit, WinitWindowAccessor};

    if picker::PICKER_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) {
        return; // a pick in progress wins
    }
    ui_state::show_window(ui);
    ui.window().with_winit_window(|window| {
        window.set_minimized(false);
        window.focus_window();
    });
    let ui_weak = ui.as_weak();
    slint::Timer::single_shot(Duration::from_millis(300), move || {
        let Some(ui) = ui_weak.upgrade() else { return };
        ui.window().with_winit_window(|window| {
            if !window.has_focus() {
                window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
            }
        });
    });
}

/// With no font installed at all, the text library panics deep inside. Say
/// what is wrong and how to fix it instead.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if info.location().is_some_and(|l| l.file().contains("fontique")) {
            eprintln!(
                "Archtoys can't find any fonts. Install one, for example DejaVu Sans \
                 (ttf-dejavu on Arch, fonts-dejavu-core on Debian and Ubuntu, \
                 dejavu-sans-fonts on Fedora), and start it again."
            );
            std::process::exit(1);
        }
        default_hook(info);
    }));
}

/// Drawn on the CPU there are no blurred shadows or rounded clipping: the UI
/// draws its stand-ins instead. Each window has its own `Skin`, so each sets it.
/// (A build has one renderer only, so this is decided at compile time.)
pub(crate) fn cpu_drawn() -> bool {
    !cfg!(feature = "opengl")
}

/// Opens a web page from the About screen in the browser.
fn open_link(url: &str) {
    if !url.starts_with("https://") {
        return;
    }
    let mut command = launch::outside_command("xdg-open");
    command.arg(url);
    if let Err(err) = launch::spawn_and_reap(command) {
        eprintln!("about: couldn't open {url}: {err}");
    }
}

/// "Open-source licences": the licence of every library Archtoys is built
/// from (generated at build time by cargo-about), opened in the browser.
fn open_licenses() {
    const LICENSES: &str = include_str!("../data/third-party-licenses.html");
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join("archtoys-licenses.html");
    if let Err(err) = std::fs::write(&path, LICENSES) {
        eprintln!("about: couldn't write the licence list: {err}");
        return;
    }
    let mut command = launch::outside_command("xdg-open");
    command.arg(&path);
    if let Err(err) = launch::spawn_and_reap(command) {
        eprintln!("about: couldn't open the licence list: {err}");
    }
}

fn main() -> Result<(), slint::PlatformError> {
    install_panic_hook();
    // Text Extractor's helpers for CI and scripts: no window, no tray
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--extract-file") {
        let code = match args.get(i + 1) {
            Some(path) => ocr::run_on_file(path),
            None => {
                eprintln!("usage: archtoys --extract-file IMAGE.png");
                2
            }
        };
        std::process::exit(code);
    }
    if args.iter().any(|a| a == "--print-install-script") {
        std::process::exit(ocr::print_install_script());
    }
    if args.iter().any(|a| a == "--check-tesseract") {
        std::process::exit(ocr::check_tesseract());
    }
    let autostart = std::env::args().any(|arg| arg == "--start-hidden" || arg == "--minimized");
    let pick_now = std::env::args().any(|arg| arg == "--pick");
    let extract_now = std::env::args().any(|arg| arg == "--extract");

    let _instance_lock = match single_instance::acquire() {
        single_instance::Lock::TakenByAnother => {
            // An autostart launch while a copy is running exits quietly.
            if !autostart {
                let method = if pick_now {
                    "Pick"
                } else if extract_now {
                    "Extract"
                } else {
                    "Show"
                };
                if let Err(err) = single_instance::send(method) {
                    eprintln!("Archtoys is already running, but it didn't answer: {err}");
                }
            }
            return Ok(());
        }
        lock => lock,
    };
    // `archtoys --pick` / `--extract` with no copy running starts in the tray
    // and picks (or captures text), like the hotkey.
    let start_hidden = autostart || pick_now || extract_now;

    let ui = AppWindow::new()?;

    // The Wayland app_id / X11 WM_CLASS. It must match archtoys.desktop, or
    // the desktop can't find our icon and shows a generic one instead.
    // (Slint doesn't read a SLINT_APP_ID variable; this is the real API, and
    // it must run after AppWindow::new() and before the window is shown.)
    if let Err(err) = slint::set_xdg_app_id("archtoys") {
        eprintln!("app id: couldn't set it: {err:?}");
    }
    ui.global::<Skin>().set_real_shadows(!cpu_drawn());
    ui.set_app_version(env!("CARGO_PKG_VERSION").into());
    if start_hidden {
        apply_hidden_startup_state(&ui);
    }
    let ui_handle = ui.as_weak();

    // The desktop's tray host may start after us (autostart at login): wait for
    // it instead of giving up, or a hidden start has no way back (issue #7).
    // The dot starts pink, like the logo, and takes the color once it is shown.
    let tray = tray::AppTray { ui: ui_handle.clone(), dot: (0xff, 0x2e, 0x93) }.assume_sni_available(true);
    match tray.spawn() {
        Ok(handle) => tray::install(handle),
        Err(err) => eprintln!("tray: failed to spawn: {err:?}"),
    }

    let history_store: HistoryStore = Arc::new(Mutex::new(vec![(203, 182, 172), (85, 85, 85)]));
    ui_state::install_models(&ui);
    editor::install(&ui);

    // The color to show at startup: the last one used, if saved.
    let mut start_color = (203, 182, 172);
    if let Some(cfg) = load_config() {
        apply_config(&ui, &history_store, &cfg);
        if let Some([r, g, b]) = cfg.last_color {
            start_color = (r, g, b);
        }
    }
    if ui.get_setting_hotkey().trim().is_empty() {
        ui.set_setting_hotkey(DEFAULT_HOTKEY_TEXT.into());
    }
    if ui.get_setting_text_hotkey().trim().is_empty() {
        ui.set_setting_text_hotkey(DEFAULT_TEXT_HOTKEY_TEXT.into());
    }
    // the size limits, with the sidebar's width if it was pinned
    window::apply_constraints(&ui);
    sync_autostart_entry(ui.get_setting_autostart());
    {
        let (save_ui, save_history) = (ui_handle.clone(), history_store.clone());
        ocr::install_ui(&ui, move || {
            if let Some(ui) = save_ui.upgrade() {
                persist_config(&ui, &save_history);
            }
        });
    }

    // --- Global hotkeys: one per toy. A toy's hotkey also switches the
    // window to that toy. ---
    let trigger: hotkey::Trigger = {
        let ui_weak = Mutex::new(ui_handle.clone());
        let history = history_store.clone();
        Arc::new(move || {
            let history = history.clone();
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(move |ui| {
                if ui.get_page() != 1 {
                    ui.invoke_go(1);
                }
                start_picker(&ui, history, PickerSource::Hotkey);
            });
        })
    };
    let extract_trigger: hotkey::Trigger = {
        let ui_weak = Mutex::new(ui_handle.clone());
        Arc::new(move || {
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(|ui| ocr::start_capture(&ui));
        })
    };
    // Shows who owns the shortcut. When the desktop does, the settings show
    // the desktop's real shortcut and a button to change it there.
    let on_status: hotkey::StatusCallback = {
        let ui_weak = Mutex::new(ui_handle.clone());
        Arc::new(move |status: HotkeyStatus| {
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(move |ui| match status {
                HotkeyStatus::SystemManaged(descriptions) => {
                    diagnostics::set_hotkey("desktop portal (GlobalShortcuts)");
                    ui.set_hotkey_system_managed(true);
                    ui.set_hotkey_display(descriptions.first().cloned().unwrap_or_default().into());
                    ui.set_text_hotkey_display(descriptions.get(1).cloned().unwrap_or_default().into());
                }
                HotkeyStatus::AppManaged => {
                    diagnostics::set_hotkey("X11 (global-hotkey)");
                    ui.set_hotkey_system_managed(false);
                }
            });
        })
    };
    let shortcuts = vec![
        Shortcut {
            id: "pick-color",
            description: "Pick a color (Archtoys)",
            text: ui.get_setting_hotkey().to_string(),
            default: DEFAULT_HOTKEY_TEXT,
            trigger: trigger.clone(),
        },
        Shortcut {
            id: "extract-text",
            description: "Extract text from the screen (Archtoys)",
            text: ui.get_setting_text_hotkey().to_string(),
            default: DEFAULT_TEXT_HOTKEY_TEXT,
            trigger: extract_trigger.clone(),
        },
    ];
    let (hotkey_service, hotkey_texts) = HotkeyService::start(shortcuts, on_status);

    // Later launches ask this copy to show itself, or to pick (`--pick`).
    let show: hotkey::Trigger = {
        let ui_weak = Mutex::new(ui_handle.clone());
        Arc::new(move || {
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(|ui| bring_to_front(&ui));
        })
    };
    let _remote = single_instance::serve(show, trigger.clone(), extract_trigger.clone());
    if pick_now {
        trigger();
    }
    if extract_now {
        extract_trigger();
    }
    let hotkey_service = Rc::new(hotkey_service);
    let active_hotkey_texts = Rc::new(RefCell::new(hotkey_texts.clone()));
    ui.set_setting_hotkey(hotkey_texts[0].clone().into());
    ui.set_setting_text_hotkey(hotkey_texts[1].clone().into());

    sync_history_model(&ui, &history_store);
    update_ui_colors(&ui, start_color);

    let settings_ui = ui_handle.clone();
    let settings_history = history_store.clone();
    ui.on_settings_changed(move || {
        if let Some(ui) = settings_ui.upgrade() {
            ui_state::refresh_rows(&ui); // names on or off
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
        // which toy's hotkey was being recorded
        let index = ui.get_hotkey_target().clamp(0, 1) as usize;
        let result = build_hotkey_from_capture(&key_text, ctrl, alt, shift, meta)
            .and_then(|candidate| hotkey_service_apply.change(index, &candidate));
        let set = |ui: &AppWindow, text: String| {
            if index == 0 {
                ui.set_setting_hotkey(text.into());
            } else {
                ui.set_setting_text_hotkey(text.into());
            }
        };
        match result {
            Ok(text) => {
                active_hotkey_texts.borrow_mut()[index] = text.clone();
                set(&ui, text);
                persist_config(&ui, &hotkey_history);
            }
            Err(err) => {
                eprintln!("hotkey: {err}");
                show_message(&ui, &err);
                set(&ui, active_hotkey_texts.borrow()[index].clone());
            }
        }
    });

    let open_settings_service = hotkey_service.clone();
    ui.on_open_hotkey_settings(move || open_settings_service.open_system_settings());

    let refresh_service = hotkey_service.clone();
    let refresh_ui = ui_handle.clone();
    ui.on_settings_opened(move || {
        refresh_service.refresh();
        if let Some(ui) = refresh_ui.upgrade() {
            ocr::page_shown(&ui); // the engine and languages in Text Extractor's settings
        }
    });

    // --- The frame (v0.7.0): the sidebar, pages ---
    let (w_ui, w_history) = (ui_handle.clone(), history_store.clone());
    ui.on_sidebar_mode_changed(move |old, new| {
        if let Some(ui) = w_ui.upgrade() {
            window::sidebar_mode_changed(&ui, old, new);
            persist_config(&ui, &w_history);
        }
    });
    let (w_ui, w_history) = (ui_handle.clone(), history_store.clone());
    ui.on_page_changed(move |page| {
        if let Some(ui) = w_ui.upgrade() {
            match page {
                0 => ocr::ensure_history(&ui), // Home shows the latest text
                2 => ocr::page_shown(&ui),
                _ => {}
            }
            persist_config(&ui, &w_history); // reopen on this page
        }
    });
    ui.on_open_link(|url| open_link(&url));
    ui.on_open_licenses(open_licenses);

    // --- Text Extractor ---
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

    let report_ui = ui_handle.clone();
    ui.on_report_problem(move || {
        if let Err(err) = diagnostics::open_issue_page() {
            eprintln!("report: couldn't open the browser: {err}");
            if let Some(ui) = report_ui.upgrade() {
                show_message(&ui, "No browser found. Report at github.com/Mujtaba1i/Archtoys/issues");
            }
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

    // If "Minimize to tray on close" is enabled, hide the window and keep the
    // event loop alive (tray stays). Otherwise, persist config and quit.
    let ui_close = ui_handle.clone();
    let close_history = history_store.clone();
    ui.window().on_close_requested(move || {
        if let Some(ui) = ui_close.upgrade() {
            if ui.get_setting_minimize_tray() {
                ui_state::hide_window(&ui);
                return slint::CloseRequestResponse::KeepWindowShown;
            }
            persist_config(&ui, &close_history);
        }
        slint::quit_event_loop().ok();
        slint::CloseRequestResponse::HideWindow
    });

    if !start_hidden {
        ui.show()?;
        ui_state::restore_size(&ui);
    }

    // reopened on Text Extractor (its history and engine) or Home (the latest
    // text), once the window is up
    let late_ui = ui_handle.clone();
    slint::Timer::single_shot(Duration::from_secs(1), move || {
        if let Some(ui) = late_ui.upgrade() {
            match ui.get_page() {
                0 => ocr::ensure_history(&ui),
                2 => ocr::page_shown(&ui),
                _ => {}
            }
        }
    });

    // what starting up needed and freed (fonts, the first frames)
    slint::Timer::single_shot(Duration::from_secs(3), ui_state::return_free_memory);

    let result = slint::run_event_loop_until_quit();
    // Quit from the tray: save the window size (and everything else) too.
    persist_config(&ui, &history_store);
    drop(hotkey_service);
    result
}
