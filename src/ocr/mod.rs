//! Text Extractor (v0.7.0): copy the text in any part of the screen.
//!
//! * `engine`: runs the `tesseract` program and reads its words.
//! * `install`: "Turn on" installs or updates Tesseract from the system's packages.
//! * `lang`: finds the language by itself and downloads its data when needed.
//! * `history`: the last 500 texts, saved until cleared.
//!
//! The capture itself is the Color Picker's frozen screen in text mode
//! (`picker::start_text_capture`); this module reads the selected area and
//! keeps the result. Nothing here runs at startup: with Text Extractor off,
//! the app costs what v0.6.2 cost.

mod engine;
mod history;
mod install;
mod lang;

use crate::{AppWindow, TextEntry};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turned on (and installed) by the user.
pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub(crate) fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
}

thread_local! {
    static HISTORY: RefCell<Option<Vec<history::Entry>>> = const { RefCell::new(None) };
    static MODEL: Rc<VecModel<TextEntry>> = Rc::new(VecModel::default());
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
    static SAVE_CONFIG: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    static CHECKED: Cell<bool> = const { Cell::new(false) };
}

/// At startup: the page's first state, from the saved setting only (no
/// program is run until Text Extractor is used).
pub(crate) fn install_ui(ui: &AppWindow, save_config: impl Fn() + 'static) {
    SAVE_CONFIG.with(|s| *s.borrow_mut() = Some(Box::new(save_config)));
    MODEL.with(|m| ui.set_text_history(ModelRc::from(m.clone())));
    ui.set_text_state(if enabled() { 2 } else { 0 });
    ui.set_text_engine_info(if enabled() { "Tesseract" } else { "Not installed" }.into());
}

fn save_config() {
    SAVE_CONFIG.with(|s| {
        if let Some(save) = s.borrow().as_ref() {
            save();
        }
    });
}

fn version_text((a, b, c): (u32, u32, u32)) -> String {
    format!("{a}.{b}.{c}")
}

/// The page is shown (or settings opened): load the history and check
/// Tesseract once.
pub(crate) fn page_shown(ui: &AppWindow) {
    ensure_history(ui);
    if !CHECKED.with(Cell::get) && ui.get_text_state() != 1 {
        CHECKED.with(|c| c.set(true));
        refresh(ui);
    }
}

/// Checks the system: the distro, Tesseract and its version.
pub(crate) fn refresh(ui: &AppWindow) {
    let distro = install::detect();
    ui.set_text_distro(distro.name().into());
    let version = engine::installed_version();
    ui.set_text_installed_version(version.map(version_text).unwrap_or_default().into());
    ui.set_text_engine_info(
        match version {
            Some(v) => format!("Tesseract {}", version_text(v)),
            None => "Not installed".to_string(),
        }
        .into(),
    );
    let usable = version.is_some_and(engine::version_ok);
    if enabled() && !usable {
        // it was removed, or replaced by one too old
        set_enabled(false);
        save_config();
    }
    let state = if enabled() {
        2
    } else if let install::Distro::Other(_, why) = &distro {
        ui.set_text_message(why.as_str().into());
        3
    } else {
        0
    };
    ui.set_text_state(state);
    languages_info(ui);
    if state == 2 {
        check_update(ui, distro);
    }
}

/// Asks the package manager whether a newer Tesseract is out, so Settings
/// offers "Update" only then. `text-update`: 0 checking, 1 up to date,
/// 2 an update is available, 3 couldn't tell.
fn check_update(ui: &AppWindow, distro: install::Distro) {
    ui.set_text_update(0);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let found = install::update_available(&distro);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_text_update(match found {
                Ok(None) => 1,
                Ok(Some(_)) => 2,
                Err(err) => {
                    eprintln!("text extractor: couldn't check for a Tesseract update ({err})");
                    3
                }
            });
        });
    });
}

/// "Check again" (a system Archtoys can't install on): a Tesseract the user
/// installed themselves turns it on.
pub(crate) fn check_again(ui: &AppWindow) {
    if engine::installed_version().is_some_and(engine::version_ok) {
        set_enabled(true);
        save_config();
    }
    refresh(ui);
}

