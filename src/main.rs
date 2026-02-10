slint::include_modules!();

use arboard::{Clipboard, SetExtLinux};
use palette::{Srgb, Hsl, Hsv, FromColor, IntoColor};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use slint::{Color, LogicalPosition, ModelRc, VecModel};
use std::thread;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{HotKey, Modifiers, Code};
use device_query::{DeviceQuery, DeviceState, Keycode};
use scrap::{Display, Capturer};
use std::time::Duration;
use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, Ordering};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray};

static PICKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static PICKER_CANCELLED: AtomicBool = AtomicBool::new(false);

const OVERLAY_WIDTH: i32 = 140;
const OVERLAY_HEIGHT: i32 = 44;
const OVERLAY_OFFSET_X: i32 = 20;
const OVERLAY_OFFSET_Y: i32 = 20;

// (unused) small helper removed in favor of X11 capture via `scrap` in the live picker.

struct AppTray {
    ui: slint::Weak<AppWindow>,
}

impl Tray for AppTray {
    fn id(&self) -> String {
        "archtoys-color-picker".into()
    }

    fn title(&self) -> String {
        "Archtoys Color Picker".into()
    }

    fn icon_name(&self) -> String {
        "archtoys".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let ui = self.ui.clone();
        let _ = ui.upgrade_in_event_loop(|ui| {
            ui.window().show().ok();
        });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|this: &mut AppTray| {
                    let ui = this.ui.clone();
                    let _ = ui.upgrade_in_event_loop(|ui: AppWindow| {
                        ui.window().show().ok();
                    });
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_this: &mut AppTray| {
                    let _ = slint::invoke_from_event_loop(|| {
                        slint::quit_event_loop().ok();
                    });
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    dark_mode: bool,
    setting_minimize: bool,
    setting_autocopy: bool,
    history: Vec<[u8; 3]>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dark_mode: false,
            setting_minimize: false,
            setting_autocopy: false,
            history: vec![],
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("archtoys-color-picker").join("config.json");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("archtoys-color-picker")
            .join("config.json");
    }
    PathBuf::from("config.json")
}

fn load_config() -> Option<AppConfig> {
    let path = config_path();
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn save_config(cfg: &AppConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!("config: create dir failed: {:?}", err);
            return;
        }
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(data) => {
            if let Err(err) = fs::write(path, data) {
                eprintln!("config: write failed: {:?}", err);
            }
        }
        Err(err) => eprintln!("config: serialize failed: {:?}", err),
    }
}

fn snapshot_config(ui: &AppWindow, history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>) -> AppConfig {
    let skin = ui.global::<Skin>();
    let history = {
        let guard = history_store.lock().unwrap();
        guard.iter().map(|(r, g, b)| [*r, *g, *b]).collect()
    };
    AppConfig {
        dark_mode: skin.get_dark_mode(),
        setting_minimize: ui.get_setting_minimize(),
        setting_autocopy: ui.get_setting_autocopy(),
        history,
    }
}

fn apply_config(ui: &AppWindow, history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>, cfg: &AppConfig) {
    let skin = ui.global::<Skin>();
    skin.set_dark_mode(cfg.dark_mode);
    ui.set_setting_minimize(cfg.setting_minimize);
    ui.set_setting_autocopy(cfg.setting_autocopy);
    if !cfg.history.is_empty() {
        let mut guard = history_store.lock().unwrap();
        guard.clear();
        for rgb in &cfg.history {
            guard.push((rgb[0], rgb[1], rgb[2]));
        }
    }
}

fn persist_config(ui: &AppWindow, history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>) {
    let cfg = snapshot_config(ui, history_store);
    save_config(&cfg);
}

fn overlay_position(x: i32, y: i32, screen_w: i32, screen_h: i32) -> (i32, i32) {
    let mut pos_x = x + OVERLAY_OFFSET_X;
    let mut pos_y = y + OVERLAY_OFFSET_Y;

    if pos_x + OVERLAY_WIDTH > screen_w {
        pos_x = x - OVERLAY_WIDTH - OVERLAY_OFFSET_X;
    }
    if pos_y + OVERLAY_HEIGHT > screen_h {
        pos_y = y - OVERLAY_HEIGHT - OVERLAY_OFFSET_Y;
    }

    let max_x = (screen_w - OVERLAY_WIDTH).max(0);
    let max_y = (screen_h - OVERLAY_HEIGHT).max(0);
    pos_x = pos_x.clamp(0, max_x);
    pos_y = pos_y.clamp(0, max_y);

    (pos_x, pos_y)
}

fn finish_picker(ui_weak: slint::Weak<AppWindow>, overlay_weak: slint::Weak<PickerOverlay>) {
    PICKER_ACTIVE.store(false, Ordering::SeqCst);
    slint::invoke_from_event_loop(move || {
        if let Some(overlay) = overlay_weak.upgrade() {
            overlay.hide().ok();
        }
    })
    .ok();

    let _ = ui_weak.upgrade_in_event_loop(move |ui| {
        if ui.get_setting_minimize() {
            ui.window().show().ok();
        }
    });
}

fn start_picker(
    ui_weak: slint::Weak<AppWindow>,
    overlay_weak: slint::Weak<PickerOverlay>,
    history_store: Arc<Mutex<Vec<(u8,u8,u8)>>>,
) {
    // Ensure only one picker thread runs at a time
    if PICKER_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return;
    }
    PICKER_CANCELLED.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        let device = DeviceState::new();

