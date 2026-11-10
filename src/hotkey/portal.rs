//! Global hotkey through the desktop's GlobalShortcuts portal (Wayland).
//!
//! The desktop owns the shortcut: it asks the user once to confirm it, stores
//! it, and the user changes it in the desktop's settings (on KDE: System
//! Settings → Keyboard → Shortcuts). We show what the desktop reports and
//! update when it changes. If the portal is missing or the user declines,
//! we switch to the X11 hotkey, which Archtoys manages itself.

use super::{to_portal_trigger, HotkeyService, HotkeyStatus, Shortcut, StatusCallback};
use crate::portal::{self, Outcome};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";

type ShortcutList = Vec<(String, HashMap<String, OwnedValue>)>;

enum Command {
    OpenSettings,
    Refresh,
}

pub struct PortalShortcut {
    commands: Mutex<Sender<Command>>,
    /// Set if we had to switch to the X11 hotkey.
    fallback: Arc<Mutex<Option<HotkeyService>>>,
}

impl PortalShortcut {
    pub fn start(shortcuts: Vec<Shortcut>, on_status: StatusCallback) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let fallback: Arc<Mutex<Option<HotkeyService>>> = Arc::new(Mutex::new(None));

        let thread_fallback = fallback.clone();
        thread::spawn(move || {
            if let Err(err) = run(&shortcuts, on_status.clone(), rx) {
                // only the hotkeys fall back; the window stays on Wayland
                eprintln!(
                    "hotkey: {err}; using X11 hotkeys instead (the window stays on Wayland; \
                     X11 hotkeys only work while an X11 app has the keyboard)"
                );
                let (service, _) = HotkeyService::start_x11(shortcuts);
                *thread_fallback.lock().unwrap() = Some(service);
                on_status(HotkeyStatus::AppManaged);
            }
        });

        Self { commands: Mutex::new(tx), fallback }
    }

    /// Changing a hotkey from inside Archtoys only works with the X11 fallback.
    pub fn change(&self, index: usize, text: &str) -> Result<String, String> {
        if let Some(service) = self.fallback.lock().unwrap().as_ref() {
            return service.change(index, text);
        }
        Err("this shortcut is managed by your desktop; change it in its settings".into())
    }

    pub fn open_settings(&self) {
        let _ = self.commands.lock().unwrap().send(Command::OpenSettings);
    }

    pub fn refresh(&self) {
        let _ = self.commands.lock().unwrap().send(Command::Refresh);
    }
}

/// The description of each of our shortcuts in a portal shortcut list, e.g.
/// "Ctrl+Alt+F" ("" for one the desktop doesn't list).
fn describe(list: &ShortcutList, ids: &[&str]) -> Vec<String> {
    ids.iter()
        .map(|wanted| {
            list.iter()
                .find(|(id, _)| id == wanted)
                .and_then(|(_, props)| props.get("trigger_description"))
                .and_then(|value| <String>::try_from(value.try_clone().ok()?).ok())
                .unwrap_or_default()
        })
        .collect()
}

fn list_from_results(results: &portal::Results) -> ShortcutList {
    results
        .get("shortcuts")
        .and_then(|value| value.try_clone().ok())
        .and_then(|value| ShortcutList::try_from(value).ok())
        .unwrap_or_default()
}

