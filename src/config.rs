//! Settings file (~/.config/archtoys-color-picker/config.json) and the
//! autostart entry (~/.config/autostart/archtoys.desktop).

use crate::color::Rgb;
use crate::formats::{FormatEntry, FormatList};
use crate::hotkey::DEFAULT_HOTKEY_TEXT;
use crate::{AppWindow, Skin};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type HistoryStore = Arc<Mutex<Vec<Rgb>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// "light", "dark" or "system". The old `dark_mode` switch is not carried
    /// over: everyone starts on the desktop's own setting.
    pub theme: String,
    /// The color card as a gradient instead of the solid color.
    pub card_gradient: bool,
    /// Show color names (card and shades). None in configs from before v0.5.0,
    /// which had NAME as a format instead: then that format decides.
    pub show_name: Option<bool>,
    /// The sliders button (Customize formats) in the header.
    #[serde(default = "default_true")]
    pub show_formats_button: bool,
    pub setting_minimize: bool,
    /// Minimize to system tray when the window's [X] close button is clicked.
    /// Defaults to `true` so the app lives in the tray by default.
    #[serde(default = "default_true")]
    pub setting_minimize_tray: bool,
    pub setting_autocopy: bool,
    pub setting_autostart: bool,
    pub setting_hotkey: String,
    pub history: Vec<[u8; 3]>,
    /// Shown formats and their order (empty = defaults).
    pub formats: Vec<FormatEntry>,
    /// The color shown when the app was last used, restored at startup.
    pub last_color: Option<[u8; 3]>,
    /// The window's size (logical pixels) when the app was last used.
    pub window_size: Option<[u32; 2]>,
    /// The color editor's Compare button.
    pub editor_compare: bool,
    /// Cancel forgets the edit (off: it keeps it, like clicking outside).
    #[serde(default = "default_true")]
    pub editor_cancel_forgets: bool,
    /// Keep the edit after a restart (then `editor_saved` holds it).
    pub editor_keep: bool,
    /// A new selected color starts the edit fresh.
    #[serde(default = "default_true")]
    pub editor_reset_on_new_color: bool,
    pub editor_saved: Option<crate::editor::SavedEdit>,
    /// Where the main window was (KDE Wayland, from KWin): x, y, width, height.
    pub window_geometry: Option<[i32; 4]>,
    /// The picker used last ("freeze frame (Wayland)"…), for Report a problem.
    pub last_picker: Option<String>,
    /// The page shown last (0 Home, 1 Color Picker, 2 Text Extractor). None in
    /// configs from before v0.7.0: the first start of v0.7.0 opens Home.
    pub last_page: Option<i32>,
    /// The sidebar kept open: 0 no, 1 its icons (a click on the menu button),
    /// 2 all of it (a double-click).
    pub sidebar_mode: i32,
    /// Text Extractor was turned on (Tesseract installed by Archtoys).
    pub text_enabled: bool,
    pub text_hotkey: String,
    /// Show the toolbar on the selection (off: copy and close at once).
    #[serde(default = "default_true")]
    pub text_tools: bool,
    /// Minimize the window for a capture and keep it minimized afterwards.
    pub text_stay_minimized: bool,
    /// Keep line breaks (off: join the lines of a paragraph).
    #[serde(default = "default_true")]
    pub text_keep_lines: bool,
    /// The Screen Ruler (v0.7.4): its hotkey, the mode it opens in (0: the
    /// last one used, else 1 Bounds to 4 Vertical), the last one used (0
    /// Bounds to 3 Vertical), the tolerance, − and + buttons for it instead
    /// of the scroll wheel, and the units (0 px, 1 cm, 2 in).
    #[serde(default = "default_ruler_hotkey")]
    pub ruler_hotkey: String,
    pub ruler_start: i32,
    #[serde(default = "default_ruler_mode")]
    pub ruler_mode: i32,
    #[serde(default = "default_ruler_tolerance")]
    pub ruler_tolerance: i32,
    pub ruler_buttons: bool,
    pub ruler_units: i32,
    /// a measure may go on past its screen's edge (only offered with several screens)
    pub ruler_cross: bool,
    /// the font finder may download fonts: None until the first-time card was answered (v0.7.5)
    pub font_online: Option<bool>,
    /// fewer moving things (the font search shows its best guess instead of every font)
    pub reduce_motion: bool,
}

