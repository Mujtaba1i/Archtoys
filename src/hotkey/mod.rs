//! The global hotkeys: one per toy (pick a color, extract text).
//!
//! Two ways to listen for it:
//! * **Portal** (Wayland): the desktop's GlobalShortcuts portal. Works on
//!   KDE Plasma 6 and recent GNOME. The desktop may ask once to confirm.
//! * **X11**: the `global-hotkey` crate. On Wayland it only sees keys that
//!   the desktop forwards to X11 apps (KDE does this for Ctrl/Alt/Meta combos).
//!
//! On Wayland we try the portal first and fall back to X11 if it is missing
//! or the user declines. `ARCHTOYS_HOTKEY_BACKEND=x11` forces X11.

mod portal;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::thread;

pub const DEFAULT_HOTKEY_TEXT: &str = "Ctrl+Super+C";
pub const DEFAULT_TEXT_HOTKEY_TEXT: &str = "Ctrl+Super+T";
pub const DEFAULT_RULER_HOTKEY_TEXT: &str = "Ctrl+Super+M";

/// One global shortcut: its id (for the desktop), what the desktop shows for
/// it, the keys wanted, the keys used if those can't be, and what it does.
#[derive(Clone)]
pub struct Shortcut {
    pub id: &'static str,
    pub description: &'static str,
    pub text: String,
    pub default: &'static str,
    pub trigger: Trigger,
}

/// Called (on a background thread) whenever the hotkey is pressed.
pub type Trigger = Arc<dyn Fn() + Send + Sync>;

/// Who owns the shortcut right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// The desktop (e.g. KDE) owns them. Change them in the desktop's settings.
    /// The desktop's description of each shortcut, in the order they were given.
    SystemManaged(Vec<String>),
    /// Archtoys owns it (X11 hotkey). Change it in Archtoys.
    AppManaged,
}

/// Called (on a background thread) whenever the status changes.
pub type StatusCallback = Arc<dyn Fn(HotkeyStatus) + Send + Sync>;

pub fn normalize_hotkey_text(input: &str) -> String {
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

pub fn parse_hotkey_text(input: &str) -> Result<(HotKey, String), String> {
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

    if matches!(upper.as_str(), "CTRL" | "CONTROL" | "SHIFT" | "ALT" | "META" | "SUPER" | "CMD" | "COMMAND" | "WIN" | "WINDOWS") {
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
                if matches!(ch, '`' | '\\' | '[' | ']' | ',' | '=' | '-' | '.' | '\'' | ';' | '/') {
                    return Some(ch.to_string());
                }
            }

            if upper.starts_with('F') && upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()).is_some_and(|n| (1..=24).contains(&n)) {
                return Some(upper);
            }

            Some(token.to_string())
        }
    }
}

