//! KDE Plasma on Wayland: make the main window invisible during a pick, and
//! visible again afterwards (and brought to the front, after Open in a text
//! capture: Wayland doesn't let an app raise its own window), through a tiny
//! KWin script.
//!
//! Hiding a window on Wayland destroys it, and the new window comes back where
//! the desktop puts it (apps can't place windows there). A window that KWin
//! draws fully transparent stays where it is, at its size, and is left out of
//! the screenshot the freeze frame is made from.

use std::sync::Mutex;
use zbus::blocking::{Connection, Proxy};

const PLUGIN: &str = "archtoys-pick-opacity";
/// The script that puts the window back where it was closed (kept loaded).
const REMEMBER: &str = "archtoys-remember-window";

/// The main window's frame geometry (x, y, width, height) as KWin last saw
/// it, for config.json.
static GEOMETRY: Mutex<Option<[i32; 4]>> = Mutex::new(None);

pub fn geometry() -> Option<[i32; 4]> {
    *GEOMETRY.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_geometry(geometry: Option<[i32; 4]>) {
    *GEOMETRY.lock().unwrap_or_else(|e| e.into_inner()) = geometry.filter(|g| sane(*g));
}

/// "120,80,1000,760" from the script (KWin's numbers can have decimals).
pub fn parse_geometry(text: &str) -> Option<[i32; 4]> {
    let n: Vec<i32> = text.split(',').map(|p| p.trim().parse::<f64>().ok().map(|v| v.round() as i32)).collect::<Option<_>>()?;
    let g: [i32; 4] = n.try_into().ok()?;
    sane(g).then_some(g)
}

fn sane([_, _, w, h]: [i32; 4]) -> bool {
    (100..20000).contains(&w) && (100..20000).contains(&h)
}

/// True on a KDE Plasma Wayland session, where KWin can do this.
pub fn available() -> bool {
    let wayland = std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"));
    let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|v| v.to_ascii_uppercase().contains("KDE"));
    wayland && kde
}

/// The KWin script that sets the main window's opacity (title "Archtoys"; the
/// picker windows have other titles), and with `raise` un-minimizes it and
/// makes it the active window (`activeWindow` in Plasma 6, `activeClient` in 5).
fn script(opacity: f64, raise: bool) -> String {
    let raise = if raise {
        "        w.minimized = false;\n\
         \x20       if (typeof workspace.activeWindow !== \"undefined\") workspace.activeWindow = w; else workspace.activeClient = w;\n"
    } else {
        ""
    };
    format!(
        "const list = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (const w of list) {{\n\
         \x20   if (w.caption === \"Archtoys\" && w.resourceClass === \"archtoys\") {{\n\
         \x20       w.opacity = {opacity};\n\
         {raise}\
         \x20   }}\n\
         }}\n"
    )
}

/// Asks KWin to draw the main window with `opacity` (0 = invisible, 1 = normal).
/// KWin reads the script file and runs it shortly after `start` returns, so
/// the file stays (in the runtime folder, cleared at logout) and the script is
/// only unloaded before the next one.
pub fn set_main_window_opacity(opacity: f64) -> Result<(), String> {
    run(PLUGIN, &format!("opacity-{opacity}"), &script(opacity, false))
}

/// Makes the main window visible, un-minimized and the active window.
pub fn raise_main_window() -> Result<(), String> {
    run(PLUGIN, "raise", &script(1.0, true))
}

/// Wayland doesn't let an app place its window, so it opened wherever the
/// desktop chose (issue #11). This KWin script (the way the "Remember Window
/// Positions" script does it) puts the main window back where it was: each
/// time it appears it gets the saved frame geometry (if that's still on a
/// screen), and when it closes or is moved or resized the script keeps the
/// new one and tells Archtoys (`SaveGeometry` on its D-Bus name), which saves
/// it in config.json for the next start. Loaded once, at startup. KWin puts
/// a new window in place in the same step it tells scripts about it, before
/// it draws a frame of it, so the window doesn't visibly jump. Only the place
/// is set: Archtoys sets its own size (`ui_state::restore_size`), and on
/// KWin 6 a size set by a script waits for the app to take it, holding the
/// move back with it. The same plain script works on Plasma 5 and 6.
pub fn remember_main_window() -> Result<(), String> {
    run(REMEMBER, "remember", &remember_script(geometry()))
}

