//! The Screen Ruler (v0.7.4): measure anything on the screen, on the same
//! frozen screen the Color Picker and Text Extractor use. Inspired by
//! PixelKit's ruler (github.com/Kuucheen/PixelKit, MIT licence).
//!
//! * `logic`: the edges, the sizes as text, the preview, the icon (tested).
//! * `measure`: the ruler on the frozen screen.
//! * `screen_size`: a screen's real size, for cm and inches.
//! * `page`: what the ruler's page does.
//!
//! The history keeps the copied measurements (sizes only, never the
//! screenshot) in `~/.local/share/archtoys/ruler-history.json`.

mod logic;
mod measure;
pub(crate) mod page;
mod screen_size;

use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::{AppWindow, RulerBar, RulerEntry, RulerFrame, RulerLabel};
pub(crate) use logic::is_bright;
use logic::{Mode, Size, Unit};
use serde::{Deserialize, Serialize};
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

const MAX: usize = 500;

/// One copied measurement.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Entry {
    /// "Spacing"
    mode: String,
    /// each shape's width and height, in screen pixels
    shapes: Vec<[u32; 2]>,
    /// the screen's mm per pixel, when it reports its size
    mm_per_px: Option<f32>,
    /// seconds since 1970 (UTC)
    time: u64,
}

impl Entry {
    fn mode(&self) -> Mode {
        Mode::from_name(&self.mode).unwrap_or(Mode::Spacing)
    }

    fn sizes(&self) -> Vec<Size> {
        self.shapes.iter().map(|[w, h]| Size { w: *w, h: *h }).collect()
    }

    /// "300 × 150, 200 × 200 px" in `unit`
    fn text(&self, unit: Unit) -> String {
        logic::text_all(self.mode(), &self.sizes(), unit, self.mm_per_px)
    }
}

thread_local! {
    static HISTORY: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    /// the colors picked, for the lines' color (the last bright one)
    static PICKS: RefCell<Option<HistoryStore>> = const { RefCell::new(None) };
    /// the history entry the page's card shows (0: the newest)
    static SHOWN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// saves the settings (config.json), set at startup
    static SAVE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

fn path() -> PathBuf {
    crate::ocr::app_data_dir().join("ruler-history.json")
}

fn load() -> Vec<Entry> {
    let mut entries: Vec<Entry> =
        std::fs::read_to_string(path()).ok().and_then(|data| serde_json::from_str(&data).ok()).unwrap_or_default();
    entries.truncate(MAX);
    entries
}

fn save(entries: &[Entry]) {
    let path = path();
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("ruler history: couldn't make {}: {err}", parent.display());
            return;
        }
    }
    match serde_json::to_string(entries) {
        Ok(data) => {
            if let Err(err) = crate::config::write_atomic(&path, data.as_bytes()) {
                eprintln!("ruler history: couldn't save it: {err}");
            }
        }
        Err(err) => eprintln!("ruler history: {err}"),
    }
}

/// At startup: the history, the icon, the page. `save` saves the settings.
pub(crate) fn install(ui: &AppWindow, picks: &HistoryStore, save: impl Fn() + 'static) {
    HISTORY.with(|h| *h.borrow_mut() = load());
    PICKS.with(|p| *p.borrow_mut() = Some(picks.clone()));
    SAVE.with(|s| *s.borrow_mut() = Some(Box::new(save)));
    refresh(ui);
}

/// Saves the ruler's settings (its mode and tolerance changed on the screen).
pub(crate) fn save_settings(_ui: &AppWindow) {
    SAVE.with(|s| {
        if let Some(save) = s.borrow().as_ref() {
            save();
        }
    });
}

/// The lines' color: the last bright pick, else Archtoys orange.
fn line_color() -> Rgb {
    PICKS.with(|p| p.borrow().as_ref().map(|store| logic::line_color(&store.lock().unwrap())).unwrap_or(logic::DEFAULT_LINE))
}

