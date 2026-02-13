slint::include_modules!();

use arboard::{Clipboard, SetExtLinux};
use device_query::{DeviceQuery, DeviceState, Keycode};
use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use image::{GenericImageView, ImageFormat};
use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};
use palette::{FromColor, Hsl, Hsv, IntoColor, Srgb};
use scrap::{Capturer, Display};
use serde::{Deserialize, Serialize};
use slint::{Color, LogicalPosition, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{ConnectionExt as _, EventMask, GrabMode, GrabStatus};
use x11rb::{CURRENT_TIME, NONE};
use zbus::blocking::{Connection as ZbusConnection, Proxy as ZbusProxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

static PICKER_ACTIVE: AtomicBool = AtomicBool::new(false);
static PICKER_CANCELLED: AtomicBool = AtomicBool::new(false);

const OVERLAY_WIDTH: i32 = 140;
const OVERLAY_HEIGHT: i32 = 44;
const OVERLAY_OFFSET_X: i32 = 20;
const OVERLAY_OFFSET_Y: i32 = 20;
const WINDOW_MIN_WIDTH: f64 = 480.0;
const WINDOW_MIN_HEIGHT: f64 = 320.0;
const WINDOW_MAX_WIDTH: f64 = 900.0;
const WINDOW_MAX_HEIGHT: f64 = 620.0;
const DEFAULT_HOTKEY_TEXT: &str = "Ctrl+Super+C";

thread_local! {
    static PICKER_OVERLAY: RefCell<Option<PickerOverlay>> = RefCell::new(None);
    static PICKER_SHIELD: RefCell<Option<PickerShieldWindow>> = RefCell::new(None);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerSource {
    Hotkey,
    Button,
}

#[derive(Debug, Clone, Copy)]
struct PickerContext {
    source: PickerSource,
    was_visible_before_trigger: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionType {
    X11,
    Wayland,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorField {
    Hex,
    Rgb,
    Hsl,
    Hsv,
}

impl ColorField {
    fn from_ui_label(label: &str) -> Option<Self> {
        match label {
            "HEX" => Some(Self::Hex),
            "RGB" => Some(Self::Rgb),
            "HSL" => Some(Self::Hsl),
            "HSV" => Some(Self::Hsv),
            _ => None,
        }
    }
}

struct X11PointerGrab {
    conn: x11rb::rust_connection::RustConnection,
}

impl X11PointerGrab {
    fn acquire() -> Result<Self, String> {
        let (conn, screen_num) =
            x11rb::connect(None).map_err(|err| format!("x11 connect failed: {err:?}"))?;
        let root = conn
            .setup()
            .roots
            .get(screen_num)
            .ok_or_else(|| "x11 root screen not found".to_string())?
            .root;

        let cookie = conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS
                    | EventMask::KEY_RELEASE,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                NONE,
                CURRENT_TIME,
            )
            .map_err(|err| format!("x11 grab_pointer request failed: {err:?}"))?;

        let reply = cookie
            .reply()
            .map_err(|err| format!("x11 grab_pointer reply failed: {err:?}"))?;

        if reply.status != GrabStatus::SUCCESS {
            return Err(format!(
                "x11 grab_pointer not successful: {:?}",
                reply.status
            ));
        }

        conn.flush()
            .map_err(|err| format!("x11 flush after grab failed: {err:?}"))?;

        Ok(Self { conn })
    }
}

impl Drop for X11PointerGrab {
    fn drop(&mut self) {
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.flush();
    }
}

fn with_picker_overlay<R>(f: impl FnOnce(&mut Option<PickerOverlay>) -> R) -> R {
    PICKER_OVERLAY.with(|slot| {
        let mut overlay = slot.borrow_mut();
        f(&mut overlay)
    })
}

fn with_picker_shield<R>(f: impl FnOnce(&mut Option<PickerShieldWindow>) -> R) -> R {
    PICKER_SHIELD.with(|slot| {
        let mut shield = slot.borrow_mut();
        f(&mut shield)
    })
}

fn ensure_picker_overlay() -> Result<slint::Weak<PickerOverlay>, slint::PlatformError> {
    with_picker_overlay(|slot| {
        if slot.is_none() {
            let overlay = PickerOverlay::new()?;
            overlay.hide().ok();
            overlay.window().on_close_requested(move || {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                slint::CloseRequestResponse::HideWindow
            });
            *slot = Some(overlay);
        }

        Ok(slot.as_ref().expect("picker overlay must exist").as_weak())
    })
}

fn ensure_picker_shield() -> Result<slint::Weak<PickerShieldWindow>, slint::PlatformError> {
    with_picker_shield(|slot| {
        if slot.is_none() {
            let shield = PickerShieldWindow::new()?;
            shield.hide().ok();
            shield.window().on_close_requested(move || {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                slint::CloseRequestResponse::HideWindow
            });
            *slot = Some(shield);
        }

        Ok(slot.as_ref().expect("picker shield must exist").as_weak())
    })
}

fn release_picker_overlay() {
    with_picker_overlay(|slot| {
        if let Some(overlay) = slot.take() {
            overlay.hide().ok();
        }
    });
}

fn release_picker_shield() {
    with_picker_shield(|slot| {
        if let Some(shield) = slot.take() {
            shield.hide().ok();
        }
    });
}

fn tray_icon_pixmap() -> Vec<Icon> {
    static ICON: OnceLock<Vec<Icon>> = OnceLock::new();
    ICON.get_or_init(|| {
        let bytes = include_bytes!("../packaging/archtoys-64.png");
        let img = match image::load_from_memory_with_format(bytes, ImageFormat::Png) {
            Ok(img) => img,
            Err(err) => {
                eprintln!("tray: failed to decode embedded icon: {err:?}");
                return vec![];
            }
        };
        let (width, height) = img.dimensions();
        let mut data = img.into_rgba8().into_vec();
        if data.len() % 4 != 0 {
            eprintln!("tray: icon data has invalid length");
            return vec![];
        }
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1); // rgba -> argb
        }
        vec![Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    })
    .clone()
}

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

    fn icon_pixmap(&self) -> Vec<Icon> {
        tray_icon_pixmap()
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
#[serde(default)]
struct AppConfig {
    dark_mode: bool,
    setting_minimize: bool,
    setting_autocopy: bool,
    setting_autostart: bool,
    setting_hotkey: String,
    history: Vec<[u8; 3]>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dark_mode: false,
            setting_minimize: false,
            setting_autocopy: false,
            setting_autostart: false,
            setting_hotkey: DEFAULT_HOTKEY_TEXT.to_string(),
            history: vec![],
        }
    }
}

fn config_base_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(dir);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".config");
    }
    PathBuf::from(".")
}

