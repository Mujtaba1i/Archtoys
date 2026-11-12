//! The Screen Ruler on the frozen screen (`freeze.rs`): the lines from the
//! mouse to the nearest edges (Spacing, Horizontal, Vertical), the boxes
//! (Bounds, up to 3), the tolerance, and copying the sizes.

use super::logic::{self, Mode, Size, Unit};
use crate::freeze::{self, Aside, Layout, Session, Toy};
use crate::{FreezePickerWindow, RulerBar, RulerLabel};
use slint::winit_030::WinitWindowAccessor;
use slint::{Color, ComponentHandle, ModelRc, VecModel};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// What the ruler opens with (the settings, and the lines' color).
pub(crate) struct Start {
    pub mode: Mode,
    pub tolerance: i32,
    /// − and + buttons for the tolerance, else the scroll wheel
    pub buttons: bool,
    pub unit: Unit,
    pub line: crate::color::Rgb,
    /// the app's theme (Skin's dark-t: 1 dark, 0 light): the frozen screen's windows don't have it
    pub dark: f32,
    /// a measure may go on past its screen's edge (Settings, with several screens)
    pub cross: bool,
}

/// A box measured in Bounds: on window `i`, its corners in screenshot
/// pixels, and its size in screen pixels.
#[derive(Clone, Copy, Debug)]
struct Shape {
    i: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    size: Size,
}

/// What the lines measure now: on window `i`, the edges found (left, right,
/// top, bottom, inclusive) around the point, all in screenshot pixels.
#[derive(Clone, Copy, Debug)]
struct Reach {
    i: usize,
    point: (i32, i32),
    edges: (i32, i32, i32, i32),
    size: Size,
}

struct State {
    mode: Cell<Mode>,
    tolerance: Cell<i32>,
    buttons: bool,
    unit: Unit,
    line: Color,
    dark: f32,
    cross: bool,
    aside: Aside,
    /// the measuring point (Spacing, Horizontal, Vertical), screenshot pixels
    point: Cell<Option<(i32, i32)>>,
    reach: Cell<Option<Reach>>,
    kept: RefCell<Vec<Shape>>,
    current: Cell<Option<Shape>>,
    /// mm per screen pixel of each window's monitor, once asked (None inside: it doesn't say)
    mm: RefCell<Vec<Option<Option<f32>>>>,
    toast: Cell<u64>,
}

thread_local! {
    /// The running ruler, for the windows' own buttons (UI thread only).
    static CURRENT: RefCell<Option<Rc<State>>> = const { RefCell::new(None) };
}

struct Measure {
    state: Rc<State>,
}

/// Shows the frozen screen for measuring. UI thread only.
pub(crate) fn show(ui_weak: slint::Weak<crate::AppWindow>, shot: freeze::Screenshot, start: Start, aside: Aside) {
    let (r, g, b) = start.line;
    let state = Rc::new(State {
        mode: Cell::new(start.mode),
        tolerance: Cell::new(start.tolerance),
        buttons: start.buttons,
        unit: start.unit,
        line: Color::from_rgb_u8(r, g, b),
        dark: start.dark,
        cross: start.cross,
        aside,
        point: Cell::new(None),
        reach: Cell::new(None),
        kept: RefCell::new(vec![]),
        current: Cell::new(None),
        mm: RefCell::new(vec![]),
        toast: Cell::new(0),
    });
    CURRENT.with(|c| *c.borrow_mut() = Some(state.clone()));
    let weak = ui_weak.clone();
    let fallback = Box::new(move || {
        if let Some(ui) = weak.upgrade() {
            crate::ui_state::show_message(&ui, "The Screen Ruler couldn't line the screenshot up with your screens");
        }
    });
    freeze::show(ui_weak, Rc::new(Measure { state }), shot, fallback);
}

/// Runs `f` with the running frozen screen and ruler, if any.
fn with(f: impl FnOnce(&Rc<Session>, &Rc<State>)) {
    let state = CURRENT.with(|c| c.borrow().clone());
    if let (Some(session), Some(state)) = (freeze::active(), state) {
        if session.running() {
            f(&session, &state);
        }
    }
}

impl State {
    /// mm per screen pixel on window `i`'s monitor, if it reports its size.
    fn mm(&self, s: &Session, i: usize) -> Option<f32> {
        if let Some(known) = self.mm.borrow().get(i).copied().flatten() {
            return known;
        }
        let windows = s.windows.borrow();
        let found = windows.get(i).and_then(|(w, monitor)| {
            let monitor = monitor.clone().or_else(|| w.window().with_winit_window(|win| win.current_monitor()).flatten())?;
            super::screen_size::mm_per_px(monitor.name().as_deref(), monitor.size().width)
        });
        let mut mm = self.mm.borrow_mut();
        if mm.len() <= i {
            mm.resize(i + 1, None);
        }
        mm[i] = Some(found);
        found
    }