/// Opens the ruler over the screen (the window steps aside first). UI thread only.
pub(crate) fn start(ui: &AppWindow) {
    if crate::picker::PICKER_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return; // a pick, a capture or a measurement is already running
    }
    crate::diagnostics::set_picker("screen ruler");
    let mode = match ui.get_setting_ruler_start() {
        0 => Mode::from_index(ui.get_setting_ruler_mode()),
        n => Mode::from_index(n - 1),
    };
    let start = measure::Start {
        mode,
        tolerance: ui.get_setting_ruler_tolerance().clamp(0, logic::TOLERANCE_MAX),
        buttons: ui.get_setting_ruler_buttons(),
        unit: Unit::from_index(ui.get_setting_ruler_units()),
        line: line_color(),
        dark: ui.global::<crate::Skin>().get_dark_t(),
        cross: ui.get_setting_ruler_cross(),
    };
    let (aside, visible) = crate::freeze::step_aside(ui);
    let ui_weak = ui.as_weak();
    let wayland = crate::picker::on_wayland();
    std::thread::spawn(move || {
        if visible {
            // give the desktop a moment to take the window off the screen
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let shot = crate::picker::take_screenshot(wayland);
        let _ = slint::invoke_from_event_loop(move || match shot {
            Ok(shot) => measure::show(ui_weak, shot, start, aside),
            Err(err) => {
                eprintln!("screen ruler: {err}");
                if let Some(ui) = ui_weak.upgrade() {
                    crate::ui_state::show_message(&ui, "The Screen Ruler couldn't take a screenshot");
                }
                crate::freeze::come_back(ui_weak, aside);
            }
        });
    });
}

/// A measurement was copied: it goes to the history (the newest first).
fn record(ui: &AppWindow, mode: Mode, sizes: &[Size], mm_per_px: Option<f32>) {
    let entry = Entry {
        mode: mode.name().to_string(),
        shapes: sizes.iter().map(|s| [s.w, s.h]).collect(),
        mm_per_px,
        time: crate::ocr::history::now(),
    };
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        add(&mut h, entry);
        save(&h);
    });
    SHOWN.with(|s| s.set(0));
    refresh(ui);
}

/// The newest first, the last 500 kept.
fn add(history: &mut Vec<Entry>, entry: Entry) {
    history.insert(0, entry);
    history.truncate(MAX);
}

/// The text of the measurement the card shows, in the chosen unit, for Copy on the page.
fn latest_text(ui: &AppWindow) -> Option<String> {
    let unit = Unit::from_index(ui.get_setting_ruler_units());
    let shown = SHOWN.with(|s| s.get());
    HISTORY.with(|h| h.borrow().get(shown).map(|e| e.text(unit)))
}

/// A click on a history entry: the card shows it.
fn select(ui: &AppWindow, index: i32) {
    SHOWN.with(|s| s.set(index.max(0) as usize));
    refresh(ui);
}

/// The entry the card shows after entry `deleted` went: the same one if it is
/// still there, else the one that took its place (or the last).
fn shown_after_delete(shown: usize, deleted: usize, len: usize) -> usize {
    let shown = if deleted < shown { shown - 1 } else { shown };
    shown.min(len.saturating_sub(1))
}

fn delete(ui: &AppWindow, index: i32) {
    HISTORY.with(|h| {
        let mut h = h.borrow_mut();
        if index >= 0 && (index as usize) < h.len() {
            h.remove(index as usize);
            save(&h);
            SHOWN.with(|s| s.set(shown_after_delete(s.get(), index as usize, h.len())));
        }
    });
    refresh(ui);
}

fn clear(ui: &AppWindow) {
    HISTORY.with(|h| {
        h.borrow_mut().clear();
        save(&[]);
    });
    SHOWN.with(|s| s.set(0));
    refresh(ui);
}

/// "Bounds · 3 shapes · Today 14:30"
fn detail(e: &Entry, now: u64) -> String {
    let when = crate::ocr::history::when_label(e.time, now);
    match e.shapes.len() {
        0 | 1 => format!("{} · {when}", e.mode),
        n => format!("{} · {n} shapes · {when}", e.mode),
    }
}

