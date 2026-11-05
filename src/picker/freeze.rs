//! Freeze-frame picker: one screenshot, shown fullscreen in our own windows,
//! one per monitor. Inside our own window we may read the mouse position,
//! even on Wayland, so we can show the same live preview and magnifier as on X11.
//!
//! The screenshot shows every monitor (their bounding box, scaled by one
//! factor). Each window shows its own monitor's part: the monitor's position
//! comes from winit, its size from the fullscreen window on it. If that
//! doesn't add up to the screenshot, the other picker is used.
//!
//! The screenshot file is deleted as soon as it has been loaded into memory,
//! and the image is freed when the pick ends.

use super::{
    center_rgb, crop_rgba, end_text_capture, finish_picker, magnifier_image, preview_text, show_hover, PickOutcome,
    PickerContext, TextRestore, MAGNIFIER_RADIUS,
};
use crate::color::Rgb;
use crate::config::HistoryStore;
use crate::{AppWindow, FreezePickerWindow, TextBox};
use slint::winit_030::winit::monitor::MonitorHandle;
use slint::winit_030::winit::window::Fullscreen;
use slint::winit_030::WinitWindowAccessor;
use slint::{Color, ComponentHandle, ModelRc, SharedPixelBuffer, VecModel};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::Duration;

pub(crate) struct Screenshot {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// A monitor's place in the desktop layout, in the layout's own units:
/// logical pixels on Wayland, physical pixels on X11.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

/// Where the screenshot sits: the layout point (x, y) is its top-left pixel,
/// and one layout unit is (sx, sy) screenshot pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    x: f32,
    y: f32,
    sx: f32,
    sy: f32,
}

/// The screenshot covers the monitors' bounding box, scaled the same way
/// across and down (within 3%). None if it doesn't (e.g. a monitor is missing).
fn fit(rects: &[Rect], img_w: u32, img_h: u32) -> Option<Fit> {
    let x0 = rects.iter().map(|r| r.x).fold(f32::INFINITY, f32::min);
    let y0 = rects.iter().map(|r| r.y).fold(f32::INFINITY, f32::min);
    let x1 = rects.iter().map(|r| r.x + r.w).fold(f32::NEG_INFINITY, f32::max);
    let y1 = rects.iter().map(|r| r.y + r.h).fold(f32::NEG_INFINITY, f32::max);
    let (w, h) = (x1 - x0, y1 - y0);
    if rects.is_empty() || !(w >= 2.0 && h >= 2.0) || img_w == 0 || img_h == 0 {
        return None;
    }
    let (sx, sy) = (img_w as f32 / w, img_h as f32 / h);
    ((sx - sy).abs() / sx < 0.03).then_some(Fit { x: x0, y: y0, sx, sy })
}

/// One monitor: its window, its place, layout units per logical pixel of the
/// window, and how many screenshot pixels one of its screen pixels is.
struct Screen {
    rect: Rect,
    unit: f32,
    step: i32,
}

struct Layout {
    fit: Fit,
    screens: Vec<Screen>,
}

impl Layout {
    fn new(fit: Fit, rects: &[Rect], units: &[f32], scales: &[f32]) -> Self {
        let screens = rects
            .iter()
            .zip(units)
            .zip(scales)
            .map(|((&rect, &unit), &scale)| Screen {
                rect,
                unit,
                step: ((fit.sx * unit / scale.max(0.1)).round() as i32).max(1),
            })
            .collect();
        Self { fit, screens }
    }

    /// A point in monitor `i`'s window (logical pixels) → screenshot pixel.
    fn pixel(&self, i: usize, x: f32, y: f32, img_w: u32, img_h: u32) -> (i32, i32) {
        let s = &self.screens[i];
        let gx = s.rect.x + x * s.unit;
        let gy = s.rect.y + y * s.unit;
        let ix = ((gx - self.fit.x) * self.fit.sx).floor() as i32;
        let iy = ((gy - self.fit.y) * self.fit.sy).floor() as i32;
        (ix.clamp(0, img_w as i32 - 1), iy.clamp(0, img_h as i32 - 1))
    }