    /// The biggest a measure on monitor `i` may be: that screen, or all of them added up.
    fn most(&self, layout: &Layout, i: usize) -> (u32, u32) {
        let screens: Vec<(u32, u32)> = (0..layout.screens.len()).map(|j| layout.pixels(j)).collect();
        logic::most(&screens, i, self.cross)
    }

    /// Measures from the point: how far its color reaches each way, on the
    /// screen the mouse is on (or every screen, but not the gaps between them).
    fn measure(&self, s: &Session) {
        let Some(point) = self.point.get() else { return };
        let reach = {
            let layout = s.layout.borrow();
            let Some(layout) = layout.as_ref() else { return };
            let Some((i, _, _)) = layout.place(point.0, point.1) else { return };
            let (w, h, t) = (s.shot.width, s.shot.height, self.tolerance.get());
            let edges = if self.cross {
                logic::reach(&s.shot.rgba, w, h, point, t, |x, y| layout.place(x, y).is_some())
            } else {
                let b = layout.bounds(i, w, h);
                logic::reach(&s.shot.rgba, w, h, point, t, |x, y| x >= b.0 && x <= b.2 && y >= b.1 && y <= b.3)
            };
            let step = layout.screens[i].step;
            let (l, r, t, b) = edges;
            // shortcut: across screens with different scaling, the size uses this screen's scale
            let size = Size { w: logic::screen_px(r - l + 1, step), h: logic::screen_px(b - t + 1, step) };
            Reach { i, point, edges, size: logic::capped(size, self.most(layout, i)) }
        };
        self.reach.set(Some(reach));
        self.redraw(s);
    }

    /// Draws what is measured on every window: the lines or the boxes, and their sizes.
    fn redraw(&self, s: &Session) {
        let mode = self.mode.get();
        let layout = s.layout.borrow();
        let Some(layout) = layout.as_ref() else { return };
        let windows = s.windows.borrow();
        let kept = self.kept.borrow();
        let current = self.current.get();
        let count = kept.len() + usize::from(current.is_some());
        for (j, (window, _)) in windows.iter().enumerate() {
            let mut bars: Vec<RulerBar> = vec![];
            let mut labels: Vec<RulerLabel> = vec![];
            if mode == Mode::Bounds {
                // the kept ones dashed (the first of three see-through), the one being drawn by the window
                for (n, shape) in kept.iter().enumerate().filter(|(_, sh)| sh.i == j || self.cross) {
                    let (x0, y0) = layout.to_window(j, shape.x0 as f32, shape.y0 as f32);
                    let (x1, y1) = layout.to_window(j, shape.x1 as f32, shape.y1 as f32);
                    let faint = logic::style(n, kept.len() + 1) == logic::Style::Faint;
                    for d in logic::dashes(x0, y0, x1 - x0, y1 - y0, 2.0, 6.0, 4.0) {
                        bars.push(RulerBar { x: d.x, y: d.y, w: d.w, h: d.h, faint });
                    }
                    if shape.i == j {
                        let text = logic::text(mode, shape.size, self.unit, self.mm(s, j)) + " · kept";
                        labels.push(RulerLabel { x: x0, y: (y0 - 30.0).max(4.0), text: text.into(), centered: false });
                    }
                }
                // across screens: the part of the box being drawn that is on this screen (its own window draws the rest)
                if let Some(shape) = current.filter(|c| c.i != j && self.cross) {
                    let (x0, y0) = layout.to_window(j, shape.x0 as f32, shape.y0 as f32);
                    let (x1, y1) = layout.to_window(j, shape.x1 as f32, shape.y1 as f32);
                    for (x, y, w, h) in
                        [(x0, y0, x1 - x0, 2.0), (x0, y1 - 2.0, x1 - x0, 2.0), (x0, y0, 2.0, y1 - y0), (x1 - 2.0, y0, 2.0, y1 - y0)]
                    {
                        bars.push(RulerBar { x, y, w, h, faint: false });
                    }
                }
                // its size by the bottom-right corner, on the screen that corner is on (across screens, maybe not its own)
                let corner_on = |c: &Shape| layout.place(c.x1 - 1, c.y1 - 1).map_or(c.i, |p| p.0);
                if let Some(shape) = current.filter(|c| corner_on(c) == j) {
                    let (x1, y1) = layout.to_window(j, shape.x1 as f32, shape.y1 as f32);
                    let text = logic::text(mode, shape.size, self.unit, self.mm(s, shape.i));
                    labels.push(RulerLabel { x: x1 + 8.0, y: y1 + 6.0, text: text.into(), centered: false });
                }
            } else if let Some(reach) = self.reach.get().filter(|r| r.i == j || self.cross) {
                let (l, r, t, b) = reach.edges;
                let (px, py) = (reach.point.0 as f32 + 0.5, reach.point.1 as f32 + 0.5);
                if mode != Mode::Vertical {
                    let (x1, y) = layout.to_window(j, l as f32, py);
                    let (x2, _) = layout.to_window(j, (r + 1) as f32, py);
                    bars.push(RulerBar { x: x1, y: y - 1.0, w: x2 - x1, h: 2.0, faint: false });
                    bars.push(RulerBar { x: x1 - 1.0, y: y - 8.0, w: 2.0, h: 16.0, faint: false });
                    bars.push(RulerBar { x: x2 - 1.0, y: y - 8.0, w: 2.0, h: 16.0, faint: false });
                }
                if mode != Mode::Horizontal {
                    let (x, y1) = layout.to_window(j, px, t as f32);
                    let (_, y2) = layout.to_window(j, px, (b + 1) as f32);
                    bars.push(RulerBar { x: x - 1.0, y: y1, w: 2.0, h: y2 - y1, faint: false });
                    bars.push(RulerBar { x: x - 8.0, y: y1 - 1.0, w: 16.0, h: 2.0, faint: false });
                    bars.push(RulerBar { x: x - 8.0, y: y2 - 1.0, w: 16.0, h: 2.0, faint: false });
                }
                if reach.i == j {
                    let (cx, cy) = layout.to_window(j, px, py);
                    let text = logic::text(mode, reach.size, self.unit, self.mm(s, j));
                    labels.push(RulerLabel { x: cx + 18.0, y: cy + 12.0, text: text.into(), centered: false });
                }
            }
            window.set_ruler_bars(ModelRc::new(VecModel::from(bars)));
            window.set_ruler_labels(ModelRc::new(VecModel::from(labels)));
            window.set_ruler_count(count as i32);
        }
    }

