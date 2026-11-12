//! Text Extractor's capture: the frozen screen (`freeze.rs`) in text mode,
//! where the user drags a rectangle over text. Reads the selection on a
//! background thread, outlines the words, and the toolbar (Copy, Copy as one
//! line, Open) or "Copy and close" ends it.

use crate::freeze::{self, Session, Toy};
use crate::picker::PICKER_ACTIVE;
use crate::ui_state::{hide_window, show_window};
use crate::{AppWindow, FreezePickerWindow, TextBox};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::Ordering;

/// How to put the main window back after a text capture: it is hidden (or
/// made see-through on KDE) during the capture, so the text behind it can be read.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextRestore {
    hidden: bool,
    faded: bool,
    /// "Open" was clicked: bring the window to the front too
    raise: bool,
}

/// One screenshot, shown frozen on every monitor, where the user drags a
/// rectangle over text. UI thread only.
pub(crate) fn start(ui: &AppWindow) {
    if PICKER_ACTIVE.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
        return; // a pick or a capture is already running
    }
    crate::diagnostics::set_picker("text capture");
    let visible = ui.window().is_visible() && !ui.window().is_minimized();
    // "Stay minimized": out of the way now, and nothing to bring back afterwards
    let minimize = visible && ui.get_setting_text_stay_minimized();
    if minimize {
        ui.window().set_minimized(true);
    }
    let faded = visible
        && !minimize
        && crate::kwin::available()
        && crate::kwin::set_main_window_opacity(0.0)
            .map_err(|err| eprintln!("text capture: couldn't fade the window ({err}), hiding it"))
            .is_ok();
    if visible && !faded && !minimize {
        hide_window(ui);
    }
    let restore = TextRestore { hidden: visible && !faded && !minimize, faded, raise: false };
    let ui_weak = ui.as_weak();
    let wayland = crate::picker::on_wayland();
    std::thread::spawn(move || {
        if visible {
            // give the desktop a moment to take the window off the screen
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
        let shot = crate::picker::take_screenshot(wayland);
        let _ = slint::invoke_from_event_loop(move || match shot {
            Ok(shot) => show(ui_weak, shot, restore),
            Err(err) => {
                eprintln!("text capture: {err}");
                if let Some(ui) = ui_weak.upgrade() {
                    crate::ui_state::show_message(&ui, "Text Extractor couldn't take a screenshot");
                }
                end(ui_weak, restore);
            }
        });
    });
}

/// Ends a text capture: the windows and screenshot go, the main window comes
/// back. Safe to call from any thread.
fn end(ui_weak: slint::Weak<AppWindow>, restore: TextRestore) {
    let result = slint::invoke_from_event_loop(move || {
        freeze::release_window();
        CURRENT.with(|c| c.borrow_mut().take());
        // Open on KDE's Wayland: KWin brings it forward (the app can't); it
        // also undoes the fade, and un-minimizes it after "Stay minimized"
        let kwin_raise = restore.raise && crate::kwin::available();
        if restore.faded || kwin_raise {
            let result = if kwin_raise { crate::kwin::raise_main_window() } else { crate::kwin::set_main_window_opacity(1.0) };
            if let Err(err) = result {
                eprintln!("text capture: couldn't bring the window back ({err})");
            }
        } else if restore.hidden {
            if let Some(ui) = ui_weak.upgrade() {
                show_window(&ui);
            }
        }
        PICKER_ACTIVE.store(false, Ordering::SeqCst);
        if restore.raise {
            if let Some(ui) = ui_weak.upgrade() {
                crate::bring_to_front(&ui);
            }
        }
        slint::Timer::single_shot(std::time::Duration::from_millis(500), crate::ui_state::return_free_memory);
    });
    if result.is_err() {
        PICKER_ACTIVE.store(false, Ordering::SeqCst);
    }
}

thread_local! {
    /// The running capture's reading, for the reader thread to hand its result
    /// to (UI thread only: what crosses threads can't hold it).
    static CURRENT: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

/// Shows the frozen screen for selecting text. UI thread only.
fn show(ui_weak: slint::Weak<AppWindow>, shot: freeze::Screenshot, restore: TextRestore) {
    let weak = ui_weak.clone();
    let fallback = Box::new(move || {
        if let Some(ui) = weak.upgrade() {
            crate::ui_state::show_message(&ui, "Text Extractor couldn't line the screenshot up with your screens");
        }
    });
    let state = Rc::new(State::default());
    CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));
    let toy = Rc::new(TextCapture { restore, state });
    freeze::show(ui_weak, toy, shot, fallback);
}

/// The last reading (with the text's height in screen pixels), and a counter
/// so a reading of an older selection (the user dragged again meanwhile) is dropped.
#[derive(Default)]
struct LastRead {
    generation: u64,
    reading: Option<(crate::ocr::Reading, u32)>,
}

#[derive(Default)]
struct State {
    text: RefCell<LastRead>,
    /// "Open" ended the capture: the main window comes to the front
    open_app: Cell<bool>,
}

struct TextCapture {
    restore: TextRestore,
    state: Rc<State>,
}

impl State {
    /// Ends the capture (once), putting the main window back.
    fn finish(&self, s: &Session, restore: TextRestore) {
        if s.end() {
            end(s.ui_weak.clone(), TextRestore { raise: self.open_app.get(), ..restore });
        }
    }