fn languages_info(ui: &AppWindow) {
    let (codes, bytes) = lang::downloaded();
    let text = if codes.is_empty() {
        "Your system's languages; others download when they're first needed".to_string()
    } else {
        let names: Vec<String> = codes.iter().filter(|c| *c != "osd").map(|c| lang::display_name(c)).collect();
        format!("Downloaded: {} · {:.1} MB", names.join(", "), bytes as f64 / 1e6)
    };
    ui.set_text_languages_info(text.into());
}

/// Deletes the downloaded languages (they come back when needed again).
pub(crate) fn delete_languages(ui: &AppWindow) {
    lang::delete_downloaded();
    languages_info(ui);
}

/// "Turn on" (or "Update"): installs Tesseract with the user's password.
pub(crate) fn turn_on(ui: &AppWindow) {
    if ui.get_text_state() == 1 {
        return;
    }
    let distro = install::detect();
    if let install::Distro::Other(_, why) = &distro {
        ui.set_text_message(why.as_str().into());
        ui.set_text_state(3);
        return;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    CANCEL.with(|c| *c.borrow_mut() = Some(cancel.clone()));
    ui.set_text_message("".into());
    ui.set_text_install_fraction(-1.0);
    ui.set_text_install_stats("".into());
    ui.set_text_install_stage("Waiting for your password…".into());
    ui.set_text_state(1);
    ui.invoke_go(2);

    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let progress = weak.clone();
        let result = install::run(&distro, cancel, move |p| {
            let _ = progress.upgrade_in_event_loop(move |ui| {
                ui.set_text_install_fraction(p.fraction.unwrap_or(-1.0));
                ui.set_text_install_stats(p.stats.into());
                ui.set_text_install_stage(p.stage.into());
            });
        });
        let version = engine::installed_version();
        // link the system's languages now, not on the first capture
        let _ = lang::prepare_dir();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            CANCEL.with(|c| c.borrow_mut().take());
            let message = match (result, version) {
                (Ok(()), Some(v)) if engine::version_ok(v) => {
                    set_enabled(true);
                    save_config();
                    String::new()
                }
                (Ok(()), Some(v)) => format!(
                    "Your system's packages have Tesseract {}, and Archtoys needs 4.1 or newer.",
                    version_text(v)
                ),
                (Ok(()), None) => "It was installed, but Archtoys can't find the tesseract program.".to_string(),
                (Err(err), _) => err,
            };
            refresh(&ui);
            ui.set_text_message(message.into());
        });
    });
}

pub(crate) fn cancel_install() {
    CANCEL.with(|c| {
        if let Some(cancel) = c.borrow().as_ref() {
            cancel.store(true, Ordering::SeqCst);
        }
    });
}

/// The hotkey, the tray, the button or `archtoys --extract`: start a capture,
/// or show the Turn on card when Text Extractor isn't on yet.
pub(crate) fn start_capture(ui: &AppWindow) {
    if ui.get_page() != 2 && ui.get_text_state() != 1 {
        ui.invoke_go(2);
    }
    if !enabled() {
        crate::ui_state::show_window(ui);
        page_shown(ui);
        return;
    }
    ensure_history(ui);
    crate::picker::start_text_capture(ui);
}

/// What Tesseract read in a capture.
#[derive(Debug, Clone)]
pub(crate) struct Reading {
    /// as the "Line breaks" setting wants it
    pub text: String,
    /// every line joined into one
    pub one_line: String,
    /// "English"
    pub language: String,
    pub sure: u8,
    /// the text's height, in the pixels of the image that was read
    pub size: f32,
    /// each word's box, in the pixels of the image that was read
    pub boxes: Vec<(f32, f32, f32, f32)>,
}

fn temp_png() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    static COUNT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    dir.join(format!("archtoys-ocr-{}-{}.png", std::process::id(), COUNT.fetch_add(1, Ordering::SeqCst)))
}

