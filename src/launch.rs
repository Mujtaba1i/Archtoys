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
    clean_environment(&mut command, std::env::var_os("APPIMAGE").is_some(), std::env::var_os("ARCHTOYS_HOST_LD_LIBRARY_PATH"));
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

/// Where "Buy me a coffee" goes: a small page in the repo (GitHub Pages,
/// docs/support/) that forwards to Ko-fi, so the Ko-fi link can change, or
/// support can stop, without a new release of the app. The README button and
/// .github/FUNDING.yml point at the same address (a CI check keeps them equal).
pub const SUPPORT_URL: &str = "https://mujtaba1i.github.io/Archtoys/support/";

/// Opens the support page in the browser. The app sends nothing itself.
pub fn open_support_page() -> std::io::Result<()> {
    let mut command = outside_command("xdg-open");
    command.arg(SUPPORT_URL);
    spawn_and_reap(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_path(command: &Command) -> Option<Option<OsString>> {
        command.get_envs().find(|(key, _)| *key == "LD_LIBRARY_PATH").map(|(_, value)| value.map(|v| v.to_owned()))
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

    #[test]
    fn the_support_page_is_this_repos_github_pages_page() {
        // github.com/<owner>/<repo> -> <owner>.github.io/<repo>/support/
        let repo = env!("CARGO_PKG_REPOSITORY").trim_end_matches('/');
        let (owner, name) = repo.trim_start_matches("https://github.com/").split_once('/').unwrap();
        assert_eq!(SUPPORT_URL, format!("https://{}.github.io/{name}/support/", owner.to_lowercase()));
    }
}
