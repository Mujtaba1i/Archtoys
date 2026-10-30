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
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
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

    if !cfg.history.is_empty() {
        let mut guard = history_store.lock().unwrap();
        guard.clear();
        for rgb in &cfg.history {
            guard.push((rgb[0], rgb[1], rgb[2]));
        }
    }
}

/// Before v0.5.0 the name was a format ("NAME" in the format list); turning
/// it off there means the name stays off.
fn show_name(cfg: &AppConfig) -> bool {
    cfg.show_name
        .unwrap_or_else(|| cfg.formats.iter().find(|f| f.key == "NAME").is_none_or(|f| f.enabled))
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
        let old: AppConfig = serde_json::from_str(r#"{"dark_mode":true,"formats":[{"key":"NAME","enabled":false},{"key":"HEX","enabled":true}]}"#).unwrap();
        assert_eq!(old.theme, "system"); // everyone starts on the desktop's setting
        assert!(!show_name(&old)); // NAME was turned off as a format
        let named: AppConfig = serde_json::from_str(r#"{"formats":[{"key":"NAME","enabled":true}]}"#).unwrap();
        assert!(show_name(&named));
        assert!(show_name(&serde_json::from_str::<AppConfig>("{}").unwrap()));
        assert!(serde_json::from_str::<AppConfig>("{}").unwrap().show_formats_button);
    }
}
