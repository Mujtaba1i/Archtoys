//! Image Resizer in the file managers' right-click menus. Dolphin and Thunar
//! get a submenu with the quick actions; Nautilus (as a script, under
//! Scripts) and Nemo get one "Image Resizer..." entry that opens Archtoys'
//! quick panel. Each runs `archtoys --resize ACTION FILES`. Turning the
//! Micro-toy off removes them all.

use std::path::{Path, PathBuf};

/// The quick actions: (id for `--resize`, menu text). None = a separator.
pub const QUICK: [Option<(&str, &str)>; 14] = [
    Some(("r25", "Resize to 25%")),
    Some(("r50", "Resize to 50%")),
    Some(("r150", "Resize to 150%")),
    Some(("r200", "Resize to 200%")),
    Some(("fit1920", "Fit in 1920 px")),
    Some(("fit1280", "Fit in 1280 px")),
    None,
    Some(("png", "Convert to PNG")),
    Some(("jpg", "Convert to JPG")),
    Some(("webp", "Convert to WebP")),
    None,
    Some(("smaller", "Make smaller")),
    None,
    Some(("custom", "Custom...")),
];

const DOLPHIN: &str = "kio/servicemenus/archtoys-image-resizer.desktop";
const NEMO: &str = "nemo/actions/archtoys-image-resizer.nemo_action";
const NAUTILUS: &str = "nautilus/scripts/Image Resizer...";
const THUNAR_ID: &str = "archtoys-resize";

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share"))
}

fn thunar_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map(PathBuf::from);
    config.unwrap_or_else(|| home().join(".config")).join("Thunar/uca.xml")
}

/// What starts Archtoys from the menu: the AppImage itself (its path stays
/// when it updates), else this binary.
fn launch_command() -> Vec<String> {
    let program = std::env::var("APPIMAGE")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::current_exe().ok().map(|p| p.to_string_lossy().to_string()))
        .unwrap_or_else(|| "archtoys".into());
    vec![program]
}

/// The command line that starts Archtoys, for a shell.
fn shell_words(program: &[String]) -> String {
    program.iter().map(|w| shell_quote(w)).collect::<Vec<_>>().join(" ")
}

/// For a shell (Thunar, Nemo, the Nautilus script): one word, whatever it holds.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// For a .desktop Exec line (Dolphin): the spec's quoting, then the key file's
/// own escaping of backslashes.
fn desktop_quote(s: &str) -> String {
    let mut q = String::from("\"");
    for c in s.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            q.push('\\');
        }
        q.push(c);
    }
    q.push('"');
    q.replace('\\', "\\\\").replace('%', "%%")
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The actions in menu order: the quick ones, the presets before Custom...
fn actions(presets: &[String]) -> Vec<Option<(String, String)>> {
    let mut list: Vec<Option<(String, String)>> = Vec::new();
    for item in QUICK {
        if let Some(("custom", _)) = item {
            for (i, name) in presets.iter().enumerate() {
                list.push(Some((format!("preset:{i}"), name.clone())));
            }
            if !presets.is_empty() {
                list.push(None);
            }
        }
        list.push(item.map(|(id, text)| (id.to_string(), text.to_string())));
    }
    list
}

pub fn dolphin_file(program: &[String], presets: &[String]) -> String {
    let list = actions(presets);
    let ids: Vec<String> = list
        .iter()
        .enumerate()
        .map(|(i, a)| match a {
            Some(_) => format!("a{i}"),
            None => "_SEPARATOR_".into(),
        })
        .collect();
    let mut s = format!(
        "# Image Resizer, added by Archtoys. Turning it off in Archtoys removes this file.\n\
         [Desktop Entry]\nType=Service\nMimeType=image/*;\nX-KDE-ServiceTypes=KonqPopupMenu/Plugin\n\
         X-KDE-Submenu=Image Resizer\nIcon=archtoys\nActions={};\n",
        ids.join(";")
    );
    for (i, a) in list.iter().enumerate() {
        if let Some((id, text)) = a {
            s.push_str(&format!(
                "\n[Desktop Action a{i}]\nName={text}\nIcon=archtoys\nExec={} --resize {} %F\n",
                program.iter().map(|w| desktop_quote(w)).collect::<Vec<_>>().join(" "),
                desktop_quote(id)
            ));
        }
    }
    s
}

