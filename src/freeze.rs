//! The frozen screen: one screenshot, shown fullscreen in our own windows,
//! one per monitor. Inside our own window we may read the mouse position,
//! even on Wayland, so a toy can show a live preview on it. Every toy that
//! works on the screen uses it: the Color Picker (`picker/frozen.rs`) and
//! Text Extractor's capture (`ocr/capture.rs`). This part only lays the
//! windows out over the monitors and tells the toy what happens (the `Toy`
//! trait); what that means is up to the toy.
//!
//! The screenshot shows every monitor (their bounding box, scaled by one
//! factor). Each window shows its own monitor's part: the monitor's position
//! comes from winit, its size from the fullscreen window on it. If that
//! doesn't add up to the screenshot, the toy's fallback is used.
//!
//! The screenshot file is deleted as soon as it has been loaded into memory,
//! and the image is freed when the frozen screen ends.

use crate::{AppWindow, FreezePickerWindow};
use slint::winit_030::winit::monitor::MonitorHandle;
use slint::winit_030::winit::window::{Fullscreen, WindowAttributes};
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, SharedPixelBuffer};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::Duration;

pub(crate) struct Screenshot {
    pub(crate) rgba: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// A monitor's place in the desktop layout, in the layout's own units:
/// logical pixels on Wayland, physical pixels on X11.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
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
pub(crate) struct Screen {
    pub(crate) rect: Rect,
    pub(crate) unit: f32,
    pub(crate) step: i32,
}

pub(crate) struct Layout {
    fit: Fit,
    pub(crate) screens: Vec<Screen>,
}

impl Layout {
    fn new(fit: Fit, rects: &[Rect], units: &[f32], scales: &[f32]) -> Self {
        let screens = rects
            .iter()
            .zip(units)
            .zip(scales)
            .map(|((&rect, &unit), &scale)| Screen { rect, unit, step: ((fit.sx * unit / scale.max(0.1)).round() as i32).max(1) })
            .collect();
        Self { fit, screens }
    }

    /// A point in monitor `i`'s window (logical pixels) → screenshot pixel.
    pub(crate) fn pixel(&self, i: usize, x: f32, y: f32, img_w: u32, img_h: u32) -> (i32, i32) {
        let s = &self.screens[i];
        let gx = s.rect.x + x * s.unit;
        let gy = s.rect.y + y * s.unit;
        let ix = ((gx - self.fit.x) * self.fit.sx).floor() as i32;
        let iy = ((gy - self.fit.y) * self.fit.sy).floor() as i32;
        (ix.clamp(0, img_w as i32 - 1), iy.clamp(0, img_h as i32 - 1))
    }

    /// Screenshot pixel → (monitor, logical point in its window). None in a
    /// gap between monitors of different sizes.
    pub(crate) fn place(&self, ix: i32, iy: i32) -> Option<(usize, f32, f32)> {
        let gx = self.fit.x + (ix as f32 + 0.5) / self.fit.sx;
        let gy = self.fit.y + (iy as f32 + 0.5) / self.fit.sy;
        let i = self.screens.iter().position(|s| s.rect.contains(gx, gy))?;
        let s = &self.screens[i];
        Some((i, (gx - s.rect.x) / s.unit, (gy - s.rect.y) / s.unit))
    }

    /// Screenshot pixel → a point in monitor `i`'s window, even outside it
    /// (a word box that crosses the monitor's edge).
    pub(crate) fn to_window(&self, i: usize, ix: f32, iy: f32) -> (f32, f32) {
        let s = &self.screens[i];
        let gx = self.fit.x + ix / self.fit.sx;
        let gy = self.fit.y + iy / self.fit.sy;
        ((gx - s.rect.x) / s.unit, (gy - s.rect.y) / s.unit)
    }