fn config_path() -> PathBuf {
    config_base_dir()
        .join("archtoys-color-picker")
        .join("config.json")
}

fn autostart_path() -> PathBuf {
    config_base_dir().join("autostart").join("archtoys.desktop")
}

fn autostart_entry_contents() -> &'static str {
    "[Desktop Entry]\nType=Application\nName=Archtoys\nComment=System-wide color picker\nExec=archtoys --start-hidden\nIcon=archtoys\nTerminal=false\nStartupWMClass=archtoys-bin\nCategories=Graphics;Utility;\nX-GNOME-Autostart-enabled=true\n"
}

fn sync_autostart_entry(enabled: bool) {
    let path = autostart_path();
    if enabled {
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                eprintln!("autostart: create dir failed: {err:?}");
                return;
            }
        }
        if let Err(err) = fs::write(&path, autostart_entry_contents()) {
            eprintln!("autostart: write failed: {err:?}");
        }
    } else if let Err(err) = fs::remove_file(&path) {
        if err.kind() != ErrorKind::NotFound {
            eprintln!("autostart: remove failed: {err:?}");
        }
    }
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
            eprintln!("config: create dir failed: {err:?}");
            return;
        }
    }
    match serde_json::to_string_pretty(cfg) {
        Ok(data) => {
            if let Err(err) = fs::write(path, data) {
                eprintln!("config: write failed: {err:?}");
            }
        }
        Err(err) => eprintln!("config: serialize failed: {err:?}"),
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
        setting_autostart: ui.get_setting_autostart(),
        setting_hotkey: ui.get_setting_hotkey().to_string(),
        history,
    }
}