    /// A short message at the bottom, for a moment.
    fn toast(&self, s: &Session, text: &str) {
        let n = self.toast.get() + 1;
        self.toast.set(n);
        for (window, _) in s.windows.borrow().iter() {
            window.set_ruler_toast(text.into());
        }
        slint::Timer::single_shot(Duration::from_millis(1600), move || {
            with(|s, state| {
                if state.toast.get() == n {
                    for (window, _) in s.windows.borrow().iter() {
                        window.set_ruler_toast("".into());
                    }
                }
            });
        });
    }

    fn pick_mode(&self, s: &Session, i: i32) {
        let mode = Mode::from_index(i);
        if mode == self.mode.get() {
            return;
        }
        self.mode.set(mode);
        // a new mode starts clean
        self.kept.borrow_mut().clear();
        self.current.set(None);
        self.reach.set(None);
        for (window, _) in s.windows.borrow().iter() {
            window.set_ruler_tool(mode.index());
            window.set_has_selection(false);
        }
        if let Some(ui) = s.ui_weak.upgrade() {
            ui.set_setting_ruler_mode(mode.index());
        }
        if mode != Mode::Bounds {
            self.measure(s);
        }
        self.redraw(s);
    }

    fn step_tolerance(&self, s: &Session, d: i32) {
        let t = (self.tolerance.get() + d.signum() * logic::TOLERANCE_STEP).clamp(0, logic::TOLERANCE_MAX);
        self.tolerance.set(t);
        for (window, _) in s.windows.borrow().iter() {
            window.set_ruler_tolerance(t);
        }
        if let Some(ui) = s.ui_weak.upgrade() {
            ui.set_setting_ruler_tolerance(t);
        }
        self.toast(s, &format!("Tolerance {t}"));
        self.measure(s);
    }

    /// The box being dragged or moved, in window `i` (logical pixels).
    /// A drag past the window's edge stops at its screen's edge (unless across screens is on).
    fn drag(&self, s: &Session, i: usize, x: f32, y: f32, w: f32, h: f32) {
        let (Some((mut x0, mut y0)), Some((mut x1, mut y1))) = (s.pixel(i, x, y), s.pixel(i, x + w, y + h)) else { return };
        let shape = {
            let layout = s.layout.borrow();
            let Some(layout) = layout.as_ref() else { return };
            if !self.cross {
                let b = layout.bounds(i, s.shot.width, s.shot.height);
                (x0, x1) = (x0.clamp(b.0, b.2 + 1), x1.clamp(b.0, b.2 + 1));
                (y0, y1) = (y0.clamp(b.1, b.3 + 1), y1.clamp(b.1, b.3 + 1));
            }
            let step = layout.screens[i].step;
            let size = Size { w: logic::screen_px(x1 - x0, step), h: logic::screen_px(y1 - y0, step) };
            Shape { i, x0, y0, x1, y1, size: logic::capped(size, self.most(layout, i)) }
        };
        self.current.set(Some(shape));
        self.redraw(s);
    }

