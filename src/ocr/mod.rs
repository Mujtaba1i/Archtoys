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

/// Reads the text in an RGBA image (the selected part of the screen).
/// Blocking: call it on a background thread. `status` says what it's doing
/// ("Downloading German…") when that takes a moment.
pub(crate) fn recognize(rgba: &[u8], width: u32, height: u32, keep_lines: bool, status: &dyn Fn(String)) -> Result<Reading, String> {
    let dir = lang::prepare_dir()?;
    let (image, scale) = engine::prepare(rgba, width, height);
    let read = |img: &image::GrayImage, scale: f32| {
        let png = temp_png();
        img.save(&png).map_err(|err| format!("couldn't save the image for Tesseract: {err}"))?;
        let result = read_png(&png, &dir, scale, keep_lines, status);
        let _ = std::fs::remove_file(&png);
        result
    };
    let mut best = read(&image, scale as f32)?;
    debug(|| format!("{scale}x: {}% sure, {:?}", best.sure, best.language));
    // Nothing read, or unsure: read it again with its lines about 32, then 20
    // pixels tall. Tesseract's Arabic reads nothing in letters much taller
    // than that (a heading, a word in an image), and its script detection
    // guesses wrong on them; the surest reading wins.
    if best.sure < SURE_READING {
        if let Some(line) = engine::line_height(&image) {
            for target in [32.0, 20.0] {
                let to = scale as f32 * target / line;
                // already about that size, or absurdly small
                if (to / scale as f32 - 1.0).abs() < 0.25 || line * to / (scale as f32) < 8.0 {
                    continue;
                }
                if let Ok(again) = read(&engine::rescale(&image, scale, to), to) {
                    debug(|| format!("lines {line:.0} px -> {target} px ({to:.2}x): {}% sure, {:?}", again.sure, again.language));
                    if again.sure > best.sure {
                        best = again;
                    }
                }
                if best.sure >= SURE_READING {
                    break;
                }
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

/// The writing system of the words read badly (another script mixed into
/// mostly English: its own detection over the whole image says "Latin").
fn unsure_script(png: &std::path::Path, dir: &std::path::Path, words: &[engine::Word]) -> Option<String> {
    let unsure: Vec<&engine::Word> = words.iter().filter(|w| (0.0..55.0).contains(&w.conf)).collect();
    let x0 = unsure.iter().map(|w| w.x).min()?;
    let y0 = unsure.iter().map(|w| w.y).min()?;
    let x1 = unsure.iter().map(|w| w.x + w.w).max()?;
    let y1 = unsure.iter().map(|w| w.y + w.h).max()?;
    let img = image::open(png).ok()?.to_luma8();
    let pad = 12;
    let (x0, y0) = ((x0 - pad).max(0) as u32, (y0 - pad).max(0) as u32);
    let (x1, y1) = (((x1 + pad) as u32).min(img.width()), ((y1 + pad) as u32).min(img.height()));
    if x1 <= x0 + 8 || y1 <= y0 + 8 {
        return None;
    }
    let crop = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
    let path = temp_png();
    crop.save(&path).ok()?;
    let script = engine::detect_script(&path, dir);
    let _ = std::fs::remove_file(&path);
    script
}

/// A reading this sure (mean word confidence) isn't tried again at another scale.
const SURE_READING: u8 = 70;

/// Reads with the layout found by Tesseract, and as one block when that finds
/// nothing (it can miss a single line or word).
fn run_words(png: &std::path::Path, langs: &str, dir: &std::path::Path) -> Result<Vec<engine::Word>, String> {
    let mut words = engine::parse_tsv(&engine::run(png, langs, dir, 3)?);
    if mean_conf(&words) < SURE_READING as f32 {
        // as one block: a single line its layout finder scrambled or missed
        let block = engine::parse_tsv(&engine::run(png, langs, dir, 6)?);
        if mean_conf(&block) > mean_conf(&words) {
            words = block;
        }
    }
    // no Hebrew, by choice: a word in Hebrew letters is dropped, whatever read it
    words.retain(|w| !w.text.chars().any(lang::hebrew));
    Ok(words)
}

fn read_png(png: &std::path::Path, dir: &std::path::Path, scale: f32, keep_lines: bool, status: &dyn Fn(String)) -> Result<Reading, String> {
    let have = |code: &str| lang::has(dir, code);
    let get = |code: &str| -> Result<(), String> {
        status(format!("Downloading {}…", lang::display_name(code)));
        let result = lang::download(dir, code);
        status("Reading…".to_string());
        result
    };

    let fetch = |codes: &[&'static str]| -> Vec<&'static str> {
        // the ones there, or downloaded now (offline: the rest are left out)
        codes.iter().copied().filter(|c| have(c) || get(c).is_ok()).collect()
    };

    // 1. English and the user's own languages (the desktop's, the keyboard
    // layouts', the input methods'): most text is in one of them
    let mut first = vec!["eng"];
    first.extend(lang::user_languages().iter().copied());
    let mut langs = fetch(&first);
    if langs.is_empty() {
        return Err("no language data: Tesseract's English is missing".into());
    }
    let mut words = run_words(png, &langs.join("+"), dir)?;

    // 2. unsure (or a word it couldn't read, maybe another script mixed in):
    // the writing system Tesseract sees there (Arabic, Cyrillic, Han ...),
    // with English for the English words mixed in. Its guess is unreliable on
    // a few words, so it must be clearly surer than a reading in the user's
    // own language.
    const GUESS_MARGIN: f32 = 10.0;
    let unsure_word = words.iter().any(|w| (0.0..55.0).contains(&w.conf));
    if mean_conf(&words) < SURE_READING as f32 || unsure_word {
        let mut script = if have("osd") { engine::detect_script(png, dir) } else { None };
        if have("osd") && unsure_word && matches!(script.as_deref(), None | Some("Latin")) {
            // mostly English: look at the words it couldn't read on their own
            script = unsure_script(png, dir, &words).or(script);
        }
        debug(|| format!("first {langs:?}: {:.0}% sure; script seen: {script:?}", mean_conf(&words)));
        // the user's own language was read first: a guess must be clearly surer
        let margin = if langs.len() > 1 { GUESS_MARGIN } else { 0.0 };
        let guess: Vec<&'static str> = lang::for_script(script.as_deref(), None)
            .into_iter()
            .filter(|c| !langs.contains(c))
            .collect();
        if !guess.is_empty() {
            let mut second = fetch(&guess);
            if !second.is_empty() {
                second.push("eng");
                if let Ok(other) = run_words(png, &second.join("+"), dir) {
                    debug(|| format!("then {second:?}: {:.0}% sure", mean_conf(&other)));
                    if mean_conf(&other) > mean_conf(&words) + margin {
                        words = other;
                        langs = second;
                    }
                }
            }
        }
    }
    let mut language = langs[0];

    // 3. which language exactly; read again with its own data if that's not what was used.
    // whatlang's own "reliable" flag stays off even for clear sentences; its
    // confidence is the useful part: sentences score 0.75-0.85, short snippets
    // ("ERROR: connection refused" scores French at 0.08) stay well under 0.5.
    const SURE_ENOUGH: f64 = 0.5;
    let text = engine::text_from_words(&words, true);
    if let Some(info) = whatlang::detect(&text) {
        let sure = info.confidence() >= SURE_ENOUGH;
        if let Some(code) = lang::from_whatlang(info.lang()) {
            if sure && !langs.contains(&code) && (have(code) || get(code).is_ok()) {
                let mut again = vec![code];
                if code != "eng" && langs.contains(&"eng") {
                    again.push("eng"); // keep English for the odd English word in a Latin text
                }
                if let Ok(better) = run_words(png, &again.join("+"), dir) {
                    if mean_conf(&better) >= mean_conf(&words) {
                        words = better;
                    }
                }
            }
            if sure {
                language = code;
            }
        }
    }

    let mut heights: Vec<i32> = words.iter().map(|w| w.h).collect();
    heights.sort_unstable();
    let size = heights.get(heights.len() / 2).copied().unwrap_or(0) as f32 / scale;
    Ok(Reading {
        text: engine::text_from_words(&words, keep_lines),
        one_line: engine::text_from_words(&words, false).replace("\n\n", " "),
        language: lang::display_name(language),
        sure: mean_conf(&words).round().clamp(0.0, 100.0) as u8,
        size,
        boxes: words.iter().map(|w| (w.x as f32 / scale, w.y as f32 / scale, w.w as f32 / scale, w.h as f32 / scale)).collect(),
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
