//! The font finder (v0.7.5): which font a selection of text is in. The words
//! Text Extractor read are drawn in candidate fonts the way a screen draws
//! them, and compared with the selection word by word. The fonts on this
//! computer are compared every time; with "Find fonts online" on, the Google
//! Fonts index picks a short list, only those are downloaded (and kept), and
//! compared the same way. Nothing about the selection leaves the computer.
//! It's a best guess: screens draw text with different scaling, smoothing
//! and hinting, and the first-time card says so.

pub(crate) mod index;
pub(crate) mod local;
pub(crate) mod matcher;
pub(crate) mod net;
pub(crate) mod page;
pub(crate) mod render;

use image::GrayImage;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

/// How many Google families the index's short list downloads and compares
/// (a longer one, 24 or 40, found no more right answers in the tests).
const SHORTLIST: usize = 12;
/// The most words compared (the longest ones): enough to tell, quick to draw.
const MAX_WORDS: usize = 6;

/// Where a guess's font is from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    Local,
    Google { path: String, designer: String, licence: u8, category: u8 },
}

/// One font the selection may be in.
#[derive(Clone, Debug)]
pub(crate) struct Guess {
    pub family: String,
    pub bold: bool,
    /// 0 to 1, how alike (matcher::sure makes it a percentage)
    pub score: f32,
    pub file: PathBuf,
    pub source: Source,
}

/// A word's box in the selection (x, y, width, height) and its text.
pub(crate) type WordBox = ((f32, f32, f32, f32), String);

/// What the search view shows while it works.
#[derive(Clone, Debug, Default)]
pub(crate) struct Progress {
    pub done: usize,
    pub total: usize,
    pub comparing: String,
    /// a download: bytes so far, its size when known, bytes per second
    pub got: u64,
    pub size: Option<u64>,
    pub speed: f64,
    /// the font file being compared (when it's on disk), to draw the text in it
    pub comparing_file: Option<PathBuf>,
    /// the surest so far: family, Bold, score, its file
    pub best: Option<(String, bool, f32, PathBuf)>,
}

/// Downloaded fonts live here (deleted from Settings).
pub(crate) fn fonts_dir() -> PathBuf {
    crate::ocr::lang::app_data_dir().join("fonts")
}

/// The downloaded fonts: how many, and their size in bytes (licence files included).
pub(crate) fn downloaded() -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(fonts_dir()) else { return (0, 0) };
    let files: Vec<_> = entries.flatten().filter_map(|e| Some((e.path(), e.metadata().ok()?.len()))).collect();
    let fonts = files.iter().filter(|(p, _)| p.extension().is_some_and(|e| e == "ttf" || e == "otf")).count();
    (fonts, files.iter().map(|f| f.1).sum())
}

pub(crate) fn delete_downloaded() {
    let _ = std::fs::remove_dir_all(fonts_dir());
}

/// The words to compare: each cut from the selection to its ink, with its text.
fn cut_words(selection: &GrayImage, words: &[WordBox]) -> Vec<(GrayImage, String)> {
    let (w, h) = selection.dimensions();
    let mut cut: Vec<(GrayImage, String)> = words
        .iter()
        .filter(|(_, text)| text.chars().any(|c| c.is_alphanumeric()))
        .filter_map(|&((x, y, bw, bh), ref text)| {
            let x0 = (x.floor() as i64 - 1).clamp(0, w as i64 - 1) as u32;
            let y0 = (y.floor() as i64 - 1).clamp(0, h as i64 - 1) as u32;
            let x1 = ((x + bw).ceil() as i64 + 1).clamp(x0 as i64 + 1, w as i64) as u32;
            let y1 = ((y + bh).ceil() as i64 + 1).clamp(y0 as i64 + 1, h as i64) as u32;
            let part = image::imageops::crop_imm(selection, x0, y0, x1 - x0, y1 - y0).to_image();
            Some((matcher::ink_box(&part)?, text.clone()))
        })
        .collect();
    cut.sort_by_key(|(_, t)| std::cmp::Reverse(t.chars().count()));
    cut.truncate(MAX_WORDS);
    cut
}

