//! The fonts on this computer, as fontconfig lists them (every desktop has
//! it): one Regular and one Bold file per family. They're compared every time,
//! with no download: most text on a screen is in one of them.

use std::path::PathBuf;
use std::process::Command;

pub(crate) struct LocalFont {
    pub family: String,
    pub path: PathBuf,
    /// this file is the family's Bold (a variable font draws Bold from its axis instead)
    pub bold: bool,
}

pub(crate) fn fonts() -> Vec<LocalFont> {
    let Ok(out) = Command::new("fc-list").args([":", "file", "family", "style"]).output() else {
        return Vec::new();
    };
    parse(&String::from_utf8_lossy(&out.stdout))
}

/// fc-list's lines ("/path/Font.ttf: Family,Other:style=Bold,Fett") → one
/// Regular and one Bold per family; italics, other weights and bitmap files left out.
fn parse(text: &str) -> Vec<LocalFont> {
    let mut lines: Vec<&str> = text.lines().collect();
    lines.sort_unstable();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in lines {
        let Some((path, rest)) = line.split_once(": ") else { continue };
        let (family, style) = rest.split_once(":style=").unwrap_or((rest, "Regular"));
        let style = style.split(',').next().unwrap_or("").trim().to_ascii_lowercase();
        if !(path.ends_with(".ttf") || path.ends_with(".otf") || path.ends_with(".ttc")) {
            continue;
        }
        let bold = style == "bold";
        if !(bold || matches!(style.as_str(), "regular" | "book" | "roman" | "normal" | "")) {
            continue;
        }
        let family = family.split(',').next().unwrap_or("").trim().to_string();
        if family.is_empty() || other_script(&family) || !seen.insert((family.to_lowercase(), bold)) {
            continue;
        }
        out.push(LocalFont { family, path: PathBuf::from(path), bold });
    }
    out
}

/// Noto families made for another script (Noto Sans Arabic, Noto Sans CJK,
/// Noto Sans Math...) carry a copy of Noto Sans's Latin letters: they'd only
/// crowd the guesses. The Latin ones stay.
pub(crate) fn other_script(family: &str) -> bool {
    let f = family.to_lowercase();
    const LATIN_NOTO: &[&str] = &["noto sans", "noto serif", "noto sans mono", "noto mono", "noto sans display", "noto serif display"];
    f.starts_with("noto ") && !LATIN_NOTO.contains(&f.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_regular_and_one_bold_per_family() {
        let text = "/f/DejaVuSans.ttf: DejaVu Sans:style=Book\n\
                    /f/DejaVuSans-Bold.ttf: DejaVu Sans:style=Bold\n\
                    /f/DejaVuSans-Oblique.ttf: DejaVu Sans:style=Oblique\n\
                    /f/Noto-Light.ttf: Noto Sans,Noto Sans Light:style=Light,Regular\n\
                    /f/Noto.ttf: Noto Sans:style=Regular\n\
                    /f/Noto2.ttf: Noto Sans:style=Regular\n\
                    /f/fixed.pcf.gz: Fixed:style=Regular";
        let fonts = parse(text);
        let mut names: Vec<(String, bool)> = fonts.iter().map(|f| (f.family.clone(), f.bold)).collect();
        names.sort();
        assert_eq!(names, vec![("DejaVu Sans".into(), false), ("DejaVu Sans".into(), true), ("Noto Sans".into(), false)]);
        assert_eq!(fonts.iter().find(|f| f.family == "Noto Sans").unwrap().path, PathBuf::from("/f/Noto.ttf"));
        // Noto for other scripts is left out, Noto for Latin stays
        assert!(other_script("Noto Sans Math") && other_script("Noto Sans CJK JP") && !other_script("Noto Sans Mono"));
    }
}
