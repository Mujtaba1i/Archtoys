//! "Turn on": installs (or updates) Tesseract from the system's own packages.
//!
//! The distro comes from /etc/os-release (Arch, Fedora or Debian/Ubuntu and
//! their families). The install runs through `pkexec`, so the desktop asks for
//! the password in its own dialog and Archtoys never sees it. It runs even
//! when Tesseract is already installed: then it updates it from the system's
//! packages, so the version is one Archtoys works with. Read-only systems
//! (Fedora Silverblue/Kinoite, SteamOS) and unknown ones get a message instead.
//!
//! Progress (size, speed, time left) is read from the package manager's own
//! output: apt's machine-readable status lines, and pacman's and dnf's progress
//! bars (run under `script`, which makes them think they write to a terminal).

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Distro {
    Arch(String),
    Fedora(String),
    Debian(String),
    /// Archtoys can't install software here: the name, and why.
    Other(String, String),
}

impl Distro {
    pub(crate) fn name(&self) -> &str {
        match self {
            Distro::Arch(n) | Distro::Fedora(n) | Distro::Debian(n) | Distro::Other(n, _) => n,
        }
    }

    fn manager(&self) -> &'static str {
        match self {
            Distro::Arch(_) => "pacman",
            Distro::Fedora(_) => "dnf",
            Distro::Debian(_) => "apt",
            Distro::Other(..) => "",
        }
    }
}

/// Reads /etc/os-release.
pub(crate) fn detect() -> Distro {
    let text = std::fs::read_to_string("/etc/os-release").or_else(|_| std::fs::read_to_string("/usr/lib/os-release")).unwrap_or_default();
    detect_from(&text, std::path::Path::new("/run/ostree-booted").exists())
}

pub(crate) fn detect_from(os_release: &str, ostree: bool) -> Distro {
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').trim_matches('\'').to_string())
            .unwrap_or_default()
    };
    let id = field("ID").to_ascii_lowercase();
    let like = field("ID_LIKE").to_ascii_lowercase();
    let variant = field("VARIANT_ID").to_ascii_lowercase();
    let name = match field("NAME") {
        n if n.is_empty() => "this system".to_string(),
        n => n,
    };
    let ids: Vec<&str> = std::iter::once(id.as_str()).chain(like.split_whitespace()).collect();

    let read_only = ostree
        || matches!(variant.as_str(), "silverblue" | "kinoite" | "sericea" | "onyx" | "cosmic-atomic")
        || matches!(id.as_str(), "steamos" | "bazzite" | "nixos");
    if read_only {
        let why = format!(
            "{name} doesn't let apps install software (its system is read-only). Install Tesseract the way {name} \
             installs software (a layered package, a toolbox, or its software center), then press Check again."
        );
        return Distro::Other(name, why);
    }
    if ids.contains(&"arch") {
        Distro::Arch(name)
    } else if ids.iter().any(|i| matches!(*i, "fedora" | "rhel" | "centos")) {
        Distro::Fedora(name)
    } else if ids.iter().any(|i| matches!(*i, "debian" | "ubuntu")) {
        Distro::Debian(name)
    } else {
        let why = format!(
            "Archtoys knows how to install Tesseract on Arch, Fedora, Debian, Ubuntu and their families, but not on \
             {name}. Install the package tesseract (often with tesseract-ocr in its name) from your software center, \
             then press Check again."
        );
        Distro::Other(name, why)
    }
}

/// What runs as administrator. pacman refreshes nothing first: refreshing the
/// package lists without a full upgrade is a partial upgrade, which Arch warns
/// can break the system, so it installs the version in the current lists.
pub(crate) fn script_for(distro: &Distro) -> Option<String> {
    // pacman's and dnf's progress bars only show on a terminal: `script` gives them one
    let tty = |cmd: &str| format!("if command -v script >/dev/null 2>&1; then script -qfec '{cmd}' /dev/null; else {cmd}; fi");
    match distro {
        Distro::Arch(_) => Some(tty("pacman -S --noconfirm tesseract tesseract-data-eng tesseract-data-osd")),
        Distro::Fedora(_) => Some(tty("if dnf --version 2>/dev/null | grep -q dnf5; then \
             dnf install -y --skip-unavailable tesseract tesseract-langpack-eng tesseract-osd && dnf upgrade -y tesseract; \
             else dnf install -y --setopt=strict=0 tesseract tesseract-langpack-eng tesseract-osd && dnf upgrade -y tesseract; fi")),
        Distro::Debian(_) => Some(
            "export DEBIAN_FRONTEND=noninteractive; apt-get update && apt-get install -y -o APT::Status-Fd=1 tesseract-ocr".to_string(),
        ),
        Distro::Other(..) => None,
    }
}

