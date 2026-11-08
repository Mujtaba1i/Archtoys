//! "Report a problem": what a bug report needs, and never anything personal
//! (no file paths, user names, color history or colors).

use std::collections::VecDeque;
use std::path::Path;
use std::sync::Mutex;

/// The last messages Archtoys printed (cleaned), for a report.
static RECENT: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());
/// How many, and how long each may be: the report travels in a link, which
/// GitHub cuts off at about 8 KB.
const RECENT_LINES: usize = 10;
const LINE_CHARS: usize = 160;

/// Keeps the last lines Archtoys prints to the terminal (its own messages,
/// a library's warning, a crash), so a report can show what happened just
/// before; they still reach the terminal. stderr becomes a pipe that a thread
/// reads, passes on and remembers. Call once, in the running app only.
pub fn keep_recent_messages() {
    use std::ffi::c_int;
    use std::io::{BufRead, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    extern "C" {
        fn pipe(fds: *mut c_int) -> c_int;
        fn dup(fd: c_int) -> c_int;
        fn dup2(old: c_int, new: c_int) -> c_int;
        fn close(fd: c_int) -> c_int;
    }
    let mut fds: [c_int; 2] = [0; 2];
    // SAFETY: POSIX calls on file descriptors this function owns; when one
    // fails, stderr is left as it was.
    unsafe {
        if pipe(fds.as_mut_ptr()) != 0 {
            return;
        }
        let terminal = dup(2);
        if terminal < 0 || dup2(fds[1], 2) < 0 {
            close(fds[0]);
            close(fds[1]);
            if terminal >= 0 {
                close(terminal);
            }
            return;
        }
        close(fds[1]);
        let reader = std::fs::File::from_raw_fd(fds[0]);
        let mut terminal = std::fs::File::from_raw_fd(terminal);
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(reader).split(b'\n') {
                let Ok(line) = line else { break };
                let _ = terminal.write_all(&line);
                let _ = terminal.write_all(b"\n");
                remember(&String::from_utf8_lossy(&line));
            }
            // never leave stderr on a pipe nobody reads (a full pipe would block the app)
            dup2(terminal.as_raw_fd(), 2);
        });
    }
}

fn remember(line: &str) {
    let home = std::env::var("HOME").unwrap_or_default();
    let user = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).unwrap_or_default();
    let host = std::fs::read_to_string("/etc/hostname").unwrap_or_default();
    let line = clean(line, &home, &user, host.trim());
    if line.trim().is_empty() {
        return;
    }
    let mut recent = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    recent.push_back(line);
    while recent.len() > RECENT_LINES {
        recent.pop_front();
    }
}

/// Nothing personal: the home folder becomes ~, the user and computer names
/// go, and a line is cut to `LINE_CHARS`.
fn clean(line: &str, home: &str, user: &str, host: &str) -> String {
    let mut line = line.to_string();
    if home.len() > 1 {
        line = line.replace(home, "~");
    }
    // the computer's name first: it often has the user's name in it ("alicepc")
    for (name, with) in [(host, "<computer>"), (user, "<user>")] {
        if name.len() >= 3 {
            line = line.replace(name, with);
        }
    }
    match line.char_indices().nth(LINE_CHARS) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

/// The recent messages, oldest first.
fn recent_messages() -> Vec<String> {
    RECENT.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect()
}

static PICKER: Mutex<&str> = Mutex::new("not used yet");
static HOTKEY: Mutex<&str> = Mutex::new("not started");

pub fn set_picker(method: &'static str) {
    *PICKER.lock().unwrap() = method;
}

pub fn set_hotkey(method: &'static str) {
    *HOTKEY.lock().unwrap() = method;
}

fn install_method() -> &'static str {
    if std::env::var_os("APPIMAGE").is_some() {
        "AppImage"
    } else if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists() {
        "Flatpak"
    } else if std::env::current_exe().is_ok_and(|exe| exe.starts_with("/usr")) {
        "package"
    } else {
        "built from source"
    }
}

fn pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim_matches('"').to_string())
}

fn env_or_unknown(name: &str) -> String {
    std::env::var(name).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "unknown".into())
}

