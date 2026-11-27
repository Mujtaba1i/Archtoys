//! One copy at a time.
//!
//! The first launch holds a lock file in the user's runtime folder (the
//! kernel releases it if the app crashes, so it can never get stuck) and
//! answers on D-Bus. A second launch asks it to show itself, or to start a
//! pick with `--pick`, then exits. Two copies would otherwise save the config
//! at the same moment and register the hotkey and tray icon twice.

use crate::hotkey::Trigger;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
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

fn user_id() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0)
}

fn lock_path() -> PathBuf {
    // a Flatpak's runtime folder is new for each run: the shared one
    if let Some(dir) = crate::sandbox::shared_runtime_dir() {
        return dir.join("archtoys.lock");
    }
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) => PathBuf::from(dir).join("archtoys.lock"),
        // No private runtime folder: /tmp is shared by every user, so the
        // user id is part of the name and users never lock each other out.
        None => std::env::temp_dir().join(format!("archtoys-{}.lock", user_id())),
    }
}

pub fn acquire() -> Lock {
    acquire_at(&lock_path())
}

fn acquire_at(path: &Path) -> Lock {
    let file = match File::options().create(true).truncate(false).write(true).open(path) {
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

/// Asks the running copy to show itself (`"Show"`), start a pick (`"Pick"`)
/// start a text capture (`"Extract"`), open the Screen Ruler (`"Measure"`) or
/// take a Screenshot to clipboard (`"Screenshot"`, when that Micro-toy is on).
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

/// Hands an Image Resizer job to the running copy.
pub fn send_resize(action: &str, files: &[String]) -> Result<(), String> {
    let conn = Connection::session().map_err(|err| format!("session bus: {err}"))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match conn.call_method(Some(BUS_NAME), OBJECT_PATH, Some(BUS_NAME), "Resize", &(action, files)) {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_launch_finds_the_lock_taken_until_the_first_one_ends() {
        let path = std::env::temp_dir().join(format!("archtoys-lock-test-{}.lock", std::process::id()));
        let first = acquire_at(&path);
        assert!(matches!(first, Lock::Held(_)));
        assert!(matches!(acquire_at(&path), Lock::TakenByAnother));
        drop(first); // the first copy ends: the system frees the lock
        assert!(matches!(acquire_at(&path), Lock::Held(_)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn without_a_runtime_folder_the_name_carries_the_user_id() {
        let name = std::env::temp_dir().join(format!("archtoys-{}.lock", user_id()));
        assert!(name.to_string_lossy().contains("archtoys-"));
    }
}

struct Remote {
    show: Trigger,
    pick: Trigger,
    extract: Trigger,
    measure: Trigger,
    screenshot: Trigger,
    resize: ResizeTrigger,
}

/// Image Resizer's start: the action and the files.
pub type ResizeTrigger = std::sync::Arc<dyn Fn(String, Vec<String>) + Send + Sync>;

#[zbus::interface(name = "io.github.Mujtaba1i.Archtoys")]
impl Remote {
    fn show(&self) {
        (self.show)();
    }

    fn pick(&self) {
        (self.pick)();
    }

    fn extract(&self) {
        (self.extract)();
    }

    fn measure(&self) {
        (self.measure)();
    }

    fn screenshot(&self) {
        (self.screenshot)();
    }

    /// Image Resizer from a file manager's menu: an action and the files (full paths).
    fn resize(&self, action: &str, files: Vec<String>) {
        (self.resize)(action.to_string(), files);
    }

    /// The KWin script (kwin.rs): where the main window was, "x,y,w,h".
    fn save_geometry(&self, geometry: &str) {
        if let Some(g) = crate::kwin::parse_geometry(geometry) {
            crate::kwin::set_geometry(Some(g));
        }
    }
}

/// Starts answering later launches. Keep the connection alive.
pub fn serve(
    show: Trigger,
    pick: Trigger,
    extract: Trigger,
    measure: Trigger,
    screenshot: Trigger,
    resize: ResizeTrigger,
) -> Option<Connection> {
    let result = zbus::blocking::connection::Builder::session()
        .and_then(|builder| builder.name(BUS_NAME))
        .and_then(|builder| builder.serve_at(OBJECT_PATH, Remote { show, pick, extract, measure, screenshot, resize }))
        .and_then(|builder| builder.build());
    match result {
        Ok(conn) => Some(conn),
        Err(err) => {
            eprintln!("single instance: can't listen on D-Bus ({err}), so a second launch will just exit");
            None
        }
    }
}