fn apply_config(ui: &AppWindow, history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>, cfg: &AppConfig) {
    let skin = ui.global::<Skin>();
    skin.set_dark_mode(cfg.dark_mode);
    ui.set_setting_minimize(cfg.setting_minimize);
    ui.set_setting_autocopy(cfg.setting_autocopy);
    ui.set_setting_autostart(cfg.setting_autostart);
    ui.set_setting_hotkey(cfg.setting_hotkey.clone().into());

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

fn apply_native_window_constraints(ui: &AppWindow) {
    use slint::winit_030::{winit, WinitWindowAccessor};

    ui.window().with_winit_window(|window| {
        window.set_min_inner_size(Some(winit::dpi::LogicalSize::new(
            WINDOW_MIN_WIDTH,
            WINDOW_MIN_HEIGHT,
        )));
        window.set_max_inner_size(Some(winit::dpi::LogicalSize::new(
            WINDOW_MAX_WIDTH,
            WINDOW_MAX_HEIGHT,
        )));
        window.set_resizable(true);
    });
}

fn normalize_hotkey_text(input: &str) -> String {
    let tokens: Vec<String> = input
        .split('+')
        .map(|token| token.trim())
        .filter(|token| !token.is_empty())
        .map(|token| match token.to_ascii_uppercase().as_str() {
            "META" | "WIN" | "WINDOWS" => "Super".to_string(),
            "CTL" => "Ctrl".to_string(),
            _ => token.to_string(),
        })
        .collect();
    tokens.join("+")
}

fn parse_hotkey_text(input: &str) -> Result<(HotKey, String), String> {
    let normalized = normalize_hotkey_text(input);
    if normalized.is_empty() {
        return Err("hotkey cannot be empty".to_string());
    }

    let parsed = std::panic::catch_unwind(|| HotKey::from_str(&normalized))
        .map_err(|_| format!("invalid hotkey `{normalized}`"))?
        .map_err(|err| format!("invalid hotkey `{normalized}`: {err}"))?;
    Ok((parsed, normalized))
}

fn normalize_captured_hotkey_key(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let token = trimmed.strip_prefix("Key.").unwrap_or(trimmed);
    let upper = token.to_ascii_uppercase();

    if matches!(
        upper.as_str(),
        "CTRL"
            | "CONTROL"
            | "SHIFT"
            | "ALT"
            | "META"
            | "SUPER"
            | "CMD"
            | "COMMAND"
            | "WIN"
            | "WINDOWS"
    ) {
        return None;
    }

    match upper.as_str() {
        "ESC" | "ESCAPE" => Some("Escape".to_string()),
        "RETURN" | "ENTER" => Some("Enter".to_string()),
        "TAB" => Some("Tab".to_string()),
        "SPACE" => Some("Space".to_string()),
        "BACKSPACE" => Some("Backspace".to_string()),
        "DELETE" | "DEL" => Some("Delete".to_string()),
        "INSERT" => Some("Insert".to_string()),
        "HOME" => Some("Home".to_string()),
        "END" => Some("End".to_string()),
        "PAGEUP" => Some("PageUp".to_string()),
        "PAGEDOWN" => Some("PageDown".to_string()),
        "UP" | "ARROWUP" => Some("ArrowUp".to_string()),
        "DOWN" | "ARROWDOWN" => Some("ArrowDown".to_string()),
        "LEFT" | "ARROWLEFT" => Some("ArrowLeft".to_string()),
        "RIGHT" | "ARROWRIGHT" => Some("ArrowRight".to_string()),
        _ => {
            let mut chars = token.chars();
            if let (Some(ch), None) = (chars.next(), chars.next()) {
                if ch.is_ascii_alphabetic() {
                    return Some(ch.to_ascii_uppercase().to_string());
                }
                if ch.is_ascii_digit() {
                    return Some(ch.to_string());
                }
                if matches!(
                    ch,
                    '`' | '\\' | '[' | ']' | ',' | '=' | '-' | '.' | '\'' | ';' | '/'
                ) {
                    return Some(ch.to_string());
                }
            }

            if upper.starts_with('F')
                && upper
                    .strip_prefix('F')
                    .and_then(|n| n.parse::<u8>().ok())
                    .is_some_and(|n| (1..=24).contains(&n))
            {
                return Some(upper);
            }

            Some(token.to_string())
        }
    }
}

fn build_hotkey_from_capture(
    key_text: &str,
    ctrl: bool,
    alt: bool,
    shift: bool,
    meta: bool,
) -> Result<String, String> {
    if !(ctrl || alt || shift || meta) {
        return Err(
            "hotkey must include at least one modifier (Ctrl, Alt, Shift, Super)".to_string(),
        );
    }

    let key = normalize_captured_hotkey_key(key_text)
        .ok_or_else(|| "press a non-modifier key together with your modifier(s)".to_string())?;

    let mut parts: Vec<String> = vec![];
    if ctrl {
        parts.push("Ctrl".to_string());
    }
    if alt {
        parts.push("Alt".to_string());
    }
    if shift {
        parts.push("Shift".to_string());
    }
    if meta {
        parts.push("Super".to_string());
    }
    parts.push(key);
    Ok(parts.join("+"))
}

fn detect_session_type() -> SessionType {
    match std::env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "x11" => SessionType::X11,
        "wayland" => SessionType::Wayland,
        _ => SessionType::Unknown,
    }
}

fn format_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

fn format_rgb(r: u8, g: u8, b: u8) -> String {
    format!("rgb({r},{g},{b})")
}

fn format_hsl(r: u8, g: u8, b: u8) -> String {
    let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let hsl: Hsl = Hsl::from_color(srgb);
    let h = hsl.hue.into_degrees().round().rem_euclid(360.0);
    let s = (hsl.saturation * 100.0).round().clamp(0.0, 100.0);
    let l = (hsl.lightness * 100.0).round().clamp(0.0, 100.0);
    format!("hsl({h:.0},{s:.0}%,{l:.0}%)")
}

fn format_hsv(r: u8, g: u8, b: u8) -> String {
    let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let hsv: Hsv = Hsv::from_color(srgb);
    let h = hsv.hue.into_degrees().round().rem_euclid(360.0);
    let s = (hsv.saturation * 100.0).round().clamp(0.0, 100.0);
    let v = (hsv.value * 100.0).round().clamp(0.0, 100.0);
    format!("hsv({h:.0},{s:.0}%,{v:.0}%)")
}