        // Use scrap for X11 frame capture (works on X11/KDE)
        let display = match Display::main() {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Could not get primary display: {:?}", e);
                finish_picker(ui_weak, overlay_weak);
                return;
            }
        };

        let mut capturer = match Capturer::new(display) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Could not create capturer: {:?}", e);
                finish_picker(ui_weak, overlay_weak);
                return;
            }
        };

        let mut prev_left_pressed: bool = false;
        let mut last_color: (u8, u8, u8) = (0, 0, 0);
        let mut last_hex: String = String::from("000000");

        loop {
            if PICKER_CANCELLED.load(Ordering::SeqCst) {
                break;
            }

            let mouse = device.get_mouse();
            let mouse_x = mouse.coords.0;
            let mouse_y = mouse.coords.1;

            // Read a frame; scrap may return WouldBlock if no frame is ready
            let width = capturer.width() as i32;
            let height = capturer.height() as i32;
            let mut updated = false;
            match capturer.frame() {
                Ok(frame) => {
                    if width > 0 && height > 0 {
                        let safe_x = mouse_x.clamp(0, width.saturating_sub(1));
                        let safe_y = mouse_y.clamp(0, height.saturating_sub(1));
                        let stride = width as usize * 4;
                        let idx = safe_y as usize * stride + safe_x as usize * 4;
                        if idx + 2 < frame.len() {
                            let b = frame[idx];
                            let g = frame[idx + 1];
                            let r = frame[idx + 2];
                            last_color = (r, g, b);
                            last_hex = format!("{:02x}{:02x}{:02x}", r, g, b);
                            updated = true;
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock {
                        // no frame ready yet
                    } else {
                        eprintln!("Capture error: {:?}", e);
                        break;
                    }
                }
            }

            let (pos_x, pos_y) = overlay_position(mouse_x, mouse_y, width, height);
            let (r, g, b) = last_color;
            let hex = last_hex.clone();
            let overlay_hex = format!("#{}", hex);

            let ui_weak2 = ui_weak.clone();
            let overlay_weak2 = overlay_weak.clone();
            let invoke_result = slint::invoke_from_event_loop(move || {
                if PICKER_CANCELLED.load(Ordering::SeqCst) {
                    if let Some(overlay) = overlay_weak2.upgrade() {
                        overlay.hide().ok();
                    }
                    return;
                }
                if updated {
                    if let Some(ui) = ui_weak2.upgrade() {
                        let (lighter_2, lighter_1, darker_1, darker_2) =
                            calculate_shades(r, g, b);

                        ui.set_current_color(Color::from_rgb_u8(r, g, b));
                        ui.set_val_hex(hex.into());

                        ui.set_shade_lighter_2(lighter_2);
                        ui.set_shade_lighter_1(lighter_1);
                        ui.set_shade_darker_1(darker_1);
                        ui.set_shade_darker_2(darker_2);
                    }
                }

                if let Some(overlay) = overlay_weak2.upgrade() {
                    overlay.set_preview_color(Color::from_rgb_u8(r, g, b));
                    overlay.set_preview_hex(overlay_hex.into());
                    let scale = overlay.window().scale_factor();
                    let logical = LogicalPosition::new(
                        pos_x as f32 / scale,
                        pos_y as f32 / scale,
                    );
                    overlay.window().set_position(logical);
                    overlay.show().ok();
                }
            })
            ;
            if let Err(err) = invoke_result {
                eprintln!("invoke_from_event_loop error: {:?}", err);
            }

            // Left-click selection handling (device_query exposes `button_pressed` as Vec<bool>)
            let left_pressed = mouse.button_pressed.get(1).copied().unwrap_or(false);
            if left_pressed && !prev_left_pressed {
                // just-clicked
                // record to history_store (thread-safe)
                {
                    let mut guard = history_store.lock().unwrap();
                    guard.insert(0, (r, g, b));
                }

                // update UI and optionally copy/open details on the UI thread
                let hs = history_store.clone();
                let ui_weak2 = ui_weak.clone();
                let hex_for_ui = last_hex.clone();
                slint::invoke_from_event_loop(move || {
                    if let Some(ui2) = ui_weak2.upgrade() {
                        // sync history store into UI model
                        let cols: Vec<Color> = {
                            let guard = hs.lock().unwrap();
                            guard.iter().map(|(r,g,b)| Color::from_rgb_u8(*r,*g,*b)).collect()
                        };
                        ui2.set_history_model(ModelRc::from(Rc::new(VecModel::from(cols))));

                        if ui2.get_setting_autocopy() {
                            let hex_clone = hex_for_ui.clone();
                            thread::spawn(move || {
                                if let Ok(mut clipboard) = Clipboard::new() {
                                    let _ = clipboard.set().wait().text(hex_clone);
                                }
                            });
                        } else {
                            ui2.window().show().ok();
                            ui2.set_current_color(Color::from_rgb_u8(r, g, b));
                            ui2.set_val_hex(hex_for_ui.into());
                        }
                        persist_config(&ui2, &hs);
                    }
                })
                .ok();

                // selection finished
                break;
            }
            prev_left_pressed = left_pressed;

            // ESC to cancel immediately
            let keys = device.get_keys();
            if keys.contains(&Keycode::Escape) {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                break;
            }

            std::thread::sleep(Duration::from_millis(16));
        }

        finish_picker(ui_weak, overlay_weak);
    });
}