    /// Screenshot pixel → (monitor, logical point in its window). None in a
    /// gap between monitors of different sizes.
    fn place(&self, ix: i32, iy: i32) -> Option<(usize, f32, f32)> {
        let gx = self.fit.x + (ix as f32 + 0.5) / self.fit.sx;
        let gy = self.fit.y + (iy as f32 + 0.5) / self.fit.sy;
        let i = self.screens.iter().position(|s| s.rect.contains(gx, gy))?;
        let s = &self.screens[i];
        Some((i, (gx - s.rect.x) / s.unit, (gy - s.rect.y) / s.unit))
    }

    /// Screenshot pixel → a point in monitor `i`'s window, even outside it
    /// (a word box that crosses the monitor's edge).
    fn to_window(&self, i: usize, ix: f32, iy: f32) -> (f32, f32) {
        let s = &self.screens[i];
        let gx = self.fit.x + ix / self.fit.sx;
        let gy = self.fit.y + iy / self.fit.sy;
        ((gx - s.rect.x) / s.unit, (gy - s.rect.y) / s.unit)
    }

    /// The pixel one screen pixel away in the direction (dx, dy), or None at
    /// the outer edge of the desktop.
    fn nudge(&self, (ix, iy): (i32, i32), dx: i32, dy: i32, img_w: u32, img_h: u32) -> Option<(i32, i32)> {
        let (i, _, _) = self.place(ix, iy)?;
        let step = self.screens[i].step;
        let (nx, ny) = (ix + dx * step, iy + dy * step);
        if nx < 0 || ny < 0 || nx >= img_w as i32 || ny >= img_h as i32 {
            return None;
        }
        self.place(nx, ny).map(|_| (nx, ny))
    }
}

/// What the frozen screen is for.
pub(crate) enum Purpose {
    /// Picking a color (the Color Picker).
    Color { history_store: HistoryStore, context: PickerContext },
    /// Selecting text (Text Extractor), and how to put the main window back.
    Text(TextRestore),
}

/// Text Extractor's capture: the last reading (with the text's height in
/// screen pixels), and a counter so a reading of an older selection (the user
/// dragged again meanwhile) is dropped.
#[derive(Default)]
struct TextCapture {
    generation: u64,
    reading: Option<(crate::ocr::Reading, u32)>,
}