fn format_canonical(field: ColorField, rgb: (u8, u8, u8)) -> String {
    let (r, g, b) = rgb;
    match field {
        ColorField::Hex => format_hex(r, g, b),
        ColorField::Rgb => format_rgb(r, g, b),
        ColorField::Hsl => format_hsl(r, g, b),
        ColorField::Hsv => format_hsv(r, g, b),
    }
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

fn update_preview_color(ui: &AppWindow, r: u8, g: u8, b: u8) {
    ui.set_current_color(Color::from_rgb_u8(r, g, b));
    let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
    ui.set_shade_lighter_2(lighter_2);
    ui.set_shade_lighter_1(lighter_1);
    ui.set_shade_darker_1(darker_1);
    ui.set_shade_darker_2(darker_2);
}

fn update_ui_colors(ui: &AppWindow, r: u8, g: u8, b: u8) {
    update_preview_color(ui, r, g, b);
    let rgb = (r, g, b);
    ui.set_val_hex(format_canonical(ColorField::Hex, rgb).into());
    ui.set_val_rgb(format_canonical(ColorField::Rgb, rgb).into());
    ui.set_val_hsl(format_canonical(ColorField::Hsl, rgb).into());
    ui.set_val_hsv(format_canonical(ColorField::Hsv, rgb).into());
}

fn update_ui_preview_except_field(ui: &AppWindow, editing_field: ColorField, r: u8, g: u8, b: u8) {
    update_preview_color(ui, r, g, b);
    let rgb = (r, g, b);

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

fn parse_hex_flexible(value: &str) -> Option<(u8, u8, u8)> {
    let clean = value.trim().trim_start_matches('#');
    if clean.len() != 6 {
        return None;
    }

    let upper = clean.to_ascii_uppercase();
    let r = u8::from_str_radix(&upper[0..2], 16).ok()?;
    let g = u8::from_str_radix(&upper[2..4], 16).ok()?;
    let b = u8::from_str_radix(&upper[4..6], 16).ok()?;
    Some((r, g, b))
}

fn inner_function_payload<'a>(value: &'a str, func_name: &str) -> &'a str {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = format!("{func_name}(");

    if lower.starts_with(&prefix) && trimmed.ends_with(')') && trimmed.len() >= prefix.len() + 1 {
        let start = prefix.len();
        let end = trimmed.len() - 1;
        trimmed[start..end].trim()
    } else {
        trimmed
    }
}

fn parse_rgb_permissive(value: &str) -> Option<(u8, u8, u8)> {
    let payload = inner_function_payload(value, "rgb");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let parse_component = |s: &str| -> Option<u8> {
        let raw = s.parse::<i32>().ok()?;
        Some(raw.clamp(0, 255) as u8)
    };

    Some((
        parse_component(parts[0])?,
        parse_component(parts[1])?,
        parse_component(parts[2])?,
    ))
}

fn parse_percentage_0_to_1(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let raw = trimmed.strip_suffix('%').unwrap_or(trimmed).trim();
    let parsed = raw.parse::<f32>().ok()?;
    Some((parsed / 100.0).clamp(0.0, 1.0))
}

