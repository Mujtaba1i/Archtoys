//! KDE Plasma on Wayland: make the main window invisible during a pick, and
//! visible again afterwards, through a tiny KWin script.
//!
//! Hiding a window on Wayland destroys it, and the new window comes back where
//! the desktop puts it (apps can't place windows there). A window that KWin
//! draws fully transparent stays where it is, at its size, and is left out of
//! the screenshot the freeze frame is made from.

use zbus::blocking::{Connection, Proxy};

const PLUGIN: &str = "archtoys-pick-opacity";

/// True on a KDE Plasma Wayland session, where KWin can do this.
pub fn available() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"));
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.to_ascii_uppercase().contains("KDE"));
    wayland && kde
}

/// The KWin script that sets the main window's opacity (title "Archtoys"; the
/// picker windows have other titles).
fn script(opacity: f64) -> String {
    format!(
        "const list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (const w of list) {{\n\
         \x20   if (w.caption === \"Archtoys\" && w.resourceClass === \"archtoys\") w.opacity = {opacity};\n\
         }}\n"
    )
}

/// Asks KWin to draw the main window with `opacity` (0 = invisible, 1 = normal).
/// KWin reads the script file and runs it shortly after `start` returns, so
/// the file stays (in the runtime folder, cleared at logout) and the script is
/// only unloaded before the next one.
pub fn set_main_window_opacity(opacity: f64) -> Result<(), String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("archtoys-kwin-opacity-{opacity}.js"));
    std::fs::write(&path, script(opacity)).map_err(|err| format!("writing the KWin script: {err}"))?;

    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    let scripting = Proxy::new(&conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting")
        .map_err(|err| format!("KWin scripting: {err}"))?;
    let _: bool = scripting.call("unloadScript", &(PLUGIN,)).unwrap_or(false);
    let id: i32 = scripting
        .call("loadScript", &(path.to_string_lossy().as_ref(), PLUGIN))
        .map_err(|err| format!("KWin loadScript: {err}"))?;
    if id < 0 {
        return Err("KWin refused the script".to_string());
    }
    scripting.call::<_, _, ()>("start", &()).map_err(|err| format!("KWin start: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_targets_only_the_main_window() {
        let js = script(0.0);
        assert!(js.contains("w.caption === \"Archtoys\""));
        assert!(js.contains("w.resourceClass === \"archtoys\""));
        assert!(js.contains("w.opacity = 0;"));
        assert!(script(1.0).contains("w.opacity = 1;"));
    }
}