/// The languages read side by side when English (and the user's own) isn't
/// sure: one per writing system, the close relatives the first of their
/// script misreads (Persian and Urdu next to Arabic, Ukrainian next to
/// Russian, Traditional next to Simplified Chinese), and the two Latin
/// languages English reads worst (Turkish, Vietnamese). Tesseract's own
/// script detection guessed wrong on a few words of most scripts; the reading
/// it is surest of is right far more often (v0.7.1: 98% of the text right in
/// 24 languages, from 80%). About 32 MB, downloaded together once.
const SWEEP: &[&str] = &[
    "ara", "fas", "urd", "rus", "ukr", "ell", "hin", "ben", "tam", "tha", "chi_sim", "chi_tra", "jpn", "kor", "tur", "vie",
];
/// Read in Latin letters (whatlang then names the exact language).
const LATIN: &[&str] = &["eng", "tur", "vie"];
/// Mean word confidence (and no word under 60) at which the first reading,
/// in English and the user's own languages, is kept without trying others.
const SURE_FIRST: f32 = 85.0;
/// Below this, the whole reading is tried again with the lines another size.
const READ_WELL: u8 = 50;
/// The text lines' height Tesseract reads at, then the one tried if that reads badly.
const LINE_TARGETS: [f32; 2] = [32.0, 22.0];
/// Letters only Persian or only Urdu has (in Arabic script).
const PERSIAN: &[char] = &['پ', 'چ', 'ژ', 'گ'];
const URDU: &[char] = &['ٹ', 'ڈ', 'ڑ', 'ں', 'ے'];

/// Reads the text in an RGBA image (the selected part of the screen).
/// Blocking: call it on a background thread. `status` says what it's doing
/// ("Downloading languages…") when that takes a moment.
pub(crate) fn recognize(rgba: &[u8], width: u32, height: u32, keep_lines: bool, status: &dyn Fn(String)) -> Result<Reading, String> {
    let dir = lang::prepare_dir()?;
    let gray = engine::gray(rgba, width, height);
    let read = |target: f32| {
        let (image, scale) = engine::normalize(&gray, target);
        let png = temp_png();
        image.save(&png).map_err(|err| format!("couldn't save the image for Tesseract: {err}"))?;
        let result = read_png(&png, &dir, scale, keep_lines, status);
        let _ = std::fs::remove_file(&png);
        result
    };
    let mut best = read(LINE_TARGETS[0])?;
    debug(|| format!("lines {} px: {} ({}% sure)", LINE_TARGETS[0], best.language, best.sure));
    if best.sure < READ_WELL {
        // nothing read well: the line height may have been misjudged
        if let Ok(again) = read(LINE_TARGETS[1]) {
            debug(|| format!("lines {} px: {} ({}% sure)", LINE_TARGETS[1], again.language, again.sure));
            if again.sure > best.sure {
                best = again;
            }
        }
    }
    Ok(best)
}

/// `ARCHTOYS_OCR_DEBUG=1`: prints what Text Extractor tried, to the terminal.
fn debug(message: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("ARCHTOYS_OCR_DEBUG").is_some()) {
        eprintln!("ocr: {}", message());
    }
}

/// The words Tesseract read with `langs` ("ara+eng"), in its layout `psm`,
/// without Hebrew (by choice: a word in Hebrew letters is dropped, whatever
/// read it).
fn tesseract_words(png: &std::path::Path, langs: &str, dir: &std::path::Path, psm: u8) -> Result<Vec<engine::Word>, String> {
    let mut words = engine::parse_tsv(&engine::run(png, langs, dir, psm)?);
    words.retain(|w| !w.text.chars().any(lang::hebrew));
    Ok(words)
}

/// With the layout Tesseract finds, and as one block when that reads badly
/// (it can scramble or miss a single line or word); the surer one.
fn run_words(png: &std::path::Path, langs: &str, dir: &std::path::Path) -> Result<Vec<engine::Word>, String> {
    let found = tesseract_words(png, langs, dir, 3)?;
    if mean_conf(&found) >= 70.0 {
        return Ok(found);
    }
    let block = tesseract_words(png, langs, dir, 6)?;
    Ok(if mean_conf(&block) > mean_conf(&found) { block } else { found })
}

