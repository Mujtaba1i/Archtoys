//! Small helper for xdg-desktop-portal calls (Screenshot, PickColor,
//! GlobalShortcuts) using zbus's blocking API.
//!
//! Portal methods answer through a separate `Response` signal on a "request"
//! object. We subscribe to that signal *before* making the call, otherwise a
//! fast answer can arrive before we listen and we would wait forever.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{DynamicType, OwnedObjectPath, OwnedValue, Value};

pub const DESTINATION: &str = "org.freedesktop.portal.Desktop";
pub const PATH: &str = "/org/freedesktop/portal/desktop";

pub type Results = HashMap<String, OwnedValue>;

/// What the user did with a portal dialog.
#[derive(Debug)]
pub enum Outcome {
    Success(Results),
    Cancelled,
}

/// A unique token for `handle_token` / `session_handle_token`.
pub fn new_token(prefix: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{}_{millis}_{n}", std::process::id())
}

/// `/org/freedesktop/portal/desktop/request/<sender>/<token>`, where
/// `<sender>` is our bus name without ':' and with '.' replaced by '_'.
fn predicted_request_path(conn: &Connection, token: &str) -> Option<String> {
    let unique = conn.unique_name()?.as_str().trim_start_matches(':').replace('.', "_");
    Some(format!("/org/freedesktop/portal/desktop/request/{unique}/{token}"))
}

/// Calls `interface.method(body)` and waits for the portal's Response.
/// `body` must already contain `handle_token` = `token` in its options.
pub fn call<B>(
    conn: &Connection,
    interface: &str,
    method: &str,
    body: &B,
    token: &str,
) -> Result<Outcome, String>
where
    B: serde::Serialize + DynamicType,
{
    let tag = format!("portal {interface}.{method}");

    // 1. Listen first.
    let mut early = match predicted_request_path(conn, token) {
        Some(path) => {
            let proxy = Proxy::new(conn, DESTINATION, path, "org.freedesktop.portal.Request")
                .map_err(|err| format!("{tag}: request proxy failed: {err}"))?;
            let signals = proxy
                .receive_signal("Response")
                .map_err(|err| format!("{tag}: subscribe failed: {err}"))?;
            Some(signals)
        }
        None => None,
    };

    // 2. Call.
    let portal = Proxy::new(conn, DESTINATION, PATH, interface)
        .map_err(|err| format!("{tag}: proxy failed: {err}"))?;
    let reply = portal
        .call_method(method, body)
        .map_err(|err| format!("{tag}: call failed: {err}"))?;
    let (handle,): (OwnedObjectPath,) = reply
        .body()
        .deserialize()
        .map_err(|err| format!("{tag}: reply decode failed: {err}"))?;

    // Very old portals may use a different path than predicted.
    let predicted = predicted_request_path(conn, token);
    if predicted.as_deref() != Some(handle.as_str()) {
        let proxy = Proxy::new(conn, DESTINATION, handle.as_str(), "org.freedesktop.portal.Request")
            .map_err(|err| format!("{tag}: request proxy failed: {err}"))?;
        early = Some(
            proxy
                .receive_signal("Response")
                .map_err(|err| format!("{tag}: subscribe failed: {err}"))?,
        );
    }

    // 3. Wait for the answer.
    let message = early
        .as_mut()
        .and_then(|signals| signals.next())
        .ok_or_else(|| format!("{tag}: response stream ended"))?;
    let (code, results): (u32, Results) = message
        .body()
        .deserialize()
        .map_err(|err| format!("{tag}: response decode failed: {err}"))?;

    match code {
        0 => Ok(Outcome::Success(results)),
        1 | 2 => Ok(Outcome::Cancelled),
        other => Err(format!("{tag}: rejected with code {other}")),
    }
}

/// Options dictionary with `handle_token` already filled in.
pub fn options(token: &str) -> HashMap<&'static str, Value<'static>> {
    let mut options = HashMap::new();
    options.insert("handle_token", Value::from(token.to_string()));
    options
}

/// Tells the portal which app we are (newer portals; harmless if missing).
/// Must be the first portal call on this connection.
pub fn register_host_app(conn: &Connection, app_id: &str) {
    let result = Proxy::new(conn, DESTINATION, PATH, "org.freedesktop.host.portal.Registry")
        .and_then(|registry| {
            let options: HashMap<&str, Value<'_>> = HashMap::new();
            registry.call_method("Register", &(app_id, options))
        });
    if let Err(err) = result {
        eprintln!("portal: host app registration not available ({err})");
    }
}

/// Reads a string-ish value (`s` or `o`) out of a results dictionary.
pub fn result_string(results: &Results, key: &str) -> Option<String> {
    let value = results.get(key)?;
    if let Ok(s) = <String>::try_from(value.clone()) {
        return Some(s);
    }
    if let Ok(path) = <OwnedObjectPath>::try_from(value.clone()) {
        return Some(path.as_str().to_string());
    }
    None
}