fn saved_js(saved: Option<[i32; 4]>) -> String {
    saved.map(|[x, y, w, h]| format!("[{x}, {y}, {w}, {h}]")).unwrap_or_else(|| "null".into())
}

fn remember_script(saved: Option<[i32; 4]>) -> String {
    let saved = saved_js(saved);
    format!(
        r#"let saved = {saved};
const isMain = w => w.caption === "Archtoys" && w.resourceClass === "archtoys";
function onScreen(s) {{
    if (!workspace.screens) return true;
    const cx = s[0] + s[2] / 2, cy = s[1] + s[3] / 2;
    return workspace.screens.some(o => cx >= o.geometry.x && cx < o.geometry.x + o.geometry.width
        && cy >= o.geometry.y && cy < o.geometry.y + o.geometry.height);
}}
function remember(w) {{
    const g = w.frameGeometry;
    saved = [g.x, g.y, g.width, g.height];
    callDBus("io.github.Mujtaba1i.Archtoys", "/io/github/Mujtaba1i/Archtoys", "io.github.Mujtaba1i.Archtoys",
        "SaveGeometry", saved.join(","));
}}
function hook(w) {{
    if (!isMain(w)) return;
    if (saved && onScreen(saved)) w.frameGeometry = {{x: saved[0], y: saved[1], width: w.frameGeometry.width, height: w.frameGeometry.height}};
    const closed = w.closed || w.windowClosed;
    if (closed) closed.connect(() => remember(w));
    const moved = w.interactiveMoveResizeFinished || w.clientFinishUserMovedResized;
    if (moved) moved.connect(() => remember(w));
}}
(workspace.windowAdded || workspace.clientAdded).connect(hook);
(workspace.windowList ? workspace.windowList() : workspace.clientList()).forEach(hook);
"#
    )
}

fn run(plugin: &str, name: &str, js: &str) -> Result<(), String> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let path = dir.join(format!("archtoys-kwin-{name}.js"));
    std::fs::write(&path, js).map_err(|err| format!("writing the KWin script: {err}"))?;

    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    let scripting =
        Proxy::new(&conn, "org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting").map_err(|err| format!("KWin scripting: {err}"))?;
    let _: bool = scripting.call("unloadScript", &(plugin,)).unwrap_or(false);
    let id: i32 =
        scripting.call("loadScript", &(path.to_string_lossy().as_ref(), plugin)).map_err(|err| format!("KWin loadScript: {err}"))?;
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
        let js = script(0.0, false);
        assert!(js.contains("w.caption === \"Archtoys\""));
        assert!(js.contains("w.resourceClass === \"archtoys\""));
        assert!(js.contains("w.opacity = 0;") && !js.contains("activeWindow"));
        let raise = script(1.0, true);
        assert!(raise.contains("w.opacity = 1;") && raise.contains("w.minimized = false;"));
        assert!(raise.contains("workspace.activeWindow = w") && raise.contains("workspace.activeClient = w"));
    }

    #[test]
    fn the_window_is_put_back_where_it_was() {
        let js = remember_script(Some([120, 80, 1000, 760]));
        assert!(js.contains("let saved = [120, 80, 1000, 760];"));
        assert!(js.contains("w.frameGeometry = {x: saved[0], y: saved[1], width: w.frameGeometry.width"));
        assert!(js.contains("\"SaveGeometry\", saved.join(\",\")"));
        assert!(remember_script(None).contains("let saved = null;"));
        // what the script sends back, and nonsense it must not keep
        assert_eq!(parse_geometry("120,80.4,1000,759.6"), Some([120, 80, 1000, 760]));
        assert_eq!(parse_geometry("1,2,3"), None);
        assert_eq!(parse_geometry("0,0,0,0"), None);
        assert_eq!(parse_geometry("a,b,c,d"), None);
    }
}