fn calculate_shades(r: u8, g: u8, b: u8) -> (Color, Color, Color, Color) {
    let lighter_2 = Color::from_rgb_u8(
        ((r as f32 * 1.5).min(255.0)) as u8,
        ((g as f32 * 1.5).min(255.0)) as u8,
        ((b as f32 * 1.5).min(255.0)) as u8,
    );
    let lighter_1 = Color::from_rgb_u8(
        ((r as f32 * 1.2).min(255.0)) as u8,
        ((g as f32 * 1.2).min(255.0)) as u8,
        ((b as f32 * 1.2).min(255.0)) as u8,
    );
    let darker_1 = Color::from_rgb_u8(
        (r as f32 * 0.7) as u8,
        (g as f32 * 0.7) as u8,
        (b as f32 * 0.7) as u8,
    );
    let darker_2 = Color::from_rgb_u8(
        (r as f32 * 0.5) as u8,
        (g as f32 * 0.5) as u8,
        (b as f32 * 0.5) as u8,
    );
    (lighter_2, lighter_1, darker_1, darker_2)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let overlay = PickerOverlay::new()?;
    overlay.hide().ok();
    overlay.window().on_close_requested(move || {
        PICKER_CANCELLED.store(true, Ordering::SeqCst);
        slint::CloseRequestResponse::HideWindow
    });
    let ui_handle = ui.as_weak();
    let overlay_weak = overlay.as_weak();

    let tray = AppTray { ui: ui_handle.clone() };
    let _tray_handle = match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tray: failed to spawn: {:?}", e);
            None
        }
    };
    let manager = GlobalHotKeyManager::new().unwrap();
    let hotkey = HotKey::new(Some(Modifiers::META | Modifiers::CONTROL), Code::KeyC);
    manager.register(hotkey).unwrap();
    let ui_hotkey = ui_handle.clone();

    // thread-safe plain-history store (source of truth for cross-thread access)
    let history_store: Arc<Mutex<Vec<(u8,u8,u8)>>> = Arc::new(Mutex::new(vec![
        (203u8, 182u8, 172u8),
        (85u8, 85u8, 85u8),
    ]));

    if let Some(cfg) = load_config() {
        apply_config(&ui, &history_store, &cfg);
    }

    // initialize UI history model from store
    let init_hist = {
        let guard = history_store.lock().unwrap();
        let cols: Vec<Color> = guard.iter().map(|(r,g,b)| Color::from_rgb_u8(*r,*g,*b)).collect();
        VecModel::from(cols)
    };
    ui.set_history_model(ModelRc::from(Rc::new(init_hist)));

    let hk_ui = ui_hotkey.clone();
    let hk_overlay = overlay_weak.clone();
    let hk_history = history_store.clone();
    std::thread::spawn(move || {
    let receiver = global_hotkey::GlobalHotKeyEvent::receiver();

    while let Ok(event) = receiver.recv() {
        if event.id == hotkey.id() {
            let ui_for_hide = hk_ui.clone();
            slint::invoke_from_event_loop(move || {
                if let Some(ui) = ui_for_hide.upgrade() {
                    if ui.get_setting_minimize() {
                        ui.window().hide().ok();
                    }
                }
            })
            .ok();
            start_picker(hk_ui.clone(), hk_overlay.clone(), hk_history.clone());
            }
        }
    });

    // history model already initialized above from `history_store`

    // Initialize shades for the default color
    let (r, g, b) = (203, 182, 172); // Default color cbb6ac
    let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
    ui.set_shade_lighter_2(lighter_2);
    ui.set_shade_lighter_1(lighter_1);
    ui.set_shade_darker_1(darker_1);
    ui.set_shade_darker_2(darker_2);

    let update_ui_colors = |ui: &AppWindow, r: u8, g: u8, b: u8| {
        ui.set_current_color(Color::from_rgb_u8(r, g, b));
        
        let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
        ui.set_shade_lighter_2(lighter_2);
        ui.set_shade_lighter_1(lighter_1);
        ui.set_shade_darker_1(darker_1);
        ui.set_shade_darker_2(darker_2);
        
        let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let hsl: Hsl = Hsl::from_color(srgb);
        let hsv: Hsv = Hsv::from_color(srgb);
        
        ui.set_val_hex(format!("{:02x}{:02x}{:02x}", r, g, b).into());
        ui.set_val_rgb(format!("rgb({}, {}, {})", r, g, b).into());
        ui.set_val_hsl(format!("hsl({:.0}, {:.0}%, {:.0}%)", hsl.hue.into_degrees(), hsl.saturation * 100.0, hsl.lightness * 100.0).into());
        ui.set_val_hsv(format!("hsv({:.0}, {:.0}%, {:.0}%)", hsv.hue.into_degrees(), hsv.saturation * 100.0, hsv.value * 100.0).into());
    };

    let settings_ui = ui_handle.clone();
    let settings_history = history_store.clone();
    ui.on_settings_changed(move || {
        if let Some(ui) = settings_ui.upgrade() {
            persist_config(&ui, &settings_history);
        }
    });

    let ui_weak = ui_handle.clone();
    let pick_history = history_store.clone();
    let pick_overlay = overlay_weak.clone();
    ui.on_pick_color(move || {
        let ui = ui_weak.unwrap();
        if ui.get_setting_minimize() {
            ui.window().hide().ok();
        }
        // start the system-wide picker (live preview). Pass history store so selection is recorded.
        start_picker(ui_weak.clone(), pick_overlay.clone(), pick_history.clone());
    });

    ui.on_copy_to_clipboard(move |text| {
        let text_to_copy = text.to_string();
        thread::spawn(move || {
            match Clipboard::new() {
                Ok(mut clipboard) => {
                    let _ = clipboard.set().wait().text(text_to_copy);
                }
                Err(e) => eprintln!("Clipboard error: {}", e),
            }
        });
    });

    let ui_weak = ui_handle.clone();
    let history_read = history_store.clone();
    ui.on_history_clicked(move |index| {
        let ui = ui_weak.unwrap();
        let guard = history_read.lock().unwrap();
        if let Some((r,g,b)) = guard.get(index as usize) {
            update_ui_colors(&ui, *r, *g, *b);
        }
    });

    let history_clear = history_store.clone();
    let ui_clear = ui_handle.clone();
    ui.on_clear_history(move || {
        {
            let mut guard = history_clear.lock().unwrap();
            guard.clear();
        }
        if let Some(ui) = ui_clear.upgrade() {
            ui.set_history_model(ModelRc::from(Rc::new(VecModel::from(Vec::<Color>::new()))));
            persist_config(&ui, &history_clear);
        }
    });

    let ui_weak = ui_handle.clone();
    let history_shades = history_store.clone();
    ui.on_shade_clicked(move |factor| {
        let ui = ui_weak.unwrap();
        let current = ui.get_current_color();
        let r = (current.red() as f32 * factor).clamp(0.0, 255.0) as u8;
        let g = (current.green() as f32 * factor).clamp(0.0, 255.0) as u8;
        let b = (current.blue() as f32 * factor).clamp(0.0, 255.0) as u8;
        {
            let mut guard = history_shades.lock().unwrap();
            guard.insert(0, (r,g,b));
        }
        // sync into UI model
        let hs = history_shades.clone();
        let ui_weak2 = ui_weak.clone();
        slint::invoke_from_event_loop(move || {
            if let Some(ui2) = ui_weak2.upgrade() {
                let cols: Vec<Color> = {
                    let guard = hs.lock().unwrap();
                    guard.iter().map(|(r,g,b)| Color::from_rgb_u8(*r,*g,*b)).collect()
                };
                ui2.set_history_model(ModelRc::from(Rc::new(VecModel::from(cols))));
            }
        })
        .ok();
        update_ui_colors(&ui, r, g, b);
        persist_config(&ui, &history_shades);
    });

    // Parse color from different formats
    fn parse_color(type_str: &str, value: &str) -> Option<(u8, u8, u8)> {
        match type_str {
            "HEX" => {
                let clean = value.trim_start_matches('#').trim();
                if clean.len() == 6 {
                    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
                    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
                    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "RGB" => {
                let clean = value.trim().trim_start_matches("rgb(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let r = parts[0].trim().parse::<u8>().ok()?;
                    let g = parts[1].trim().parse::<u8>().ok()?;
                    let b = parts[2].trim().parse::<u8>().ok()?;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "HSL" => {
                let clean = value.trim().trim_start_matches("hsl(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let h = parts[0].trim().parse::<f32>().ok()?;
                    let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    
                    let hsl = Hsl::new(h, s, l);
                    let rgb: Srgb = hsl.into_color();
                    let r = (rgb.red * 255.0).round() as u8;
                    let g = (rgb.green * 255.0).round() as u8;
                    let b = (rgb.blue * 255.0).round() as u8;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "HSV" => {
                let clean = value.trim().trim_start_matches("hsv(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let h = parts[0].trim().parse::<f32>().ok()?;
                    let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    let v = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    
                    let hsv = Hsv::new(h, s, v);
                    let rgb: Srgb = hsv.into_color();
                    let r = (rgb.red * 255.0).round() as u8;
                    let g = (rgb.green * 255.0).round() as u8;
                    let b = (rgb.blue * 255.0).round() as u8;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    let ui_weak = ui_handle.clone();
    ui.on_value_edited(move |type_str, value| {
        let ui = ui_weak.unwrap();
        if let Some((r, g, b)) = parse_color(&type_str.to_string(), &value.to_string()) {
            ui.set_current_color(Color::from_rgb_u8(r, g, b));
            
            let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
            ui.set_shade_lighter_2(lighter_2);
            ui.set_shade_lighter_1(lighter_1);
            ui.set_shade_darker_1(darker_1);
            ui.set_shade_darker_2(darker_2);
            
            let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            let hsl: Hsl = Hsl::from_color(srgb);
            let hsv: Hsv = Hsv::from_color(srgb);
            
            // Update other fields but not the one being edited
            if type_str != "HEX" {
                ui.set_val_hex(format!("{:02x}{:02x}{:02x}", r, g, b).into());
            }
            if type_str != "RGB" {
                ui.set_val_rgb(format!("rgb({}, {}, {})", r, g, b).into());
            }
            if type_str != "HSL" {
                ui.set_val_hsl(format!("hsl({:.0}, {:.0}%, {:.0}%)", hsl.hue.into_degrees(), hsl.saturation * 100.0, hsl.lightness * 100.0).into());
            }
            if type_str != "HSV" {
                ui.set_val_hsv(format!("hsv({:.0}, {:.0}%, {:.0}%)", hsv.hue.into_degrees(), hsv.saturation * 100.0, hsv.value * 100.0).into());
            }
        }
    });

    let ui_weak = ui_handle.clone();
    let history_accept = history_store.clone();
    ui.on_value_accepted(move |type_str, value| {
        let ui = ui_weak.unwrap();
        if let Some((r, g, b)) = parse_color(&type_str.to_string(), &value.to_string()) {
            {
                let mut guard = history_accept.lock().unwrap();
                guard.insert(0, (r,g,b));
            }
            // sync into UI model
            let hs = history_accept.clone();
            if let Some(ui2) = ui_weak.upgrade() {
                let cols: Vec<Color> = {
                    let guard = hs.lock().unwrap();
                    guard.iter().map(|(r,g,b)| Color::from_rgb_u8(*r,*g,*b)).collect()
                };
                ui2.set_history_model(ModelRc::from(Rc::new(VecModel::from(cols))));
            }
            update_ui_colors(&ui, r, g, b);
            persist_config(&ui, &history_accept);
        }
    });

    let ui_close = ui_handle.clone();
    ui.window().on_close_requested(move || {
        let _ = ui_close.upgrade_in_event_loop(|ui| {
            ui.set_close_confirm_open(true);
        });
        slint::CloseRequestResponse::KeepWindowShown
    });

    let ui_close = ui_handle.clone();
    let close_history = history_store.clone();
    ui.on_close_confirm_close(move || {
        if let Some(ui) = ui_close.upgrade() {
            ui.set_close_confirm_open(false);
            persist_config(&ui, &close_history);
        }
        slint::quit_event_loop().ok();
        std::process::exit(0);
    });

    let ui_min = ui_handle.clone();
    ui.on_close_confirm_minimize(move || {
        let _ = ui_min.upgrade_in_event_loop(|ui| {
            ui.set_close_confirm_open(false);
            ui.window().hide().ok();
        });
    });

    ui.show()?;
    slint::run_event_loop_until_quit()
}
