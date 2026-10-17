//! Starting programs outside Archtoys: a browser, the desktop's settings.

use std::ffi::OsString;
use std::process::Command;

/// Gives a program started from inside an AppImage the library path the
/// AppImage itself was started with. The launcher puts its bundled libraries
/// first in `LD_LIBRARY_PATH`; a browser must not load those older copies.
fn clean_environment(command: &mut Command, in_appimage: bool, host_library_path: Option<OsString>) {
    if !in_appimage {
        return;
    }
    match host_library_path {
        Some(path) => command.env("LD_LIBRARY_PATH", path),
        None => command.env_remove("LD_LIBRARY_PATH"),
    };
    command.env_remove("ARCHTOYS_HOST_LD_LIBRARY_PATH");
}

/// A command for a program outside the app.
pub fn outside_command(program: &str) -> Command {
    let mut command = Command::new(program);
    clean_environment(
        &mut command,
        std::env::var_os("APPIMAGE").is_some(),
        std::env::var_os("ARCHTOYS_HOST_LD_LIBRARY_PATH"),
    );
    command
}

/// Starts the program and collects it when it ends, so it never lingers as a
/// finished-but-unreaped process.
pub fn spawn_and_reap(mut command: Command) -> std::io::Result<()> {
    let mut child = command.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_path(command: &Command) -> Option<Option<OsString>> {
        command
            .get_envs()
            .find(|(key, _)| *key == "LD_LIBRARY_PATH")
            .map(|(_, value)| value.map(|v| v.to_owned()))
    }

    #[test]
    fn outside_an_appimage_nothing_changes() {
        let mut command = Command::new("true");
        clean_environment(&mut command, false, Some("/x".into()));
        assert_eq!(library_path(&command), None);
    }

    #[test]
    fn in_an_appimage_the_original_library_path_comes_back() {
        let mut command = Command::new("true");
        clean_environment(&mut command, true, Some("/host/lib".into()));
        assert_eq!(library_path(&command), Some(Some("/host/lib".into())));
    }

    #[test]
    fn in_an_appimage_started_without_a_library_path_none_is_passed_on() {
        let mut command = Command::new("true");
        clean_environment(&mut command, true, None);
        assert_eq!(library_path(&command), Some(None)); // removed
    }
}
