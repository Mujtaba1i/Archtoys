//! Updates (v0.8.1): how Archtoys was installed (`installed.rs`), the daily
//! check, and updating the way it was installed.
//!
//! The check asks GitHub for the newest release (its version, notes and
//! files), once after the start and then once a day, on a background thread.
//! No update, no network or any error: nothing is shown. Updating with a
//! package manager or the .deb is one script run through pkexec, so the
//! password is asked first; the AppImage replaces its own file (no password).

pub mod flatpak;
pub mod installed;
pub mod page;

use installed::Installed;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub const REPO: &str = "Mujtaba1i/Archtoys";

/// What the newest release says about itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// "0.8.1"
    pub version: String,
    pub page: String,
    pub notes: Vec<String>,
    /// (file name, download link)
    pub assets: Vec<(String, String)>,
}

impl Release {
    fn asset(&self, name: &str) -> Option<&str> {
        self.assets.iter().find(|(n, _)| n == name).map(|(_, url)| url.as_str())
    }
}

/// The running file, remembered at the start: after a package update
/// /proc/self/exe points at the deleted old file.
pub fn remember_exe() {
    exe();
}

fn exe() -> &'static PathBuf {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().and_then(|p| p.canonicalize()).unwrap_or_else(|_| PathBuf::from("archtoys")))
}

/// "v0.8.1" or "0.8.1" as numbers.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u64>().ok());
    let v = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(v)
}

/// Is `latest` newer than `current`?
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// GitHub's answer for the newest release.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v["tag_name"].as_str()?;
    parse_version(tag)?;
    let assets = v["assets"]
        .as_array()
        .map(|list| {
            list.iter().filter_map(|a| Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string()))).collect()
        })
        .unwrap_or_default();
    Some(Release {
        version: tag.trim_start_matches('v').to_string(),
        page: v["html_url"].as_str().unwrap_or("").to_string(),
        notes: notes(v["body"].as_str().unwrap_or("")),
        assets,
    })
}

/// The release notes' first changes, as plain text (no Markdown): the list
/// before the folded Dependabot panel.
fn notes(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("<details") {
            break;
        }
        let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) else { continue };
        let plain = strip_markdown(item);
        if !plain.is_empty() {
            out.push(plain);
        }
        if out.len() == 4 {
            break;
        }
    }
    out
}

/// "**New** [link](url) `code`" -> "New link code"
fn strip_markdown(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        match rest[start..].find("](").and_then(|mid| rest[start + mid..].find(')').map(|end| (mid, start + mid + end))) {
            Some((mid, end)) => {
                out.push_str(&rest[..start]);
                out.push_str(&rest[start + 1..start + mid]);
                rest = &rest[end + 1..];
            }
            None => break,
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "").trim().to_string()
}

/// The SHA-256 SHA256SUMS gives for `file`.
fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == file && hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
    })
}

/// Asks GitHub for the newest release. Blocking: call it off the UI thread.
/// ARCHTOYS_UPDATE_API replaces the address (tests: a file:// release).
pub fn latest() -> Result<Release, String> {
    let url = std::env::var("ARCHTOYS_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let json = crate::fontfind::net::text(&url)?;
    parse_release(&json).ok_or_else(|| "GitHub's answer had no version".to_string())
}

/// The newer release, if there is one the user didn't skip.
pub fn available(skipped: &str) -> Option<Release> {
    let release = latest().ok()?;
    (is_newer(&release.version, env!("CARGO_PKG_VERSION")) && release.version != skipped).then_some(release)
}

/// Can Archtoys install this update itself (else: the release page)?
pub fn can_install(how: &Installed) -> bool {
    match how {
        // through Flatpak's portal, from Flathub
        Installed::Flatpak => flatpak::ready(),
        _ => !matches!(how, Installed::Other),
    }
}

/// The steps the dialog lists for this kind of install.
pub fn steps(how: &Installed) -> Vec<&'static str> {
    match how {
        Installed::Aur(_) => vec!["Downloading", "Building from source", "Installing"],
        Installed::AurBin => vec!["Downloading", "Preparing", "Installing"],
        Installed::Rpm => vec!["Downloading and installing"],
        Installed::Deb => vec!["Downloading", "Checking the download", "Installing"],
        Installed::Apt => vec!["Checking the package lists", "Installing"],
        Installed::AppImage(_) => vec!["Downloading", "Checking the download", "Replacing the AppImage"],
        Installed::Flatpak => vec!["Downloading and installing"],
        Installed::Other => vec![],
    }
}