fn parse_hsl_permissive(value: &str) -> Option<(u8, u8, u8)> {
    let payload = inner_function_payload(value, "hsl");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()?.rem_euclid(360.0);
    let s = parse_percentage_0_to_1(parts[1])?;
    let l = parse_percentage_0_to_1(parts[2])?;

    let hsl = Hsl::new(h, s, l);
    let rgb: Srgb = hsl.into_color();

    Some((
        (rgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
    ))
}

fn parse_hsv_permissive(value: &str) -> Option<(u8, u8, u8)> {
    let payload = inner_function_payload(value, "hsv");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()?.rem_euclid(360.0);
    let s = parse_percentage_0_to_1(parts[1])?;
    let v = parse_percentage_0_to_1(parts[2])?;

    let hsv = Hsv::new(h, s, v);
    let rgb: Srgb = hsv.into_color();

    Some((
        (rgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
    ))
}

fn parse_color(field: ColorField, value: &str) -> Option<(u8, u8, u8)> {
    match field {
        ColorField::Hex => parse_hex_flexible(value),
        ColorField::Rgb => parse_rgb_permissive(value),
        ColorField::Hsl => parse_hsl_permissive(value),
        ColorField::Hsv => parse_hsv_permissive(value),
    }
}

fn sync_history_model(ui: &AppWindow, history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>) {
    let colors: Vec<Color> = {
        let guard = history_store.lock().unwrap();
        guard
            .iter()
            .map(|(r, g, b)| Color::from_rgb_u8(*r, *g, *b))
            .collect()
    };
    ui.set_history_model(ModelRc::from(Rc::new(VecModel::from(colors))));
}

fn push_history(history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>, rgb: (u8, u8, u8)) {
    let mut guard = history_store.lock().unwrap();
    guard.insert(0, rgb);
}

fn copy_text_async(text: String) {
    thread::spawn(move || match Clipboard::new() {
        Ok(mut clipboard) => {
            let _ = clipboard.set().wait().text(text);
        }
        Err(err) => eprintln!("Clipboard error: {err}"),
    });
}

fn apply_selected_color(
    ui: &AppWindow,
    history_store: &Arc<Mutex<Vec<(u8, u8, u8)>>>,
    r: u8,
    g: u8,
    b: u8,
) {
    push_history(history_store, (r, g, b));
    sync_history_model(ui, history_store);
    update_ui_colors(ui, r, g, b);

    if ui.get_setting_autocopy() {
        copy_text_async(format_hex(r, g, b));
    } else {
        ui.window().show().ok();
    }

    persist_config(ui, history_store);
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

fn finish_picker(ui_weak: slint::Weak<AppWindow>, context: PickerContext, selected: bool) {
    if let Err(err) = slint::invoke_from_event_loop(move || {
        release_picker_overlay();
        release_picker_shield();

        if let Some(ui) = ui_weak.upgrade() {
            let stealth = selected
                && context.source == PickerSource::Hotkey
                && !context.was_visible_before_trigger
                && ui.get_setting_autocopy();

            if ui.get_setting_minimize() && !stealth {
                ui.window().show().ok();
            }
        }

        PICKER_ACTIVE.store(false, Ordering::SeqCst);
    }) {
        eprintln!("finish_picker: invoke_from_event_loop error: {err:?}");
        PICKER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

fn start_x11_picker(
    ui_weak: slint::Weak<AppWindow>,
    history_store: Arc<Mutex<Vec<(u8, u8, u8)>>>,
    context: PickerContext,
) {
    let overlay_weak = match ensure_picker_overlay() {
        Ok(overlay_weak) => overlay_weak,
        Err(err) => {
            eprintln!("overlay: failed to create picker window: {err:?}");
            PICKER_ACTIVE.store(false, Ordering::SeqCst);
            return;
        }
    };

    let shield_weak = match ensure_picker_shield() {
        Ok(shield_weak) => Some(shield_weak),
        Err(err) => {
            eprintln!("shield: failed to create picker shield window: {err:?}");
            None
        }
    };

    if let Some(shield) = shield_weak.as_ref().and_then(|w| w.upgrade()) {
        shield.show().ok();
    }

    thread::spawn(move || {
        let _pointer_grab = match X11PointerGrab::acquire() {
            Ok(guard) => Some(guard),
            Err(err) => {
                eprintln!("x11 pointer grab warning: {err}");
                None
            }
        };

        let device = DeviceState::new();

        let display = match Display::main() {
            Ok(display) => display,
            Err(err) => {
                eprintln!("x11 picker: could not get primary display: {err:?}");
                finish_picker(ui_weak, context, false);
                return;
            }
        };

        let mut capturer = match Capturer::new(display) {
            Ok(capturer) => capturer,
            Err(err) => {
                eprintln!("x11 picker: could not create capturer: {err:?}");
                finish_picker(ui_weak, context, false);
                return;
            }
        };

        let mut prev_left_pressed = false;
        let mut last_color: (u8, u8, u8) = (0, 0, 0);
        let mut last_hex = String::from("000000");
        let mut selected = false;

        loop {
            if PICKER_CANCELLED.load(Ordering::SeqCst) {
                break;
            }

            let mouse = device.get_mouse();
            let mouse_x = mouse.coords.0;
            let mouse_y = mouse.coords.1;

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
                            last_hex = format!("{:02X}{:02X}{:02X}", r, g, b);
                            updated = true;
                        }
                    }
                }
                Err(err) => {
                    if err.kind() != ErrorKind::WouldBlock {
                        eprintln!("x11 picker: capture error: {err:?}");
                        break;
                    }
                }
            }

            let (pos_x, pos_y) = overlay_position(mouse_x, mouse_y, width, height);
            let (r, g, b) = last_color;
            let overlay_hex = format!("#{}", last_hex);

            let ui_weak2 = ui_weak.clone();
            let overlay_weak2 = overlay_weak.clone();
            let shield_weak2 = shield_weak.clone();
            let invoke_result = slint::invoke_from_event_loop(move || {
                if PICKER_CANCELLED.load(Ordering::SeqCst) {
                    if let Some(overlay) = overlay_weak2.upgrade() {
                        overlay.hide().ok();
                    }
                    if let Some(shield_weak) = shield_weak2 {
                        if let Some(shield) = shield_weak.upgrade() {
                            shield.hide().ok();
                        }
                    }
                    return;
                }

                if updated {
                    if let Some(ui) = ui_weak2.upgrade() {
                        update_preview_color(&ui, r, g, b);
                        ui.set_val_hex(overlay_hex.clone().into());
                    }
                }

                if let Some(overlay) = overlay_weak2.upgrade() {
                    overlay.set_preview_color(Color::from_rgb_u8(r, g, b));
                    overlay.set_preview_hex(overlay_hex.into());
                    let scale = overlay.window().scale_factor();
                    let logical = LogicalPosition::new(pos_x as f32 / scale, pos_y as f32 / scale);
                    overlay.window().set_position(logical);
                    overlay.show().ok();
                }
            });
            if let Err(err) = invoke_result {
                eprintln!("x11 picker: invoke_from_event_loop error: {err:?}");
            }

            let left_pressed = mouse.button_pressed.get(1).copied().unwrap_or(false);
            if left_pressed && !prev_left_pressed {
                let hs = history_store.clone();
                let ui_weak2 = ui_weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        apply_selected_color(&ui, &hs, r, g, b);
                    }
                });

                selected = true;
                break;
            }
            prev_left_pressed = left_pressed;

            let keys = device.get_keys();
            if keys.contains(&Keycode::Escape) {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                break;
            }

            thread::sleep(Duration::from_millis(16));
        }

        finish_picker(ui_weak, context, selected);
    });
}

fn next_portal_handle_token(prefix: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}_{millis}_{}", std::process::id())
}