    /// Shift-click: keeps the box on the screen, to draw another. Three at most.
    fn keep(&self, s: &Session) {
        let Some(current) = self.current.get() else { return };
        if !logic::room_to_keep(self.kept.borrow().len()) {
            self.toast(s, "3 is the most");
            return;
        }
        self.kept.borrow_mut().push(current);
        self.current.set(None);
        for (window, _) in s.windows.borrow().iter() {
            window.set_has_selection(false);
        }
        self.redraw(s);
    }

    /// Copies what is measured, puts it in the history and closes.
    fn copy(&self, s: &Session) {
        let mode = self.mode.get();
        let (sizes, i): (Vec<Size>, usize) = if mode == Mode::Bounds {
            let mut shapes = self.kept.borrow().clone();
            shapes.extend(self.current.get());
            (shapes.iter().map(|sh| sh.size).collect(), shapes.last().map(|sh| sh.i).unwrap_or(0))
        } else {
            match self.reach.get() {
                Some(r) => (vec![r.size], r.i),
                None => (vec![], 0),
            }
        };
        if sizes.is_empty() {
            self.toast(s, if mode == Mode::Bounds { "Drag a box first" } else { "Move over what to measure first" });
            return;
        }
        let mm = self.mm(s, i);
        let text = if sizes.len() > 1 { logic::text_all(mode, &sizes, self.unit, mm) } else { logic::text(mode, sizes[0], self.unit, mm) };
        crate::ui_state::copy_text_async(text);
        if let Some(ui) = s.ui_weak.upgrade() {
            super::record(&ui, mode, &sizes, mm);
        }
        self.finish(s);
    }

    fn finish(&self, s: &Session) {
        if s.end() {
            self.end(s);
        }
    }

    fn end(&self, s: &Session) {
        CURRENT.with(|c| c.borrow_mut().take());
        if let Some(ui) = s.ui_weak.upgrade() {
            super::save_settings(&ui);
        }
        freeze::come_back(s.ui_weak.clone(), self.aside);
    }
}

impl Toy for Measure {
    fn window_created(&self, window: &FreezePickerWindow, i: usize) {
        let st = &self.state;
        window.global::<crate::Skin>().set_dark_t(st.dark);
        window.set_text_mode(false);
        window.set_ruler_mode(true);
        window.set_ruler_tool(st.mode.get().index());
        window.set_ruler_line(st.line);
        window.set_ruler_tolerance(st.tolerance.get());
        window.set_ruler_buttons(st.buttons);
        window.on_ruler_tool_picked(|m| with(|s, st| st.pick_mode(s, m)));
        window.on_ruler_tolerance_step(|d| with(|s, st| st.step_tolerance(s, d)));
        window.on_ruler_copy(|| with(|s, st| st.copy(s)));
        window.on_ruler_keep(|| with(|s, st| st.keep(s)));
        window.on_ruler_box(move |x, y, w, h, _done| with(|s, st| st.drag(s, i, x, y, w, h)));
    }

    fn pointer(&self, s: &Rc<Session>, point: (i32, i32)) {
        if self.state.mode.get() != Mode::Bounds {
            self.state.point.set(Some(point));
            self.state.measure(s);
        }
    }

    /// A click copies (Spacing, Horizontal, Vertical; Bounds draws with drags).
    fn click(&self, s: &Rc<Session>, _i: usize, _x: f32, _y: f32) {
        if self.state.mode.get() != Mode::Bounds {
            self.state.copy(s);
        }
    }

    /// The measuring point moves one screen pixel (ten with Shift).
    fn nudge(&self, s: &Rc<Session>, dx: i32, dy: i32) {
        let st = &self.state;
        if st.mode.get() == Mode::Bounds {
            return;
        }
        let (w, h) = (s.shot.width, s.shot.height);
        let moved = {
            let layout = s.layout.borrow();
            let Some(layout) = layout.as_ref() else { return };
            let start = st.point.get().unwrap_or_else(|| {
                let r = layout.screens[0].rect;
                layout.pixel(0, (r.w / layout.screens[0].unit) / 2.0, (r.h / layout.screens[0].unit) / 2.0, w, h)
            });
            layout.nudge(start, dx, dy, w, h).unwrap_or(start)
        };
        st.point.set(Some(moved));
        st.measure(s);
    }

    fn enter(&self, s: &Rc<Session>) {
        self.state.copy(s);
    }

    fn cancelled(&self, s: &Rc<Session>) {
        self.state.end(s);
    }

    fn gave_up(&self, s: &Rc<Session>) {
        self.state.end(s);
    }
}