/// What the installing card shows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Progress {
    /// 0-1, or None while the size isn't known
    pub fraction: Option<f32>,
    /// "9.3 of 15.0 MB · 2.1 MB/s · about 3 s left"
    pub stats: String,
    /// "Downloading with pacman"
    pub stage: String,
}

/// "12.3 MiB", "2,345 kB", "12 M" → bytes.
fn parse_size(number: &str, unit: &str) -> Option<f64> {
    let value: f64 = number.replace(',', "").parse().ok()?;
    let factor = match unit.trim_end_matches(['.', ',']) {
        "B" => 1.0,
        "KiB" | "k" | "K" => 1024.0,
        "MiB" | "M" => 1024.0 * 1024.0,
        "GiB" | "G" => 1024.0 * 1024.0 * 1024.0,
        "kB" | "KB" => 1000.0,
        "MB" => 1e6,
        "GB" => 1e9,
        _ => return None,
    };
    Some(value * factor)
}

/// The first "<number> <unit>" after `marker` in `line`.
fn size_after(line: &str, marker: &str) -> Option<f64> {
    let rest = &line[line.find(marker)? + marker.len()..];
    let mut tokens = rest.split_whitespace();
    let number = tokens.next()?;
    // "12MiB" (no space) or "12 MiB"
    if let Some(split) = number.find(|c: char| c.is_ascii_alphabetic()) {
        return parse_size(&number[..split], &number[split..]);
    }
    parse_size(number, tokens.next()?)
}

/// Drops terminal color and cursor codes.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                // CSI: ESC [ ... letter (colors, cursor moves)
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                // OSC: ESC ] ... BEL or ESC \ (dnf5's taskbar progress)
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Turns the package manager's output, line by line, into progress.
pub(crate) struct Parser {
    manager: &'static str,
    total: Option<f64>,
    /// pacman and dnf print an overall "Total" line with several packages:
    /// once seen, only those count (the per-package bars restart at 0%)
    overall_lines: bool,
    stage: String,
    fraction: Option<f32>,
    last: Option<(f64, f64)>,
    speed: Option<f64>,
}

impl Parser {
    pub(crate) fn new(manager: &'static str) -> Self {
        Self {
            manager,
            total: None,
            overall_lines: false,
            stage: format!("Getting ready ({manager})"),
            fraction: None,
            last: None,
            speed: None,
        }
    }

    /// One line of output, `at` seconds after the start. Some when the card changes.
    pub(crate) fn feed(&mut self, raw: &str, at: f64) -> Option<Progress> {
        let line = strip_ansi(raw);
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let before = (self.fraction, self.stage.clone());

        // the download size
        for marker in ["Total Download Size:", "Total download size:", "Need to download", "Need to get"] {
            if let Some(size) = size_after(line, marker) {
                self.total = Some(size);
            }
        }

        if let Some(rest) = line.strip_prefix("dlstatus:") {
            // apt: dlstatus:<n>:<percent>:<what>
            if let Some(p) = rest.split(':').nth(1).and_then(|p| p.parse::<f32>().ok()) {
                self.fraction = Some((p / 100.0).clamp(0.0, 1.0));
                self.stage = "Downloading with apt".into();
            }
        } else if let Some(rest) = line.strip_prefix("pmstatus:") {
            let what = rest.splitn(3, ':').nth(2).unwrap_or("Installing").trim();
            self.fraction = None;
            self.stage = what.to_string();
        } else if line.starts_with("Hit:") || line.starts_with("Get:") || line.starts_with("Reading package lists") {
            self.stage = "Updating the package lists (apt)".into();
        } else if line.contains("installing") || line.contains("Installing") || line.contains("Running transaction") {
            self.fraction = None;
            self.stage = format!("Installing ({})", self.manager);
        } else if let Some(percent) = line.split_whitespace().rev().find_map(|t| t.strip_suffix('%')?.parse::<f32>().ok()) {
            let overall = line.contains("Total");
            if overall {
                self.overall_lines = true;
            }
            if overall || !self.overall_lines {
                self.fraction = Some((percent / 100.0).clamp(0.0, 1.0));
                self.stage = format!("Downloading with {}", self.manager);
            }
        }

        if (self.fraction, self.stage.clone()) == before {
            return None;
        }
        Some(self.progress(at))
    }