    /// The pixel one screen pixel away in the direction (dx, dy), or None at
    /// the outer edge of the desktop.
    pub(crate) fn nudge(&self, (ix, iy): (i32, i32), dx: i32, dy: i32, img_w: u32, img_h: u32) -> Option<(i32, i32)> {
        let (i, _, _) = self.place(ix, iy)?;
        let step = self.screens[i].step;
        let (nx, ny) = (ix + dx * step, iy + dy * step);
        if nx < 0 || ny < 0 || nx >= img_w as i32 || ny >= img_h as i32 {
            return None;
        }
        self.place(nx, ny).map(|_| (nx, ny))
    }
}

/// A toy on the frozen screen: what it is told, and how it answers. Each
/// window only sends what its overlay uses (the color picker's mouse moves,
/// clicks, arrow keys and Enter, or Text Extractor's selection and toolbar),
/// so a toy leaves the others as they are. A toy ends the frozen screen
/// itself (`Session::end`); Esc, a right click or closing a window end it
/// through `cancelled`.
pub(crate) trait Toy {
    /// A window was made for this toy: which overlay it shows.
    fn window_created(&self, _window: &FreezePickerWindow) {}
    /// The mouse is over this screenshot pixel (once per event-loop turn, and
    /// once the windows are laid out if it was already over one).
    fn pointer(&self, _s: &Rc<Session>, _point: (i32, i32)) {}
    /// A click at this point of window `i` (logical pixels).
    fn click(&self, _s: &Rc<Session>, _i: usize, _x: f32, _y: f32) {}
    /// An arrow key: one screen pixel in that direction.
    fn nudge(&self, _s: &Rc<Session>, _dx: i32, _dy: i32) {}
    /// Enter.
    fn enter(&self, _s: &Rc<Session>) {}
    /// Text Extractor: a new selection started (or the old one moved).
    fn region_started(&self, _s: &Rc<Session>) {}
    /// Text Extractor: the selection in window `i` is done.
    fn region_done(&self, _s: &Rc<Session>, _i: usize, _x: f32, _y: f32, _w: f32, _h: f32) {}
    /// Text Extractor: a toolbar button ("copy", "line", "open").
    fn tool(&self, _s: &Rc<Session>, _action: &str) {}
    /// Esc, a right click or a closed window ended it (exactly once).
    fn cancelled(&self, s: &Rc<Session>);
    /// The screenshot couldn't be used: the windows are gone, and the
    /// fallback runs next. Undo what the toy changed.
    fn gave_up(&self, _s: &Rc<Session>) {}
    /// The windows have been up for 150 ms (and it hasn't ended).
    fn shown(&self, _s: &Rc<Session>) {}
}

/// One running frozen screen.
pub(crate) struct Session {
    pub(crate) shot: Screenshot,
    image: slint::Image,
    pub(crate) ui_weak: slint::Weak<AppWindow>,
    toy: Rc<dyn Toy>,
    /// The windows, with the monitor each was put on (none: just one window).
    pub(crate) windows: RefCell<Vec<(FreezePickerWindow, Option<MonitorHandle>)>>,
    pub(crate) layout: RefCell<Option<Layout>>,
    /// The newest mouse position (window, x, y), not handled yet.
    latest: Cell<Option<(usize, f32, f32)>>,
    /// Where the mouse was before the windows were laid out: the desktop
    /// reports it as soon as a window appears under the mouse, which is
    /// before the layout. Shown once laid out, so the magnifier is there at once.
    early: Cell<Option<(usize, f32, f32)>>,
    done: Cell<bool>,
    fallback: RefCell<Option<Box<dyn FnOnce()>>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Rc<Session>>> = const { RefCell::new(None) };
}

/// The running frozen screen, if any. UI thread only.
pub(crate) fn active() -> Option<Rc<Session>> {
    ACTIVE.with(|a| a.borrow().clone())
}

/// Cancels the frozen screen, if one is running. UI thread only.
pub(crate) fn cancel_active() {
    if let Some(session) = active() {
        session.cancel();
    }
}

/// Arrow keys pressed in the main window. UI thread only.
pub(crate) fn nudge_active(dx: i32, dy: i32) {
    if let Some(session) = active() {
        session.toy.clone().nudge(&session, dx, dy);
    }
}

/// Enter pressed in the main window. UI thread only.
pub(crate) fn enter_active() {
    if let Some(session) = active() {
        session.toy.clone().enter(&session);
    }
}

/// Hides and drops the windows and the screenshot. UI thread only.
pub(crate) fn release_window() {
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
    let img = loaded.map_err(|err| format!("reading screenshot {}: {err}", path.display()))?.into_rgba8();
    let (width, height) = img.dimensions();
    Ok(Screenshot { rgba: img.into_raw(), width, height })
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
pub(crate) fn one_per_place<T, K: PartialEq>(items: Vec<T>, key: impl Fn(&T) -> K) -> Vec<T> {
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

/// Each monitor's place, its layout units per logical pixel, and its scale.
type Measured = (Vec<Rect>, Vec<f32>, Vec<f32>);

thread_local! {
    /// The monitor the next picker window opens full screen on (`window_attributes`).
    static NEXT_MONITOR: RefCell<Option<MonitorHandle>> = const { RefCell::new(None) };
}

/// Slint's hook for every new window (main.rs): a picker window made for a
/// monitor asks for it when it's created, so KWin opens it there instead of
/// on the monitor in use, on top of another picker window. (KWin doesn't give
/// back the frames of a covered window, and its third redraw then waited
/// forever: the app froze. Asking again later is ignored while full screen.)
pub(crate) fn window_attributes(attrs: WindowAttributes) -> WindowAttributes {
    match NEXT_MONITOR.with(|m| m.borrow_mut().take()) {
        Some(monitor) => attrs.with_fullscreen(Some(Fullscreen::Borderless(Some(monitor)))),
        None => attrs,
    }
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
        .with_winit_window(|window| window.window_handle().is_ok_and(|handle| matches!(handle.as_raw(), RawWindowHandle::Wayland(..))))
        .unwrap_or(false)
}

impl Session {
    /// Ends the frozen screen: true the first time only, so the toy finishes
    /// (and puts the main window back) exactly once.
    pub(crate) fn end(&self) -> bool {
        !self.done.replace(true)
    }

    /// Not ended yet.
    pub(crate) fn running(&self) -> bool {
        !self.done.get()
    }

    /// Esc, a right click or a closed window.
    fn cancel(self: &Rc<Self>) {
        if self.end() {
            self.toy.clone().cancelled(self);
        }
    }

    fn hide_all(&self) {
        for (window, _) in self.windows.borrow().iter() {
            window.hide().ok();
        }
    }

    /// The screenshot can't be used: hide everything and use the fallback.
    fn give_up(self: &Rc<Self>) {
        self.done.set(true);
        release_window();
        self.hide_all();
        self.toy.clone().gave_up(self);
        if let Some(fallback) = self.fallback.borrow_mut().take() {
            fallback();
        }
    }

    /// A point in window `i` (logical pixels) → screenshot pixel, once laid out.
    pub(crate) fn pixel(&self, i: usize, x: f32, y: f32) -> Option<(i32, i32)> {
        let layout = self.layout.borrow();
        let layout = layout.as_ref()?;
        Some(layout.pixel(i, x, y, self.shot.width, self.shot.height))
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
                session.toy.clone().pointer(&session, point);
            } else {
                session.early.set(Some((i, x, y)));
            }
        });
    }