/// One running freeze pick (or text capture).
struct Session {
    shot: Screenshot,
    image: slint::Image,
    ui_weak: slint::Weak<AppWindow>,
    purpose: Purpose,
    text: RefCell<TextCapture>,
    /// The windows, with the monitor each was put on (none: just one window).
    windows: RefCell<Vec<(FreezePickerWindow, Option<MonitorHandle>)>>,
    layout: RefCell<Option<Layout>>,
    /// The pick point, in screenshot pixels.
    point: Cell<Option<(i32, i32)>>,
    /// The window showing the preview card.
    shown_on: Cell<Option<usize>>,
    /// The newest mouse position (window, x, y), not handled yet.
    latest: Cell<Option<(usize, f32, f32)>>,
    /// Where the mouse was before the windows were laid out: the desktop
    /// reports it as soon as a window appears under the mouse, which is
    /// before the layout. Shown once laid out, so the magnifier is there at once.
    early: Cell<Option<(usize, f32, f32)>>,
    done: Cell<bool>,
    /// "Open" ended the text capture: the main window comes to the front
    open_app: Cell<bool>,
    fallback: RefCell<Option<Box<dyn FnOnce()>>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

fn active() -> Option<Rc<Session>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Cancels the freeze pick, if one is running. UI thread only.
pub(crate) fn cancel_active() {
    if let Some(session) = active() {
        session.finish(PickOutcome::Cancelled);
    }
}

/// Arrow keys pressed in the main window. UI thread only.
pub(crate) fn nudge_active(dx: i32, dy: i32) {
    if let Some(session) = active() {
        session.nudge(dx, dy);
    }
}

/// Enter pressed in the main window. UI thread only.
pub(crate) fn pick_active_point() {
    if let Some(session) = active() {
        session.pick_point();
    }
}

/// Puts the main window above the freeze window, without moving it, so the
/// user sees the live values while picking ("Minimize on Pick" off).
fn raise_main_window(ui: &AppWindow) {
    ui.set_keep_on_top(true); // X11: the freeze window is always-on-top too
    ui.set_picking(true);
    ui.window().with_winit_window(|window| window.focus_window());
    ui.invoke_focus_pick_scope();
}

/// Undoes `raise_main_window`. UI thread only.
pub(super) fn end_main_window_raise(ui: &AppWindow) {
    ui.set_keep_on_top(false);
    ui.set_picking(false);
}

/// Hides and drops the freeze windows and the screenshot. UI thread only.
pub(super) fn release_window() {
    if let Some(session) = ACTIVE.with(|a| a.borrow_mut().take()) {
        session.hide_all();
    }
}

/// Loads a PNG screenshot and deletes the file right away.
pub(crate) fn load_png(path: &Path) -> Result<Screenshot, String> {
    let loaded = image::open(path);
    if let Err(err) = std::fs::remove_file(path) {
        eprintln!("freeze picker: could not delete {}: {err}", path.display());
    }
    let img = loaded
        .map_err(|err| format!("reading screenshot {}: {err}", path.display()))?
        .into_rgba8();
    let (width, height) = img.dimensions();
    Ok(Screenshot {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn logical_size(w: &FreezePickerWindow) -> (f32, f32, f32) {
    let size = w.window().size();
    let scale = w.window().scale_factor();
    (size.width as f32 / scale, size.height as f32 / scale, scale)
}

/// One window per place on the screen. winit can list the same monitor twice
/// (seen on KDE Wayland: a laptop's one screen as eDP-1 twice), and mirrored
/// screens share a place too. Two windows stacked on one place hide the
/// preview: it is drawn on the lower one.
fn one_per_place<T, K: PartialEq>(items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
    let mut seen = Vec::new();
    items
        .into_iter()
        .filter(|item| {
            let k = key(item);
            let new = !seen.contains(&k);
            if new {
                seen.push(k);
            }
            new
        })
        .collect()
}

/// Makes the window fullscreen on `monitor`, once winit has created it.
fn put_on(window: &FreezePickerWindow, monitor: MonitorHandle) {
    let weak = window.as_weak();
    let started = slint::spawn_local(async move {
        let Some(window) = weak.upgrade() else { return };
        if let Ok(w) = window.window().winit_window().await {
            w.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
        }
    });
    if let Err(err) = started {
        eprintln!("freeze picker: couldn't move a window to its monitor: {err:?}");
    }
}

fn is_wayland(w: &FreezePickerWindow) -> bool {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    w.window()
        .with_winit_window(|window| {
            window
                .window_handle()
                .is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(..)))
        })
        .unwrap_or(false)
}

impl Session {
    /// Ends picking (or the text capture) exactly once.
    fn finish(&self, outcome: PickOutcome) {
        if !self.done.replace(true) {
            match &self.purpose {
                Purpose::Color { history_store, context } => {
                    finish_picker(self.ui_weak.clone(), history_store.clone(), *context, outcome)
                }
                Purpose::Text(restore) => {
                    end_text_capture(self.ui_weak.clone(), TextRestore { raise: self.open_app.get(), ..*restore })
                }
            }
        }
    }

    // --- Text Extractor's capture ---

    /// A new selection started (or the old one moved): forget what was read.
    fn region_started(&self) {
        self.text.borrow_mut().generation += 1;
        self.text.borrow_mut().reading = None;
        for (window, _) in self.windows.borrow().iter() {
            window.set_words(ModelRc::default());
            window.set_tools(false);
            window.set_busy(false);
        }
    }

    /// The selection is done: read the text in it (on a background thread).
    fn region_done(self: &Rc<Self>, i: usize, x: f32, y: f32, w: f32, h: f32) {
        let Some(((x0, y0), (x1, y1))) = self.pixel(i, x, y).zip(self.pixel(i, x + w, y + h)) else { return };
        let (cw, ch) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
        // the selected pixels, as RGBA
        let mut crop = vec![0u8; (cw * ch * 4) as usize];
        for row in 0..ch {
            let src = (((y0 as u32 + row) * self.shot.width + x0 as u32) * 4) as usize;
            let dst = (row * cw * 4) as usize;
            crop[dst..dst + (cw * 4) as usize].copy_from_slice(&self.shot.rgba[src..src + (cw * 4) as usize]);
        }
        let generation = {
            let mut text = self.text.borrow_mut();
            text.generation += 1;
            text.generation
        };
        if let Some((window, _)) = self.windows.borrow().get(i) {
            window.set_busy_text("Reading…".into());
            window.set_busy(true);
        }
        let keep_lines = self.ui_weak.upgrade().is_some_and(|ui| ui.get_setting_text_keep_lines());
        let window_weak = self.windows.borrow().get(i).map(|(w, _)| w.as_weak());
        let (tx, rx) = std::sync::mpsc::channel::<Result<crate::ocr::Reading, String>>();
        std::thread::spawn(move || {
            let status = |text: String| {
                if let Some(w) = window_weak.clone() {
                    let _ = w.upgrade_in_event_loop(move |w| w.set_busy_text(text.into()));
                }
            };
            let _ = tx.send(crate::ocr::recognize(&crop, cw, ch, keep_lines, &status));
            let _ = slint::invoke_from_event_loop(move || {
                // the session still running (the generation drops a stale reading)
                let (Some(session), Ok(result)) = (active(), rx.recv()) else { return };
                session.reading_done(i, generation, (x0, y0), result);
            });
        });
    }

    /// The text was read (UI thread).
    fn reading_done(&self, i: usize, generation: u64, origin: (i32, i32), result: Result<crate::ocr::Reading, String>) {
        if self.done.get() || self.text.borrow().generation != generation {
            return; // a newer selection, or the capture ended
        }
        let Some(ui) = self.ui_weak.upgrade() else { return };
        let windows = self.windows.borrow();
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
        let layout = self.layout.borrow();
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
            self.finish(PickOutcome::Cancelled);
        }
    }

    /// A toolbar button: "copy", "line" (as one line) or "open" (in the app).
    fn tool(&self, action: &str) {
        let Some((reading, size_px)) = self.text.borrow().reading.clone() else { return };
        if let Some(ui) = self.ui_weak.upgrade() {
            crate::ocr::add_reading(&ui, &reading, size_px);
        }
        match action {
            "copy" => crate::ocr::copy(&reading.text),
            "line" => crate::ocr::copy(&reading.one_line),
            _ => {
                if let Some(ui) = self.ui_weak.upgrade() {
                    ui.invoke_go(2);
                    ui.set_text_detail(0);
                }
                // shown and brought to the front once the capture has ended
                self.open_app.set(true);
            }
        }
        self.finish(PickOutcome::Cancelled);
    }

    fn hide_all(&self) {
        for (window, _) in self.windows.borrow().iter() {
            window.hide().ok();
        }
    }

    /// The screenshot can't be used: hide everything and use the other picker.
    fn give_up(&self) {
        self.done.set(true);
        release_window();
        self.hide_all();
        if let Purpose::Text(restore) = &self.purpose {
            end_text_capture(self.ui_weak.clone(), *restore);
        }
        if let Some(fallback) = self.fallback.borrow_mut().take() {
            fallback();
        }
    }

    fn rgb_at(&self, (ix, iy): (i32, i32)) -> (Vec<u8>, Rgb) {
        let area = crop_rgba(&self.shot.rgba, self.shot.width, self.shot.height, ix, iy, MAGNIFIER_RADIUS);
        let rgb = center_rgb(&area, MAGNIFIER_RADIUS);
        (area, rgb)
    }

    fn pixel(&self, i: usize, x: f32, y: f32) -> Option<(i32, i32)> {
        let layout = self.layout.borrow();
        let layout = layout.as_ref()?;
        Some(layout.pixel(i, x, y, self.shot.width, self.shot.height))
    }

    /// Shows the preview at the pick point, on the monitor it is on.
    fn show_point(&self, marker: bool) {
        // a text capture has no color preview: showing one put the color under
        // the mouse on the Color Picker's card, and nothing put it back
        if matches!(self.purpose, Purpose::Text(_)) {
            return;
        }
        let Some(point) = self.point.get() else { return };
        let Some((i, x, y)) = self.layout.borrow().as_ref().and_then(|l| l.place(point.0, point.1)) else {
            return;
        };
        let (area, rgb) = self.rgb_at(point);
        let windows = self.windows.borrow();
        if let Some(old) = self.shown_on.replace(Some(i)).filter(|&old| old != i) {
            if let Some((w, _)) = windows.get(old) {
                w.set_preview_hex("".into());
                w.set_marker(false);
            }
        }
        let Some((w, _)) = windows.get(i) else { return };
        w.set_cursor_x(x);
        w.set_cursor_y(y);
        w.set_marker(marker);
        w.set_magnifier(magnifier_image(&area, MAGNIFIER_RADIUS));
        w.set_preview_color(Color::from_rgb_u8(rgb.0, rgb.1, rgb.2));
        w.set_preview_hex(preview_text(rgb));
        drop(windows);
        if let Some(ui) = self.ui_weak.upgrade() {
            show_hover(&ui, rgb);
        }
    }

    /// A mouse move reports x and y separately, and moves come in bursts.
    /// Keep only the newest position and update once per event-loop turn,
    /// so the work never piles up (that's what made hovering choppy).
    fn pointer_moved(self: &Rc<Self>, i: usize, x: f32, y: f32) {
        if self.latest.replace(Some((i, x, y))).is_some() {
            return; // already scheduled
        }
        let weak = Rc::downgrade(self);
        slint::Timer::single_shot(Duration::ZERO, move || {
            let Some(session) = weak.upgrade() else { return };
            let Some((i, x, y)) = session.latest.take() else { return };
            if let Some(point) = session.pixel(i, x, y) {
                session.point.set(Some(point));
                session.show_point(false);
            } else {
                session.early.set(Some((i, x, y)));
            }
        });
    }

    fn nudge(&self, dx: i32, dy: i32) {
        let (w, h) = (self.shot.width, self.shot.height);
        let moved = {
            let layout = self.layout.borrow();
            let Some(layout) = layout.as_ref() else { return };
            // no point yet (the mouse hasn't moved): start in the middle of the first monitor
            let start = self.point.get().unwrap_or_else(|| {
                let r = layout.screens[0].rect;
                let (x, y) = ((r.w / layout.screens[0].unit) / 2.0, (r.h / layout.screens[0].unit) / 2.0);
                layout.pixel(0, x, y, w, h)
            });
            layout.nudge(start, dx, dy, w, h).or(Some(start))
        };
        self.point.set(moved);
        self.show_point(true);
    }

    fn pick_point(&self) {
        if let Some(point) = self.point.get() {
            let (_, rgb) = self.rgb_at(point);
            self.finish(PickOutcome::Picked(rgb));
        }
    }

    /// Creates, wires and shows window number `i`.
    fn add_window(self: &Rc<Self>, monitor: Option<MonitorHandle>) -> Result<(), String> {
        let window = FreezePickerWindow::new().map_err(|err| format!("window failed: {err:?}"))?;
        window.global::<crate::Skin>().set_real_shadows(!crate::cpu_drawn());
        window.set_screenshot_image(self.image.clone());
        window.set_text_mode(matches!(self.purpose, Purpose::Text(_)));
        let i = self.windows.borrow().len();
        let weak: Weak<Session> = Rc::downgrade(self);
        {
            let weak = weak.clone();
            window.on_region_started(move || {
                if let Some(s) = weak.upgrade() {
                    s.region_started();
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_region_done(move |x, y, w, h| {
                if let Some(s) = weak.upgrade() {
                    s.region_done(i, x, y, w, h);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_tool(move |action| {
                if let Some(s) = weak.upgrade() {
                    s.tool(&action);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_pointer_moved(move |x, y| {
                if let Some(s) = weak.upgrade() {
                    s.pointer_moved(i, x, y);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_pick_at(move |x, y| {
                let Some(s) = weak.upgrade() else { return };
                if let Some(point) = s.pixel(i, x, y) {
                    let (_, rgb) = s.rgb_at(point);
                    s.finish(PickOutcome::Picked(rgb));
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_cancel_pick(move || {
                if let Some(s) = weak.upgrade() {
                    s.finish(PickOutcome::Cancelled);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_nudge(move |dx, dy| {
                if let Some(s) = weak.upgrade() {
                    s.nudge(dx, dy);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_pick_point(move || {
                if let Some(s) = weak.upgrade() {
                    s.pick_point();
                }
            });
        }
        window.window().on_close_requested(move || {
            if let Some(s) = weak.upgrade() {
                s.finish(PickOutcome::Cancelled);
            }
            slint::CloseRequestResponse::HideWindow
        });
        window.show().map_err(|err| format!("show failed: {err:?}"))?;
        if let Some(monitor) = &monitor {
            put_on(&window, monitor.clone());
        }
        self.windows.borrow_mut().push((window, monitor));
        Ok(())
    }

    /// With several monitors: moves the first window to the first monitor and
    /// opens one more window on each of the others.
    fn cover_all_monitors(self: &Rc<Self>) {
        let monitors: Vec<MonitorHandle> = {
            let windows = self.windows.borrow();
            let Some((first, _)) = windows.first() else { return };
            first
                .window()
                .with_winit_window(|w| w.available_monitors().collect())
                .unwrap_or_default()
        };
        let monitors = one_per_place(monitors, |m| (m.position(), m.size()));
        if monitors.len() < 2 {
            return;
        }
        if let Some((first, slot)) = self.windows.borrow_mut().first_mut() {
            put_on(first, monitors[0].clone());
            *slot = Some(monitors[0].clone());
        }
        for monitor in monitors.into_iter().skip(1) {
            if let Err(err) = self.add_window(Some(monitor)) {
                eprintln!("freeze picker: {err}");
                self.give_up();
                return;
            }
        }
    }

    /// Each monitor's place, once every window has its fullscreen size on
    /// its own monitor. None while they are still getting there.
    fn measure(&self) -> Option<(Vec<Rect>, Vec<f32>, Vec<f32>)> {
        let windows = self.windows.borrow();
        let (mut rects, mut units, mut scales) = (vec![], vec![], vec![]);
        for (window, monitor) in windows.iter() {
            let (w, h, scale) = logical_size(window);
            if w <= 100.0 || h <= 100.0 {
                return None;
            }
            let Some(monitor) = monitor else {
                // just one monitor: the window is the whole layout
                rects.push(Rect { x: 0.0, y: 0.0, w, h });
                units.push(1.0);
                scales.push(scale);
                continue;
            };
            let on_it = window.window().with_winit_window(|win| win.current_monitor().as_ref() == Some(monitor));
            if on_it != Some(true) {
                return None;
            }
            let pos = monitor.position();
            if is_wayland(window) {
                // winit reports the logical position times the monitor's whole-number scale
                let k = monitor.scale_factor() as f32;
                rects.push(Rect { x: pos.x as f32 / k, y: pos.y as f32 / k, w, h });
                units.push(1.0);
            } else {
                rects.push(Rect { x: pos.x as f32, y: pos.y as f32, w: w * scale, h: h * scale });
                units.push(scale);
            }
            scales.push(scale);
        }
        Some((rects, units, scales))
    }

    /// Checks every 15 ms (up to 3 s) until every window is in place (the
    /// same in two checks in a row, so a window still going fullscreen isn't
    /// measured), then shows each monitor its part of the screenshot, or falls back.
    fn lay_out(self: Rc<Self>, attempt: u32, previous: Option<Vec<Rect>>) {
        slint::Timer::single_shot(Duration::from_millis(15), move || {
            if self.done.get() {
                return;
            }
            let measured = self.measure();
            let settled = matches!((&measured, &previous), (Some((now, _, _)), Some(before)) if now == before);
            let (Some((rects, units, scales)), true) = (measured.clone(), settled) else {
                if attempt < 200 {
                    self.lay_out(attempt + 1, measured.map(|m| m.0));
                } else {
                    eprintln!("freeze picker: the windows didn't cover the monitors; using the other picker");
                    self.give_up();
                }
                return;
            };
            let Some(fit) = fit(&rects, self.shot.width, self.shot.height) else {
                eprintln!(
                    "freeze picker: screenshot {}x{} doesn't match the monitors {rects:?}; using the other picker",
                    self.shot.width, self.shot.height
                );
                self.give_up();
                return;
            };
            for ((window, _), rect) in self.windows.borrow().iter().zip(&rects) {
                window.set_clip_x(((rect.x - fit.x) * fit.sx).round() as i32);
                window.set_clip_y(((rect.y - fit.y) * fit.sy).round() as i32);
                window.set_clip_width((rect.w * fit.sx).round() as i32);
                window.set_clip_height((rect.h * fit.sy).round() as i32);
                window.set_ready(true);
            }
            *self.layout.borrow_mut() = Some(Layout::new(fit, &rects, &units, &scales));
            // show the magnifier at once, where the mouse is (no need to move it)
            let under_mouse = self.windows.borrow().iter().enumerate().find_map(|(i, (w, _))| {
                w.get_hovered().then(|| (i, w.get_hover_x(), w.get_hover_y()))
            });
            let early = self.early.take();
            if let Some(point) = under_mouse.or(early).and_then(|(i, x, y)| self.pixel(i, x, y)) {
                self.point.set(Some(point));
                self.show_point(false);
            }
        });
    }
}

/// Shows the freeze windows for picking a color. UI thread only. `fallback`
/// runs instead if the screenshot cannot be used.
pub(crate) fn show(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    shot: Screenshot,
    fallback: Box<dyn FnOnce()>,
) {
    show_for(ui_weak, Purpose::Color { history_store, context }, shot, fallback);
}

/// Shows the freeze windows for selecting text (Text Extractor). UI thread only.
pub(crate) fn show_text(ui_weak: slint::Weak<AppWindow>, shot: Screenshot, restore: TextRestore) {
    let weak = ui_weak.clone();
    let fallback = Box::new(move || {
        if let Some(ui) = weak.upgrade() {
            crate::ui_state::show_message(&ui, "Text Extractor couldn't line the screenshot up with your screens");
        }
    });
    show_for(ui_weak, Purpose::Text(restore), shot, fallback);
}

fn show_for(ui_weak: slint::Weak<AppWindow>, purpose: Purpose, shot: Screenshot, fallback: Box<dyn FnOnce()>) {
    let text_mode = matches!(purpose, Purpose::Text(_));
    let buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&shot.rgba, shot.width, shot.height);
    let session = Rc::new(Session {
        shot,
        image: slint::Image::from_rgba8(buffer),
        ui_weak: ui_weak.clone(),
        purpose,
        text: RefCell::new(TextCapture::default()),
        windows: RefCell::new(vec![]),
        layout: RefCell::new(None),
        point: Cell::new(None),
        shown_on: Cell::new(None),
        latest: Cell::new(None),
        early: Cell::new(None),
        done: Cell::new(false),
        open_app: Cell::new(false),
        fallback: RefCell::new(Some(fallback)),
    });
    if let Err(err) = session.add_window(None) {
        eprintln!("freeze picker: {err}");
        session.give_up();
        return;
    }
    ACTIVE.with(|a| *a.borrow_mut() = Some(session.clone()));
    // winit knows the monitors once it has created the first window
    let first = session.windows.borrow()[0].0.as_weak();
    let s = session.clone();
    let started = slint::spawn_local(async move {
        if let Some(window) = first.upgrade() {
            let _ = window.window().winit_window().await;
        }
        if !s.done.get() {
            s.cover_all_monitors();
        }
        if !s.done.get() {
            s.lay_out(0, None);
        }
    });
    if let Err(err) = started {
        eprintln!("freeze picker: {err:?}");
        session.clone().lay_out(0, None);
    }

    // If the main window is open (and not minimized), bring it above the
    // freeze windows once they are up, so it isn't hidden behind the screenshot.
    // (Not for text: the window was hidden so the text behind it can be read.)
    slint::Timer::single_shot(Duration::from_millis(150), move || {
        if session.done.get() || text_mode {
            return;
        }
        if let Some(ui) = ui_weak.upgrade() {
            if ui.window().is_visible() && !ui.window().is_minimized() {
                raise_main_window(&ui);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn a_monitor_listed_twice_gets_one_window() {
        // (x, y, width): the laptop screen twice, then a second monitor
        let monitors = vec![(0, 0, 1920), (0, 0, 1920), (1920, 0, 2560)];
        assert_eq!(one_per_place(monitors, |m| *m), vec![(0, 0, 1920), (1920, 0, 2560)]);
    }

    #[test]
    fn one_monitor_maps_window_position_to_image_pixel() {
        // 1920x1080 logical window, 3840x2160 screenshot (200% scaling)
        let rects = [r(0.0, 0.0, 1920.0, 1080.0)];
        let f = fit(&rects, 3840, 2160).unwrap();
        let layout = Layout::new(f, &rects, &[1.0], &[2.0]);
        assert_eq!(layout.pixel(0, 0.0, 0.0, 3840, 2160), (0, 0));
        assert_eq!(layout.pixel(0, 960.0, 540.0, 3840, 2160), (1920, 1080));
        assert_eq!(layout.pixel(0, 5000.0, -3.0, 3840, 2160), (3839, 0));
        assert!(fit(&[r(0.0, 0.0, 1706.7, 960.0)], 2560, 1440).is_some()); // 150% scaling
        assert!(fit(&[r(0.0, 0.0, 1920.0, 1080.0)], 3840, 1080).is_none()); // a monitor we have no window for
    }

    #[test]
    fn two_monitors_with_different_scaling_and_heights() {
        // Wayland: a 2560x1440 monitor at 200% (1280x720 logical) left of a
        // 1920x1080 one at 100%, top-aligned. KWin shoots the workspace at
        // the highest scale: (1280 + 1920) x 1080 logical * 2.
        let rects = [r(0.0, 0.0, 1280.0, 720.0), r(1280.0, 0.0, 1920.0, 1080.0)];
        let f = fit(&rects, 6400, 2160).unwrap();
        let layout = Layout::new(f, &rects, &[1.0, 1.0], &[2.0, 1.0]);
        // the right monitor's top-left pixel
        assert_eq!(layout.pixel(1, 0.0, 0.0, 6400, 2160), (2560, 0));
        assert_eq!(layout.place(2560, 0).map(|p| p.0), Some(1));
        // one screen pixel is 1 screenshot pixel on the 200% monitor, 2 on the 100% one
        assert_eq!(layout.screens[0].step, 1);
        assert_eq!(layout.screens[1].step, 2);
        // right arrow from the left monitor's last column crosses over
        let edge = layout.pixel(0, 1279.9, 100.0, 6400, 2160);
        assert_eq!(edge, (2559, 200));
        assert_eq!(layout.place(edge.0, edge.1).map(|p| p.0), Some(0));
        let over = layout.nudge(edge, 1, 0, 6400, 2160).unwrap();
        assert_eq!(layout.place(over.0, over.1).map(|p| p.0), Some(1));
        // and left goes back
        let back = layout.nudge(over, -1, 0, 6400, 2160).unwrap();
        assert_eq!(layout.place(back.0, back.1).map(|p| p.0), Some(0));
        // below the shorter left monitor is no screen: down stops at its bottom row
        let bottom = layout.pixel(0, 100.0, 719.9, 6400, 2160);
        assert_eq!(layout.nudge(bottom, 0, 1, 6400, 2160), None);
        // the outer edges stop too
        assert_eq!(layout.nudge((0, 0), -1, 0, 6400, 2160), None);
        assert_eq!(layout.nudge((6399, 2159), 1, 0, 6400, 2160), None);
        // the window-local point of a pixel round-trips
        let (i, x, y) = layout.place(4000, 1000).unwrap();
        assert_eq!(layout.pixel(i, x, y, 6400, 2160), (4000, 1000));
    }

    #[test]
    fn monitors_in_any_layout() {
        // one above the other, the top one offset to the right and starting at
        // a negative position (as KDE allows)
        let rects = [r(-200.0, -1080.0, 1920.0, 1080.0), r(0.0, 0.0, 2560.0, 1440.0)];
        let f = fit(&rects, 2760, 2520).unwrap();
        assert_eq!((f.x, f.y), (-200.0, -1080.0));
        let layout = Layout::new(f, &rects, &[1.0, 1.0], &[1.0, 1.0]);
        assert_eq!(layout.pixel(1, 0.0, 0.0, 2760, 2520), (200, 1080));
        // the gap left of the bottom monitor's top part isn't a screen
        assert_eq!(layout.place(10, 2000), None);
        // X11: physical layout, the window is in logical pixels at 150%
        let rects = [r(0.0, 0.0, 1920.0, 1080.0), r(1920.0, 0.0, 3840.0, 2160.0)];
        let f = fit(&rects, 5760, 2160).unwrap();
        let layout = Layout::new(f, &rects, &[1.0, 1.5], &[1.0, 1.5]);
        assert_eq!(layout.pixel(1, 100.0, 100.0, 5760, 2160), (1920 + 150, 150));
        assert_eq!(layout.screens[1].step, 1);
    }
}