pub fn thunar_actions(program: &[String], presets: &[String]) -> String {
    let mut s = String::new();
    for (id, text) in actions(presets).into_iter().flatten() {
        s.push_str(&format!(
            "<action>\n\t<icon>archtoys</icon>\n\t<name>{}</name>\n\t<submenu>Image Resizer</submenu>\n\t<unique-id>{THUNAR_ID}-{}</unique-id>\n\t\
             <command>{} --resize {} %F</command>\n\t<description>Image Resizer (Archtoys)</description>\n\t<range>*</range>\n\t\
             <patterns>*</patterns>\n\t<image-files/>\n</action>\n",
            xml_escape(&text),
            xml_escape(&id.replace(':', "-")),
            xml_escape(&shell_words(program)),
            xml_escape(&shell_quote(&id))
        ));
    }
    s
}

/// uca.xml without Archtoys' actions. Everything else stays as it was.
pub fn thunar_remove(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<action>") {
        let Some(len) = rest[start..].find("</action>") else { break };
        let end = start + len + "</action>".len();
        out.push_str(&rest[..start]);
        if !rest[start..end].contains(&format!("<unique-id>{THUNAR_ID}-")) {
            out.push_str(&rest[start..end]);
        } else {
            // its line break too
            let after = &rest[end..];
            let trimmed = after.trim_start_matches([' ', '\t']).strip_prefix('\n').unwrap_or(after);
            rest = trimmed;
            continue;
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// uca.xml with Archtoys' actions (replacing older ones) before `</actions>`.
pub fn thunar_merge(xml: &str, actions: &str) -> String {
    let base = if xml.contains("</actions>") {
        thunar_remove(xml)
    } else {
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n</actions>\n".into()
    };
    let at = base.rfind("</actions>").unwrap_or(base.len());
    format!("{}{actions}{}", &base[..at], &base[at..])
}

pub fn nautilus_script(program: &[String]) -> String {
    format!(
        "#!/bin/sh\n# Image Resizer, added by Archtoys. Turning it off in Archtoys removes this file.\n\
         # Nautilus gives the selected files one per line.\nset -f\nIFS='\n'\n\
         if [ -n \"$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\" ]; then\n  exec {p} --resize panel $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS\nfi\n\
         exec {p} --resize panel \"$@\"\n",
        p = shell_words(program)
    )
}

pub fn nemo_action(program: &[String]) -> String {
    format!(
        "# Image Resizer, added by Archtoys. Turning it off in Archtoys removes this file.\n[Nemo Action]\n\
         Name=Image Resizer...\nComment=Resize or convert the selected pictures\nExec={} --resize panel %F\n\
         Icon-Name=archtoys\nSelection=NotNone\nMimetypes=image/*;\n",
        shell_words(program)
    )
}

fn installed(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|d| d.join(program).is_file()))
}