/// How alike a font's drawing of the words is with the selection's: drawn
/// big (4x) and shrunk smoothly, compared at twice the selection's size.
fn exact(face: &render::Face, words: &[(GrayImage, String)], bold: bool) -> f32 {
    let (sel, word) = &words[0];
    let Some(at64) = face.draw(word, 64.0, bold).and_then(|d| matcher::ink_box(&d)) else { return 0.0 };
    let px = (64.0 * sel.height() as f32 / at64.height().max(1) as f32).clamp(5.0, 300.0);
    let (mut total, mut weight) = (0f32, 0f32);
    for (sel, word) in words {
        if let Some(drawn) = face.draw(word, px * 4.0, bold) {
            total += matcher::word_score(sel, &drawn) * sel.width() as f32;
            weight += sel.width() as f32;
        }
    }
    if weight > 0.0 {
        total / weight
    } else {
        0.0
    }
}

/// How many of the surest guesses are downloaded (Google ones), to draw
/// their names and the sample in them.
const SHOWN: usize = 3;

/// The selection (dark text on light, Text Extractor's prepared gray) and
/// its words with their boxes → the guesses, surest first (one per family).
/// The fonts on this computer are all compared; for Google Fonts the index
/// picks a short list, which is downloaded (once) and compared the same way.
pub(crate) fn find(
    selection: &GrayImage,
    words: &[WordBox],
    online: bool,
    progress: &dyn Fn(&Progress),
    stop: &AtomicBool,
) -> Result<Vec<Guess>, String> {
    let words = cut_words(selection, words);
    if words.is_empty() {
        return Err("no words to compare".into());
    }
    let stopped = || stop.load(std::sync::atomic::Ordering::Relaxed);
    let locals = local::fonts();
    let mut state = Progress { total: locals.len() + usize::from(online), ..Default::default() };
    let mut guesses: Vec<Guess> = Vec::new();
    let keep = |g: Guess, state: &mut Progress, guesses: &mut Vec<Guess>| {
        if state.best.as_ref().is_none_or(|b| g.score > b.2) {
            state.best = Some((g.family.clone(), g.bold, g.score, g.file.clone()));
        }
        guesses.push(g);
    };

    // 1. the fonts on this computer: Regular, and Bold (its own file, or its weight axis)
    for font in &locals {
        if stopped() {
            break;
        }
        state.comparing = font.family.clone();
        state.comparing_file = Some(font.path.clone());
        progress(&state);
        if let Some(face) = render::Face::open(&font.path) {
            let weights: &[bool] = if !font.bold && face.has_weight_axis() { &[false, true] } else { &[false] };
            for &drawn_bold in weights {
                let score = exact(&face, &words, drawn_bold);
                let g = Guess {
                    family: font.family.clone(),
                    bold: font.bold || drawn_bold,
                    score,
                    file: font.path.clone(),
                    source: Source::Local,
                };
                keep(g, &mut state, &mut guesses);
            }
        }
        state.done += 1;
    }

    // 2. Google Fonts, through the index
    if online && !stopped() {
        state.comparing = "the Google Fonts list".into();
        progress(&state);
        let show = |got: u64, size: Option<u64>, speed: f64| {
            let mut s = state.clone();
            (s.got, s.size, s.speed) = (got, size, speed);
            progress(&s);
        };
        match index::get(&show, stop).and_then(|path| index::shortlist(&path, &words, SHORTLIST)) {
            Ok(shortlist) => {
                if std::env::var_os("ARCHTOYS_FONTS_DEBUG").is_some() {
                    let names: Vec<String> =
                        shortlist.iter().map(|(s, b, f)| format!("{}{} {:.3}", f.name, if *b { " Bold" } else { "" }, s)).collect();
                    eprintln!("shortlist: {}", names.join(", "));
                }
                state.total += shortlist.len();
                for (thumb, bold, family) in shortlist {
                    if stopped() {
                        break;
                    }
                    let file = fonts_dir().join(family.path.replace('/', "_"));
                    state.comparing = family.name.clone();
                    state.comparing_file = file.exists().then(|| file.clone());
                    if !file.exists() {
                        let show = |got: u64, size: Option<u64>, speed: f64| {
                            let mut s = state.clone();
                            (s.got, s.size, s.speed) = (got, size, speed);
                            progress(&s);
                        };
                        if net::download(&net::google_url(&family.path), &file, &show, stop).is_ok() {
                            licence_file(&family.path, family.licence);
                        }
                    }
                    // not downloaded (offline, stopped): the index's own measure
                    // shown as it's compared (once downloaded)
                    state.comparing_file = file.exists().then(|| file.clone());
                    progress(&state);
                    let score = render::Face::open(&file).map_or(thumb, |face| exact(&face, &words, bold && face.has_weight_axis()));
                    let source =
                        Source::Google { path: family.path, designer: family.designer, licence: family.licence, category: family.category };
                    keep(Guess { family: family.name, bold, score, file, source }, &mut state, &mut guesses);
                    state.done += 1;
                }
            }
            Err(err) => eprintln!("font finder: {err}"),
        }
        state.done += 1;
    }

    // one guess per family (its best weight), surest first
    guesses.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut seen = std::collections::HashSet::new();
    guesses.retain(|g| seen.insert(g.family.to_lowercase()));
    guesses.truncate(5);

    // 3. the surest Google fonts downloaded (once), to draw their names and the sample in them
    for g in guesses.iter().take(SHOWN) {
        let Source::Google { path, licence, .. } = &g.source else { continue };
        if g.file.exists() || stopped() {
            continue;
        }
        state.comparing = g.family.clone();
        state.total += 1;
        let show = |got: u64, size: Option<u64>, speed: f64| {
            let mut s = state.clone();
            (s.got, s.size, s.speed) = (got, size, speed);
            progress(&s);
        };
        if net::download(&net::google_url(path), &g.file, &show, stop).is_ok() {
            licence_file(path, *licence);
        }
        state.done += 1;
    }
    (state.got, state.size, state.speed) = (0, None, 0.0);
    progress(&state);
    Ok(guesses)
}