    fn progress(&mut self, at: f64) -> Progress {
        const MB: f64 = 1e6;
        let stats = match (self.total, self.fraction) {
            (Some(total), Some(fraction)) => {
                let done = total * fraction as f64;
                if let Some((t0, d0)) = self.last {
                    if at > t0 + 0.2 && done >= d0 {
                        let now = (done - d0) / (at - t0);
                        self.speed = Some(self.speed.map_or(now, |s| s * 0.6 + now * 0.4));
                        self.last = Some((at, done));
                    }
                } else {
                    self.last = Some((at, done));
                }
                let mut stats = format!("{:.1} of {:.1} MB", done / MB, total / MB);
                if let Some(speed) = self.speed.filter(|s| *s > 1.0) {
                    let left = ((total - done) / speed).ceil() as u64;
                    stats.push_str(&format!(" · {:.1} MB/s · about {left} s left", speed / MB));
                }
                stats
            }
            (Some(total), None) => format!("{:.1} MB", total / MB),
            _ => String::new(),
        };
        Progress { fraction: self.fraction, stats, stage: self.stage.clone() }
    }
}

/// Installs Tesseract. Blocking: call it on a background thread. `cancel`
/// stops it; `on_progress` gets the card's text (on that thread).
pub(crate) fn run(distro: &Distro, cancel: Arc<AtomicBool>, on_progress: impl Fn(Progress) + Send + 'static) -> Result<(), String> {
    let script = script_for(distro).ok_or_else(|| "Archtoys can't install software here".to_string())?;
    let mut child = Command::new("pkexec")
        .args(["/bin/sh", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                "pkexec isn't installed, so Archtoys can't ask for your password. Install polkit, then try again.".to_string()
            }
            _ => format!("couldn't start the install: {err}"),
        })?;

    let started = Instant::now();
    let manager = distro.manager();
    let mut stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut parser = Parser::new(manager);
        let mut line = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if let Some(p) = parser.feed(&String::from_utf8_lossy(&line), started.elapsed().as_secs_f64()) {
                        on_progress(p);
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
        }
    });
    let mut stderr = child.stderr.take().expect("piped stderr");
    let err_reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });

    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.kill();
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => return Err(format!("the install stopped: {err}")),
        }
    };
    let _ = reader.join();
    let errors = err_reader.join().unwrap_or_default();

    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled. Nothing was changed, or the package manager finished what it had started.".into());
    }
    match status.code() {
        Some(0) => Ok(()),
        // pkexec: the password dialog was closed, or the user isn't allowed
        Some(126) | Some(127) => Err("The password wasn't given, so nothing was installed.".into()),
        _ => {
            let last = errors.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no details");
            Err(format!("Installing didn't work: {last}"))
        }
    }
}

/// The newer Tesseract the system's packages offer ("5.5.1"), if there is
/// one. Asks the package manager as the user (no password): pacman and apt
/// answer from the package lists as they were last refreshed, dnf refreshes
/// them itself. Blocking: call it on a background thread.
pub(crate) fn update_available(distro: &Distro) -> Result<Option<String>, String> {
    let run = |cmd: &str, args: &[&str]| {
        Command::new(cmd)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
            .map_err(|err| format!("couldn't run {cmd}: {err}"))
    };
    let output = match distro {
        // checkupdates (pacman-contrib) reads fresh lists without a password
        Distro::Arch(_) => run("checkupdates", &[]).or_else(|_| run("pacman", &["-Qu", "tesseract"]))?,
        Distro::Fedora(_) => run("dnf", &["-q", "list", "--upgrades", "tesseract"])?,
        Distro::Debian(_) => run("apt", &["list", "--upgradable", "tesseract-ocr"])?,
        Distro::Other(..) => return Ok(None),
    };
    Ok(parse_update(distro.manager(), &output))
}