/// How much of what was read is in Latin letters (0-1).
fn latin_share(words: &[engine::Word]) -> f32 {
    let letters: Vec<char> = words.iter().flat_map(|w| w.text.chars()).filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return 0.0;
    }
    letters.iter().filter(|&&c| c < '\u{0250}').count() as f32 / letters.len() as f32
}

/// Of the readings side by side, the right one: the surest, except that a
/// close call goes to Simplified Chinese (the more common), and to Persian
/// or Urdu when their reading has letters only they use.
fn pick<'a>(tries: &'a [(&'static str, Vec<engine::Word>)]) -> Option<&'a (&'static str, Vec<engine::Word>)> {
    let best = tries.iter().max_by(|a, b| mean_conf(&a.1).total_cmp(&mean_conf(&b.1)))?;
    let find = |code: &str| tries.iter().find(|t| t.0 == code);
    let top = mean_conf(&best.1);
    if best.0 == "chi_tra" {
        if let Some(sim) = find("chi_sim").filter(|t| mean_conf(&t.1) >= top - 3.0) {
            return Some(sim);
        }
    }
    if matches!(best.0, "ara" | "fas" | "urd") {
        for (code, letters) in [("urd", URDU), ("fas", PERSIAN)] {
            let own = |t: &&(&'static str, Vec<engine::Word>)| t.1.iter().any(|w| w.text.contains(letters));
            if let Some(t) = find(code).filter(own).filter(|t| mean_conf(&t.1) >= top - 10.0) {
                return Some(t);
            }
        }
    }
    // read in Latin letters (Chinese and Japanese read Latin text too, without
    // its accents): the surest Latin reading
    if !LATIN.contains(&best.0) && latin_share(&best.1) > 0.95 {
        if let Some(latin) = tries.iter().filter(|t| LATIN.contains(&t.0)).max_by(|a, b| mean_conf(&a.1).total_cmp(&mean_conf(&b.1))) {
            return Some(latin);
        }
    }
    Some(best)
}

fn read_png(png: &std::path::Path, dir: &std::path::Path, scale: f32, keep_lines: bool, status: &dyn Fn(String)) -> Result<Reading, String> {
    let have = |code: &str| lang::has(dir, code);
    let get = |code: &str| -> Result<(), String> {
        status(format!("Downloading {}…", lang::display_name(code)));
        let result = lang::download(dir, code);
        status("Reading…".to_string());
        result
    };

    // 1. English and the user's own languages (the desktop's, the keyboard
    // layouts', the input methods'): most text is in one of them
    let mut first = vec!["eng"];
    first.extend(lang::user_languages().iter().copied());
    let first: Vec<&'static str> = first.into_iter().filter(|c| have(c) || get(c).is_ok()).collect();
    if first.is_empty() {
        return Err("no language data: Tesseract's English is missing".into());
    }
    let mut langs = first.clone();
    let mut words = run_words(png, &first.join("+"), dir)?;
    let mut code = first[0];
    let sure = mean_conf(&words) >= SURE_FIRST && words.iter().all(|w| w.conf < 0.0 || w.conf >= 60.0);
    debug(|| format!("first {first:?}: {:.0}% sure{}", mean_conf(&words), if sure { ", kept" } else { "" }));

    // 2. not sure: every script's language side by side (in parallel), and
    // the right one of them
    if !sure {
        let missing: Vec<&'static str> = SWEEP.iter().copied().filter(|c| !have(c)).collect();
        if !missing.is_empty() {
            status(format!("Downloading {} languages (once, about 32 MB)…", missing.len()));
            let failed = lang::download_all(dir, &missing);
            if !failed.is_empty() {
                eprintln!("text extractor: couldn't download {failed:?}");
            }
            status("Reading…".to_string());
        }
        let tries: Vec<(&'static str, Vec<engine::Word>)> = std::thread::scope(|scope| {
            let jobs: Vec<_> = SWEEP
                .iter()
                .copied()
                .filter(|c| have(c) && !first.contains(c))
                .map(|c| (c, scope.spawn(move || tesseract_words(png, c, dir, 6))))
                .collect();
            let mut tries: Vec<_> = jobs.into_iter().filter_map(|(c, job)| Some((c, job.join().ok()?.ok()?))).collect();
            tries.push((first[0], words.clone()));
            tries
        });
        debug(|| {
            let mut all: Vec<(f32, &str)> = tries.iter().map(|(c, w)| (mean_conf(w), *c)).collect();
            all.sort_by(|a, b| b.0.total_cmp(&a.0));
            let all: Vec<String> = all.iter().map(|(conf, c)| format!("{c} {conf:.0}")).collect();
            format!("side by side: {}", all.join(", "))
        });
        if let Some((best, best_words)) = pick(&tries).cloned() {
            if best != first[0] {
                code = best;
                langs = vec![best];
                words = best_words;
                if !LATIN.contains(&best) {
                    // read again with English too, for English words mixed in
                    langs.push("eng");
                    if let Ok(mixed) = run_words(png, &langs.join("+"), dir) {
                        if mean_conf(&mixed) >= mean_conf(&words) {
                            words = mixed;
                        }
                    }
                }
            }
        }
        debug(|| format!("picked {code} ({:.0}% sure)", mean_conf(&words)));
    }

    // 3. Latin letters: which language exactly (whatlang), read again with its
    // own data if that's not what was used. whatlang's own "reliable" flag
    // stays off even for clear sentences; its confidence is the useful part:
    // sentences score 0.75-0.85, short snippets ("ERROR: connection refused"
    // scores French at 0.08) stay well under 0.5.
    let mut language = code;
    if LATIN.contains(&code) || first.contains(&code) {
        const SURE_ENOUGH: f64 = 0.5;
        let text = engine::text_from_words(&words, true);
        if let Some(info) = whatlang::detect(&text).filter(|i| i.confidence() >= SURE_ENOUGH) {
            if let Some(named) = lang::from_whatlang(info.lang()) {
                if !langs.contains(&named) && (have(named) || get(named).is_ok()) {
                    let mut again = vec![named];
                    if named != "eng" {
                        again.push("eng"); // the odd English word in a Latin text
                    }
                    if let Ok(better) = run_words(png, &again.join("+"), dir) {
                        if mean_conf(&better) >= mean_conf(&words) {
                            words = better;
                        }
                    }
                }
                language = named;
            }
        }
    }

    let pad = engine::PAD as f32;
    let mut heights: Vec<i32> = words.iter().map(|w| w.h).collect();
    heights.sort_unstable();
    let size = heights.get(heights.len() / 2).copied().unwrap_or(0) as f32 / scale;
    Ok(Reading {
        text: engine::text_from_words(&words, keep_lines),
        one_line: engine::text_from_words(&words, false).replace("\n\n", " "),
        language: lang::display_name(language),
        sure: mean_conf(&words).round().clamp(0.0, 100.0) as u8,
        size,
        boxes: words
            .iter()
            .map(|w| ((w.x as f32 - pad) / scale, (w.y as f32 - pad) / scale, w.w as f32 / scale, w.h as f32 / scale))
            .collect(),
    })
}

fn mean_conf(words: &[engine::Word]) -> f32 {
    let confs: Vec<f32> = words.iter().map(|w| w.conf).filter(|c| *c >= 0.0).collect();
    if confs.is_empty() {
        0.0
    } else {
        confs.iter().sum::<f32>() / confs.len() as f32
    }
}

/// Loads the text history once (Text Extractor's page and Home show it).
pub(crate) fn ensure_history(ui: &AppWindow) {
    let loaded = HISTORY.with(|h| h.borrow().is_some());
    if !loaded {
        HISTORY.with(|h| *h.borrow_mut() = Some(history::load()));
        sync_history(ui);
    }
}

fn to_row(entry: &history::Entry, now: u64) -> TextEntry {
    TextEntry {
        text: entry.text.as_str().into(),
        preview: preview(&entry.text).into(),
        language: entry.language.as_str().into(),
        sure: entry.sure as i32,
        size: entry.size as i32,
        words: entry.words as i32,
        when: history::when_label(entry.time, now).into(),
    }
}

/// The text on one line, short enough for a history row.
fn preview(text: &str) -> String {
    let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(160) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

fn sync_history(_ui: &AppWindow) {
    let now = history::now();
    let rows: Vec<TextEntry> =
        HISTORY.with(|h| h.borrow().as_ref().map(|list| list.iter().map(|e| to_row(e, now)).collect()).unwrap_or_default());
    MODEL.with(|m| m.set_vec(rows));
}

/// A capture was read: it goes to the top of the history. `size_px` is the
/// text's height in screen pixels.
pub(crate) fn add_reading(ui: &AppWindow, reading: &Reading, size_px: u32) {
    ensure_history(ui);
    let entry = history::Entry {
        text: reading.text.clone(),
        language: reading.language.clone(),
        sure: reading.sure,
        size: size_px,
        words: reading.boxes.len() as u32,
        time: history::now(),
    };
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        let list = h.get_or_insert_with(Vec::new);
        history::push(list, entry);
        history::save(list);
    });
    sync_history(ui);
    ui.set_text_detail(-1);
}

pub(crate) fn copy(text: &str) {
    if !text.trim().is_empty() {
        crate::ui_state::copy_text_async(text.to_string());
    }
}

pub(crate) fn delete(ui: &AppWindow, index: i32) {
    ensure_history(ui);
    HISTORY.with(|h| {
        if let Some(list) = h.borrow_mut().as_mut() {
            if index >= 0 && (index as usize) < list.len() {
                list.remove(index as usize);
                history::save(list);
            }
        }
    });
    sync_history(ui);
}

pub(crate) fn clear(ui: &AppWindow) {
    HISTORY.with(|h| *h.borrow_mut() = Some(Vec::new()));
    history::save(&[]);
    sync_history(ui);
    ui.set_text_detail(-1);
}

/// `archtoys --extract-file image.png` (used by CI's accuracy test): prints
/// the language, how sure it is and the text, and exits.
pub(crate) fn run_on_file(path: &str) -> i32 {
    let img = match image::open(path) {
        Ok(img) => img.into_rgba8(),
        Err(err) => {
            eprintln!("can't read {path}: {err}");
            return 2;
        }
    };
    let (w, h) = img.dimensions();
    match recognize(img.as_raw(), w, h, true, &|s| eprintln!("{s}")) {
        Ok(reading) => {
            println!("language: {}", reading.language);
            println!("sure: {}", reading.sure);
            println!("size: {:.0}", reading.size);
            println!("words: {}", reading.boxes.len());
            println!("---");
            println!("{}", reading.text);
            0
        }
        Err(err) => {
            eprintln!("{err}");
            1
        }
    }
}

/// `archtoys --print-install-script` (used by CI's install test): what "Turn
/// on" would run as administrator on this system.
pub(crate) fn print_install_script() -> i32 {
    let distro = install::detect();
    match install::script_for(&distro) {
        Some(script) => {
            println!("{script}");
            0
        }
        None => {
            eprintln!("{}", if let install::Distro::Other(_, why) = distro { why } else { String::new() });
            1
        }
    }
}

/// `archtoys --check-tesseract` (CI): exits 0 when a Tesseract Archtoys works with is installed.
pub(crate) fn check_tesseract() -> i32 {
    match engine::installed_version() {
        Some(v) if engine::version_ok(v) => {
            println!("tesseract {} ok", version_text(v));
            0
        }
        Some(v) => {
            println!("tesseract {} is too old (needs 4.1)", version_text(v));
            1
        }
        None => {
            println!("no tesseract");
            1
        }
    }
}