pub fn build_hotkey_from_capture(key_text: &str, ctrl: bool, alt: bool, shift: bool, meta: bool) -> Result<String, String> {
    if !(ctrl || alt || shift || meta) {
        return Err("hotkey must include at least one modifier (Ctrl, Alt, Shift, Super)".to_string());
    }

    let key =
        normalize_captured_hotkey_key(key_text).ok_or_else(|| "press a non-modifier key together with your modifier(s)".to_string())?;

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

/// "Ctrl+Super+C" -> "CTRL+LOGO+c" (the format the portal's
/// `preferred_trigger` uses: XDG shortcut modifiers + an xkb key name).
pub fn to_portal_trigger(text: &str) -> String {
    normalize_hotkey_text(text)
        .split('+')
        .map(|token| {
            let upper = token.to_ascii_uppercase();
            match upper.as_str() {
                "CTRL" | "CONTROL" => "CTRL".to_string(),
                "ALT" => "ALT".to_string(),
                "SHIFT" => "SHIFT".to_string(),
                "SUPER" | "META" | "LOGO" => "LOGO".to_string(),
                "ESCAPE" => "Escape".to_string(),
                "ENTER" => "Return".to_string(),
                "TAB" => "Tab".to_string(),
                "SPACE" => "space".to_string(),
                "BACKSPACE" => "BackSpace".to_string(),
                "DELETE" => "Delete".to_string(),
                "INSERT" => "Insert".to_string(),
                "HOME" => "Home".to_string(),
                "END" => "End".to_string(),
                "PAGEUP" => "Prior".to_string(),
                "PAGEDOWN" => "Next".to_string(),
                "ARROWUP" => "Up".to_string(),
                "ARROWDOWN" => "Down".to_string(),
                "ARROWLEFT" => "Left".to_string(),
                "ARROWRIGHT" => "Right".to_string(),
                "`" => "grave".to_string(),
                "\\" => "backslash".to_string(),
                "[" => "bracketleft".to_string(),
                "]" => "bracketright".to_string(),
                "," => "comma".to_string(),
                "=" => "equal".to_string(),
                "-" => "minus".to_string(),
                "." => "period".to_string(),
                "'" => "apostrophe".to_string(),
                ";" => "semicolon".to_string(),
                "/" => "slash".to_string(),
                _ if upper.len() > 1 && upper.starts_with('F') && upper[1..].parse::<u8>().is_ok() => upper,
                _ => token.to_ascii_lowercase(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// The X11 hotkeys in use: each one's key combination, its text and what it does.
struct X11Key {
    hotkey: HotKey,
    text: String,
    trigger: Trigger,
}

enum Backend {
    X11 { manager: GlobalHotKeyManager, keys: Arc<Mutex<Vec<X11Key>>> },
    Portal(portal::PortalShortcut),
}

/// Owns whichever backend is in use. Keep it alive for the app's lifetime.
pub struct HotkeyService {
    backend: Option<Backend>,
}

/// The shortcut's keys, or its default if they can't be read.
fn parse_or_default(text: &str, default: &str) -> (HotKey, String) {
    match parse_hotkey_text(text) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("hotkey: {err}, falling back to {default}");
            parse_hotkey_text(default).expect("default hotkey text must parse")
        }
    }
}

impl HotkeyService {
    /// Starts listening for the shortcuts. Returns the service and the text of
    /// each shortcut actually in use (a bad one falls back to its default).
    pub fn start(shortcuts: Vec<Shortcut>, on_status: StatusCallback) -> (Self, Vec<String>) {
        let shortcuts: Vec<Shortcut> = shortcuts
            .into_iter()
            .map(|s| {
                let (_, text) = parse_or_default(&s.text, s.default);
                Shortcut { text, ..s }
            })
            .collect();
        let texts: Vec<String> = shortcuts.iter().map(|s| s.text.clone()).collect();

        let forced_x11 = std::env::var("ARCHTOYS_HOTKEY_BACKEND").map(|v| v.eq_ignore_ascii_case("x11")).unwrap_or(false);
        let wayland = std::env::var("XDG_SESSION_TYPE").map(|v| v.eq_ignore_ascii_case("wayland")).unwrap_or(false);

        if wayland && !forced_x11 {
            // The portal answers asynchronously (the desktop may show a
            // confirmation dialog); it switches to X11 by itself if needed.
            let shortcut = portal::PortalShortcut::start(shortcuts, on_status);
            return (Self { backend: Some(Backend::Portal(shortcut)) }, texts);
        }

        on_status(HotkeyStatus::AppManaged);
        Self::start_x11(shortcuts)
    }

    pub(crate) fn start_x11(shortcuts: Vec<Shortcut>) -> (Self, Vec<String>) {
        let texts: Vec<String> = shortcuts.iter().map(|s| s.text.clone()).collect();
        // Without an X server (Wayland with no XWayland) the X11 library crashes.
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("hotkey: no X11 display, so no hotkeys (use archtoys --pick or --extract from a desktop shortcut)");
            return (Self { backend: None }, texts);
        }
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(err) => {
                eprintln!("hotkey: manager init failed: {err}");
                return (Self { backend: None }, texts);
            }
        };

        // All at once: the library's X11 thread looks for requests only every
        // 50 ms, and each separate register waits for it (a second hotkey made
        // the window appear ~50 ms later).
        let wanted: Vec<(HotKey, String)> = shortcuts.iter().map(|s| parse_or_default(&s.text, s.default)).collect();
        let all: Vec<HotKey> = wanted.iter().map(|(hotkey, _)| *hotkey).collect();
        if manager.register_all(&all).is_ok() {
            let keys: Vec<X11Key> = shortcuts
                .into_iter()
                .zip(wanted)
                .map(|(shortcut, (hotkey, text))| X11Key { hotkey, text, trigger: shortcut.trigger })
                .collect();
            return Self::listen_x11(manager, keys);
        }
        // one is taken (another app has it): register them one by one, with fallbacks
        let mut keys = Vec::new();
        for shortcut in shortcuts {
            let (hotkey, text) = parse_or_default(&shortcut.text, shortcut.default);
            let (hotkey, text) = match manager.register(hotkey) {
                Ok(()) => (hotkey, text),
                Err(err) => {
                    eprintln!("hotkey: register failed for `{text}`: {err}, falling back to {}", shortcut.default);
                    let fallback = parse_hotkey_text(shortcut.default).expect("default hotkey text must parse");
                    if let Err(err) = manager.register(fallback.0) {
                        eprintln!("hotkey: fallback register failed: {err}");
                    }
                    fallback
                }
            };
            keys.push(X11Key { hotkey, text, trigger: shortcut.trigger });
        }
        Self::listen_x11(manager, keys)
    }

    fn listen_x11(manager: GlobalHotKeyManager, keys: Vec<X11Key>) -> (Self, Vec<String>) {
        let texts = keys.iter().map(|k| k.text.clone()).collect();

        let keys = Arc::new(Mutex::new(keys));
        let listen = keys.clone();
        thread::spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                if event.state != HotKeyState::Pressed {
                    continue;
                }
                let trigger = listen.lock().unwrap().iter().find(|k| k.hotkey.id() == event.id).map(|k| k.trigger.clone());
                if let Some(trigger) = trigger {
                    trigger();
                }
            }
        });

        (Self { backend: Some(Backend::X11 { manager, keys }) }, texts)
    }

    /// Switches shortcut `index` to new keys. Returns the text now in use.
    pub fn change(&self, index: usize, text: &str) -> Result<String, String> {
        let (new_hotkey, new_text) = parse_hotkey_text(text)?;
        match &self.backend {
            Some(Backend::X11 { manager, keys }) => {
                let mut keys = keys.lock().unwrap();
                if keys.iter().enumerate().any(|(i, k)| i != index && k.hotkey.id() == new_hotkey.id()) {
                    return Err(format!("`{new_text}` is already the other toy's hotkey"));
                }
                let Some(current) = keys.get_mut(index) else {
                    return Err("no such hotkey".into());
                };
                if current.hotkey.id() == new_hotkey.id() {
                    current.text = new_text.clone();
                    return Ok(new_text);
                }
                let _ = manager.unregister(current.hotkey);
                if let Err(err) = manager.register(new_hotkey) {
                    let _ = manager.register(current.hotkey);
                    return Err(format!("failed to register `{new_text}`: {err}"));
                }
                current.hotkey = new_hotkey;
                current.text = new_text.clone();
                Ok(new_text)
            }
            Some(Backend::Portal(shortcut)) => shortcut.change(index, &new_text),
            None => {
                eprintln!("hotkey: no hotkey backend available, saving only");
                Ok(new_text)
            }
        }
    }
}