/// The new version in the package manager's list of updates, without the
/// epoch and the package release ("1:5.5.1-1.fc43" -> "5.5.1").
fn parse_update(manager: &str, output: &str) -> Option<String> {
    let version = output.lines().find_map(|line| {
        let mut cols = line.split_whitespace();
        let name = cols.next()?;
        match manager {
            // "tesseract 5.5.0-1 -> 5.5.1-1"
            "pacman" => (name == "tesseract").then(|| line.rsplit("->").next().map(str::trim)).flatten(),
            // "tesseract.x86_64   5.5.1-1.fc43   updates"
            "dnf" => name.starts_with("tesseract.").then(|| cols.next()).flatten(),
            // "tesseract-ocr/noble-updates 5.3.4-1build5 amd64 [upgradable from: ...]"
            _ => name.starts_with("tesseract-ocr/").then(|| cols.next()).flatten(),
        }
    })?;
    let version = version.rsplit(':').next()?.split('-').next()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_are_read_from_each_package_manager() {
        assert_eq!(parse_update("pacman", "firefox 1-1 -> 2-1\ntesseract 5.5.0-1 -> 5.5.1-1\n"), Some("5.5.1".into()));
        assert_eq!(parse_update("pacman", "tesseract-data-eng 2:4.1-1 -> 2:4.2-1\n"), None);
        assert_eq!(parse_update("dnf", "Available upgrades\ntesseract.x86_64   1:5.5.1-1.fc43   updates\n"), Some("5.5.1".into()));
        assert_eq!(parse_update("dnf", ""), None);
        assert_eq!(
            parse_update("apt", "Listing...\ntesseract-ocr/noble-updates 5.3.4-1build5 amd64 [upgradable from: 5.3.4-1build4]\n"),
            Some("5.3.4".into())
        );
        assert_eq!(parse_update("apt", "Listing...\n"), None);
    }

    #[test]
    fn distros_are_recognised() {
        let arch = "NAME=\"Arch Linux\"\nID=arch\n";
        assert_eq!(detect_from(arch, false), Distro::Arch("Arch Linux".into()));
        let endeavour = "NAME='EndeavourOS'\nID=endeavouros\nID_LIKE=arch\n";
        assert_eq!(detect_from(endeavour, false), Distro::Arch("EndeavourOS".into()));
        let fedora = "NAME=\"Fedora Linux\"\nID=fedora\nVARIANT_ID=kde\n";
        assert_eq!(detect_from(fedora, false), Distro::Fedora("Fedora Linux".into()));
        let nobara = "NAME=\"Nobara Linux\"\nID=nobara\nID_LIKE=\"rhel centos fedora\"\n";
        assert!(matches!(detect_from(nobara, false), Distro::Fedora(_)));
        let mint = "NAME=\"Linux Mint\"\nID=linuxmint\nID_LIKE=\"ubuntu debian\"\n";
        assert_eq!(detect_from(mint, false), Distro::Debian("Linux Mint".into()));
        let silverblue = "NAME=\"Fedora Linux\"\nID=fedora\nVARIANT_ID=silverblue\n";
        assert!(matches!(detect_from(silverblue, false), Distro::Other(..)));
        assert!(matches!(detect_from(fedora, true), Distro::Other(..))); // any ostree system
        let gentoo = "NAME=Gentoo\nID=gentoo\n";
        assert!(matches!(detect_from(gentoo, false), Distro::Other(_, why) if why.contains("software center")));
    }

    #[test]
    fn every_supported_distro_has_an_install_script() {
        let arch = script_for(&Distro::Arch("Arch Linux".into())).unwrap();
        assert!(arch.contains("pacman -S --noconfirm tesseract") && !arch.contains("-Sy"));
        assert!(script_for(&Distro::Fedora("Fedora".into())).unwrap().contains("dnf install -y"));
        assert!(script_for(&Distro::Debian("Ubuntu".into())).unwrap().contains("APT::Status-Fd=1 tesseract-ocr"));
        assert_eq!(script_for(&Distro::Other("x".into(), "y".into())), None);
    }

    /// CI runs these files in clean Arch, Fedora and Ubuntu containers
    /// (.github/scripts/tesseract-install-test.sh): they must be what the app runs.
    #[test]
    fn the_ci_install_scripts_are_the_apps() {
        let file = |name: &str| {
            std::fs::read_to_string(format!("{}/.github/scripts/tesseract-install/{name}.sh", env!("CARGO_MANIFEST_DIR"))).unwrap()
        };
        assert_eq!(file("arch").trim_end(), script_for(&Distro::Arch(String::new())).unwrap());
        assert_eq!(file("fedora").trim_end(), script_for(&Distro::Fedora(String::new())).unwrap());
        assert_eq!(file("debian").trim_end(), script_for(&Distro::Debian(String::new())).unwrap());
    }

    #[test]
    fn apt_progress_is_read() {
        let mut p = Parser::new("apt");
        assert_eq!(p.feed("Need to get 4,567 kB of archives.", 0.0), None);
        let first = p.feed("dlstatus:1:10.0:Retrieving file 1 of 3", 0.0).unwrap();
        assert_eq!(first.fraction, Some(0.1));
        assert_eq!(first.stats, "0.5 of 4.6 MB");
        let later = p.feed("dlstatus:2:60.0:Retrieving file 2 of 3", 1.0).unwrap();
        assert!(later.stats.starts_with("2.7 of 4.6 MB · 2.3 MB/s · about 1 s left"), "{}", later.stats);
        let installing = p.feed("pmstatus:tesseract-ocr:50:Installing tesseract-ocr (amd64)", 2.0).unwrap();
        assert_eq!((installing.fraction, installing.stage.as_str()), (None, "Installing tesseract-ocr (amd64)"));
    }

    #[test]
    fn pacman_progress_prefers_the_overall_line() {
        let mut p = Parser::new("pacman");
        p.feed("Total Download Size:    11.06 MiB", 0.0);
        let one = p.feed(" tesseract-5.3.4-1-x86_64    2.1 MiB  1.5 MiB/s 00:01 [###---]  40%", 0.5).unwrap();
        assert_eq!(one.fraction, Some(0.4));
        let total = p.feed("\u{1b}[1m Total (1/3)   4.0 MiB  2.0 MiB/s 00:03 [####--]  36%\u{1b}[0m", 1.0).unwrap();
        assert_eq!(total.fraction, Some(0.36));
        // a package's own bar after the overall one doesn't move the bar back
        assert_eq!(p.feed(" leptonica-1.84.1-1-x86_64  0.2 MiB  1.0 MiB/s 00:01 [#-----]  10%", 1.2), None);
        let installing = p.feed("(3/3) installing tesseract", 2.0).unwrap();
        assert_eq!(installing.stage, "Installing (pacman)");
    }

    #[test]
    fn dnf5_progress_is_read() {
        let mut p = Parser::new("dnf");
        p.feed("Total size of inbound packages is 8 MiB. Need to download 8 MiB.", 0.0);
        let line = "[1/7] Total                               25% |  75.0 KiB/s |  45.3 KiB |  01m46s\u{1b}]9;4;1;0\u{1b}\\\u{1b}[3A";
        let progress = p.feed(line, 1.0).unwrap();
        assert_eq!(progress.fraction, Some(0.25));
        assert_eq!(progress.stats, "2.1 of 8.4 MB");
        assert_eq!(strip_ansi("a\u{1b}]9;4;1;0\u{1b}\\b\u{1b}[0Jc"), "abc");
    }

    #[test]
    fn sizes_are_read_in_any_unit() {
        assert_eq!(size_after("Total download size: 12 M", "Total download size:"), Some(12.0 * 1024.0 * 1024.0));
        assert_eq!(size_after("Need to download 2.5MiB.", "Need to download"), Some(2.5 * 1024.0 * 1024.0));
        assert_eq!(size_after("Need to get 1,234 kB of archives.", "Need to get"), Some(1_234_000.0));
    }
}
