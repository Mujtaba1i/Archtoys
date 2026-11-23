//! How this copy of Archtoys was installed (v0.8.1), for the updater and the
//! "Report a problem" issue: the system's own package manager is asked which
//! package owns the running file, so it's never a guess from the path.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq)]
pub enum Installed {
    /// From the AUR, built from source (`archtoys`, or another name like `archtoys-git`)
    Aur(String),
    /// From the AUR, the ready-made build (`archtoys-bin`)
    AurBin,
    /// An RPM (Fedora, from COPR)
    Rpm,
    /// The .deb from the release page
    Deb,
    /// The AppImage: the file to replace when updating
    AppImage(PathBuf),
    Flatpak,
    /// The tarball, or built from source by hand
    Other,
}

impl Installed {
    /// For the issue report: "AUR (archtoys-bin)", ".deb", ...
    pub fn label(&self) -> String {
        match self {
            Installed::Aur(name) => format!("AUR ({name})"),
            Installed::AurBin => "AUR (archtoys-bin)".into(),
            Installed::Rpm => "RPM (Fedora COPR)".into(),
            Installed::Deb => ".deb".into(),
            Installed::AppImage(_) => "AppImage".into(),
            Installed::Flatpak => "Flatpak".into(),
            Installed::Other => "tarball or built from source".into(),
        }
    }
}

/// Found once (each package manager is asked at most once per start).
pub fn installed() -> &'static Installed {
    static FOUND: OnceLock<Installed> = OnceLock::new();
    FOUND.get_or_init(detect)
}

fn detect() -> Installed {
    if let Some(file) = std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()) {
        return Installed::AppImage(PathBuf::from(file));
    }
    if std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists() {
        return Installed::Flatpak;
    }
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else { return Installed::Other };
    let exe = exe.to_string_lossy().into_owned();
    // a missing package manager, or one that doesn't own the file, fails: try the next
    if let Some(name) = ask("pacman", &["-Qqo", &exe]) {
        return from_pacman(&name);
    }
    if ask("rpm", &["-qf", "--qf", "%{NAME}", &exe]).is_some_and(|n| n.starts_with("archtoys")) {
        return Installed::Rpm;
    }
    if ask("dpkg", &["-S", &exe]).is_some_and(|out| from_dpkg(&out)) {
        return Installed::Deb;
    }
    Installed::Other
}

fn ask(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).env("LC_ALL", "C").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// `pacman -Qqo` prints the owning package's name.
fn from_pacman(name: &str) -> Installed {
    match name.lines().next().unwrap_or("").trim() {
        "archtoys-bin" => Installed::AurBin,
        n if n.starts_with("archtoys") => Installed::Aur(n.to_string()),
        _ => Installed::Other,
    }
}

/// `dpkg -S` prints "archtoys: /usr/bin/archtoys".
fn from_dpkg(out: &str) -> bool {
    out.lines().any(|line| line.split(':').next().is_some_and(|pkg| pkg.trim() == "archtoys"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pacman_names_the_aur_package() {
        assert_eq!(from_pacman("archtoys\n"), Installed::Aur("archtoys".into()));
        assert_eq!(from_pacman("archtoys-bin"), Installed::AurBin);
        assert_eq!(from_pacman("archtoys-git"), Installed::Aur("archtoys-git".into()));
        assert_eq!(from_pacman("something-else"), Installed::Other);
        assert_eq!(from_pacman(""), Installed::Other);
    }

    #[test]
    fn dpkg_owner_must_be_archtoys() {
        assert!(from_dpkg("archtoys: /usr/bin/archtoys"));
        assert!(!from_dpkg("archtoys-other: /usr/bin/archtoys"));
        assert!(!from_dpkg("dpkg-query: no path found matching pattern /home/me/archtoys"));
    }

    #[test]
    fn labels_say_which_package() {
        assert_eq!(Installed::Aur("archtoys".into()).label(), "AUR (archtoys)");
        assert_eq!(Installed::AurBin.label(), "AUR (archtoys-bin)");
        assert_eq!(Installed::Deb.label(), ".deb");
        assert_eq!(Installed::Other.label(), "tarball or built from source");
    }
}