/// A downloaded font's licence, next to it (OFL.txt, LICENSE.txt or UFL.txt in its folder in google/fonts).
fn licence_file(path: &str, licence: u8) {
    let Some((folder, _)) = path.rsplit_once('/') else { return };
    let name = match licence {
        1 => "LICENSE.txt",
        2 => "UFL.txt",
        _ => "OFL.txt",
    };
    let dest = fonts_dir().join(format!("{}_{name}", folder.replace('/', "_")));
    if !dest.exists() {
        let _ = net::download(&net::google_url(&format!("{folder}/{name}")), &dest, &|_, _, _| {}, &AtomicBool::new(false));
    }
}

/// `archtoys --find-font IMAGE`: reads the text, finds its font, prints the
/// guesses (the CI test and the benchmark use it). Online unless
/// ARCHTOYS_FONTS_OFFLINE is set.
pub(crate) fn run_on_file(path: &str) -> i32 {
    let img = match image::open(path) {
        Ok(img) => img.into_rgba8(),
        Err(err) => {
            eprintln!("can't read {path}: {err}");
            return 2;
        }
    };
    let (w, h) = img.dimensions();
    let reading = match crate::ocr::recognize(img.as_raw(), w, h, true, &|s| eprintln!("{s}")) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("{err}");
            return 1;
        }
    };
    let gray = crate::ocr::prepared_gray(img.as_raw(), w, h);
    let words: Vec<_> = reading.boxes.iter().copied().zip(reading.words.iter().cloned()).collect();
    let online = std::env::var_os("ARCHTOYS_FONTS_OFFLINE").is_none();
    let started = std::time::Instant::now();
    match find(&gray, &words, online, &|_| {}, &AtomicBool::new(false)) {
        Ok(guesses) => {
            println!("text: {}", reading.one_line);
            println!("seconds: {:.2}", started.elapsed().as_secs_f32());
            for g in guesses.iter().filter(|_| std::env::var_os("ARCHTOYS_FONTS_DEBUG").is_some()) {
                eprintln!("  {:.4} {}{}", g.score, g.family, if g.bold { " Bold" } else { "" });
            }
            for g in &guesses {
                let from = if g.source == Source::Local { "local" } else { "google" };
                println!("guess: {}{} | {}% | {from}", g.family, if g.bold { " Bold" } else { "" }, matcher::sure(g.score));
            }
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}
