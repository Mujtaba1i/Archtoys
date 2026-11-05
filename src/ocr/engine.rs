//! Running Tesseract (the `tesseract` program, installed from the system's
//! packages) and reading what it found. Archtoys never links Tesseract: it
//! starts it for each capture, so its memory is gone when it ends, a crash in
//! it can't take the app down, and Archtoys starts fine without it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The oldest Tesseract Archtoys works with: Ubuntu 22.04 (the oldest system
/// the releases support) ships 4.1.1.
pub(crate) const MIN_VERSION: (u32, u32) = (4, 1);

/// "tesseract 5.3.4", "tesseract v4.1.1-rc2" ... → (5, 3, 4).
pub(crate) fn parse_version(output: &str) -> Option<(u32, u32, u32)> {
    let line = output.lines().find(|l| l.trim_start().to_ascii_lowercase().starts_with("tesseract"))?;
    let version = line.split_whitespace().nth(1)?.trim_start_matches('v');
    let mut parts = version.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

pub(crate) fn version_ok((major, minor, _): (u32, u32, u32)) -> bool {
    (major, minor) >= MIN_VERSION
}

fn output_text(out: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
}

/// The installed Tesseract's version, or None when there is none.
pub(crate) fn installed_version() -> Option<(u32, u32, u32)> {
    let out = Command::new("tesseract").arg("--version").output().ok()?;
    parse_version(&output_text(&out))
}

/// `tesseract --list-langs` starts with: List of available languages in "/usr/share/tessdata/" (3):
fn parse_list_dir(text: &str) -> Option<PathBuf> {
    let start = text.find('"')? + 1;
    let end = start + text[start..].find('"')?;
    Some(PathBuf::from(&text[start..end]))
}

/// The folder the system's Tesseract keeps its languages in.
pub(crate) fn system_tessdata() -> Option<PathBuf> {
    if let Some(dir) = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .ok()
        .and_then(|out| parse_list_dir(&output_text(&out)))
        .filter(|dir| dir.is_dir())
    {
        return Some(dir);
    }
    [
        "/usr/share/tessdata",
        "/usr/share/tesseract-ocr/5/tessdata",
        "/usr/share/tesseract-ocr/4.00/tessdata",
        "/usr/share/tesseract/tessdata",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|dir| dir.is_dir())
}

/// One word Tesseract found: its box (in the image it read), how sure it is
/// (0-100) and which block, paragraph and line it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Word {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub conf: f32,
    pub text: String,
    pub block: u32,
    pub par: u32,
    pub line: u32,
}

/// Reads Tesseract's TSV output (`tesseract image stdout tsv`): the words.
pub(crate) fn parse_tsv(tsv: &str) -> Vec<Word> {
    let mut words = Vec::new();
    for row in tsv.lines().skip(1) {
        let cols: Vec<&str> = row.splitn(12, '\t').collect();
        if cols.len() < 12 || cols[0] != "5" {
            continue; // level 5 = a word
        }
        // without the invisible direction marks Tesseract puts in Arabic text
        let text = cols[11].trim_matches(|c: char| c.is_whitespace() || matches!(c, '\u{200e}' | '\u{200f}'));
        if text.is_empty() {
            continue;
        }
        let num = |i: usize| cols[i].trim().parse::<i32>().unwrap_or(0);
        words.push(Word {
            block: num(2) as u32,
            par: num(3) as u32,
            line: num(4) as u32,
            x: num(6),
            y: num(7),
            w: num(8),
            h: num(9),
            conf: cols[10].trim().parse::<f32>().unwrap_or(-1.0),
            text: text.to_string(),
        });
    }
    words
}

