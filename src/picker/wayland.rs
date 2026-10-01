//! Wayland picking.
//!
//! 1. Freeze frame: ask the Screenshot portal for one screenshot (no dialog
//!    for the user to click through) and pick from it with a magnifier.
//! 2. If that is not possible: KDE's own color picker (KWin), then the
//!    portal's PickColor. These only return the final color, no preview.

use super::{finish_picker, freeze, PickOutcome, PickerContext};
use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::portal::{self, Outcome};
use crate::AppWindow;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Value;

pub(super) fn start(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    hid_window: bool,
) {
    thread::spawn(move || {
        if hid_window {
            thread::sleep(Duration::from_millis(250));
        }

        let shot = match take_screenshot() {
            Ok(Some(path)) => freeze::load_png(&path),
            Ok(None) => Err("screenshot was declined".to_string()),
            Err(err) => Err(err),
        };

        match shot {
            Ok(shot) => {
                let _ = slint::invoke_from_event_loop(move || {
                    let (ui2, history2) = (ui_weak.clone(), history_store.clone());
                    freeze::show(
                        ui_weak,
                        history_store,
                        context,
                        shot,
                        Box::new(move || start_legacy(ui2, history2, context)),
                    );
                });
            }
            Err(err) => {
                eprintln!("wayland picker: freeze frame unavailable ({err}); using the system picker");
                run_legacy(ui_weak, history_store, context);
            }
        }
    });
}

/// Non-interactive screenshot through the portal. Returns the PNG's path.
fn take_screenshot() -> Result<Option<PathBuf>, String> {
    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    portal::register_host_app(&conn, "archtoys");

    let token = portal::new_token("archtoys_shot");
    let mut options = portal::options(&token);
    options.insert("interactive", Value::from(false));
    options.insert("modal", Value::from(false));

    match portal::call(&conn, "org.freedesktop.portal.Screenshot", "Screenshot", &("", options), &token)? {
        Outcome::Cancelled => Ok(None),
        Outcome::Success(results) => {
            let uri = portal::result_string(&results, "uri").ok_or("screenshot: no uri in reply")?;
            let path = url::Url::parse(&uri)
                .map_err(|err| format!("screenshot: bad uri {uri}: {err}"))?
                .to_file_path()
                .map_err(|_| format!("screenshot: not a local file: {uri}"))?;
            Ok(Some(path))
        }
    }
}

/// The desktop's own picker (KWin, then the portal), without freeze frame.
pub(super) fn start_system(ui_weak: slint::Weak<AppWindow>, history_store: HistoryStore, context: PickerContext) {
    start_legacy(ui_weak, history_store, context);
}

fn start_legacy(ui_weak: slint::Weak<AppWindow>, history_store: HistoryStore, context: PickerContext) {
    thread::spawn(move || run_legacy(ui_weak, history_store, context));
}

fn run_legacy(ui_weak: slint::Weak<AppWindow>, history_store: HistoryStore, context: PickerContext) {
    let result = match pick_color_via_kwin() {
        Ok(picked) => Ok(picked),
        Err(kwin_err) => {
            eprintln!("wayland picker: kwin picker unavailable ({kwin_err}), trying portal");
            pick_color_via_portal()
        }
    };
    let outcome = match result {
        Ok(Some(rgb)) => PickOutcome::Picked(rgb),
        Ok(None) => PickOutcome::Cancelled,
        Err(err) => {
            eprintln!("wayland picker: {err}");
            PickOutcome::Cancelled
        }
    };
    finish_picker(ui_weak, history_store, context, outcome);
}

fn pick_color_via_portal() -> Result<Option<Rgb>, String> {
    let conn = Connection::session().map_err(|err| format!("portal: session bus failed: {err}"))?;
    portal::register_host_app(&conn, "archtoys");

    let token = portal::new_token("archtoys_pick");
    let options = portal::options(&token);
    let results = match portal::call(&conn, "org.freedesktop.portal.Screenshot", "PickColor", &("", options), &token)? {
        Outcome::Success(results) => results,
        Outcome::Cancelled => return Ok(None),
    };

    let color_value = results
        .get("color")
        .ok_or_else(|| "portal: response did not include color".to_string())?;
    let (red, green, blue): (f64, f64, f64) = color_value
        .clone()
        .try_into()
        .map_err(|_| "portal: color type conversion failed".to_string())?;

    let to_u8 = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    Ok(Some((to_u8(red), to_u8(green), to_u8(blue))))
}

fn pick_color_via_kwin() -> Result<Option<Rgb>, String> {
    let connection = Connection::session().map_err(|err| format!("kwin: session bus failed: {err}"))?;
    let proxy = Proxy::new(&connection, "org.kde.KWin", "/ColorPicker", "org.kde.kwin.ColorPicker")
        .map_err(|err| format!("kwin: color picker proxy failed: {err}"))?;

    let pick_reply = match proxy.call_method("pick", &()) {
        Ok(reply) => reply,
        Err(err) => {
            let message = err.to_string();
            if message.to_ascii_lowercase().contains("cancel") {
                return Ok(None);
            }
            return Err(format!("kwin: pick call failed: {message}"));
        }
    };

    // KWin replies with `(u)` on many builds; accept both `u` and `(u)`.
    let argb = pick_reply
        .body()
        .deserialize::<u32>()
        .or_else(|_| pick_reply.body().deserialize::<(u32,)>().map(|tuple| tuple.0))
        .map_err(|err| format!("kwin: pick decode failed: {err}"))?;

    if (argb >> 24) & 0xff == 0 {
        return Ok(None);
    }
    Ok(Some((
        ((argb >> 16) & 0xff) as u8,
        ((argb >> 8) & 0xff) as u8,
        (argb & 0xff) as u8,
    )))
}