fn wait_for_portal_response(
    connection: &ZbusConnection,
    handle_path: &OwnedObjectPath,
) -> Result<(u32, HashMap<String, OwnedValue>), String> {
    let request_proxy = ZbusProxy::new(
        connection,
        "org.freedesktop.portal.Desktop",
        handle_path.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|err| format!("portal: request proxy failed: {err}"))?;

    let mut responses = request_proxy
        .receive_signal("Response")
        .map_err(|err| format!("portal: response stream failed: {err}"))?;

    let response_message = responses
        .next()
        .ok_or_else(|| "portal: response stream ended".to_string())?;

    response_message
        .body()
        .deserialize()
        .map_err(|err| format!("portal: response decode failed: {err}"))
}

fn pick_color_via_portal() -> Result<Option<(u8, u8, u8)>, String> {
    let connection =
        ZbusConnection::session().map_err(|err| format!("portal: session bus failed: {err}"))?;

    let screenshot_proxy = ZbusProxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.portal.Screenshot",
    )
    .map_err(|err| format!("portal: screenshot proxy failed: {err}"))?;

    let handle_token = next_portal_handle_token("archtoys_pick");
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("handle_token", Value::from(handle_token.as_str()));

    let reply = screenshot_proxy
        .call_method("PickColor", &("", &options))
        .map_err(|err| format!("portal: PickColor call failed: {err}"))?;

    let (handle_path,): (OwnedObjectPath,) = reply
        .body()
        .deserialize()
        .map_err(|err| format!("portal: PickColor reply decode failed: {err}"))?;

    let (response_code, results) = wait_for_portal_response(&connection, &handle_path)?;
    if response_code == 1 || response_code == 2 {
        return Ok(None);
    }
    if response_code != 0 {
        return Err(format!(
            "portal: PickColor request rejected with code {response_code}"
        ));
    }

    let color_value = results
        .get("color")
        .ok_or_else(|| "portal: response did not include color".to_string())?;

    let (red, green, blue): (f64, f64, f64) = color_value
        .clone()
        .try_into()
        .map_err(|_| "portal: color type conversion failed".to_string())?;

    let to_u8 = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Ok(Some((to_u8(red), to_u8(green), to_u8(blue))))
}

fn pick_color_via_kwin() -> Result<Option<(u8, u8, u8)>, String> {
    let connection =
        ZbusConnection::session().map_err(|err| format!("kwin: session bus failed: {err}"))?;

    let proxy = ZbusProxy::new(
        &connection,
        "org.kde.KWin",
        "/ColorPicker",
        "org.kde.kwin.ColorPicker",
    )
    .map_err(|err| format!("kwin: color picker proxy failed: {err}"))?;

    let pick_reply = match proxy.call_method("pick", &()) {
        Ok(reply) => reply,
        Err(err) => {
            let message = err.to_string();
            if message.to_ascii_lowercase().contains("cancel") {
                return Ok(None);
            }
            return Err(format!("kwin: pick call failed: {message}"));
        }
    };

    // KWin currently replies with signature `(u)` on many builds; accept both `u` and `(u)`.
    let argb = pick_reply
        .body()
        .deserialize::<u32>()
        .or_else(|_| {
            pick_reply
                .body()
                .deserialize::<(u32,)>()
                .map(|tuple| tuple.0)
        })
        .map_err(|err| format!("kwin: pick decode failed: {err}"))?;

    let alpha = ((argb >> 24) & 0xff) as u8;
    if alpha == 0 {
        return Ok(None);
    }

    let red = ((argb >> 16) & 0xff) as u8;
    let green = ((argb >> 8) & 0xff) as u8;
    let blue = (argb & 0xff) as u8;
    Ok(Some((red, green, blue)))
}

fn start_wayland_picker(
    ui_weak: slint::Weak<AppWindow>,
    history_store: Arc<Mutex<Vec<(u8, u8, u8)>>>,
    context: PickerContext,
) {
    thread::spawn(move || {
        let result = match pick_color_via_kwin() {
            Ok(picked) => Ok(picked),
            Err(kwin_err) => {
                eprintln!("wayland picker: kwin picker unavailable ({kwin_err}), trying portal");
                pick_color_via_portal()
            }
        };

        match result {
            Ok(Some((r, g, b))) => {
                let history_store2 = history_store.clone();
                let ui_weak2 = ui_weak.clone();
                let invoke_result = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = ui_weak2.upgrade() {
                        apply_selected_color(&ui, &history_store2, r, g, b);
                    }
                });

                if let Err(err) = invoke_result {
                    eprintln!("wayland picker: invoke_from_event_loop failed: {err:?}");
                    finish_picker(ui_weak, context, false);
                    return;
                }

                finish_picker(ui_weak, context, true);
            }
            Ok(None) => {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                finish_picker(ui_weak, context, false);
            }
            Err(err) => {
                eprintln!("wayland picker: {err}");
                finish_picker(ui_weak, context, false);
            }
        }
    });
}