    /// Creates, wires and shows window number `i`.
    fn add_window(self: &Rc<Self>, monitor: Option<MonitorHandle>) -> Result<(), String> {
        NEXT_MONITOR.with(|m| *m.borrow_mut() = monitor.clone());
        let window = FreezePickerWindow::new();
        NEXT_MONITOR.with(|m| m.borrow_mut().take());
        let window = window.map_err(|err| format!("window failed: {err:?}"))?;
        window.global::<crate::Skin>().set_real_shadows(!crate::cpu_drawn());
        window.set_screenshot_image(self.image.clone());
        self.toy.window_created(&window);
        let i = self.windows.borrow().len();
        let weak: Weak<Session> = Rc::downgrade(self);
        {
            let weak = weak.clone();
            window.on_region_started(move || {
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().region_started(&s);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_region_done(move |x, y, w, h| {
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().region_done(&s, i, x, y, w, h);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_tool(move |action| {
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().tool(&s, &action);
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
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().click(&s, i, x, y);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_cancel_pick(move || {
                if let Some(s) = weak.upgrade() {
                    s.cancel();
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_nudge(move |dx, dy| {
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().nudge(&s, dx, dy);
                }
            });
        }
        {
            let weak = weak.clone();
            window.on_pick_point(move || {
                if let Some(s) = weak.upgrade() {
                    s.toy.clone().enter(&s);
                }
            });
        }
        window.window().on_close_requested(move || {
            if let Some(s) = weak.upgrade() {
                s.cancel();
            }
            slint::CloseRequestResponse::HideWindow
        });
        window.show().map_err(|err| format!("show failed: {err:?}"))?;
        self.windows.borrow_mut().push((window, monitor));
        Ok(())
    }

    /// With several monitors: the first window keeps the monitor KWin opened it
    /// on, and one more window opens on each of the others (none on top of
    /// another, see `window_attributes`); then the windows are laid out.
    /// Waits up to ~300 ms to learn that monitor (once the window is shown).
    fn cover_all_monitors(self: &Rc<Self>, tries: u32) {
        let (monitors, current) = {
            let windows = self.windows.borrow();
            let Some((first, _)) = windows.first() else { return };
            first.window().with_winit_window(|w| (w.available_monitors().collect::<Vec<_>>(), w.current_monitor())).unwrap_or_default()
        };
        let monitors = one_per_place(monitors, |m| (m.position(), m.size()));
        if monitors.len() < 2 {
            self.clone().lay_out(0, None);
            return;
        }
        let Some(here) = current.clone().filter(|c| monitors.contains(c)).or_else(|| (tries >= 20).then(|| monitors[0].clone())) else {
            let s = self.clone();
            slint::Timer::single_shot(Duration::from_millis(15), move || {
                if !s.done.get() {
                    s.cover_all_monitors(tries + 1);
                }
            });
            return;
        };
        if let Some((first, slot)) = self.windows.borrow_mut().first_mut() {
            if current.as_ref() != Some(&here) {
                put_on(first, here.clone());
            }
            *slot = Some(here.clone());
        }
        for monitor in monitors.into_iter().filter(|m| *m != here) {
            if let Err(err) = self.add_window(Some(monitor)) {
                eprintln!("freeze picker: {err}");
                self.give_up();
                return;
            }
        }
        self.clone().lay_out(0, None);
    }

    /// Asks again to full-screen each window on its own monitor, for the ones
    /// that are on another.
    fn ask_monitors_again(&self) {
        for (window, monitor) in self.windows.borrow().iter() {
            let Some(monitor) = monitor else { continue };
            let current = window.window().with_winit_window(|win| win.current_monitor()).flatten();
            if current.as_ref() != Some(monitor) {
                // already full screen (on the wrong one): asking again is ignored, so leave it first
                let monitor = monitor.clone();
                window.window().with_winit_window(move |win| {
                    win.set_fullscreen(None);
                    win.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
                });
            }
        }
    }

    /// Each monitor's place, once every window has its fullscreen size on
    /// its own monitor; while they are still getting there, why not.
    fn measure(&self) -> Result<Measured, String> {
        let windows = self.windows.borrow();
        let (mut rects, mut units, mut scales) = (vec![], vec![], vec![]);
        for (i, (window, monitor)) in windows.iter().enumerate() {
            let (w, h, scale) = logical_size(window);
            if w <= 100.0 || h <= 100.0 {
                return Err(format!("window {} is {w:.0}x{h:.0}, not fullscreen yet", i + 1));
            }
            let Some(monitor) = monitor else {
                // just one monitor: the window is the whole layout
                rects.push(Rect { x: 0.0, y: 0.0, w, h });
                units.push(1.0);
                scales.push(scale);
                continue;
            };
            let current = window.window().with_winit_window(|win| win.current_monitor()).flatten();
            if current.as_ref() != Some(monitor) {
                return Err(format!(
                    "window {} should be on {:?} but is on {:?}",
                    i + 1,
                    monitor.name(),
                    current.as_ref().and_then(|m| m.name())
                ));
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
        Ok((rects, units, scales))
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
            let settled = matches!((&measured, &previous), (Ok((now, _, _)), Some(before)) if now == before);
            let (Ok((rects, units, scales)), true) = (measured.clone(), settled) else {
                // KWin 6 can full-screen a window on the monitor it first put it on and
                // ignore which one it was asked for (two monitors stayed black: issue
                // #12): ask again for the windows on the wrong one
                if attempt == 3 || attempt % 10 == 9 {
                    self.ask_monitors_again();
                }
                if attempt < 200 {
                    self.lay_out(attempt + 1, measured.ok().map(|m| m.0));
                } else {
                    let why = match &measured {
                        Err(why) => why.clone(),
                        Ok((rects, _, _)) => format!("they kept moving ({rects:?})"),
                    };
                    eprintln!("freeze picker: the windows didn't cover the monitors in 3 s: {why}; using the other picker");
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
            eprintln!(
                "freeze picker: ready after {} ms on {} monitor(s) {rects:?}, screenshot {}x{}",
                (attempt + 1) * 15,
                rects.len(),
                self.shot.width,
                self.shot.height
            );
            // show the magnifier at once, where the mouse is (no need to move it)
            let under_mouse = self
                .windows
                .borrow()
                .iter()
                .enumerate()
                .find_map(|(i, (w, _))| w.get_hovered().then(|| (i, w.get_hover_x(), w.get_hover_y())));
            let early = self.early.take();
            if let Some(point) = under_mouse.or(early).and_then(|(i, x, y)| self.pixel(i, x, y)) {
                self.toy.clone().pointer(&self, point);
            }
        });
    }
}

/// Shows the frozen screen for `toy`. UI thread only. `fallback` runs instead
/// if the screenshot cannot be used.
pub(crate) fn show(ui_weak: slint::Weak<AppWindow>, toy: Rc<dyn Toy>, shot: Screenshot, fallback: Box<dyn FnOnce()>) {
    let buffer = SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&shot.rgba, shot.width, shot.height);
    let session = Rc::new(Session {
        shot,
        image: slint::Image::from_rgba8(buffer),
        ui_weak,
        toy,
        windows: RefCell::new(vec![]),
        layout: RefCell::new(None),
        latest: Cell::new(None),
        early: Cell::new(None),
        done: Cell::new(false),
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
            s.cover_all_monitors(0);
        }
    });
    if let Err(err) = started {
        eprintln!("freeze picker: {err:?}");
        session.clone().lay_out(0, None);
    }

    slint::Timer::single_shot(Duration::from_millis(150), move || {
        if !session.done.get() {
            session.toy.clone().shown(&session);
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
