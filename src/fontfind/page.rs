//! The font finder in the app (v0.7.5): Find font in a capture, a font
//! entry's search and answer on Text Extractor's page, and its settings.
//! UI thread only, except the search itself (a background thread).

use super::{find, matcher, render, Guess, Progress, Source};
use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::ocr::history::{Entry, FontFind, FontGuess};
use crate::{AppWindow, FontFinder};
use image::GrayImage;
use slint::{ComponentHandle, Image, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Find font is offered from this "sure" up (the text reading's): it draws
/// the words that were read, so a wrong reading compares the wrong words.
pub(crate) const MIN_SURE: u8 = 70;
/// Whether Find font can be offered for a reading, and if not, why (for the
/// toolbar). It compares Latin letters only: its fonts are Latin fonts, and
/// Arabic came back backwards with 0%.
pub(crate) fn offer(reading: &crate::ocr::Reading) -> Result<(), String> {
    let letters: Vec<char> = reading.text.chars().filter(|c| c.is_alphabetic()).collect();
    let latin = letters.iter().filter(|&&c| (c as u32) < 0x250).count();
    if letters.is_empty() || latin * 5 < letters.len() * 4 {
        return Err(format!("Find font reads Latin letters only for now, not {}", reading.language));
    }
    if reading.sure < MIN_SURE {
        return Err(format!("Not sure enough of the text to find its font ({}% sure)", reading.sure));
    }
    Ok(())
}

/// Archtoys blue: the font finder's accent before any calm pick.
const BLUE: Rgb = (0x2F, 0x8F, 0xDD);

thread_local! {
    /// the search running: its entry's id, and its Stop
    static RUNNING: RefCell<Option<(u64, Arc<AtomicBool>)>> = const { RefCell::new(None) };
    /// the font entry open on the page
    static SHOWN: Cell<Option<u64>> = const { Cell::new(None) };
    /// the guess shown (a chip), and the text typed to see in it
    static CHOSEN: Cell<usize> = const { Cell::new(0) };
    static SAMPLE: RefCell<String> = const { RefCell::new(String::new()) };
    static PICKS: RefCell<Option<HistoryStore>> = const { RefCell::new(None) };
    static SAVE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// Font entries' pictures of the selection (only theirs: Search again needs it).
pub(crate) fn pictures_dir() -> PathBuf {
    crate::ocr::lang::app_data_dir().join("font-pictures")
}

/// The accent (the easter egg): the newest CALM pick, a color that isn't loud
/// (the ruler takes the bright ones, so one pick never changes both) and dark
/// enough for white text on it; else Archtoys blue.
fn calm_accent(picks_newest_first: &[Rgb]) -> Rgb {
    picks_newest_first
        .iter()
        .copied()
        .find(|&c| {
            let [_, a, b] = crate::color::rgb_to_oklab(c);
            (a * a + b * b).sqrt() >= 0.04 && !crate::ruler::is_bright(c) && contrast(c, (255, 255, 255)) >= 4.5
        })
        .unwrap_or(BLUE)
}

/// The accent, and its shades for text and lines on a dark and on a light background.
fn set_accent(g: &FontFinder<'_>) {
    let accent = PICKS.with(|p| p.borrow().as_ref().map(|s| calm_accent(&s.lock().unwrap())).unwrap_or(BLUE));
    let color = |(r, g, b): Rgb| slint::Color::from_rgb_u8(r, g, b);
    g.set_accent(color(accent));
    g.set_ink_on_dark(color(readable(accent, (0x1d, 0x1f, 0x25), (255, 255, 255))));
    g.set_ink_on_light(color(readable(accent, (255, 255, 255), (0, 0, 0))));
}

/// `c` mixed toward `toward` just enough to read on `bg` (4.5 : 1): a dark
/// navy pick was dark text on the dark theme's cards.
fn readable(c: Rgb, bg: Rgb, toward: Rgb) -> Rgb {
    (0..=20)
        .map(|step| {
            let t = step as f32 / 20.0;
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
            (mix(c.0, toward.0), mix(c.1, toward.1), mix(c.2, toward.2))
        })
        .find(|&m| contrast(m, bg) >= 4.5)
        .unwrap_or(toward)
}

/// WCAG contrast of two colors (1 to 21).
fn contrast(a: Rgb, b: Rgb) -> f32 {
    let lum = |(r, g, b): Rgb| {
        let ch = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(r) + 0.7152 * ch(g) + 0.0722 * ch(b)
    };
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The selection's text and background colors: the background is the most
/// common color, the text the average of the pixels farthest from it.
fn colors(rgba: &[u8]) -> (Rgb, Rgb) {
    let mut counts = std::collections::HashMap::new();
    for p in rgba.as_chunks::<4>().0.iter() {
        *counts.entry((p[0] & 0xF8, p[1] & 0xF8, p[2] & 0xF8)).or_insert(0u32) += 1;
    }
    let mode = counts.into_iter().max_by_key(|e| e.1).map(|e| e.0).unwrap_or((255, 255, 255));
    let near: Vec<Rgb> =
        rgba.as_chunks::<4>().0.iter().filter(|p| (p[0] & 0xF8, p[1] & 0xF8, p[2] & 0xF8) == mode).map(|p| (p[0], p[1], p[2])).collect();
    let avg = |px: &[Rgb]| {
        let n = px.len().max(1) as u32;
        let s = px.iter().fold((0u32, 0u32, 0u32), |a, c| (a.0 + c.0 as u32, a.1 + c.1 as u32, a.2 + c.2 as u32));
        ((s.0 / n) as u8, (s.1 / n) as u8, (s.2 / n) as u8)
    };
    let background = avg(&near);
    let dist = |c: Rgb| {
        (c.0 as i32 - background.0 as i32).abs() + (c.1 as i32 - background.1 as i32).abs() + (c.2 as i32 - background.2 as i32).abs()
    };
    let mut ink: Vec<Rgb> = rgba.as_chunks::<4>().0.iter().map(|p| (p[0], p[1], p[2])).collect();
    ink.sort_by_key(|&c| std::cmp::Reverse(dist(c)));
    ink.truncate((ink.len() / 50).max(1));
    (avg(&ink), background)
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// A drawing (dark on white) as a white picture whose ink is opaque: Slint
/// tints it with the theme's text color.
fn ink_image(gray: &GrayImage) -> Image {
    let Some(ink) = matcher::ink_box(gray) else { return Image::default() };
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(ink.width(), ink.height());
    for (px, g) in buffer.make_mut_slice().iter_mut().zip(ink.pixels()) {
        *px = Rgba8Pixel { r: 255, g: 255, b: 255, a: 255 - g.0[0] };
    }
    Image::from_rgba8(buffer)
}

/// A drawing on paper, as it is.
fn paper_image(gray: &GrayImage) -> Image {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(gray.width(), gray.height());
    for (px, g) in buffer.make_mut_slice().iter_mut().zip(gray.pixels()) {
        let v = g.0[0];
        *px = Rgba8Pixel { r: v, g: v, b: v, a: 255 };
    }
    Image::from_rgba8(buffer)
}

/// The text to draw in a guess: what was typed, else the entry's first line.
fn sample(entry: &Entry) -> String {
    let typed = SAMPLE.with(|s| s.borrow().clone());
    if !typed.trim().is_empty() {
        return typed;
    }
    let line = entry.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    line.chars().take(40).collect()
}

fn draw(file: &std::path::Path, text: &str, bold: bool, px: f32) -> Option<GrayImage> {
    render::Face::open(file)?.draw(text, px, bold)
}

fn style_name(category: u8, family: &str) -> &'static str {
    match category {
        0 => "Sans serif",
        1 => "Serif",
        2 => "Display",
        3 => "Handwriting",
        4 => "Monospace",
        // a local font: what its name says
        _ if family.to_lowercase().contains("mono") || family.to_lowercase().contains("code") => "Monospace",
        _ if family.to_lowercase().contains("serif") && !family.to_lowercase().contains("sans") => "Serif",
        _ => "Sans serif",
    }
}

fn licence_name(licence: u8) -> &'static str {
    match licence {
        1 => "Apache License 2.0",
        2 => "Ubuntu Font Licence",
        _ => "SIL Open Font License",
    }
}

fn label(g: &FontGuess) -> String {
    format!("{}{}", g.family, if g.bold { " Bold" } else { "" })
}

/// Settings' lines: the downloaded fonts and the saved pictures.
fn settings_info(ui: &AppWindow) {
    let g = ui.global::<FontFinder>();
    let (fonts, bytes) = super::downloaded();
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1e6);
    g.set_fonts_info(
        if fonts == 0 {
            "None yet".to_string()
        } else {
            format!("{fonts} font{} · {}, with their licence files", if fonts == 1 { "" } else { "s" }, mb(bytes))
        }
        .into(),
    );
    let pictures = crate::ocr::font_pictures();
    let size: u64 = pictures.iter().filter_map(|p| std::fs::metadata(pictures_dir().join(p)).ok()).map(|m| m.len()).sum();
    g.set_pictures_info(
        if pictures.is_empty() {
            "None yet".to_string()
        } else {
            format!(
                "{} picture{} · {}, kept with font entries. Deleting them keeps the fonts found",
                pictures.len(),
                if pictures.len() == 1 { "" } else { "s" },
                mb(size)
            )
        }
        .into(),
    );
}

/// At startup: the callbacks, the settings' state. `save` saves the settings.
pub(crate) fn wire(ui: &AppWindow, picks: &HistoryStore, save: impl Fn() + 'static) {
    PICKS.with(|p| *p.borrow_mut() = Some(picks.clone()));
    SAVE.with(|s| *s.borrow_mut() = Some(Box::new(save)));
    let g = ui.global::<FontFinder>();
    set_accent(&g);
    settings_info(ui);
    let weak = ui.as_weak();
    let with = move |f: &dyn Fn(&AppWindow)| {
        if let Some(ui) = weak.upgrade() {
            f(&ui);
        }
    };
    let w = with.clone();
    g.on_opened(move |index| w(&|ui| opened(ui, index)));
    let w = with.clone();
    g.on_allow(move |online| {
        w(&|ui| {
            let g = ui.global::<FontFinder>();
            g.set_online(online);
            g.set_asked(true);
            SAVE.with(|s| s.borrow().as_ref().map(|f| f()));
            if let Some(id) = SHOWN.with(Cell::get) {
                start(ui, id);
            }
        })
    });
    g.on_stop(|| RUNNING.with(|r| r.borrow().as_ref().map(|(_, stop)| stop.store(true, Ordering::Relaxed))).unwrap_or(()));
    let w = with.clone();
    g.on_search_again(move || {
        w(&|ui| {
            if let Some(id) = SHOWN.with(Cell::get) {
                start(ui, id);
            }
        })
    });
    let w = with.clone();
    g.on_choose(move |i| {
        CHOSEN.with(|c| c.set(i.max(0) as usize));
        w(&|ui| show_current(ui));
    });
    let w = with.clone();
    g.on_sample(move |text| {
        SAMPLE.with(|s| *s.borrow_mut() = text.to_string());
        w(&|ui| show_current(ui));
    });
    let w = with.clone();
    g.on_copy_name(move || {
        w(&|_| {
            let Some((_, entry)) = SHOWN.with(Cell::get).and_then(crate::ocr::entry) else { return };
            if let Some(guess) = entry.font.as_ref().and_then(|f| f.guesses.get(CHOSEN.with(Cell::get))) {
                crate::ui_state::copy_text_async(label(guess));
            }
        })
    });
    let w = with.clone();
    g.on_delete_fonts(move || {
        super::delete_downloaded();
        w(&|ui| {
            settings_info(ui);
            show_current(ui);
        });
    });
    let w = with.clone();
    g.on_delete_pictures(move || {
        let _ = std::fs::remove_dir_all(pictures_dir());
        w(&|ui| {
            crate::ocr::forget_font_pictures(ui);
            settings_info(ui);
            show_current(ui);
        });
    });
    g.on_online_changed(move |online| {
        with(&|ui| {
            let g = ui.global::<FontFinder>();
            g.set_online(online);
            g.set_asked(true);
            SAVE.with(|s| s.borrow().as_ref().map(|f| f()));
        })
    });
}

/// Find font in a capture: the selection's picture saved, a font entry made,
/// the app opened on it, and the search started (or, the very first time,
/// the question whether it may download fonts).
pub(crate) fn find_from_capture(ui: &AppWindow, reading: &crate::ocr::Reading, size_px: u32, rgba: &[u8], width: u32, height: u32) {
    let name = format!("{}.png", crate::ocr::history::now());
    let _ = std::fs::create_dir_all(pictures_dir());
    let saved = image::save_buffer(pictures_dir().join(&name), rgba, width, height, image::ExtendedColorType::Rgba8).is_ok();
    let (text_color, background) = colors(rgba);
    let font = FontFind {
        picture: if saved { name } else { String::new() },
        boxes: reading.boxes.clone(),
        words: reading.words.clone(),
        text_color: [text_color.0, text_color.1, text_color.2],
        background: [background.0, background.1, background.2],
        ..Default::default()
    };
    crate::ocr::add_entry(ui, reading, size_px, Some(font));
    SAMPLE.with(|s| s.borrow_mut().clear());
    ui.invoke_go(2);
    // opening it (index 0) shows it: opened() starts the search
    ui.set_text_detail(-1);
    ui.set_text_detail(0);
}

/// An entry opened on Text Extractor's page: a font entry shows its part.
fn opened(ui: &AppWindow, index: i32) {
    let id = crate::ocr::id_at(index);
    let font_entry = id.and_then(crate::ocr::entry).filter(|(_, e)| e.font.is_some());
    if SHOWN.with(Cell::get) != id {
        CHOSEN.with(|c| c.set(0));
        SAMPLE.with(|s| s.borrow_mut().clear());
    }
    SHOWN.with(|s| s.set(font_entry.as_ref().map(|(_, e)| e.time)));
    let Some((_, entry)) = font_entry else { return };
    let font = entry.font.clone().unwrap_or_default();
    let running = RUNNING.with(|r| r.borrow().as_ref().map(|(i, _)| *i)) == Some(entry.time);
    if !font.done && !running {
        // a new entry (or one a quit cut short): search, once allowed
        if ui.global::<FontFinder>().get_asked() {
            start(ui, entry.time);
        } else {
            show(ui, &entry);
            ui.global::<FontFinder>().set_state(0);
        }
        return;
    }
    show(ui, &entry);
}

/// The entry open now, again (a chip chosen, text typed, files deleted).
fn show_current(ui: &AppWindow) {
    if let Some((_, entry)) = SHOWN.with(Cell::get).and_then(crate::ocr::entry) {
        show(ui, &entry);
    }
}

/// A font entry's part on the page: the search, or its answer.
fn show(ui: &AppWindow, entry: &Entry) {
    let g = ui.global::<FontFinder>();
    let Some(font) = entry.font.as_ref() else { return };
    set_accent(&g);
    let picture = (!font.picture.is_empty()).then(|| pictures_dir().join(&font.picture)).filter(|p| p.exists());
    g.set_picture_gone(picture.is_none());
    g.set_picture(picture.and_then(|p| Image::load_from_path(&p).ok()).unwrap_or_default());
    if RUNNING.with(|r| r.borrow().as_ref().map(|(i, _)| *i)) == Some(entry.time) {
        g.set_state(1);
        return;
    }
    if !font.done {
        return;
    }
    if font.guesses.is_empty() {
        g.set_why("It compared the fonts it has and none looked alike enough. More text, or text that isn't bold or styled, gives it more to go on.".into());
        g.set_state(3);
        return;
    }
    let chosen = CHOSEN.with(Cell::get).min(font.guesses.len() - 1);
    let guess = &font.guesses[chosen];
    let chips: Vec<SharedString> = font.guesses.iter().take(3).map(|g| format!("{} · {}%", label(g), g.sure).into()).collect();
    g.set_guesses(ModelRc::new(VecModel::from(chips)));
    g.set_chosen(chosen as i32);
    let file = PathBuf::from(&guess.file);
    g.set_name(label(guess).into());
    g.set_name_image(draw(&file, &label(guess), guess.bold, 44.0).map(|d| ink_image(&d)).unwrap_or_default());
    g.set_sure(guess.sure as i32);
    g.set_size(format!("{} px", entry.size).into());
    g.set_pt(format!("{}", (entry.size as f32 * 0.75).round()).into());
    g.set_weight(if guess.bold { "700" } else { "400" }.into());
    g.set_style(style_name(guess.category, &guess.family).into());
    let (text, bg) =
        ((font.text_color[0], font.text_color[1], font.text_color[2]), (font.background[0], font.background[1], font.background[2]));
    g.set_text_color(slint::Color::from_rgb_u8(text.0, text.1, text.2));
    g.set_bg_color(slint::Color::from_rgb_u8(bg.0, bg.1, bg.2));
    g.set_text_hex(hex(text).into());
    g.set_bg_hex(hex(bg).into());
    g.set_contrast(format!("{:.1} : 1", contrast(text, bg)).into());
    g.set_guess_image(draw(&file, &sample(entry), guess.bold, 44.0).map(|d| paper_image(&d)).unwrap_or_default());
    g.set_credit(
        if guess.local {
            format!("{} · on your computer", guess.family)
        } else if guess.designer.is_empty() {
            format!("{} · Google Fonts · {}", guess.family, licence_name(guess.licence))
        } else {
            format!("{} · by {} · Google Fonts · {}", guess.family, guess.designer, licence_name(guess.licence))
        }
        .into(),
    );
    g.set_state(2);
}

fn saved_guess(g: &Guess) -> FontGuess {
    let (local, designer, licence, category) = match &g.source {
        Source::Local => (true, String::new(), 0, 9),
        Source::Google { designer, licence, category, .. } => (false, designer.clone(), *licence, *category),
    };
    FontGuess {
        family: g.family.clone(),
        bold: g.bold,
        sure: matcher::sure(g.score),
        file: g.file.to_string_lossy().into_owned(),
        local,
        designer,
        licence,
        category,
    }
}

/// Searches a font entry's picture (on a background thread), showing the
/// progress while its entry is open, and saves the answer in the history.
fn start(ui: &AppWindow, id: u64) {
    RUNNING.with(|r| {
        if let Some((_, stop)) = r.borrow_mut().take() {
            stop.store(true, Ordering::Relaxed);
        }
    });
    let Some((_, entry)) = crate::ocr::entry(id) else { return };
    let Some(font) = entry.font.clone() else { return };
    let g = ui.global::<FontFinder>();
    let picture = pictures_dir().join(&font.picture);
    let Ok(img) = image::open(&picture).map(|i| i.into_rgba8()) else {
        crate::ocr::update_font(ui, id, |f| f.done = true);
        show_current(ui);
        return;
    };
    let stop = Arc::new(AtomicBool::new(false));
    RUNNING.with(|r| *r.borrow_mut() = Some((id, stop.clone())));
    crate::ocr::update_font(ui, id, |f| {
        f.done = false;
        f.guesses.clear();
    });
    g.set_progress(-1.0);
    g.set_count("".into());
    g.set_download("".into());
    g.set_left("".into());
    g.set_best("".into());
    g.set_comparing("the fonts on your computer".into());
    g.set_comparing_image(Image::default());
    show_current(ui);
    let online = g.get_online();
    let reduce_motion = ui.get_setting_reduce_motion();
    let text = sample(&entry);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let (w, h) = img.dimensions();
        let gray = crate::ocr::prepared_gray(img.as_raw(), w, h);
        let words: Vec<_> = font.boxes.iter().copied().zip(font.words.iter().cloned()).collect();
        let started = std::time::Instant::now();
        let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
        let progress = |p: &Progress| {
            // at most ten updates a second
            let mut last = last.lock().unwrap();
            if last.elapsed().as_millis() < 100 && p.done < p.total {
                return;
            }
            *last = std::time::Instant::now();
            let (p, text, elapsed) = (p.clone(), text.clone(), started.elapsed().as_secs_f32());
            let _ = weak.upgrade_in_event_loop(move |ui| progress_shown(&ui, id, &p, &text, elapsed, reduce_motion));
        };
        let result = find(&gray, &words, online, &progress, &stop);
        let guesses: Vec<FontGuess> = result.map(|gs| gs.iter().map(saved_guess).collect()).unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            RUNNING.with(|r| {
                if r.borrow().as_ref().map(|(i, _)| *i) == Some(id) {
                    *r.borrow_mut() = None;
                }
            });
            crate::ocr::update_font(&ui, id, |f| {
                f.guesses = guesses;
                f.done = true;
            });
            CHOSEN.with(|c| c.set(0));
            settings_info(&ui);
            show_current(&ui);
        });
    });
}

