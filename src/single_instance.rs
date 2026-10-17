//! One copy at a time.
//!
//! The first launch holds a lock file in the user's runtime folder (the
//! kernel releases it if the app crashes, so it can never get stuck) and
//! answers on D-Bus. A second launch asks it to show itself, or to start a
//! pick with `--pick`, then exits. Two copies would otherwise save the config
//! at the same moment and register the hotkey and tray icon twice.

use crate::hotkey::Trigger;
use std::fs::{File, TryLockError};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};
use zbus::blocking::Connection;

const BUS_NAME: &str = "io.github.Mujtaba1i.Archtoys";
const OBJECT_PATH: &str = "/io/github/Mujtaba1i/Archtoys";

pub enum Lock {
    /// We are the only copy. Keep this alive until the app exits.
    Held(#[allow(dead_code)] File),
    /// Another copy is running.
    TakenByAnother,
    /// The lock file couldn't be used; run anyway rather than not at all.
    Unavailable,
}

pub fn acquire() -> Lock {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let file = match File::options().create(true).truncate(false).write(true).open(dir.join("archtoys.lock")) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("single instance: can't open the lock file: {err}");
            return Lock::Unavailable;
        }
    };
    match file.try_lock() {
        Ok(()) => Lock::Held(file),
        Err(TryLockError::WouldBlock) => Lock::TakenByAnother,
        Err(TryLockError::Error(err)) => {
            eprintln!("single instance: can't lock: {err}");
            Lock::Unavailable
        }
    }
}

/// Asks the running copy to show itself (`"Show"`) or start a pick (`"Pick"`).
pub fn send(method: &str) -> Result<(), String> {
    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    // Right after its own start, the first copy holds the lock but may not
    // have its D-Bus name yet.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some(BUS_NAME), method, &()) {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(err.to_string()),
        }
    }
}

struct Remote {
    show: Trigger,
    pick: Trigger,
}

#[zbus::interface(name = "io.github.Mujtaba1i.Archtoys")]
impl Remote {
    fn show(&self) {
        (self.show)();
    }

    fn pick(&self) {
        (self.pick)();
    }
}

/// Starts answering later launches. Keep the connection alive.
pub fn serve(show: Trigger, pick: Trigger) -> Option<Connection> {
    let result = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.name(BUS_NAME))
        .and_then(|builder| builder.serve_at(OBJECT_PATH, Remote { show, pick }))
        .and_then(|builder| builder.build());
    match result {
        Ok(conn) => Some(conn),
        Err(err) => {
            eprintln!("single instance: can't listen on D-Bus ({err}); a second launch will just exit");
            None
        }
    }
}