/// The page, Home's Recent and the icon, from the history.
fn refresh(ui: &AppWindow) {
    let now = crate::ocr::history::now();
    let (r, g, b) = line_color();
    let line = Color::from_rgb_u8(r, g, b);
    ui.set_ruler_line(line);
    HISTORY.with(|h| {
        let h = h.borrow();
        let shown = SHOWN.with(|s| s.get()).min(h.len().saturating_sub(1));
        let rows: Vec<RulerEntry> = h
            .iter()
            .enumerate()
            .map(|(i, e)| RulerEntry { text: e.text(Unit::Px).into(), detail: detail(e, now).into(), selected: i == shown && h.len() > 1 })
            .collect();
        ui.set_ruler_history(ModelRc::new(VecModel::from(rows)));
        let Some(newest) = h.first() else {
            ui.set_ruler_has_latest(false);
            ui.set_home_last_measure("".into());
            ui.set_ruler_icon(icon(4));
            return;
        };
        // the card: the entry clicked in the history, else the newest
        let latest = &h[shown];
        let (mode, sizes) = (latest.mode(), latest.sizes());
        let last_shape = sizes.last().copied().unwrap_or(Size { w: 0, h: 0 });
        let shapes = if sizes.len() > 1 { format!(" · {} SHAPES", sizes.len()) } else { String::new() };
        let when = crate::ocr::history::when_label(latest.time, now).to_uppercase();
        let which = if shown == 0 { "LATEST · " } else { "" };
        ui.set_ruler_has_latest(true);
        ui.set_ruler_latest_title(format!("{which}{}{shapes} · {when}", mode.name().to_uppercase()).into());
        ui.set_ruler_latest_big(logic::text(mode, last_shape, Unit::Px, None).into());
        ui.set_ruler_latest_physical(logic::physical_line(mode, last_shape, latest.mm_per_px).into());
        ui.set_ruler_latest_wide(sizes.len() > 1);
        let (pw, ph, preview) = logic::preview(mode, &sizes);
        let (mut bars, mut frames, mut labels) = (vec![], vec![], vec![]);
        for shape in &preview {
            let faint = shape.style == logic::Style::Faint;
            bars.extend(shape.bars.iter().map(|b| RulerBar { x: b.x, y: b.y, w: b.w, h: b.h, faint, fill: false }));
            if let Some(f) = shape.frame {
                if shape.style == logic::Style::Solid {
                    frames.push(RulerFrame { x: f.x, y: f.y, w: f.w, h: f.h, solid: true });
                } else {
                    bars.extend(logic::dashes(f.x, f.y, f.w, f.h, 2.0, 5.0, 3.0).iter().map(|b| RulerBar {
                        x: b.x,
                        y: b.y,
                        w: b.w,
                        h: b.h,
                        faint,
                        fill: false,
                    }));
                }
            }
            labels.push(RulerLabel { x: shape.label_x, y: shape.label_y, text: shape.label.clone().into(), centered: preview.len() > 1 });
        }
        ui.set_ruler_preview_w(pw);
        ui.set_ruler_preview_h(ph);
        ui.set_ruler_preview_bars(ModelRc::new(VecModel::from(bars)));
        ui.set_ruler_preview_frames(ModelRc::new(VecModel::from(frames)));
        ui.set_ruler_preview_labels(ModelRc::new(VecModel::from(labels)));
        // Home and the icon: always the newest
        ui.set_home_last_measure(newest.text(Unit::Px).into());
        ui.set_home_last_measure_detail(detail(newest, now).into());
        // the icon's teeth: a tooth per cm of the newest shape's longest side
        let newest_shape = newest.sizes().last().copied().unwrap_or(Size { w: 0, h: 0 });
        ui.set_ruler_icon(icon(logic::teeth(newest_shape.w.max(newest_shape.h), newest.mm_per_px)));
    });
}

fn icon(teeth: u32) -> slint::Image {
    slint::Image::load_from_svg_data(logic::icon_svg(teeth, 1.5).as_bytes()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_keeps_the_last_500_newest_first() {
        let mut history = vec![];
        for n in 0..=MAX as u32 {
            add(&mut history, Entry { mode: "Bounds".into(), shapes: vec![[n, n]], mm_per_px: None, time: u64::from(n) });
        }
        assert_eq!(history.len(), 500);
        assert_eq!(history[0].shapes, vec![[500, 500]]);
        assert_eq!(history[499].shapes, vec![[1, 1]]); // the oldest went
                                                       // only sizes go in the file (no screenshot): it reads back the same
        let data = serde_json::to_string(&history[..1]).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Entry>>(&data).unwrap(), history[..1]);
        assert_eq!(history[0].text(Unit::Px), "500 × 500 px");
    }

    #[test]
    fn the_card_keeps_showing_its_entry_after_a_delete() {
        // showing entry 3 of 6
        assert_eq!(shown_after_delete(3, 1, 5), 2); // one above it went: it moved up
        assert_eq!(shown_after_delete(3, 4, 5), 3); // one below it went: it stays
        assert_eq!(shown_after_delete(3, 3, 5), 3); // it went: the next one takes its place
        assert_eq!(shown_after_delete(5, 5, 5), 4); // the last went: the new last
        assert_eq!(shown_after_delete(0, 0, 0), 0); // nothing left
    }
}