impl HotkeyService {
    /// Opens the desktop's shortcut settings (only meaningful when the
    /// desktop manages the shortcuts).
    pub fn open_system_settings(&self) {
        match &self.backend {
            Some(Backend::Portal(shortcut)) => shortcut.open_settings(),
            _ => portal::open_desktop_shortcut_settings(),
        }
    }

    /// Asks the desktop for the current shortcuts again.
    pub fn refresh(&self) {
        if let Some(Backend::Portal(shortcut)) = &self.backend {
            shortcut.refresh();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_trigger_format() {
        assert_eq!(to_portal_trigger("Ctrl+Super+C"), "CTRL+LOGO+c");
        assert_eq!(to_portal_trigger("Alt+Shift+F5"), "ALT+SHIFT+F5");
        assert_eq!(to_portal_trigger("Ctrl+Space"), "CTRL+space");
        assert_eq!(to_portal_trigger("meta+PageUp"), "LOGO+Prior");
        assert_eq!(to_portal_trigger("Ctrl+/"), "CTRL+slash");
    }

    #[test]
    fn capture_builds_normalized_text() {
        assert_eq!(build_hotkey_from_capture("c", true, false, false, true).unwrap(), "Ctrl+Super+C");
        assert!(build_hotkey_from_capture("c", false, false, false, false).is_err());
        assert!(build_hotkey_from_capture("Shift", false, false, true, false).is_err());
    }

    #[test]
    fn default_hotkey_parses() {
        assert!(parse_hotkey_text(DEFAULT_HOTKEY_TEXT).is_ok());
        assert!(parse_hotkey_text(DEFAULT_TEXT_HOTKEY_TEXT).is_ok());
        assert_ne!(parse_hotkey_text(DEFAULT_HOTKEY_TEXT).unwrap().0.id(), parse_hotkey_text(DEFAULT_TEXT_HOTKEY_TEXT).unwrap().0.id());
        assert_eq!(normalize_hotkey_text(" ctrl + win + c "), "ctrl+Super+c");
    }
}