/// The desktop's version: "plasmashell 6.2.4" -> "Plasma 6.2.4", "GNOME Shell 47.2" -> "GNOME 47.2".
fn desktop_version() -> String {
    let ask = |program: &str| {
        crate::launch::outside_command(program)
            .arg("--version")
            .output()
            .ok()
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_uppercase();
    let found = if desktop.contains("KDE") {
        ask("plasmashell").map(|v| v.replace("plasmashell", "Plasma"))
    } else if desktop.contains("GNOME") {
        ask("gnome-shell").map(|v| v.replace("GNOME Shell", "GNOME"))
    } else {
        None
    };
    found.unwrap_or_else(|| "unknown".into())
}

/// The screens and scales a report needs, from the window: "2: 2560x1440 at
/// 150%, 1920x1080 at 100%" and Archtoys' own scale.
pub struct Screens {
    pub screens: String,
    pub scale: f32,
}

fn info(screens: &Screens) -> String {
    let distro = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| pretty_name(&text))
        .unwrap_or_else(|| "unknown".into());
    // set by hand, Archtoys' scale differs from the desktop's (issue #11)
    let forced = std::env::var("SLINT_SCALE_FACTOR").ok().filter(|v| !v.is_empty());
    format!(
        "- Archtoys: {}\n- Installed as: {}\n- Distro: {}\n- Desktop: {} ({})\n- Session: {}\n- Screens: {}\n- Archtoys scale: {:.0}%{}\n- Drawn with: {}\n- Picker: {}\n- Hotkey: {}\n- Text Extractor: {}\n",
        env!("CARGO_PKG_VERSION"),
        install_method(),
        distro,
        env_or_unknown("XDG_CURRENT_DESKTOP"),
        desktop_version(),
        env_or_unknown("XDG_SESSION_TYPE"),
        screens.screens,
        screens.scale * 100.0,
        forced.map(|v| format!(" (SLINT_SCALE_FACTOR={v})")).unwrap_or_default(),
        if cfg!(feature = "opengl") { "OpenGL" } else { "CPU" },
        PICKER.lock().unwrap(),
        HOTKEY.lock().unwrap(),
        crate::ocr::report_line(),
    )
}

/// GitHub's new-issue page with the details already in the body.
pub fn issue_url(screens: &Screens) -> String {
    let recent = recent_messages();
    let messages = if recent.is_empty() {
        String::new()
    } else {
        // backquotes would end the code block early
        format!(
            "\n<details><summary>Recent messages from Archtoys</summary>\n\n```\n{}\n```\n</details>\n",
            recent.join("\n").replace("```", "'''")
        )
    };
    let body = format!(
        "{}{messages}\n### What is broken\n\n\n### How to reproduce it\n\n\n### What you expected\n\n",
        info(screens)
    );
    concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new?body=").to_string() + &percent_encode(&body)
}

/// Every byte except letters, digits and `-._~` as `%XX`, for a link.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Opens the report page in the browser. The app sends nothing itself.
pub fn open_issue_page(screens: &Screens) -> std::io::Result<()> {
    let mut command = crate::launch::outside_command("xdg-open");
    command.arg(issue_url(screens));
    crate::launch::spawn_and_reap(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_distro_name() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("ID=arch\n"), None);
    }

    #[test]
    fn messages_are_kept_without_anything_personal() {
        assert_eq!(
            clean("can't read /home/alice/.config/x: alice@alicepc said no", "/home/alice", "alice", "alicepc"),
            "can't read ~/.config/x: <user>@<computer> said no"
        );
        assert_eq!(clean(&"x".repeat(500), "", "", "").chars().count(), LINE_CHARS + 1);
        for i in 0..15 {
            remember(&format!("message {i}"));
        }
        let recent = recent_messages();
        // (another test may add one meanwhile: they share the list)
        assert_eq!(recent.len(), RECENT_LINES);
        assert!(recent.iter().any(|l| l == "message 14") && !recent.iter().any(|l| l == "message 0"));
    }

    #[test]
    fn report_link_has_the_details_and_nothing_personal() {
        if let Ok(home) = std::env::var("HOME") {
            remember(&format!("picker: couldn't load {home}/shot.png"));
        }
        let url = issue_url(&Screens { screens: "2: 2560x1440 at 150%, 1920x1080 at 100%".into(), scale: 1.5 });
        assert!(url.starts_with("https://github.com/Mujtaba1i/Archtoys/issues/new?body="));
        let encoded = url.split_once("?body=").unwrap().1;
        assert!(encoded.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b)));
        let body = String::from_utf8(crate::portal::percent_decode(encoded).unwrap()).unwrap();
        assert!(body.contains(env!("CARGO_PKG_VERSION")));
        assert!(body.contains("### How to reproduce it"));
        assert!(body.contains("- Screens: 2: 2560x1440 at 150%, 1920x1080 at 100%"));
        assert!(body.contains("- Archtoys scale: 150%"));
        assert!(body.contains("- Drawn with: ") && body.contains("- Text Extractor: "));
        assert!(body.contains("Recent messages from Archtoys"));
        // No paths: the home folder would also reveal the user name.
        if let Some(home) = std::env::var("HOME").ok().filter(|h| h.len() > 1) {
            assert!(!body.contains(&home), "the report contains the home folder");
        }
    }
}
