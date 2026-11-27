//! Updates for the Flatpak (v0.9.1): Flathub has the new version, not
//! GitHub. Flatpak's own portal watches the app's remote (Flathub), says when
//! a newer build is there, installs it when asked, and starts the new version.

use super::Event;
use std::collections::HashMap;
use std::sync::OnceLock;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const PORTAL: &str = "org.freedesktop.portal.Flatpak";
const PATH: &str = "/org/freedesktop/portal/Flatpak";
const MONITOR: &str = "org.freedesktop.portal.Flatpak.UpdateMonitor";
/// the portal's Spawn flag: run the newest installed version, not this one
const LATEST_VERSION: u32 = 2;

/// The portal's update monitor, once made.
static WATCH: OnceLock<(Connection, OwnedObjectPath)> = OnceLock::new();

/// Is Flathub being watched (so Update can install through the portal)?
pub fn ready() -> bool {
    WATCH.get().is_some()
}

/// Watches Flathub; `on_update` runs (on that thread) each time a newer build is there.
pub fn watch(on_update: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        if let Err(err) = watch_blocking(on_update) {
            eprintln!("update: Flatpak's update portal isn't there ({err}), so Archtoys can't see Flathub's updates");
        }
    });
}

fn watch_blocking(on_update: impl Fn()) -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let portal = Proxy::new(&conn, PORTAL, PATH, PORTAL).map_err(|e| e.to_string())?;
    let options: HashMap<&str, Value> = HashMap::new();
    let reply = portal.call_method("CreateUpdateMonitor", &(options,)).map_err(|e| e.to_string())?;
    let path: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
    let monitor = Proxy::new(&conn, PORTAL, path.clone(), MONITOR).map_err(|e| e.to_string())?;
    let signals = monitor.receive_signal("UpdateAvailable").map_err(|e| e.to_string())?;
    let _ = WATCH.set((conn.clone(), path));
    eprintln!("update: watching Flathub through Flatpak's portal");
    for _ in signals {
        eprintln!("update: Flathub has a newer build");
        on_update();
    }
    Err("the update portal stopped".into())
}

/// One Progress signal: (status, percent, error message).
/// status: 0 running, 1 nothing to update, 2 done, 3 failed.
pub fn read_progress(info: &HashMap<String, OwnedValue>) -> (u32, u32, String) {
    let num = |k: &str| info.get(k).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    let text = info.get("error_message").and_then(|v| String::try_from(v.clone()).ok()).unwrap_or_default();
    (num("status"), num("progress"), text)
}

/// Installs Flathub's newer build through the portal (the desktop may ask first).
pub fn update(on_event: &dyn Fn(Event)) -> Result<(), String> {
    let (conn, path) = WATCH.get().ok_or("Archtoys isn't connected to Flatpak's update portal")?;
    let monitor = Proxy::new(conn, PORTAL, path.clone(), MONITOR).map_err(|e| e.to_string())?;
    // listen before asking, or a fast answer is lost
    let progress = monitor.receive_signal("Progress").map_err(|e| e.to_string())?;
    let options: HashMap<&str, Value> = HashMap::new();
    monitor.call_method("Update", &("", options)).map_err(|e| format!("the update portal said no: {e}"))?;
    on_event(Event::Step(0));
    for message in progress {
        let (info,): (HashMap<String, OwnedValue>,) = message.body().deserialize().map_err(|e| e.to_string())?;
        match read_progress(&info) {
            (0, percent, _) => on_event(Event::Percent(percent.min(100) as f32 / 100.0)),
            (1, ..) => return Err("Flathub doesn't have the new version yet. Try again later.".into()),
            (2, ..) => {
                eprintln!("update: Flathub's newer build is installed");
                return Ok(());
            }
            (_, _, err) => return Err(if err.is_empty() { "the update failed".into() } else { err }),
        }
    }
    Err("the update portal stopped answering".into())
}

/// Starts the newly installed version, then this one quits.
pub fn restart() -> std::io::Error {
    match spawn_latest() {
        Ok(()) => std::process::exit(0),
        Err(err) => std::io::Error::other(err),
    }
}

fn spawn_latest() -> Result<(), String> {
    let conn = Connection::session().map_err(|e| e.to_string())?;
    let portal = Proxy::new(&conn, PORTAL, PATH, PORTAL).map_err(|e| e.to_string())?;
    let bytes = |s: &str| [s.as_bytes(), b"\0"].concat();
    // a second after this copy quits, so the new one isn't told "already running"
    let argv: Vec<Vec<u8>> = ["sh", "-c", "sleep 1; exec archtoys"].iter().map(|s| bytes(s)).collect();
    let fds: HashMap<u32, zbus::zvariant::Fd> = HashMap::new();
    // our session's variables too (which display the new copy may use is decided by the portal's own environment)
    let envs: HashMap<&str, String> = ["DISPLAY", "WAYLAND_DISPLAY", "XDG_SESSION_TYPE", "XDG_CURRENT_DESKTOP"]
        .into_iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k, v)))
        .collect();
    let options: HashMap<&str, Value> = HashMap::new();
    portal.call_method("Spawn", &(bytes("/"), argv, fds, envs, LATEST_VERSION, options)).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_signals_are_read() {
        let mut info: HashMap<String, OwnedValue> = HashMap::new();
        info.insert("status".into(), OwnedValue::from(0u32));
        info.insert("progress".into(), OwnedValue::from(42u32));
        assert_eq!(read_progress(&info), (0, 42, String::new()));
        info.insert("status".into(), OwnedValue::from(3u32));
        info.insert("error_message".into(), OwnedValue::try_from(Value::from("no space")).unwrap());
        assert_eq!(read_progress(&info), (3, 42, "no space".into()));
        assert_eq!(read_progress(&HashMap::new()), (0, 0, String::new()));
    }
}