    /// The text was read (UI thread).
    fn reading_done(
        &self,
        s: &Session,
        restore: TextRestore,
        i: usize,
        generation: u64,
        origin: (i32, i32),
        result: Result<crate::ocr::Reading, String>,
    ) {
        if self.text.borrow().generation != generation {
            return; // a newer selection
        }
        let Some(ui) = s.ui_weak.upgrade() else { return };
        let windows = s.windows.borrow();
        let Some((window, _)) = windows.get(i) else { return };
        let reading = match result {
            Ok(reading) if !reading.text.trim().is_empty() => reading,
            Ok(_) => {
                window.set_busy_text("No text found here. Drag again, or Esc".into());
                return;
            }
            Err(err) => {
                eprintln!("text capture: {err}");
                window.set_busy_text(format!("Couldn't read it: {err}").into());
                return;
            }
        };
        window.set_busy(false);

        // outline the words, in the window's coordinates
        let layout = s.layout.borrow();
        if let Some(layout) = layout.as_ref() {
            let boxes: Vec<TextBox> = reading
                .boxes
                .iter()
                .map(|&(bx, by, bw, bh)| {
                    let (wx, wy) = layout.to_window(i, origin.0 as f32 + bx, origin.1 as f32 + by);
                    let (wx2, wy2) = layout.to_window(i, origin.0 as f32 + bx + bw, origin.1 as f32 + by + bh);
                    TextBox { x: wx, y: wy, w: wx2 - wx, h: wy2 - wy }
                })
                .collect();
            window.set_words(ModelRc::new(VecModel::from(boxes)));
        }
        // the text's height on screen (the screenshot can have more pixels than the screen)
        let step = layout.as_ref().and_then(|l| l.screens.get(i)).map(|s| s.step).unwrap_or(1).max(1) as f32;
        drop(layout);
        let size_px = (reading.size / step).round() as u32;

        // Only what is copied or opened goes to the history: a selection that
        // is dragged again, moved or cancelled is forgotten.
        if ui.get_setting_text_tools() {
            window.set_tools_info(format!("{} · {}% sure", reading.language, reading.sure).into());
            window.set_tools(true);
            self.text.borrow_mut().reading = Some((reading, size_px));
        } else {
            // copy and close at once
            crate::ocr::copy(&reading.text);
            crate::ocr::add_reading(&ui, &reading, size_px);
            drop(windows);
            self.finish(s, restore);
        }
    }
}

impl Toy for TextCapture {
    fn window_created(&self, window: &FreezePickerWindow, _i: usize) {
        window.set_text_mode(true);
    }

    /// A new selection started (or the old one moved): forget what was read.
    fn region_started(&self, s: &Rc<Session>) {
        self.state.text.borrow_mut().generation += 1;
        self.state.text.borrow_mut().reading = None;
        for (window, _) in s.windows.borrow().iter() {
            window.set_words(ModelRc::default());
            window.set_tools(false);
            window.set_busy(false);
        }
    }

    /// The selection is done: read the text in it (on a background thread).
    fn region_done(&self, s: &Rc<Session>, i: usize, x: f32, y: f32, w: f32, h: f32) {
        let Some(((x0, y0), (x1, y1))) = s.pixel(i, x, y).zip(s.pixel(i, x + w, y + h)) else { return };
        let (cw, ch) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
        // the selected pixels, as RGBA
        let mut crop = vec![0u8; (cw * ch * 4) as usize];
        for row in 0..ch {
            let src = (((y0 as u32 + row) * s.shot.width + x0 as u32) * 4) as usize;
            let dst = (row * cw * 4) as usize;
            crop[dst..dst + (cw * 4) as usize].copy_from_slice(&s.shot.rgba[src..src + (cw * 4) as usize]);
        }
        let generation = {
            let mut text = self.state.text.borrow_mut();
            text.generation += 1;
            text.generation
        };
        if let Some((window, _)) = s.windows.borrow().get(i) {
            window.set_busy_text("Reading…".into());
            window.set_busy(true);
        }
        let keep_lines = s.ui_weak.upgrade().is_some_and(|ui| ui.get_setting_text_keep_lines());
        let window_weak = s.windows.borrow().get(i).map(|(w, _)| w.as_weak());
        let (tx, rx) = std::sync::mpsc::channel::<Result<crate::ocr::Reading, String>>();
        let restore = self.restore;
        std::thread::spawn(move || {
            let status = |text: String| {
                if let Some(w) = window_weak.clone() {
                    let _ = w.upgrade_in_event_loop(move |w| w.set_busy_text(text.into()));
                }
            };
            let _ = tx.send(crate::ocr::recognize(&crop, cw, ch, keep_lines, &status));
            let _ = slint::invoke_from_event_loop(move || {
                // the capture still running (the generation drops a stale reading)
                let state = CURRENT.with(|c| c.borrow().clone());
                let (Some(session), Some(state), Ok(result)) = (freeze::active(), state, rx.recv()) else { return };
                if session.running() {
                    state.reading_done(&session, restore, i, generation, (x0, y0), result);
                }
            });
        });
    }

    /// A toolbar button: "copy", "line" (as one line) or "open" (in the app).
    fn tool(&self, s: &Rc<Session>, action: &str) {
        let Some((reading, size_px)) = self.state.text.borrow().reading.clone() else { return };
        if let Some(ui) = s.ui_weak.upgrade() {
            crate::ocr::add_reading(&ui, &reading, size_px);
        }
        match action {
            "copy" => crate::ocr::copy(&reading.text),
            "line" => crate::ocr::copy(&reading.one_line),
            _ => {
                if let Some(ui) = s.ui_weak.upgrade() {
                    ui.invoke_go(2);
                    ui.set_text_detail(0);
                }
                // shown and brought to the front once the capture has ended
                self.state.open_app.set(true);
            }
        }
        self.state.finish(s, self.restore);
    }

    fn cancelled(&self, s: &Rc<Session>) {
        end(s.ui_weak.clone(), TextRestore { raise: self.state.open_app.get(), ..self.restore });
    }

    fn gave_up(&self, s: &Rc<Session>) {
        end(s.ui_weak.clone(), self.restore);
    }
}