/// Creates a portal session and binds every shortcut in it (one dialog).
/// Returns the session path and the desktop's description of each shortcut.
fn bind(conn: &Connection, shortcuts: &[Shortcut]) -> Result<(String, Vec<String>), String> {
    // CreateSession
    let token = portal::new_token("archtoys_hk");
    let session_token = portal::new_token("archtoys_session");
    let mut options = portal::options(&token);
    options.insert("session_handle_token", Value::from(session_token));
    let session = match portal::call(conn, INTERFACE, "CreateSession", &(options,), &token)? {
        Outcome::Success(results) => {
            portal::result_string(&results, "session_handle").ok_or("GlobalShortcuts: no session_handle in reply")?
        }
        Outcome::Cancelled => return Err("GlobalShortcuts: session was refused".into()),
    };
    let session_path = OwnedObjectPath::try_from(session.clone()).map_err(|err| format!("GlobalShortcuts: bad session path: {err}"))?;

    // BindShortcuts. `preferred_trigger` is only a suggestion: the first time,
    // the desktop asks the user; after that it keeps its own setting.
    let ids: Vec<&str> = shortcuts.iter().map(|s| s.id).collect();
    let list: Vec<(&str, HashMap<&str, Value<'_>>)> = shortcuts
        .iter()
        .map(|s| {
            let mut props: HashMap<&str, Value<'_>> = HashMap::new();
            props.insert("description", Value::from(s.description));
            props.insert("preferred_trigger", Value::from(to_portal_trigger(&s.text)));
            (s.id, props)
        })
        .collect();
    let token = portal::new_token("archtoys_bind");
    let options = portal::options(&token);
    let descriptions = match portal::call(conn, INTERFACE, "BindShortcuts", &(session_path, list, "", options), &token)? {
        Outcome::Success(results) => describe(&list_from_results(&results), &ids),
        Outcome::Cancelled => return Err("GlobalShortcuts: you declined the shortcuts".into()),
    };
    for (s, d) in shortcuts.iter().zip(&descriptions) {
        eprintln!("hotkey: desktop shortcut for {} is \"{d}\" (suggested {})", s.id, to_portal_trigger(&s.text));
    }
    Ok((session, descriptions))
}

fn list_shortcuts(conn: &Connection, session: &str, ids: &[&str]) -> Result<Vec<String>, String> {
    let session_path = OwnedObjectPath::try_from(session.to_string()).map_err(|err| format!("bad session path: {err}"))?;
    let token = portal::new_token("archtoys_list");
    let options = portal::options(&token);
    match portal::call(conn, INTERFACE, "ListShortcuts", &(session_path, options), &token)? {
        Outcome::Success(results) => Ok(describe(&list_from_results(&results), ids)),
        Outcome::Cancelled => Err("ListShortcuts was cancelled".into()),
    }
}

/// Opens the desktop's own keyboard-shortcut settings.
pub(super) fn open_desktop_shortcut_settings() {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
    let attempts: &[&[&str]] = if desktop.contains("KDE") {
        &[&["systemsettings", "kcm_keys"], &["systemsettings5", "keys"]]
    } else if desktop.contains("GNOME") {
        &[&["gnome-control-center", "keyboard"]]
    } else {
        &[&["systemsettings", "kcm_keys"], &["gnome-control-center", "keyboard"]]
    };
    for cmd in attempts {
        let mut command = crate::launch::outside_command(cmd[0]);
        command.args(&cmd[1..]);
        if crate::launch::spawn_and_reap(command).is_ok() {
            return;
        }
    }
    eprintln!("hotkey: couldn't open the desktop's shortcut settings");
}

fn run(shortcuts: &[Shortcut], on_status: StatusCallback, commands: mpsc::Receiver<Command>) -> Result<(), String> {
    let ids: Vec<&'static str> = shortcuts.iter().map(|s| s.id).collect();
    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    portal::register_host_app(&conn, "archtoys");

    let probe = Proxy::new(&conn, portal::DESTINATION, portal::PATH, INTERFACE).map_err(|err| format!("GlobalShortcuts proxy: {err}"))?;
    let version: u32 = probe.get_property("version").map_err(|_| "this desktop has no GlobalShortcuts portal".to_string())?;
    eprintln!("hotkey: GlobalShortcuts portal version {version}");

    // Listen before binding, so nothing is missed.
    let activated = probe.receive_signal("Activated").map_err(|err| format!("GlobalShortcuts subscribe: {err}"))?;
    let changed = probe.receive_signal("ShortcutsChanged").map_err(|err| format!("GlobalShortcuts subscribe: {err}"))?;

    let (session, descriptions) = bind(&conn, shortcuts)?;
    on_status(HotkeyStatus::SystemManaged(descriptions));

    // The user changed the shortcut in the desktop's settings.
    {
        let session = session.clone();
        let on_status = on_status.clone();
        let ids = ids.clone();
        thread::spawn(move || {
            for message in changed {
                if let Ok((path, list)) = message.body().deserialize::<(OwnedObjectPath, ShortcutList)>() {
                    if path.as_str() == session {
                        on_status(HotkeyStatus::SystemManaged(describe(&list, &ids)));
                    }
                }
            }
        });
    }

    // Requests from the UI.
    {
        let conn = conn.clone();
        let session = session.clone();
        let on_status = on_status.clone();
        let ids = ids.clone();
        thread::spawn(move || {
            while let Ok(command) = commands.recv() {
                match command {
                    Command::Refresh => match list_shortcuts(&conn, &session, &ids) {
                        Ok(descriptions) => on_status(HotkeyStatus::SystemManaged(descriptions)),
                        Err(err) => eprintln!("hotkey: refresh failed: {err}"),
                    },
                    Command::OpenSettings => {
                        // Portal version 2 can open the right settings page itself.
                        let opened = version >= 2
                            && OwnedObjectPath::try_from(session.clone()).ok().is_some_and(|path| {
                                let options: HashMap<&str, Value<'_>> = HashMap::new();
                                Proxy::new(&conn, portal::DESTINATION, portal::PATH, INTERFACE)
                                    .and_then(|p| p.call_method("ConfigureShortcuts", &(path, "", options)))
                                    .is_ok()
                            });
                        if !opened {
                            open_desktop_shortcut_settings();
                        }
                    }
                }
            }
        });
    }

    for message in activated {
        let Ok((path, shortcut_id, _timestamp, _options)) =
            message.body().deserialize::<(OwnedObjectPath, String, u64, HashMap<String, OwnedValue>)>()
        else {
            continue;
        };
        if path.as_str() != session {
            continue;
        }
        if let Some(shortcut) = shortcuts.iter().find(|s| s.id == shortcut_id) {
            (shortcut.trigger)();
        }
    }
    Ok(())
}