/// The text: words joined by spaces, lines by line breaks (or joined into
/// sentences), paragraphs by a blank line. A word broken at the end of a line
/// ("exam-" + "ple") is put back together when lines are joined.
pub(crate) fn text_from_words(words: &[Word], keep_lines: bool) -> String {
    let mut paragraphs: Vec<Vec<String>> = Vec::new();
    let mut last_par: Option<(u32, u32)> = None;
    let mut last_line: Option<(u32, u32, u32)> = None;
    for word in words {
        let par = (word.block, word.par);
        let line = (word.block, word.par, word.line);
        if last_par != Some(par) {
            paragraphs.push(Vec::new());
            last_par = Some(par);
            last_line = None;
        }
        let lines = paragraphs.last_mut().expect("a paragraph was just added");
        if last_line != Some(line) {
            lines.push(String::new());
            last_line = Some(line);
        }
        let current = lines.last_mut().expect("a line was just added");
        if needs_space(current.chars().last(), word.text.chars().next()) {
            current.push(' ');
        }
        current.push_str(&word.text);
    }
    paragraphs
        .into_iter()
        .map(|lines| {
            if keep_lines {
                return lines.join("\n");
            }
            let mut joined = String::new();
            for line in lines {
                if joined.ends_with('-') && line.chars().next().is_some_and(|c| c.is_lowercase()) {
                    joined.pop();
                } else if needs_space(joined.chars().last(), line.chars().next()) {
                    joined.push(' ');
                }
                joined.push_str(&line);
            }
            joined
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Chinese, Japanese, Thai, Lao, Khmer and Burmese don't put spaces between
/// words; Tesseract hands each character (or cluster) over as a word. Which
/// of the two families a character is in (0: one written with spaces).
fn unspaced(c: char) -> u8 {
    match c as u32 {
        0x0E00..=0x0EFF | 0x1000..=0x109F | 0x1780..=0x17FF => 1, // Thai, Lao, Myanmar, Khmer
        0x3000..=0x30FF // CJK punctuation, Hiragana, Katakana
        | 0x31F0..=0x31FF // Katakana extensions
        | 0x3400..=0x4DBF // CJK extension A
        | 0x4E00..=0x9FFF // CJK
        | 0xF900..=0xFAFF // CJK compatibility
        | 0xFF00..=0xFFEF // full-width forms
        | 0x20000..=0x3FFFF => 2, // CJK extensions B-
        _ => 0,
    }
}

/// A space between two pieces of text, except at the start and between two
/// characters of the same script written without spaces.
fn needs_space(before: Option<char>, after: Option<char>) -> bool {
    match (before, after) {
        (None, _) => false,
        (Some(b), Some(a)) => unspaced(b) == 0 || unspaced(b) != unspaced(a),
        _ => true,
    }
}

/// Runs Tesseract on a PNG with the given languages ("eng+deu"); returns its
/// TSV. `psm` 3 finds the layout itself, 6 reads one block (better for a line
/// or a word, where 3 sometimes finds nothing).
pub(crate) fn run(png: &Path, langs: &str, tessdata: &Path, psm: u8) -> Result<String, String> {
    let out = Command::new("tesseract")
        .arg(png)
        .arg("stdout")
        .args(["-l", langs, "--psm", &psm.to_string(), "--tessdata-dir"])
        .arg(tessdata)
        // TSV out, as settings: the `tsv` config file lives in the system's
        // tessdata folder, not in Archtoys' own one given above
        .args(["-c", "tessedit_create_tsv=1", "-c", "tessedit_create_txt=0"])
        // one thread: faster for a screen-sized crop, and it doesn't take every core
        .env("OMP_THREAD_LIMIT", "1")
        .output()
        .map_err(|err| format!("couldn't start tesseract: {err}"))?;
    if !out.status.success() {
        let message = String::from_utf8_lossy(&out.stderr);
        return Err(format!("tesseract failed: {}", message.lines().last().unwrap_or("").trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// "Script: Arabic" in Tesseract's script detection (`--psm 0`) output.
fn parse_script(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.strip_prefix("Script:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Which writing system the image is in (Latin, Arabic, Cyrillic, Han ...),
/// from Tesseract's script detection. None when it can't tell (too little text).
pub(crate) fn detect_script(png: &Path, tessdata: &Path) -> Option<String> {
    let out = Command::new("tesseract")
        .arg(png)
        .arg("stdout")
        .args(["-l", "osd", "--psm", "0", "--tessdata-dir"])
        .arg(tessdata)
        // it skips anything under 50 letters by default ("Too few characters"),
        // so a line or a few words of Arabic were read as English; 5 is enough
        // to tell the scripts apart (tested from one word up)
        .args(["-c", "min_characters_to_try=5"])
        .env("OMP_THREAD_LIMIT", "1")
        .output()
        .ok()?;
    parse_script(&output_text(&out))
}

/// Screen text made easy to read: gray, dark text on a light background, and
/// enlarged (Tesseract is trained on print, where letters are 20-30 px tall;
/// screen text is often 10-16 px). Returns the image and how much it was enlarged.
pub(crate) fn prepare(rgba: &[u8], width: u32, height: u32) -> (image::GrayImage, u32) {
    let mut gray = image::GrayImage::new(width, height);
    let mut sum: u64 = 0;
    for (i, px) in gray.pixels_mut().enumerate() {
        let p = &rgba[i * 4..i * 4 + 3];
        let y = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8;
        sum += y as u64;
        px.0 = [y];
    }
    // light text on a dark background (a dark theme, a terminal): flip it
    if width * height > 0 && sum / (width as u64 * height as u64) < 128 {
        for px in gray.pixels_mut() {
            px.0[0] = 255 - px.0[0];
        }
    }
    let scale = if height <= 60 {
        4
    } else if height <= 240 {
        3
    } else {
        2
    };
    // keep the enlarged image a sensible size
    let scale = scale.min((8000 / width.max(height).max(1)).max(1));
    (enlarge(&gray, scale), scale)
}

/// How tall the text lines in a prepared image are (in its pixels): the
/// median height of the runs of rows with ink in them, ink being darker than
/// Otsu's threshold. None when there is no ink (a blank image).
pub(crate) fn line_height(img: &image::GrayImage) -> Option<f32> {
    let (w, h) = img.dimensions();
    let threshold = otsu(img);
    let min_ink = (w / 200).max(2);
    let mut runs = Vec::new();
    let mut run = 0u32;
    for y in 0..h {
        let ink = (0..w).filter(|&x| img.get_pixel(x, y).0[0] < threshold).count() as u32;
        if ink >= min_ink {
            run += 1;
        } else {
            if run >= 3 {
                runs.push(run);
            }
            run = 0;
        }
    }
    if run >= 3 {
        runs.push(run);
    }
    runs.sort_unstable();
    runs.get(runs.len() / 2).map(|&r| r as f32)
}

/// Otsu's threshold: the gray level that best splits the image in two.
fn otsu(img: &image::GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let sum: f64 = hist.iter().enumerate().map(|(i, &n)| i as f64 * n as f64).sum();
    let (mut weight, mut sum_below, mut best, mut threshold) = (0u64, 0f64, -1f64, 128u8);
    for (t, &n) in hist.iter().enumerate() {
        weight += n;
        if weight == 0 || weight == total {
            continue;
        }
        sum_below += t as f64 * n as f64;
        let (w0, w1) = (weight as f64, (total - weight) as f64);
        let between = w0 * w1 * (sum_below / w0 - (sum - sum_below) / w1).powi(2);
        if between > best {
            best = between;
            threshold = t as u8 + 1;
        }
    }
    threshold
}

/// The screen image at another scale (`to` times its size), when the first
/// reading found nothing: Tesseract's Arabic reads letters about 16-24 px tall
/// and nothing much bigger, so large Arabic text is read smaller.
pub(crate) fn rescale(prepared: &image::GrayImage, scale: u32, to: f32) -> image::GrayImage {
    let (w, h) = prepared.dimensions();
    let size = |n: u32| ((n / scale) as f32 * to).round().max(1.0) as u32;
    image::imageops::resize(prepared, size(w), size(h), image::imageops::FilterType::CatmullRom)
}

fn enlarge(gray: &image::GrayImage, scale: u32) -> image::GrayImage {
    if scale == 1 {
        return gray.clone();
    }
    let (w, h) = gray.dimensions();
    image::imageops::resize(gray, w * scale, h * scale, image::imageops::FilterType::CatmullRom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_and_checked() {
        assert_eq!(parse_version("tesseract 5.3.4\n leptonica-1.84.1"), Some((5, 3, 4)));
        assert_eq!(parse_version("tesseract v4.1.1-rc2-22-g08899\n"), Some((4, 1, 1)));
        assert_eq!(parse_version("tesseract 5.5.0-1"), Some((5, 5, 0)));
        assert_eq!(parse_version("command not found"), None);
        assert!(version_ok((4, 1, 1)) && version_ok((5, 0, 0)));
        assert!(!version_ok((4, 0, 0)) && !version_ok((3, 5, 2)));
    }

    #[test]
    fn the_system_language_folder_is_read_from_list_langs() {
        let text = "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nosd\n";
        assert_eq!(parse_list_dir(text), Some(PathBuf::from("/usr/share/tessdata/")));
        assert_eq!(parse_list_dir("nothing"), None);
    }

    #[test]
    fn the_script_is_read_from_script_detection() {
        let osd = "Page number: 0\nOrientation in degrees: 0\nRotate: 0\nScript: Arabic\nScript confidence: 2.33\n";
        assert_eq!(parse_script(osd), Some("Arabic".to_string()));
        assert_eq!(parse_script("Too few characters. Skipping this page"), None);
    }

    fn tsv(rows: &[(u32, u32, u32, &str)]) -> String {
        let mut out = String::from("level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n");
        out.push_str("1\t1\t0\t0\t0\t0\t0\t0\t100\t100\t-1\t\n"); // the page: not a word
        for (i, (block, par, line, text)) in rows.iter().enumerate() {
            out.push_str(&format!("5\t1\t{block}\t{par}\t{line}\t{i}\t{}\t10\t30\t12\t91.5\t{text}\n", i * 40));
        }
        out
    }

    #[test]
    fn words_become_lines_and_paragraphs() {
        let words = parse_tsv(&tsv(&[
            (1, 1, 1, "Release"),
            (1, 1, 1, "notes"),
            (2, 1, 1, "Archtoys"),
            (2, 1, 1, "picks"),
            (2, 1, 2, "exam-"),
            (2, 1, 3, "ple"),
            (2, 1, 3, "text."),
        ]));
        assert_eq!(words.len(), 7);
        assert_eq!(words[0].conf, 91.5);
        assert_eq!((words[1].x, words[1].w, words[1].h), (40, 30, 12));
        assert_eq!(text_from_words(&words, true), "Release notes\n\nArchtoys picks\nexam-\nple text.");
        assert_eq!(text_from_words(&words, false), "Release notes\n\nArchtoys picks example text.");
    }

    #[test]
    fn chinese_japanese_and_thai_get_no_spaces_between_characters() {
        let words = parse_tsv(&tsv(&[(1, 1, 1, "设"), (1, 1, 1, "置"), (1, 1, 1, "and"), (1, 1, 1, "ダウ"), (1, 1, 1, "ン"), (1, 1, 1, "นี่"), (1, 1, 1, "คือ")]));
        assert_eq!(text_from_words(&words, true), "设置 and ダウン นี่คือ");
        assert_eq!(text_from_words(&words, false), "设置 and ダウン นี่คือ");
    }

    #[test]
    fn line_height_is_measured_from_the_ink() {
        // white, with two "lines" of black 12 and 14 rows tall, and one of 30
        let mut img = image::GrayImage::from_pixel(200, 120, image::Luma([250]));
        for (top, height) in [(10, 12), (40, 14), (70, 30)] {
            for y in top..top + height {
                for x in 20..180 {
                    img.put_pixel(x, y, image::Luma([20]));
                }
            }
        }
        assert_eq!(line_height(&img), Some(14.0));
        assert_eq!(line_height(&image::GrayImage::from_pixel(50, 50, image::Luma([255]))), None);
    }

    #[test]
    fn dark_screen_text_is_flipped_and_enlarged() {
        // 2x1 image: near-black pixels -> inverted to near-white
        let rgba = [10, 10, 10, 255, 20, 20, 20, 255];
        let (img, scale) = prepare(&rgba, 2, 1);
        assert_eq!(scale, 4);
        assert_eq!(img.dimensions(), (8, 4));
        assert!(img.get_pixel(0, 0).0[0] > 200);
    }
}