fn write(path: &Path, text: &str, executable: bool) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    crate::config::write_atomic(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))?;
    if executable {
        use std::os::unix::fs::PermissionsExt;
        // Dolphin (Plasma 6) only shows service menus that may run
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Adds the menus of the file managers this computer has (all of them when it
/// has none of them, so one installed later finds them). The names it added.
pub fn install(presets: &[String]) -> Result<Vec<&'static str>, String> {
    let program = launch_command();
    let found: Vec<&str> = ["dolphin", "thunar", "nautilus", "nemo"].into_iter().filter(|p| installed(p)).collect();
    let want = |p: &str| found.is_empty() || found.contains(&p);
    let mut added = Vec::new();
    if want("dolphin") {
        write(&data_dir().join(DOLPHIN), &dolphin_file(&program, presets), true)?;
        added.push("Dolphin");
    }
    if want("thunar") {
        let path = thunar_file();
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        write(&path, &thunar_merge(&old, &thunar_actions(&program, presets)), false)?;
        added.push("Thunar");
    }
    if want("nautilus") {
        write(&data_dir().join(NAUTILUS), &nautilus_script(&program), true)?;
        added.push("Nautilus");
    }
    if want("nemo") {
        write(&data_dir().join(NEMO), &nemo_action(&program), false)?;
        added.push("Nemo");
    }
    Ok(added)
}

/// Takes Archtoys' entries out of every file manager's menu.
pub fn remove() {
    for file in [DOLPHIN, NAUTILUS, NEMO] {
        let _ = std::fs::remove_file(data_dir().join(file));
    }
    let path = thunar_file();
    if let Ok(old) = std::fs::read_to_string(&path) {
        let new = thunar_remove(&old);
        if new != old {
            let _ = crate::config::write_atomic(&path, new.as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dolphin_gets_a_submenu_with_separators_and_presets_before_custom() {
        let f = dolphin_file(&["/opt/My Apps/archtoys".into()], &["Web 800".into()]);
        assert!(f.contains("X-KDE-Submenu=Image Resizer"));
        assert!(f.contains("Actions=a0;a1;a2;a3;a4;a5;_SEPARATOR_;a7;a8;a9;_SEPARATOR_;a11;_SEPARATOR_;a13;_SEPARATOR_;a15;"), "{f}");
        assert!(f.contains("Name=Resize to 25%\nIcon=archtoys\nExec=\"/opt/My Apps/archtoys\" --resize \"r25\" %F"));
        let preset = f.find("Name=Web 800").unwrap();
        assert!(preset < f.find("Name=Custom...").unwrap());
        assert!(f.contains("--resize \"preset:0\" %F"));
        // no presets: no preset part, no extra separator
        assert!(!dolphin_file(&["archtoys".into()], &[]).contains("preset"));
    }

    #[test]
    fn desktop_quoting_escapes_twice() {
        assert_eq!(desktop_quote("/a b/arch$toys"), "\"/a b/arch\\\\$toys\"");
        assert_eq!(desktop_quote("100%"), "\"100%%\"");
    }

    #[test]
    fn thunar_keeps_the_user_s_own_actions() {
        let user = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<actions>\n<action>\n\t<name>Open Terminal Here</name>\n\t<unique-id>1-1</unique-id>\n</action>\n</actions>\n";
        let merged = thunar_merge(user, &thunar_actions(&["/usr/bin/archtoys".into()], &[]));
        assert!(merged.contains("Open Terminal Here"));
        assert!(merged.contains("<name>Resize to 25%</name>\n\t<submenu>Image Resizer</submenu>"));
        assert!(merged.contains("<command>'/usr/bin/archtoys' --resize 'r25' %F</command>"));
        // again: replaced, not doubled
        let twice = thunar_merge(&merged, &thunar_actions(&["/usr/bin/archtoys".into()], &[]));
        assert_eq!(twice.matches("Resize to 25%").count(), 1);
        assert_eq!(thunar_remove(&merged), user);
        // no file yet
        assert!(thunar_merge("", "<action>x</action>\n").contains("<actions>\n<action>x</action>\n</actions>"));
    }

    #[test]
    fn scripts_quote_the_program() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert!(nautilus_script(&["/x/archtoys".into()]).contains("exec '/x/archtoys' --resize panel $NAUTILUS_SCRIPT_SELECTED_FILE_PATHS"));
        assert!(nemo_action(&["/x/archtoys".into()]).contains("Exec='/x/archtoys' --resize panel %F"));
        // a Flatpak: the desktop starts it through flatpak run
        let flatpak: Vec<String> = ["flatpak", "run", "--command=archtoys", "io.github.Mujtaba1i.Archtoys"].map(String::from).to_vec();
        assert!(dolphin_file(&flatpak, &[])
            .contains("Exec=\"flatpak\" \"run\" \"--command=archtoys\" \"io.github.Mujtaba1i.Archtoys\" --resize \"r25\" %F"));
    }
}