fn default_ruler_hotkey() -> String {
    crate::hotkey::DEFAULT_RULER_HOTKEY_TEXT.to_string()
}

fn default_ruler_mode() -> i32 {
    1 // Spacing
}

fn default_ruler_tolerance() -> i32 {
    30
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            theme: "system".to_string(),
            card_gradient: false,
            show_name: None,
            show_formats_button: true,
            setting_minimize: false,
            setting_minimize_tray: true,
            setting_autocopy: false,
            setting_autostart: false,
            setting_hotkey: DEFAULT_HOTKEY_TEXT.to_string(),
            history: vec![],
            formats: vec![],
            last_color: None,
            window_size: None,
            editor_compare: false,
            editor_cancel_forgets: true,
            editor_keep: false,
            editor_reset_on_new_color: true,
            editor_saved: None,
            window_geometry: None,
            last_picker: None,
            last_page: None,
            sidebar_mode: 0,
            text_enabled: false,
            text_hotkey: crate::hotkey::DEFAULT_TEXT_HOTKEY_TEXT.to_string(),
            text_tools: true,
            text_stay_minimized: false,
            text_keep_lines: true,
            ruler_hotkey: default_ruler_hotkey(),
            ruler_start: 0,
            ruler_mode: default_ruler_mode(),
            ruler_tolerance: default_ruler_tolerance(),
            ruler_buttons: false,
            ruler_units: 0,
            ruler_cross: false,
            font_online: None,
            reduce_motion: false,
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
    config_base_dir().join("archtoys-color-picker").join("config.json")
}

fn autostart_path() -> PathBuf {
    config_base_dir().join("autostart").join("archtoys.desktop")
}

fn autostart_entry_contents() -> &'static str {
    "[Desktop Entry]\nType=Application\nName=Archtoys\nComment=System-wide color picker\nExec=archtoys --minimized\nIcon=archtoys\nTerminal=false\nStartupNotify=false\nStartupWMClass=archtoys\nCategories=Graphics;Utility;\nX-GNOME-Autostart-enabled=true\n"
}

pub fn sync_autostart_entry(enabled: bool) {
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

pub fn load_config() -> Option<AppConfig> {
    let data = fs::read_to_string(config_path()).ok()?;
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
            if let Err(err) = write_atomic(&path, data.as_bytes()) {
                eprintln!("config: write failed: {err:?}");
            }
        }
        Err(err) => eprintln!("config: serialize failed: {err:?}"),
    }
}

/// Writes a temporary file next to `path`, then renames it over `path`, so a
/// crash or power cut mid-save leaves either the old file or the new one.
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(&tmp, path)
}

