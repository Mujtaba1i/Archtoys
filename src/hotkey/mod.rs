//! The global hotkey.
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
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

pub const DEFAULT_HOTKEY_TEXT: &str = "Ctrl+Super+C";

/// Called (on a background thread) whenever the hotkey is pressed.
pub type Trigger = Arc<dyn Fn() + Send + Sync>;

/// Who owns the shortcut right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotkeyStatus {
    /// The desktop (e.g. KDE) owns it. Change it in the desktop's settings.
    /// Contains the desktop's description of the current shortcut.
    SystemManaged(String),
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

    if matches!(
        upper.as_str(),
        "CTRL" | "CONTROL" | "SHIFT" | "ALT" | "META" | "SUPER" | "CMD" | "COMMAND" | "WIN"
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

pub fn build_hotkey_from_capture(
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

enum Backend {
    X11 {
        manager: GlobalHotKeyManager,
        active: Mutex<(HotKey, String)>,
        active_id: Arc<AtomicU32>,
    },
    Portal(portal::PortalShortcut),
}

/// Owns whichever backend is in use. Keep it alive for the app's lifetime.
pub struct HotkeyService {
    backend: Option<Backend>,
}

impl HotkeyService {
    /// Starts listening for `text`. Returns the service and the hotkey text
    /// that is actually in use (it falls back to the default if `text` is bad).
    pub fn start(text: &str, trigger: Trigger, on_status: StatusCallback) -> (Self, String) {
        let (wanted_hotkey, wanted_text) = match parse_hotkey_text(text) {
            Ok(parsed) => parsed,
            Err(err) => {
                eprintln!("hotkey: {err}; falling back to {DEFAULT_HOTKEY_TEXT}");
                parse_hotkey_text(DEFAULT_HOTKEY_TEXT).expect("default hotkey text must parse")
            }
        };

        let forced_x11 = std::env::var("ARCHTOYS_HOTKEY_BACKEND")
            .map(|v| v.eq_ignore_ascii_case("x11"))
            .unwrap_or(false);
        let wayland = std::env::var("XDG_SESSION_TYPE")
            .map(|v| v.eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);

        if wayland && !forced_x11 {
            // The portal answers asynchronously (the desktop may show a
            // confirmation dialog); it switches to X11 by itself if needed.
            let shortcut = portal::PortalShortcut::start(&wanted_text, trigger, on_status);
            return (
                Self {
                    backend: Some(Backend::Portal(shortcut)),
                },
                wanted_text,
            );
        }

        let _ = wanted_hotkey;
        on_status(HotkeyStatus::AppManaged);
        Self::start_x11(&wanted_text, trigger)
    }

    pub(crate) fn start_x11(text: &str, trigger: Trigger) -> (Self, String) {
        let manager = match GlobalHotKeyManager::new() {
            Ok(manager) => manager,
            Err(err) => {
                eprintln!("hotkey: manager init failed: {err}");
                return (Self { backend: None }, text.to_string());
            }
        };

        let (hotkey, hotkey_text) = match parse_hotkey_text(text) {
            Ok(parsed) => parsed,
            Err(_) => parse_hotkey_text(DEFAULT_HOTKEY_TEXT).expect("default hotkey text must parse"),
        };

        let (hotkey, hotkey_text) = match manager.register(hotkey) {
            Ok(()) => (hotkey, hotkey_text),
            Err(err) => {
                eprintln!("hotkey: register failed for `{hotkey_text}`: {err}; falling back to {DEFAULT_HOTKEY_TEXT}");
                let fallback = parse_hotkey_text(DEFAULT_HOTKEY_TEXT).expect("default hotkey text must parse");
                if let Err(err) = manager.register(fallback.0) {
                    eprintln!("hotkey: fallback register failed: {err}");
                }
                fallback
            }
        };

        let active_id = Arc::new(AtomicU32::new(hotkey.id()));
        let listen_id = active_id.clone();
        thread::spawn(move || {
            let receiver = GlobalHotKeyEvent::receiver();
            while let Ok(event) = receiver.recv() {
                if event.state == HotKeyState::Pressed
                    && event.id == listen_id.load(Ordering::SeqCst)
                {
                    trigger();
                }
            }
        });

        (
            Self {
                backend: Some(Backend::X11 {
                    manager,
                    active: Mutex::new((hotkey, hotkey_text.clone())),
                    active_id,
                }),
            },
            hotkey_text,
        )
    }

    /// Switches to a new hotkey. Returns the text now in use.
    pub fn change(&self, text: &str) -> Result<String, String> {
        let (new_hotkey, new_text) = parse_hotkey_text(text)?;
        match &self.backend {
            Some(Backend::X11 {
                manager,
                active,
                active_id,
            }) => {
                let mut current = active.lock().unwrap();
                if current.0.id() == new_hotkey.id() {
                    current.1 = new_text.clone();
                    return Ok(new_text);
                }
                let _ = manager.unregister(current.0);
                if let Err(err) = manager.register(new_hotkey) {
                    let _ = manager.register(current.0);
                    return Err(format!("failed to register `{new_text}`: {err}"));
                }
                *current = (new_hotkey, new_text.clone());
                active_id.store(new_hotkey.id(), Ordering::SeqCst);
                Ok(new_text)
            }
            Some(Backend::Portal(shortcut)) => shortcut.change(&new_text),
            None => {
                eprintln!("hotkey: no hotkey backend available; saving only");
                Ok(new_text)
            }
        }
    }
}

impl HotkeyService {
    /// Opens the desktop's shortcut settings (only meaningful when the
    /// desktop manages the shortcut).
    pub fn open_system_settings(&self) {
        match &self.backend {
            Some(Backend::Portal(shortcut)) => shortcut.open_settings(),
            _ => portal::open_desktop_shortcut_settings(),
        }
    }

    /// Asks the desktop for the current shortcut again.
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
        assert_eq!(normalize_hotkey_text(" ctrl + win + c "), "ctrl+Super+c");
    }
}
