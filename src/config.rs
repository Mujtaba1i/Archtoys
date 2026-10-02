//! Settings file (~/.config/archtoys-color-picker/config.json) and the
//! autostart entry (~/.config/autostart/archtoys.desktop).

use crate::color::Rgb;
use crate::formats::{FormatEntry, FormatList};
use crate::hotkey::DEFAULT_HOTKEY_TEXT;
use crate::{AppWindow, Skin};
use serde::{Deserialize, Serialize};
use slint::ComponentHandle;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub type HistoryStore = Arc<Mutex<Vec<Rgb>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub dark_mode: bool,
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
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            dark_mode: false,
            setting_minimize: false,
            setting_minimize_tray: true,
            setting_autocopy: false,
            setting_autostart: false,
            setting_hotkey: DEFAULT_HOTKEY_TEXT.to_string(),
            history: vec![],
            formats: vec![],
            last_color: None,
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
            if let Err(err) = fs::write(path, data) {
                eprintln!("config: write failed: {err:?}");
            }
        }
        Err(err) => eprintln!("config: serialize failed: {err:?}"),
    }
}

fn snapshot_config(ui: &AppWindow, history_store: &HistoryStore) -> AppConfig {
    let skin = ui.global::<Skin>();
    let history = {
        let guard = history_store.lock().unwrap();
        guard.iter().map(|(r, g, b)| [*r, *g, *b]).collect()
    };
    AppConfig {
        dark_mode: skin.get_dark_mode(),
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
    }
}

pub fn apply_config(ui: &AppWindow, history_store: &HistoryStore, cfg: &AppConfig) {
    let skin = ui.global::<Skin>();
    skin.set_dark_mode(cfg.dark_mode);
    ui.set_setting_minimize(cfg.setting_minimize);
    ui.set_setting_minimize_tray(cfg.setting_minimize_tray);
    ui.set_setting_autocopy(cfg.setting_autocopy);
    ui.set_setting_autostart(cfg.setting_autostart);
    ui.set_setting_hotkey(cfg.setting_hotkey.clone().into());
    crate::ui_state::set_formats(ui, FormatList::from_saved(&cfg.formats));

    if !cfg.history.is_empty() {
        let mut guard = history_store.lock().unwrap();
        guard.clear();
        for rgb in &cfg.history {
            guard.push((rgb[0], rgb[1], rgb[2]));
        }
    }
}

pub fn persist_config(ui: &AppWindow, history_store: &HistoryStore) {
    save_config(&snapshot_config(ui, history_store));
}
