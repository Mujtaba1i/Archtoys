//! Running as a Flatpak (v0.9.1). Inside the sandbox `$XDG_RUNTIME_DIR` is
//! private to each run, and `$XDG_DATA_HOME` / `$XDG_CONFIG_HOME` point into
//! `~/.var/app/<id>`. What must be shared (the one-copy lock, KWin's scripts)
//! or must reach the real desktop (the file managers' menus, autostart) goes
//! where both sides see it.

use std::path::PathBuf;

/// The Flatpak's app id, when running as one.
pub fn flatpak_id() -> Option<String> {
    std::env::var("FLATPAK_ID").ok().filter(|id| !id.is_empty())
}

/// `$XDG_RUNTIME_DIR/app/<id>`: shared by every run of this Flatpak and
/// seen at the same path outside (KWin reads its scripts from there).
pub fn shared_runtime_dir() -> Option<PathBuf> {
    let id = flatpak_id()?;
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join("app").join(id);
    dir.is_dir().then_some(dir)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// The real ~/.local/share (in a Flatpak too: Flatpak keeps the desktop's own as HOST_XDG_DATA_HOME).
pub fn user_data_dir() -> PathBuf {
    let key = if flatpak_id().is_some() { "HOST_XDG_DATA_HOME" } else { "XDG_DATA_HOME" };
    std::env::var_os(key).filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

/// The real ~/.config.
pub fn user_config_dir() -> PathBuf {
    let key = if flatpak_id().is_some() { "HOST_XDG_CONFIG_HOME" } else { "XDG_CONFIG_HOME" };
    std::env::var_os(key).filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

/// The words that start Archtoys from outside: `flatpak run ... <id>` for a
/// Flatpak, the AppImage itself (its path stays when it updates), else this binary.
pub fn launch_command() -> Vec<String> {
    if let Some(id) = flatpak_id() {
        return vec!["flatpak".into(), "run".into(), "--command=archtoys".into(), id];
    }
    let program = std::env::var("APPIMAGE")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_else(|| "archtoys".into());
    vec![program]
}

/// Start at login, through the Background portal (a Flatpak can't write ~/.config/autostart).
pub fn request_autostart(enabled: bool) {
    std::thread::spawn(move || {
        if let Err(err) = request_background(enabled) {
            eprintln!("autostart: the desktop's Background portal said no ({err})");
        }
    });
}

fn request_background(enabled: bool) -> Result<(), String> {
    use zbus::zvariant::Value;
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let token = crate::portal::new_token("archtoys_autostart");
    let mut options = crate::portal::options(&token);
    options.insert("reason", Value::from("Start Archtoys in the tray when you log in"));
    options.insert("autostart", Value::from(enabled));
    options.insert("commandline", Value::from(vec!["archtoys", "--minimized"]));
    match crate::portal::call(&conn, "org.freedesktop.portal.Background", "RequestBackground", &("", options), &token)? {
        crate::portal::Outcome::Success(_) => Ok(()),
        crate::portal::Outcome::Cancelled => Err("cancelled".into()),
    }
}
