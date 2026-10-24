//! "Report a problem": what a bug report needs, and never anything personal
//! (no file paths, user names, color history or colors).

use std::path::Path;
use std::sync::Mutex;

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

fn info() -> String {
    let distro = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| pretty_name(&text))
        .unwrap_or_else(|| "unknown".into());
    format!(
        "- Archtoys: {}\n- Installed as: {}\n- Distro: {}\n- Desktop: {}\n- Session: {}\n- Picker: {}\n- Hotkey: {}\n",
        env!("CARGO_PKG_VERSION"),
        install_method(),
        distro,
        env_or_unknown("XDG_CURRENT_DESKTOP"),
        env_or_unknown("XDG_SESSION_TYPE"),
        PICKER.lock().unwrap(),
        HOTKEY.lock().unwrap(),
    )
}

/// GitHub's new-issue page with the details already in the body.
pub fn issue_url() -> String {
    let body = format!(
        "{}\n### What is broken\n\n\n### How to reproduce it\n\n\n### What you expected\n\n",
        info()
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
pub fn open_issue_page() -> std::io::Result<()> {
    let mut command = crate::launch::outside_command("xdg-open");
    command.arg(issue_url());
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
    fn report_link_has_the_details_and_nothing_personal() {
        let url = issue_url();
        assert!(url.starts_with("https://github.com/Mujtaba1i/Archtoys/issues/new?body="));
        let encoded = url.split_once("?body=").unwrap().1;
        assert!(encoded.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%".contains(&b)));
        let body = String::from_utf8(crate::portal::percent_decode(encoded).unwrap()).unwrap();
        assert!(body.contains(env!("CARGO_PKG_VERSION")));
        assert!(body.contains("### How to reproduce it"));
        // No paths: the home folder would also reveal the user name.
        if let Some(home) = std::env::var("HOME").ok().filter(|h| h.len() > 1) {
            assert!(!body.contains(&home), "the report contains the home folder");
        }
    }
}
