// The UI is compiled by the `slint!` macro, not by a build script: the build
// script route also compiles every image format Slint's compiler can encode (an
// AV1 video encoder among them), about 60 crates the app never uses.
slint::slint! { export * from "ui/app.slint"; }

mod color;
mod config;
mod diagnostics;
mod editor;
mod formats;
mod freeze;
mod hotkey;
mod kwin;
mod launch;
mod name_index;
mod names;
mod ocr;
mod picker;
mod portal;
mod ruler;
mod search;
mod single_instance;
mod tray;
mod ui_state;
mod window;

use config::{apply_config, load_config, persist_config, sync_autostart_entry, HistoryStore};
use hotkey::{
    build_hotkey_from_capture, HotkeyService, HotkeyStatus, Shortcut, DEFAULT_HOTKEY_TEXT, DEFAULT_RULER_HOTKEY_TEXT,
    DEFAULT_TEXT_HOTKEY_TEXT,
};
use ksni::blocking::TrayMethods;
use picker::{start_picker, PickerSource};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use ui_state::{show_message, sync_history_model, update_ui_colors};

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
        let wayland = window.window_handle().is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(..)));
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
    // 12- or 24-hour times, before any other thread runs (setlocale)
    ocr::history::init_clock();
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
    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("archtoys {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
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
    let measure_now = std::env::args().any(|arg| arg == "--measure");

    let _instance_lock = match single_instance::acquire() {
        single_instance::Lock::TakenByAnother => {
            // An autostart launch while a copy is running exits quietly.
            if !autostart {
                let method = if pick_now {
                    "Pick"
                } else if extract_now {
                    "Extract"
                } else if measure_now {
                    "Measure"
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
    // the running app (not a CLI tool or a second launch): keep its last
    // messages for Report a problem
    diagnostics::keep_recent_messages();
    // `archtoys --pick` / `--extract` with no copy running starts in the tray
    // and picks (or captures text), like the hotkey.
    let start_hidden = autostart || pick_now || extract_now || measure_now;

    // each picker window opens full screen on its own monitor (freeze.rs)
    slint::BackendSelector::new().backend_name("winit".into()).with_winit_window_attributes_hook(freeze::window_attributes).select()?;
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
    if ui.get_setting_ruler_hotkey().trim().is_empty() {
        ui.set_setting_ruler_hotkey(DEFAULT_RULER_HOTKEY_TEXT.into());
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
        let (save_ui, save_history) = (ui_handle.clone(), history_store.clone());
        ruler::install(&ui, &history_store, move || {
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
    let measure_trigger: hotkey::Trigger = {
        let ui_weak = Mutex::new(ui_handle.clone());
        Arc::new(move || {
            let _ = ui_weak.lock().unwrap().upgrade_in_event_loop(|ui| {
                if ui.get_page() != 3 {
                    ui.invoke_go(3);
                }
                ruler::start(&ui);
            });
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
                    ui.set_ruler_hotkey_display(descriptions.get(2).cloned().unwrap_or_default().into());
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
        Shortcut {
            id: "measure",
            description: "Measure on the screen (Archtoys)",
            text: ui.get_setting_ruler_hotkey().to_string(),
            default: DEFAULT_RULER_HOTKEY_TEXT,
            trigger: measure_trigger.clone(),
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
    let _remote = single_instance::serve(show, trigger.clone(), extract_trigger.clone(), measure_trigger.clone());
    if pick_now {
        trigger();
    }
    if extract_now {
        extract_trigger();
    }
    if measure_now {
        measure_trigger();
    }
    let hotkey_service = Rc::new(hotkey_service);
    let active_hotkey_texts = Rc::new(RefCell::new(hotkey_texts.clone()));
    ui.set_setting_hotkey(hotkey_texts[0].clone().into());
    ui.set_setting_text_hotkey(hotkey_texts[1].clone().into());
    ui.set_setting_ruler_hotkey(hotkey_texts[2].clone().into());

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
        let index = ui.get_hotkey_target().clamp(0, 2) as usize;
        let result = build_hotkey_from_capture(&key_text, ctrl, alt, shift, meta)
            .and_then(|candidate| hotkey_service_apply.change(index, &candidate));
        let set = |ui: &AppWindow, text: String| match index {
            0 => ui.set_setting_hotkey(text.into()),
            1 => ui.set_setting_text_hotkey(text.into()),
            _ => ui.set_setting_ruler_hotkey(text.into()),
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
            ui.set_several_screens(window::screen_count(&ui) > 1); // offers the ruler's "Across screens"
        }
    });

    // --- The frame (v0.7.0): the sidebar, pages ---
    let w_ui = ui_handle.clone();
    ui.on_window_move(move || {
        if let Some(ui) = w_ui.upgrade() {
            window::start_move(&ui);
        }
    });
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

    // --- Each toy's page ---
    picker::page::wire(&ui, &history_store);
    ocr::page::wire(&ui);
    ruler::page::wire(&ui, &history_store);

    let report_ui = ui_handle.clone();
    ui.on_report_problem(move || {
        // the screens and scales, for the report (no screen names)
        let screens = report_ui
            .upgrade()
            .map(|ui| {
                use slint::winit_030::WinitWindowAccessor;
                let list = ui.window().with_winit_window(|w| {
                    w.available_monitors()
                        .map(|m| ((m.position().x, m.position().y), (m.size().width, m.size().height), m.scale_factor()))
                        .collect::<Vec<_>>()
                });
                diagnostics::Screens {
                    screens: list.map(diagnostics::screens_line).unwrap_or_else(|| "unknown".into()),
                    scale: ui.window().scale_factor(),
                }
            })
            .unwrap_or(diagnostics::Screens { screens: "unknown".into(), scale: 1.0 });
        if let Err(err) = diagnostics::open_issue_page(&screens) {
            eprintln!("report: couldn't open the browser: {err}");
            if let Some(ui) = report_ui.upgrade() {
                show_message(&ui, "No browser found. Report at github.com/Mujtaba1i/Archtoys/issues");
            }
        }
    });

    let coffee_ui = ui_handle.clone();
    ui.on_buy_coffee(move || {
        if let Err(err) = launch::open_support_page() {
            eprintln!("support: couldn't open the browser: {err}");
            if let Some(ui) = coffee_ui.upgrade() {
                show_message(&ui, "No browser found. Support Archtoys at mujtaba1i.github.io/Archtoys/support");
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

    // KDE Wayland: KWin puts the window back where it was (kwin.rs)
    if kwin::available() {
        if let Err(err) = kwin::remember_main_window() {
            eprintln!("window: KWin can't put the window back where it was ({err})");
        }
    }
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