/// A user name safe to put in a script (makepkg runs as that user, never root).
fn safe_user(name: &str) -> Option<&str> {
    let ok = !name.is_empty()
        && name.len() <= 32
        && name != "root"
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        && !name.starts_with('-');
    ok.then_some(name)
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The root script that updates a package install. `@@step N` lines move the
/// dialog to step N; curl's progress bar ("###  45.3%") gives the percent.
/// Exit 3: the package source doesn't have the new version yet.
pub fn script(how: &Installed, release: &Release, user: &str, deb_sum: Option<&str>) -> Result<String, String> {
    let v = &release.version;
    match how {
        Installed::Aur(_) | Installed::AurBin => {
            let name = match how {
                Installed::Aur(n) => n.as_str(),
                _ => "archtoys-bin",
            };
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err("unexpected package name".into());
            }
            let user = safe_user(user).ok_or("Archtoys can't build as this user")?;
            Ok(format!(
                r#"set -e
d=$(mktemp -d)
trap 'rm -rf "$d"' EXIT
chown {user} "$d"
echo "@@step 0"
runuser -u {user} -- git clone --quiet --depth 1 https://aur.archlinux.org/{name}.git "$d/{name}"
cd "$d/{name}"
[ "$(sed -n 's/^pkgver=//p' PKGBUILD)" = {v} ] || exit 3
echo "@@step 1"
deps=$(runuser -u {user} -- makepkg --printsrcinfo | sed -n 's/^[[:space:]]*\(make\)\{{0,1\}}depends = //p' | sed 's/[<>=].*//')
[ -z "$deps" ] || pacman -S --needed --noconfirm --asdeps $deps
runuser -u {user} -- makepkg --noconfirm --force
echo "@@step 2"
pacman -U --noconfirm $(ls ./*.pkg.tar.* | grep -v -- '-debug-')
"#
            ))
        }
        Installed::Rpm => Ok(format!(
            r#"set -e
echo "@@step 0"
dnf upgrade -y --refresh archtoys
[ "$(rpm -q --qf '%{{VERSION}}' archtoys)" = {v} ] || exit 3
"#
        )),
        Installed::Deb => {
            let file = format!("archtoys_{v}_amd64.deb");
            let url = release.asset(&file).ok_or("the release has no .deb")?;
            let sum = deb_sum.ok_or("the release has no SHA256SUMS line for the .deb")?;
            Ok(format!(
                r#"set -e
f=$(mktemp --suffix=.deb)
trap 'rm -f "$f"' EXIT
echo "@@step 0"
curl -fL --progress-bar --connect-timeout 15 -o "$f" {url} 2>&1
echo "@@step 1"
echo "{sum}  $f" | sha256sum -c --quiet -
echo "@@step 2"
chmod 644 "$f"
DEBIAN_FRONTEND=noninteractive apt-get install -y "$f"
"#,
                url = quote(url)
            ))
        }
        // the APT repository: apt checks its signature and the package's hash itself
        Installed::Apt => Ok(format!(
            r#"set -e
export DEBIAN_FRONTEND=noninteractive
echo "@@step 0"
apt-get update
echo "@@step 1"
apt-get install -y --only-upgrade archtoys
[ "$(dpkg-query -W -f='${{Version}}' archtoys)" = {v} ] || exit 3
"#
        )),
        _ => Err("Archtoys can't update this install itself".into()),
    }
}

/// Progress from one line of the script's output.
#[derive(Debug, PartialEq)]
pub enum Event {
    Step(usize),
    Percent(f32),
}

pub fn parse_line(line: &str) -> Option<Event> {
    let line = line.trim();
    if let Some(n) = line.strip_prefix("@@step ") {
        return n.trim().parse().ok().map(Event::Step);
    }
    // curl's bar: "######        45.3%" (only curl's: apt's "100%" lines aren't the download)
    if !line.starts_with('#') {
        return None;
    }
    let pct = line.strip_suffix('%')?.rsplit([' ', '#']).next()?;
    pct.parse::<f32>().ok().filter(|p| (0.0..=100.0).contains(p)).map(|p| Event::Percent(p / 100.0))
}

/// Updates this install. Blocking: call it on a background thread.
/// `on_event` gets each step and percent (on that thread).
pub fn run(how: &Installed, release: &Release, on_event: &dyn Fn(Event)) -> Result<(), String> {
    if let Installed::AppImage(file) = how {
        return replace_appimage(file, release, on_event);
    }
    if *how == Installed::Flatpak {
        return flatpak::update(on_event);
    }
    let deb_sum = match how {
        Installed::Deb => {
            let sums = release.asset("SHA256SUMS").ok_or("the release has no SHA256SUMS")?;
            sum_for(&crate::fontfind::net::text(sums)?, &format!("archtoys_{}_amd64.deb", release.version))
        }
        _ => None,
    };
    let user = std::env::var("ARCHTOYS_UPDATE_USER")
        .ok()
        .or_else(|| Command::new("id").arg("-un").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()))
        .unwrap_or_default();
    let text = script(how, release, &user, deb_sum.as_deref())?;
    // already root (the package checks run it so): no password to ask
    let root = Command::new("id").arg("-u").output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0");
    let mut command = if root { Command::new("/bin/sh") } else { Command::new("pkexec") };
    if !root {
        command.arg("/bin/sh");
    }
    let mut child =
        command.args(["-c", &text]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| {
            match e.kind() {
                std::io::ErrorKind::NotFound => "pkexec isn't installed, so Archtoys can't ask for your password".to_string(),
                _ => e.to_string(),
            }
        })?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut line = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = stdout.read(&mut buf) {
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if let Some(event) = parse_line(&String::from_utf8_lossy(&line)) {
                    on_event(event);
                }
                line.clear();
            } else {
                line.push(b);
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let errors = errors.join().unwrap_or_default();
    match status.code() {
        Some(0) => Ok(()),
        Some(3) => Err(format!("Archtoys {} isn't in your package source yet. Try again later.", release.version)),
        Some(126) | Some(127) if !root => Err("The password wasn't given, so nothing was changed.".into()),
        _ => {
            let last = errors.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("no details");
            Err(format!("Updating didn't work: {last}"))
        }
    }
}

/// Downloads the new AppImage next to the old one, checks it against
/// SHA256SUMS and puts it in its place. The file is the user's: no password.
fn replace_appimage(file: &std::path::Path, release: &Release, on_event: &dyn Fn(Event)) -> Result<(), String> {
    let name = format!("Archtoys-{}-x86_64.AppImage", release.version);
    let url = release.asset(&name).ok_or("the release has no AppImage")?;
    let sums = release.asset("SHA256SUMS").ok_or("the release has no SHA256SUMS")?;
    let sum = sum_for(&crate::fontfind::net::text(sums)?, &name).ok_or("SHA256SUMS doesn't list the AppImage")?;
    let dir = file.parent().ok_or("the AppImage has no folder")?;
    let new = dir.join(".archtoys-update.AppImage");
    on_event(Event::Step(0));
    let stop = std::sync::atomic::AtomicBool::new(false);
    let progress = |got: u64, total: Option<u64>, _speed: f64| {
        if let Some(total) = total.filter(|t| *t > 0) {
            on_event(Event::Percent(got as f32 / total as f32));
        }
    };
    crate::fontfind::net::download(url, &new, &crate::verify::Expected::Sha256(sum), &progress, &stop)
        .map_err(|e| format!("Updating didn't work: {e}"))?;
    on_event(Event::Step(1));
    on_event(Event::Step(2));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(file).map(|m| m.permissions().mode()).unwrap_or(0o755) | 0o111;
    std::fs::set_permissions(&new, std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    std::fs::rename(&new, file).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        format!("Updating didn't work: {e}")
    })
}

/// Starts the updated Archtoys in place of this one (exec: the one-copy lock
/// and the D-Bus name close with this process, so the new one takes them).
pub fn restart(how: &Installed) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let target = match how {
        Installed::Flatpak => return flatpak::restart(),
        Installed::AppImage(file) => file.clone(),
        _ => exe().clone(),
    };
    Command::new(target).exec()
}

/// `archtoys --update [--dry-run]`: updates from a terminal (and in the
/// package checks), printing each step. Exit 0 also when there's nothing newer.
pub fn cli(dry_run: bool) -> i32 {
    let how = installed::installed();
    println!("Archtoys {} ({})", env!("CARGO_PKG_VERSION"), how.label());
    let release = match latest() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Couldn't check for updates: {e}");
            return 1;
        }
    };
    if !is_newer(&release.version, env!("CARGO_PKG_VERSION")) {
        println!("Up to date (the newest is {}).", release.version);
        return 0;
    }
    println!("Archtoys {} is out.", release.version);
    if !can_install(how) {
        println!("Download it from {}", release.page);
        return 0;
    }
    if dry_run {
        let sum = matches!(how, Installed::Deb).then_some("0".repeat(64));
        match script(how, &release, &std::env::var("USER").unwrap_or_default(), sum.as_deref()) {
            Ok(text) => println!("{text}"),
            Err(e) => println!("{e}"),
        }
        return 0;
    }
    let steps = steps(how);
    let result = run(how, &release, &|event| match event {
        Event::Step(i) => println!("{}", steps.get(i).copied().unwrap_or("Working")),
        Event::Percent(_) => {}
    });
    match result {
        Ok(()) => {
            println!("Updated to {}.", release.version);
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release() -> Release {
        Release {
            version: "0.8.1".into(),
            page: "https://github.com/Mujtaba1i/Archtoys/releases/tag/v0.8.1".into(),
            notes: vec![],
            assets: vec![
                ("archtoys_0.8.1_amd64.deb".into(), "https://example.com/archtoys_0.8.1_amd64.deb".into()),
                ("SHA256SUMS".into(), "https://example.com/SHA256SUMS".into()),
            ],
        }
    }

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("v0.8.1", "0.8.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("0.8.0", "0.8.0"));
        assert!(!is_newer("0.7.9", "0.8.0"));
        assert!(!is_newer("v0.8.1-rc1", "0.8.0"));
        assert!(!is_newer("nonsense", "0.8.0"));
    }

    #[test]
    fn github_release_is_read() {
        let json = r###"{"tag_name":"v0.8.1","html_url":"https://github.com/x/releases/tag/v0.8.1",
            "body":"## Changes\n- **NEW!** Updates from inside [Archtoys](https://x)\n- CI: `faster`\n<details><summary>Dependabot</summary>\n- Bump x\n</details>",
            "assets":[{"name":"SHA256SUMS","browser_download_url":"https://x/SHA256SUMS"}]}"###;
        let r = parse_release(json).unwrap();
        assert_eq!(r.version, "0.8.1");
        assert_eq!(r.notes, vec!["NEW! Updates from inside Archtoys", "CI: faster"]);
        assert_eq!(r.asset("SHA256SUMS"), Some("https://x/SHA256SUMS"));
        assert_eq!(parse_release(r#"{"tag_name":"latest"}"#), None);
        assert_eq!(parse_release("not json"), None);
    }

    #[test]
    fn sha256sums_line_is_found() {
        let a = "a".repeat(64);
        let sums = format!("{a}  archtoys_0.8.1_amd64.deb\n{}  Archtoys-0.8.1-x86_64.AppImage\n", "b".repeat(64));
        assert_eq!(sum_for(&sums, "archtoys_0.8.1_amd64.deb"), Some(a));
        assert_eq!(sum_for(&sums, "archtoys_0.8.0_amd64.deb"), None);
        assert_eq!(sum_for("short  archtoys_0.8.1_amd64.deb", "archtoys_0.8.1_amd64.deb"), None);
    }

    #[test]
    fn progress_lines_are_read() {
        assert_eq!(parse_line("@@step 2"), Some(Event::Step(2)));
        assert_eq!(parse_line("###########          50.0%"), Some(Event::Percent(0.5)));
        assert_eq!(parse_line("#=#=#"), None);
        assert_eq!(parse_line("Reading package lists... 100%"), None);
        assert_eq!(parse_line("resolving dependencies"), None);
    }

    #[test]
    fn deb_script_checks_the_download_before_installing() {
        let sum = "c".repeat(64);
        let s = script(&Installed::Deb, &release(), "me", Some(&sum)).unwrap();
        let check = s.find("sha256sum -c").unwrap();
        assert!(check < s.find("apt-get install").unwrap());
        assert!(s.contains("trap 'rm -f \"$f\"' EXIT"), "the downloaded .deb is always deleted");
        assert!(s.contains("'https://example.com/archtoys_0.8.1_amd64.deb'"));
        assert!(script(&Installed::Deb, &release(), "me", None).is_err());
    }

    #[test]
    fn aur_builds_as_the_user_and_installs_as_root() {
        let s = script(&Installed::Aur("archtoys".into()), &release(), "mujtaba", None).unwrap();
        assert!(s.contains("runuser -u mujtaba -- makepkg"));
        assert!(s.contains("aur.archlinux.org/archtoys.git"));
        assert!(s.contains("= 0.8.1 ] || exit 3"));
        assert!(s.find("makepkg --noconfirm").unwrap() < s.find("pacman -U").unwrap());
        let bin = script(&Installed::AurBin, &release(), "mujtaba", None).unwrap();
        assert!(bin.contains("aur.archlinux.org/archtoys-bin.git"));
        // never root, never something that could break the script
        assert!(script(&Installed::AurBin, &release(), "root", None).is_err());
        assert!(script(&Installed::AurBin, &release(), "me; rm -rf /", None).is_err());
        assert!(script(&Installed::AurBin, &release(), "", None).is_err());
    }

    #[test]
    fn rpm_checks_it_got_the_new_version() {
        let s = script(&Installed::Rpm, &release(), "me", None).unwrap();
        assert!(s.contains("dnf upgrade -y --refresh archtoys"));
        assert!(s.contains("= 0.8.1 ] || exit 3"));
    }

    #[test]
    fn apt_upgrades_only_archtoys_and_checks_the_version() {
        let s = script(&Installed::Apt, &release(), "me", None).unwrap();
        assert!(s.contains("apt-get install -y --only-upgrade archtoys"));
        assert!(s.contains("dpkg-query -W -f='${Version}' archtoys)\" = 0.8.1 ] || exit 3"), "{s}");
        assert_eq!(steps(&Installed::Apt).len(), 2);
        assert!(can_install(&Installed::Apt));
    }

    #[test]
    fn only_the_tarball_and_flatpak_need_the_release_page() {
        assert!(can_install(&Installed::Deb));
        assert!(can_install(&Installed::AppImage("/x".into())));
        assert!(!can_install(&Installed::Other));
        assert!(!can_install(&Installed::Flatpak));
        assert!(script(&Installed::Other, &release(), "me", None).is_err());
    }
}

#[cfg(test)]
mod script_syntax {
    use super::*;

    /// Every generated script is valid shell (`sh -n`).
    #[test]
    fn scripts_parse() {
        let mut r = tests_release();
        r.assets.push(("archtoys_0.8.1_amd64.deb".into(), "https://example.com/a.deb".into()));
        let sum = "d".repeat(64);
        for how in [Installed::Aur("archtoys".into()), Installed::AurBin, Installed::Rpm, Installed::Deb] {
            let text = script(&how, &r, "builder", Some(&sum)).unwrap();
            let out = Command::new("sh").args(["-n", "-c", &text]).output().unwrap();
            assert!(out.status.success(), "{how:?}: {}", String::from_utf8_lossy(&out.stderr));
        }
    }

    fn tests_release() -> Release {
        Release { version: "0.8.1".into(), page: String::new(), notes: vec![], assets: vec![] }
    }
}