fn snapshot_config(ui: &AppWindow, history_store: &HistoryStore) -> AppConfig {
    let skin = ui.global::<Skin>();
    let history = {
        let guard = history_store.lock().unwrap();
        guard.iter().map(|(r, g, b)| [*r, *g, *b]).collect()
    };
    AppConfig {
        theme: ["light", "dark", "system"][skin.get_theme().clamp(0, 2) as usize].to_string(),
        card_gradient: ui.get_setting_card_gradient(),
        show_name: Some(ui.get_setting_show_name()),
        show_formats_button: ui.get_setting_formats_button(),
        setting_minimize: ui.get_setting_minimize(),
        setting_minimize_tray: ui.get_setting_minimize_tray(),
        setting_autocopy: ui.get_setting_autocopy(),
        setting_autostart: ui.get_setting_autostart(),
        setting_hotkey: ui.get_setting_hotkey().to_string(),
        history,
        formats: crate::ui_state::formats().to_saved(),
        last_color: {
            let (r, g, b) = crate::ui_state::committed_rgb();
            Some([r, g, b])
        },
        window_size: crate::ui_state::window_size(ui).map(|(w, h)| [w, h]),
        editor_compare: ui.get_setting_editor_compare(),
        editor_cancel_forgets: ui.get_setting_editor_cancel_forgets(),
        editor_keep: ui.get_setting_editor_keep(),
        editor_reset_on_new_color: ui.get_setting_editor_reset(),
        editor_saved: crate::editor::saved(ui),
        window_geometry: crate::kwin::geometry(),
        last_picker: crate::diagnostics::picker_to_save(),
        last_page: Some(ui.get_page()),
        sidebar_mode: ui.get_sidebar_mode(),
        text_enabled: crate::ocr::enabled(),
        text_hotkey: ui.get_setting_text_hotkey().to_string(),
        text_tools: ui.get_setting_text_tools(),
        text_stay_minimized: ui.get_setting_text_stay_minimized(),
        text_keep_lines: ui.get_setting_text_keep_lines(),
        ruler_hotkey: ui.get_setting_ruler_hotkey().to_string(),
        ruler_start: ui.get_setting_ruler_start(),
        ruler_mode: ui.get_setting_ruler_mode(),
        ruler_tolerance: ui.get_setting_ruler_tolerance(),
        ruler_buttons: ui.get_setting_ruler_buttons(),
        ruler_units: ui.get_setting_ruler_units(),
        ruler_cross: ui.get_setting_ruler_cross(),
        font_online: {
            let g = ui.global::<crate::FontFinder>();
            g.get_asked().then(|| g.get_online())
        },
        reduce_motion: ui.get_setting_reduce_motion(),
    }
}

pub fn apply_config(ui: &AppWindow, history_store: &HistoryStore, cfg: &AppConfig) {
    let skin = ui.global::<Skin>();
    skin.set_theme(match cfg.theme.as_str() {
        "light" => 0,
        "dark" => 1,
        _ => 2,
    });
    ui.set_setting_card_gradient(cfg.card_gradient);
    ui.set_setting_show_name(show_name(cfg));
    ui.set_setting_formats_button(cfg.show_formats_button);
    ui.set_setting_minimize(cfg.setting_minimize);
    ui.set_setting_minimize_tray(cfg.setting_minimize_tray);
    ui.set_setting_autocopy(cfg.setting_autocopy);
    ui.set_setting_autostart(cfg.setting_autostart);
    ui.set_setting_hotkey(cfg.setting_hotkey.clone().into());
    crate::ui_state::set_formats(ui, FormatList::from_saved(&cfg.formats));
    crate::ui_state::set_window_size(cfg.window_size);
    ui.set_setting_editor_compare(cfg.editor_compare);
    ui.set_setting_editor_cancel_forgets(cfg.editor_cancel_forgets);
    ui.set_setting_editor_keep(cfg.editor_keep);
    ui.set_setting_editor_reset(cfg.editor_reset_on_new_color);
    if let (true, Some(saved)) = (cfg.editor_keep, &cfg.editor_saved) {
        crate::editor::restore(saved);
    }

    crate::kwin::set_geometry(cfg.window_geometry);
    crate::diagnostics::set_last_picker(cfg.last_picker.clone());
    ui.set_page(cfg.last_page.unwrap_or(0).clamp(0, 3));
    ui.set_sidebar_mode(cfg.sidebar_mode.clamp(0, 2));
    crate::ocr::set_enabled(cfg.text_enabled);
    if !cfg.text_hotkey.trim().is_empty() {
        ui.set_setting_text_hotkey(cfg.text_hotkey.clone().into());
    }
    ui.set_setting_text_tools(cfg.text_tools);
    ui.set_setting_text_stay_minimized(cfg.text_stay_minimized);
    ui.set_setting_text_keep_lines(cfg.text_keep_lines);
    if !cfg.ruler_hotkey.trim().is_empty() {
        ui.set_setting_ruler_hotkey(cfg.ruler_hotkey.clone().into());
    }
    ui.set_setting_ruler_start(cfg.ruler_start.clamp(0, 4));
    ui.set_setting_ruler_mode(cfg.ruler_mode.clamp(0, 3));
    ui.set_setting_ruler_tolerance(cfg.ruler_tolerance.clamp(0, 255));
    ui.set_setting_ruler_buttons(cfg.ruler_buttons);
    ui.set_setting_ruler_units(cfg.ruler_units.clamp(0, 2));
    ui.set_setting_ruler_cross(cfg.ruler_cross);
    if let Some(online) = cfg.font_online {
        ui.global::<crate::FontFinder>().set_online(online);
        ui.global::<crate::FontFinder>().set_asked(true);
    }
    ui.set_setting_reduce_motion(cfg.reduce_motion);

    if !cfg.history.is_empty() {
        let mut guard = history_store.lock().unwrap();
        guard.clear();
        // the last 500 (older versions kept every color)
        for rgb in cfg.history.iter().take(crate::ui_state::MAX_COLOR_HISTORY) {
            guard.push((rgb[0], rgb[1], rgb[2]));
        }
    }
}