fn start_picker(
    ui_weak: slint::Weak<AppWindow>,
    history_store: Arc<Mutex<Vec<(u8, u8, u8)>>>,
    context: PickerContext,
) {
    if PICKER_ACTIVE
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    PICKER_CANCELLED.store(false, Ordering::SeqCst);

    match detect_session_type() {
        SessionType::Wayland => start_wayland_picker(ui_weak, history_store, context),
        SessionType::X11 | SessionType::Unknown => {
            start_x11_picker(ui_weak, history_store, context)
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let start_hidden = std::env::args().any(|arg| arg == "--start-hidden");

    let ui = AppWindow::new()?;
    apply_native_window_constraints(&ui);
    let ui_handle = ui.as_weak();

    let tray = AppTray {
        ui: ui_handle.clone(),
    };
    let _tray_handle = match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(err) => {
            eprintln!("tray: failed to spawn: {err:?}");
            None
        }
    };

    let history_store: Arc<Mutex<Vec<(u8, u8, u8)>>> =
        Arc::new(Mutex::new(vec![(203u8, 182u8, 172u8), (85u8, 85u8, 85u8)]));

    if let Some(cfg) = load_config() {
        apply_config(&ui, &history_store, &cfg);
    }
    if ui.get_setting_hotkey().trim().is_empty() {
        ui.set_setting_hotkey(DEFAULT_HOTKEY_TEXT.into());
    }
    sync_autostart_entry(ui.get_setting_autostart());

    let hotkey_manager = match GlobalHotKeyManager::new() {
        Ok(manager) => Some(Arc::new(manager)),
        Err(err) => {
            eprintln!("hotkey: manager init failed: {err}");
            None
        }
    };

    let (configured_hotkey, configured_hotkey_text) =
        match parse_hotkey_text(&ui.get_setting_hotkey().to_string()) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("hotkey: {err}; falling back to {DEFAULT_HOTKEY_TEXT}");
                parse_hotkey_text(DEFAULT_HOTKEY_TEXT)
                    .expect("default hotkey text must always parse")
            }
        };

    let (registered_hotkey, registered_hotkey_text) = if let Some(manager) = &hotkey_manager {
        match manager.register(configured_hotkey) {
            Ok(()) => (configured_hotkey, configured_hotkey_text),
            Err(err) => {
                eprintln!(
                    "hotkey: register failed for `{}`: {err}; falling back to {}",
                    configured_hotkey_text, DEFAULT_HOTKEY_TEXT
                );
                let (fallback_hotkey, fallback_text) = parse_hotkey_text(DEFAULT_HOTKEY_TEXT)
                    .expect("default hotkey text must always parse");
                if let Err(fallback_err) = manager.register(fallback_hotkey) {
                    eprintln!("hotkey: fallback register failed: {fallback_err}");
                }
                (fallback_hotkey, fallback_text)
            }
        }
    } else {
        (configured_hotkey, configured_hotkey_text)
    };

    ui.set_setting_hotkey(registered_hotkey_text.clone().into());

    let active_hotkey = Arc::new(Mutex::new(registered_hotkey));
    let active_hotkey_id = Arc::new(Mutex::new(registered_hotkey.id()));
    let active_hotkey_text = Arc::new(Mutex::new(registered_hotkey_text));

    sync_history_model(&ui, &history_store);
    update_ui_colors(&ui, 203, 182, 172);

    if hotkey_manager.is_some() {
        let hk_ui = ui_handle.clone();
        let hk_history = history_store.clone();
        let hk_active_id = active_hotkey_id.clone();
        thread::spawn(move || {
            let receiver = global_hotkey::GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                let current_hotkey_id = *hk_active_id.lock().unwrap();
                if event.id == current_hotkey_id {
                    let ui_for_pick = hk_ui.clone();
                    let history_for_pick = hk_history.clone();

                    let _ = ui_for_pick.upgrade_in_event_loop(move |ui| {
                        let was_visible = ui.window().is_visible();
                        if ui.get_setting_minimize() {
                            ui.window().hide().ok();
                        }

                        start_picker(
                            ui.as_weak(),
                            history_for_pick,
                            PickerContext {
                                source: PickerSource::Hotkey,
                                was_visible_before_trigger: was_visible,
                            },
                        );
                    });
                }
            }
        });
    }

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
    let hotkey_manager_apply = hotkey_manager.clone();
    let hotkey_active = active_hotkey.clone();
    let hotkey_active_id = active_hotkey_id.clone();
    let hotkey_active_text = active_hotkey_text.clone();
    ui.on_hotkey_captured(move |key_text, ctrl, alt, shift, meta| {
        let Some(ui) = hotkey_ui.upgrade() else {
            return;
        };

        let candidate =
            match build_hotkey_from_capture(&key_text.to_string(), ctrl, alt, shift, meta) {
                Ok(candidate) => candidate,
                Err(err) => {
                    eprintln!("hotkey: {err}");
                    let current_text = hotkey_active_text.lock().unwrap().clone();
                    ui.set_setting_hotkey(current_text.into());
                    return;
                }
            };

        let (new_hotkey, normalized_text) = match parse_hotkey_text(&candidate) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("hotkey: {err}");
                let current_text = hotkey_active_text.lock().unwrap().clone();
                ui.set_setting_hotkey(current_text.into());
                return;
            }
        };

        let old_hotkey = *hotkey_active.lock().unwrap();
        if new_hotkey.id() == old_hotkey.id() {
            ui.set_setting_hotkey(normalized_text.clone().into());
            *hotkey_active_text.lock().unwrap() = normalized_text;
            persist_config(&ui, &hotkey_history);
            return;
        }

        if let Some(manager) = hotkey_manager_apply.as_ref() {
            let _ = manager.unregister(old_hotkey);
            if let Err(err) = manager.register(new_hotkey) {
                eprintln!("hotkey: failed to register `{}`: {err}", normalized_text);
                let _ = manager.register(old_hotkey);
                let previous_text = hotkey_active_text.lock().unwrap().clone();
                ui.set_setting_hotkey(previous_text.into());
                return;
            }
        } else {
            eprintln!("hotkey: global manager unavailable on this session; saving only");
        }

        *hotkey_active.lock().unwrap() = new_hotkey;
        *hotkey_active_id.lock().unwrap() = new_hotkey.id();
        *hotkey_active_text.lock().unwrap() = normalized_text.clone();
        ui.set_setting_hotkey(normalized_text.into());
        persist_config(&ui, &hotkey_history);
    });

    let pick_ui = ui_handle.clone();
    let pick_history = history_store.clone();
    ui.on_pick_color(move || {
        if let Some(ui) = pick_ui.upgrade() {
            let was_visible = ui.window().is_visible();
            if ui.get_setting_minimize() {
                ui.window().hide().ok();
            }

            start_picker(
                ui.as_weak(),
                pick_history.clone(),
                PickerContext {
                    source: PickerSource::Button,
                    was_visible_before_trigger: was_visible,
                },
            );
        }
    });

    ui.on_copy_to_clipboard(move |text| {
        copy_text_async(text.to_string());
    });

    let history_click_ui = ui_handle.clone();
    let history_click_store = history_store.clone();
    ui.on_history_clicked(move |index| {
        if let Some(ui) = history_click_ui.upgrade() {
            let guard = history_click_store.lock().unwrap();
            if let Some((r, g, b)) = guard.get(index as usize) {
                update_ui_colors(&ui, *r, *g, *b);
            }
        }
    });

    let clear_ui = ui_handle.clone();
    let clear_history = history_store.clone();
    ui.on_clear_history(move || {
        let Some(ui) = clear_ui.upgrade() else {
            return;
        };
        let current = ui.get_current_color();
        let keep_rgb = (current.red(), current.green(), current.blue());

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
            let current = ui.get_current_color();
            let r = (current.red() as f32 * factor).clamp(0.0, 255.0) as u8;
            let g = (current.green() as f32 * factor).clamp(0.0, 255.0) as u8;
            let b = (current.blue() as f32 * factor).clamp(0.0, 255.0) as u8;

            push_history(&shade_history, (r, g, b));
            sync_history_model(&ui, &shade_history);
            update_ui_colors(&ui, r, g, b);
            persist_config(&ui, &shade_history);
        }
    });

    let edited_ui = ui_handle.clone();
    ui.on_value_edited(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str.to_string()) else {
            return;
        };

        if let Some(ui) = edited_ui.upgrade() {
            if let Some((r, g, b)) = parse_color(field, &value.to_string()) {
                update_ui_preview_except_field(&ui, field, r, g, b);
            }
        }
    });

    let accepted_ui = ui_handle.clone();
    let accepted_history = history_store.clone();
    ui.on_value_accepted(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str.to_string()) else {
            return;
        };

        if let Some(ui) = accepted_ui.upgrade() {
            if let Some((r, g, b)) = parse_color(field, &value.to_string()) {
                push_history(&accepted_history, (r, g, b));
                sync_history_model(&ui, &accepted_history);
                update_ui_colors(&ui, r, g, b);
                persist_config(&ui, &accepted_history);
            } else {
                update_ui_colors(&ui, 0, 0, 0);
            }
        }
    });

    let blurred_ui = ui_handle.clone();
    ui.on_value_blurred(move |type_str, value| {
        let Some(field) = ColorField::from_ui_label(&type_str.to_string()) else {
            return;
        };

        if let Some(ui) = blurred_ui.upgrade() {
            if let Some((r, g, b)) = parse_color(field, &value.to_string()) {
                update_ui_colors(&ui, r, g, b);
            } else {
                update_ui_colors(&ui, 0, 0, 0);
            }
        }
    });

    let ui_close = ui_handle.clone();
    ui.window().on_close_requested(move || {
        let _ = ui_close.upgrade_in_event_loop(|ui| {
            ui.set_close_confirm_open(true);
        });
        slint::CloseRequestResponse::KeepWindowShown
    });

    let close_ui = ui_handle.clone();
    let close_history = history_store.clone();
    ui.on_close_confirm_close(move || {
        if let Some(ui) = close_ui.upgrade() {
            ui.set_close_confirm_open(false);
            persist_config(&ui, &close_history);
        }
        slint::quit_event_loop().ok();
        std::process::exit(0);
    });

    let minimize_ui = ui_handle.clone();
    ui.on_close_confirm_minimize(move || {
        let _ = minimize_ui.upgrade_in_event_loop(|ui| {
            ui.set_close_confirm_open(false);
            ui.window().hide().ok();
        });
    });

    if !start_hidden {
        ui.show()?;
    }

    slint::run_event_loop_until_quit()
}