/// The search view, while its entry is open.
fn progress_shown(ui: &AppWindow, id: u64, p: &Progress, text: &str, elapsed: f32, reduce_motion: bool) {
    if SHOWN.with(Cell::get) != Some(id) {
        return;
    }
    let g = ui.global::<FontFinder>();
    g.set_comparing(p.comparing.clone().into());
    g.set_progress(if p.total > 0 { p.done as f32 / p.total as f32 } else { -1.0 });
    g.set_count(format!("{} of {} fonts", p.done, p.total).into());
    g.set_download(if p.got > 0 { format!("{} KB · {:.1} MB/s", p.got / 1000, p.speed / 1e6) } else { String::new() }.into());
    let left = if p.done > 0 && p.done < p.total { elapsed / p.done as f32 * (p.total - p.done) as f32 } else { 0.0 };
    g.set_left(if left >= 1.0 { format!("about {} s left", left.round()) } else { String::new() }.into());
    if let Some((family, bold, score, _)) = &p.best {
        g.set_best(format!("{family}{} · {}%", if *bold { " Bold" } else { "" }, matcher::sure(*score)).into());
    }
    // the text in the font being compared (Reduce motion: the best so far, not every font flashing past)
    let shown = if reduce_motion { p.best.as_ref().map(|b| (b.3.clone(), b.1)) } else { p.comparing_file.clone().map(|f| (f, false)) };
    if let Some(img) = shown.and_then(|(f, b)| draw(&f, text, b, 44.0)).filter(|d| matcher::ink_box(d).is_some()) {
        g.set_comparing_image(paper_image(&img));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_accent_is_the_last_calm_pick() {
        // a bright orange (the ruler's), then a calm navy, then a pale gray
        assert_eq!(calm_accent(&[(252, 76, 2), (32, 60, 120), (200, 200, 200)]), (32, 60, 120));
        // only loud or gray picks: Archtoys blue
        assert_eq!(calm_accent(&[(252, 76, 2), (128, 128, 128)]), BLUE);
    }

    #[test]
    fn accent_text_is_readable_on_both_themes() {
        let navy = (20, 30, 70);
        assert!(contrast(readable(navy, (0x1d, 0x1f, 0x25), (255, 255, 255)), (0x1d, 0x1f, 0x25)) >= 4.5);
        assert!(contrast(readable(BLUE, (255, 255, 255), (0, 0, 0)), (255, 255, 255)) >= 4.5);
        // already readable: unchanged
        assert_eq!(readable(navy, (255, 255, 255), (0, 0, 0)), navy);
    }

    #[test]
    fn colors_and_contrast_of_a_selection() {
        // white paper, 3 dark pixels
        let mut rgba = vec![];
        for i in 0..100 {
            let v = if i < 3 { 34 } else { 253 };
            rgba.extend([v, v, v, 255]);
        }
        let (text, bg) = colors(&rgba);
        assert_eq!((text, bg), ((34, 34, 34), (253, 253, 253)));
        assert!((contrast((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 0.01);
    }
}