/// Before v0.5.0 the name was a format ("NAME" in the format list); turning
/// it off there means the name stays off.
fn show_name(cfg: &AppConfig) -> bool {
    cfg.show_name.unwrap_or_else(|| cfg.formats.iter().find(|f| f.key == "NAME").is_none_or(|f| f.enabled))
}

pub fn persist_config(ui: &AppWindow, history_store: &HistoryStore) {
    save_config(&snapshot_config(ui, history_store));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("archtoys-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, "old").unwrap();
        // A save that died after writing the temp file must not touch the real one.
        fs::write(path.with_extension("json.tmp"), "half-writ").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");

        write_atomic(&path, b"{\"setting_autocopy\":true}").unwrap();
        assert!(serde_json::from_str::<AppConfig>(&fs::read_to_string(&path).unwrap()).unwrap().setting_autocopy);
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn configs_from_before_v0_5_0_keep_their_choices() {
        let old: AppConfig =
            serde_json::from_str(r#"{"dark_mode":true,"formats":[{"key":"NAME","enabled":false},{"key":"HEX","enabled":true}]}"#).unwrap();
        assert_eq!(old.theme, "system"); // everyone starts on the desktop's setting
        assert!(!show_name(&old)); // NAME was turned off as a format
        let named: AppConfig = serde_json::from_str(r#"{"formats":[{"key":"NAME","enabled":true}]}"#).unwrap();
        assert!(show_name(&named));
        assert!(show_name(&serde_json::from_str::<AppConfig>("{}").unwrap()));
        assert!(serde_json::from_str::<AppConfig>("{}").unwrap().show_formats_button);
    }

    #[test]
    fn configs_from_before_v0_7_0_open_home_with_text_extractor_off() {
        let old: AppConfig = serde_json::from_str(r#"{"setting_autocopy":true}"#).unwrap();
        assert_eq!(old.last_page, None); // the first start of v0.7.0 opens Home
        assert!(old.sidebar_mode == 0 && !old.text_enabled && !old.text_stay_minimized);
        assert!(old.text_tools && old.text_keep_lines);
        assert_eq!(old.text_hotkey, "Ctrl+Super+T");
    }

    #[test]
    fn configs_from_before_the_color_editor_get_its_defaults() {
        let old: AppConfig = serde_json::from_str(r#"{"setting_autocopy":true}"#).unwrap();
        assert!(!old.editor_compare && !old.editor_keep && old.editor_saved.is_none());
        assert!(old.editor_cancel_forgets && old.editor_reset_on_new_color);
    }
}
