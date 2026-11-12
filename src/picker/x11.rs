//! X11 picker: live preview + magnifier that follows the cursor.
//!
//! Each frame we ask the X server for only the 11×11 pixels around the
//! cursor (instead of capturing the whole screen), which also works on every
//! monitor, because the root window spans all of them. The screen is only
//! redrawn when something actually changed. The preview stays on the
//! cursor's monitor (RandR). The arrow keys move the pointer one pixel and
//! Enter picks; only those keys are taken from other apps while picking.

use super::{
    center_rgb, finish_picker, frozen, magnifier_image, preview_text, show_hover, PickOutcome, PickerContext, MAGNIFIER_RADIUS,
    PICKER_CANCELLED,
};
use crate::config::HistoryStore;
use crate::{AppWindow, PickerOverlay, PickerShieldWindow};
use slint::{Color, ComponentHandle, LogicalPosition};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection as _;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    ConnectionExt as _, EventMask, Grab, GrabMode, GrabStatus, ImageFormat, ImageOrder, KeyButMask, ModMask, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

/// The overlay's smallest size (it gets wider for long values).
const OVERLAY_WIDTH: i32 = 126;
const OVERLAY_HEIGHT: i32 = 158;
const OVERLAY_OFFSET_X: i32 = 20;
const OVERLAY_OFFSET_Y: i32 = 20;

thread_local! {
    static PICKER_OVERLAY: RefCell<Option<PickerOverlay>> = const { RefCell::new(None) };
    static PICKER_SHIELD: RefCell<Option<PickerShieldWindow>> = const { RefCell::new(None) };
}

/// Hides and drops the overlay and shield windows. UI thread only.
pub(super) fn release_windows() {
    PICKER_OVERLAY.with(|slot| {
        if let Some(overlay) = slot.borrow_mut().take() {
            overlay.hide().ok();
        }
    });
    PICKER_SHIELD.with(|slot| {
        if let Some(shield) = slot.borrow_mut().take() {
            shield.hide().ok();
        }
    });
}

fn ensure_overlay() -> Result<slint::Weak<PickerOverlay>, slint::PlatformError> {
    PICKER_OVERLAY.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let overlay = PickerOverlay::new()?;
            overlay.global::<crate::Skin>().set_real_shadows(!crate::cpu_drawn());
            overlay.window().on_close_requested(|| {
                PICKER_CANCELLED.store(true, Ordering::SeqCst);
                slint::CloseRequestResponse::HideWindow
            });
            *slot = Some(overlay);
        }
        Ok(slot.as_ref().expect("overlay exists").as_weak())
    })
}

fn show_shield() {
    PICKER_SHIELD.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            match PickerShieldWindow::new() {
                Ok(shield) => {
                    shield.window().on_close_requested(|| {
                        PICKER_CANCELLED.store(true, Ordering::SeqCst);
                        slint::CloseRequestResponse::HideWindow
                    });
                    *slot = Some(shield);
                }
                Err(err) => eprintln!("shield: failed to create picker shield window: {err:?}"),
            }
        }
        if let Some(shield) = slot.as_ref() {
            shield.show().ok();
        }
    });
}

/// Keeps other apps from receiving clicks (and the arrow keys and Enter)
/// while picking.
struct PointerGrab {
    conn: RustConnection,
    root: Window,
    /// Key codes: left, up, right, down, Enter, keypad Enter.
    keys: [Option<u8>; 6],
}

/// What happened since the last frame.
#[derive(Default)]
struct Input {
    left: bool,
    right: bool,
    enter: bool,
    dx: i32,
    dy: i32,
}

const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;
const XK_RETURN: u32 = 0xff0d;
const XK_KP_ENTER: u32 = 0xff8d;

impl PointerGrab {
    fn acquire() -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|err| format!("connect: {err}"))?;
        let root = conn.setup().roots.get(screen_num).ok_or("no root screen")?.root;
        let reply = conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                NONE,
                NONE,
                CURRENT_TIME,
            )
            .map_err(|err| format!("grab_pointer: {err}"))?
            .reply()
            .map_err(|err| format!("grab_pointer reply: {err}"))?;
        if reply.status != GrabStatus::SUCCESS {
            return Err(format!("grab_pointer not successful: {:?}", reply.status));
        }
        let keys = [XK_LEFT, XK_UP, XK_RIGHT, XK_DOWN, XK_RETURN, XK_KP_ENTER].map(|sym| keycode_of(&conn, sym));
        // Also with Caps Lock / Num Lock on. A key another app already grabbed
        // just stays with that app (the error is ignored).
        let locks = [ModMask::from(0u16), ModMask::LOCK, ModMask::M2, ModMask::LOCK | ModMask::M2];
        for key in keys.iter().flatten() {
            for mods in locks {
                // with Shift too: Shift and an arrow moves ten pixels (v0.7.4)
                for shift in [ModMask::from(0u16), ModMask::SHIFT] {
                    let _ = conn.grab_key(false, root, mods | shift, *key, GrabMode::ASYNC, GrabMode::ASYNC);
                }
            }
        }
        conn.flush().map_err(|err| format!("flush: {err}"))?;
        Ok(Self { conn, root, keys })
    }
}

impl PointerGrab {
    /// Button and key presses delivered to the grab since the last call.
    /// Unlike polling the button state, this never misses a click, however
    /// short (e.g. a touchpad tap). A held arrow key repeats.
    fn input(&self) -> Input {
        let mut input = Input::default();
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::ButtonPress(press) => match press.detail {
                    1 => input.left = true,
                    3 => input.right = true,
                    _ => {}
                },
                Event::KeyPress(key) => {
                    let step = if u16::from(key.state) & u16::from(KeyButMask::SHIFT) != 0 { 10 } else { 1 };
                    match self.keys.iter().position(|k| *k == Some(key.detail)) {
                        Some(0) => input.dx -= step,
                        Some(1) => input.dy -= step,
                        Some(2) => input.dx += step,
                        Some(3) => input.dy += step,
                        Some(_) => input.enter = true,
                        None => {}
                    }
                }
                _ => {}
            }
        }
        input
    }

    /// Moves the pointer by (dx, dy) pixels and waits until the X server has
    /// done it, so the next pointer query sees the new position.
    fn nudge(&self, dx: i32, dy: i32) {
        let _ = self.conn.warp_pointer(NONE, NONE, 0, 0, 0, 0, dx as i16, dy as i16);
        if let Ok(cookie) = self.conn.get_input_focus() {
            let _ = cookie.reply();
        }
    }
}

impl Drop for PointerGrab {
    fn drop(&mut self) {
        let _ = self.conn.ungrab_key(Grab::ANY, self.root, ModMask::ANY);
        let _ = self.conn.ungrab_pointer(CURRENT_TIME);
        let _ = self.conn.flush();
    }
}

/// Reads pixels from the X server.
struct Sampler {
    conn: RustConnection,
    root: Window,
    width: i32,
    height: i32,
    lsb_first: bool,
    /// The key code of Esc on this keyboard layout, if it has one.
    escape: Option<u8>,
    /// The monitors (x, y, width, height), in root window pixels.
    monitors: Vec<Monitor>,
}

type Monitor = (i32, i32, i32, i32);

impl Sampler {
    fn connect() -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|err| format!("connect: {err}"))?;
        let setup = conn.setup();
        let screen = setup.roots.get(screen_num).ok_or("no root screen")?;
        let bpp = setup.pixmap_formats.iter().find(|f| f.depth == screen.root_depth).map(|f| f.bits_per_pixel).unwrap_or(0);
        if bpp != 32 {
            return Err(format!("unsupported screen format ({bpp} bits per pixel)"));
        }
        let (root, width, height) = (screen.root, screen.width_in_pixels as i32, screen.height_in_pixels as i32);
        let lsb_first = setup.image_byte_order == ImageOrder::LSB_FIRST;
        let escape = keycode_of(&conn, XK_ESCAPE);
        let monitors = monitors(&conn, root);
        Ok(Self { conn, root, width, height, lsb_first, escape, monitors })
    }

    /// Is Esc held down right now? Asked every frame instead of grabbing the
    /// keyboard, so the rest of the desktop keeps working while picking.
    fn escape_down(&self) -> bool {
        let Some(code) = self.escape else { return false };
        self.conn
            .query_keymap()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|keymap| keymap.keys[usize::from(code / 8)] & (1 << (code % 8)) != 0)
    }

    /// Cursor position and whether the left / right buttons are down.
    fn pointer(&self) -> Result<(i32, i32, bool, bool), String> {
        let reply = self
            .conn
            .query_pointer(self.root)
            .map_err(|err| format!("query_pointer: {err}"))?
            .reply()
            .map_err(|err| format!("query_pointer reply: {err}"))?;
        let mask = u16::from(reply.mask);
        Ok((
            reply.root_x as i32,
            reply.root_y as i32,
            mask & u16::from(KeyButMask::BUTTON1) != 0,
            mask & u16::from(KeyButMask::BUTTON3) != 0,
        ))
    }

    /// RGBA of a rectangle fully inside the screen.
    fn rect(&self, x: i32, y: i32, w: i32, h: i32) -> Result<Vec<u8>, String> {
        let reply = self
            .conn
            .get_image(ImageFormat::Z_PIXMAP, self.root, x as i16, y as i16, w as u16, h as u16, !0)
            .map_err(|err| format!("get_image: {err}"))?
            .reply()
            .map_err(|err| format!("get_image reply: {err}"))?;
        zpixmap32_to_rgba(&reply.data, (w * h) as usize, self.lsb_first)
    }

    /// RGBA of the (2r+1)² area around (cx, cy); black outside the screen.
    fn area(&self, cx: i32, cy: i32, radius: i32) -> Result<Vec<u8>, String> {
        let size = 2 * radius + 1;
        let x0 = (cx - radius).max(0);
        let y0 = (cy - radius).max(0);
        let x1 = (cx + radius).min(self.width - 1);
        let y1 = (cy + radius).min(self.height - 1);
        let mut out = vec![0u8; (size * size * 4) as usize];
        for i in 0..(size * size) as usize {
            out[i * 4 + 3] = 255;
        }
        if x0 > x1 || y0 > y1 {
            return Ok(out);
        }
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let rect = self.rect(x0, y0, w, h)?;
        for y in 0..h {
            for x in 0..w {
                let src = ((y * w + x) * 4) as usize;
                let ox = x0 + x - (cx - radius);
                let oy = y0 + y - (cy - radius);
                let dst = ((oy * size + ox) * 4) as usize;
                out[dst..dst + 4].copy_from_slice(&rect[src..src + 4]);
            }
        }
        Ok(out)
    }
}

const XK_ESCAPE: u32 = 0xff1b;

/// The active monitors from RandR; empty if the server can't tell.
fn monitors(conn: &RustConnection, root: Window) -> Vec<Monitor> {
    let Some(reply) = conn.randr_get_monitors(root, true).ok().and_then(|c| c.reply().ok()) else {
        return vec![];
    };
    reply
        .monitors
        .iter()
        .filter(|m| m.width > 0 && m.height > 0)
        .map(|m| (i32::from(m.x), i32::from(m.y), i32::from(m.width), i32::from(m.height)))
        .collect()
}

/// The monitor the point is on (the smallest, if they overlap); the nearest
/// one if it's in a gap; the whole screen if there's no monitor list.
fn monitor_at(monitors: &[Monitor], x: i32, y: i32, screen: Monitor) -> Monitor {
    let distance = |&(mx, my, mw, mh): &Monitor| {
        let dx = (mx - x).max(x - (mx + mw - 1)).max(0);
        let dy = (my - y).max(y - (my + mh - 1)).max(0);
        (dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)), mw.saturating_mul(mh))
    };
    monitors.iter().copied().min_by_key(distance).unwrap_or(screen)
}

/// The key code that types `keysym` (for example Esc) on the current layout.
fn keycode_of(conn: &RustConnection, keysym: u32) -> Option<u8> {
    let setup = conn.setup();
    let (first, last) = (setup.min_keycode, setup.max_keycode);
    let mapping = conn.get_keyboard_mapping(first, last - first + 1).ok()?.reply().ok()?;
    let per_key = usize::from(mapping.keysyms_per_keycode.max(1));
    let index = mapping.keysyms.chunks(per_key).position(|syms| syms.contains(&keysym))?;
    first.checked_add(u8::try_from(index).ok()?)
}

/// X11 32-bit ZPixmap → RGBA. LSB-first servers send B,G,R,x; MSB-first x,R,G,B.
fn zpixmap32_to_rgba(data: &[u8], pixels: usize, lsb_first: bool) -> Result<Vec<u8>, String> {
    if data.len() < pixels * 4 {
        return Err(format!("short image: {} bytes for {pixels} pixels", data.len()));
    }
    let mut out = Vec::with_capacity(pixels * 4);
    for px in data[..pixels * 4].as_chunks::<4>().0 {
        if lsb_first {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        } else {
            out.extend_from_slice(&[px[1], px[2], px[3], 255]);
        }
    }
    Ok(out)
}

/// Where the overlay (w × h) goes for the cursor at (x, y): beside it, on the
/// other side near the edges of the cursor's monitor, never across them.
fn overlay_position(x: i32, y: i32, w: i32, h: i32, (mx, my, mw, mh): Monitor) -> (i32, i32) {
    let mut pos_x = x + OVERLAY_OFFSET_X;
    let mut pos_y = y + OVERLAY_OFFSET_Y;
    if pos_x + w > mx + mw {
        pos_x = x - w - OVERLAY_OFFSET_X;
    }
    if pos_y + h > my + mh {
        pos_y = y - h - OVERLAY_OFFSET_Y;
    }
    (pos_x.clamp(mx, (mx + mw - w).max(mx)), pos_y.clamp(my, (my + mh - h).max(my)))
}

/// Live X11 picking. Call on the UI thread.
pub(super) fn start(ui_weak: slint::Weak<AppWindow>, history_store: HistoryStore, context: PickerContext) {
    let overlay_weak = match ensure_overlay() {
        Ok(weak) => weak,
        Err(err) => {
            eprintln!("overlay: failed to create picker window: {err:?}");
            finish_picker(ui_weak, history_store, context, PickOutcome::Cancelled);
            return;
        }
    };
    show_shield();

    thread::spawn(move || {
        let outcome = run_live(&ui_weak, &overlay_weak).unwrap_or_else(|err| {
            eprintln!("x11 picker: {err}");
            PickOutcome::Cancelled
        });
        finish_picker(ui_weak, history_store, context, outcome);
    });
}

fn run_live(ui_weak: &slint::Weak<AppWindow>, overlay_weak: &slint::Weak<PickerOverlay>) -> Result<PickOutcome, String> {
    let grab = PointerGrab::acquire().map_err(|err| eprintln!("x11 pointer grab warning: {err}")).ok();
    let sampler = Sampler::connect()?;

    let screen = (0, 0, sampler.width, sampler.height);
    let monitors = Arc::new(sampler.monitors.clone());

    // Ignore a button that is still held from the click that started picking.
    let (_, _, mut prev_left, mut prev_right) = sampler.pointer()?;
    let mut last: Option<(i32, i32, Vec<u8>)> = None;

    // The newest hover state, and whether the UI already has an update
    // queued. If the UI is busy we just replace the data instead of queuing
    // another update, so updates can't pile up and lag behind the mouse.
    type Hover = (crate::color::Rgb, Vec<u8>, i32, i32); // color, pixels, cursor x, y
    let newest: Arc<Mutex<Option<Hover>>> = Arc::new(Mutex::new(None));
    let queued = Arc::new(AtomicBool::new(false));

    loop {
        if PICKER_CANCELLED.load(Ordering::SeqCst) {
            return Ok(PickOutcome::Cancelled);
        }
        if sampler.escape_down() {
            return Ok(PickOutcome::Cancelled);
        }

        // Clicks and keys: events from the grab (reliable), plus the button
        // state as a backup in case the grab failed.
        let input = grab.as_ref().map(|g| g.input()).unwrap_or_default();
        if let (Some(g), true) = (&grab, input.dx != 0 || input.dy != 0) {
            g.nudge(input.dx, input.dy);
        }

        let (x, y, left, right) = sampler.pointer()?;
        let area = sampler.area(x, y, MAGNIFIER_RADIUS)?;
        let rgb = center_rgb(&area, MAGNIFIER_RADIUS);

        if input.left || input.enter || (left && !prev_left) {
            return Ok(PickOutcome::Picked(rgb));
        }
        if input.right || (right && !prev_right) {
            return Ok(PickOutcome::Cancelled);
        }
        prev_left = left;
        prev_right = right;

        // Redraw only when the position or the pixels changed.
        let changed = last.as_ref().map(|(lx, ly, la)| *lx != x || *ly != y || *la != area).unwrap_or(true);
        if changed {
            *newest.lock().unwrap() = Some((rgb, area.clone(), x, y));
            if queued.swap(true, Ordering::SeqCst) {
                last = Some((x, y, area));
                thread::sleep(Duration::from_millis(16));
                continue; // the queued update will pick up the newest data
            }
            let ui_weak = ui_weak.clone();
            let overlay_weak = overlay_weak.clone();
            let newest = newest.clone();
            let queued = queued.clone();
            let monitors = monitors.clone();
            let _ = slint::invoke_from_event_loop(move || {
                queued.store(false, Ordering::SeqCst);
                let Some((rgb, area_for_ui, x, y)) = newest.lock().unwrap().take() else {
                    return;
                };
                if PICKER_CANCELLED.load(Ordering::SeqCst) {
                    return;
                }
                if let Some(ui) = ui_weak.upgrade() {
                    show_hover(&ui, rgb);
                }
                if let Some(overlay) = overlay_weak.upgrade() {
                    overlay.set_magnifier(magnifier_image(&area_for_ui, MAGNIFIER_RADIUS));
                    overlay.set_preview_color(Color::from_rgb_u8(rgb.0, rgb.1, rgb.2));
                    overlay.set_preview_hex(preview_text(rgb));
                    let scale = overlay.window().scale_factor();
                    // the overlay is wider for long values (logical size, so it's right before the first draw too)
                    let w = (overlay.get_card_width() * scale).ceil() as i32;
                    let h = (OVERLAY_HEIGHT as f32 * scale).ceil() as i32;
                    let monitor = monitor_at(&monitors, x, y, screen);
                    let (pos_x, pos_y) = overlay_position(x, y, w.max(OVERLAY_WIDTH), h, monitor);
                    overlay.window().set_position(LogicalPosition::new(pos_x as f32 / scale, pos_y as f32 / scale));
                    overlay.show().ok();
                }
            });
            last = Some((x, y, area));
        }

        thread::sleep(Duration::from_millis(16));
    }
}

/// Freeze-frame picking on X11 (only with ARCHTOYS_PICKER=freeze; mainly
/// for testing the Wayland picker on an X11 machine).
pub(super) fn start_freeze(ui_weak: slint::Weak<AppWindow>, history_store: HistoryStore, context: PickerContext, hid_window: bool) {
    thread::spawn(move || {
        if hid_window {
            thread::sleep(Duration::from_millis(250));
        }
        let shot = capture_screen_to_png().and_then(|path| crate::freeze::load_png(&path));
        let _ = slint::invoke_from_event_loop(move || match shot {
            Ok(shot) => {
                let (ui2, history2) = (ui_weak.clone(), history_store.clone());
                frozen::show(ui_weak, history_store, context, shot, Box::new(move || start(ui2, history2, context)));
            }
            Err(err) => {
                eprintln!("freeze picker: {err}, using the live picker");
                crate::diagnostics::set_picker("live (X11)");
                start(ui_weak, history_store, context);
            }
        });
    });
}

/// Saves the whole screen as a temporary PNG, like the portal does on Wayland.
pub(super) fn capture_screen_to_png() -> Result<PathBuf, String> {
    let sampler = Sampler::connect()?;
    let rgba = sampler.rect(0, 0, sampler.width, sampler.height)?;
    let path = std::env::temp_dir().join(format!("archtoys-freeze-{}.png", std::process::id()));
    image::save_buffer(&path, &rgba, sampler.width as u32, sampler.height as u32, image::ExtendedColorType::Rgba8)
        .map_err(|err| format!("saving screenshot: {err}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_both_byte_orders() {
        let lsb = [10, 20, 30, 0]; // B G R x
        assert_eq!(zpixmap32_to_rgba(&lsb, 1, true).unwrap(), vec![30, 20, 10, 255]);
        let msb = [0, 30, 20, 10]; // x R G B
        assert_eq!(zpixmap32_to_rgba(&msb, 1, false).unwrap(), vec![30, 20, 10, 255]);
        assert!(zpixmap32_to_rgba(&lsb, 2, true).is_err());
    }

    #[test]
    fn overlay_flips_near_edges() {
        let screen = (0, 0, 1920, 1080);
        assert_eq!(overlay_position(100, 100, 126, 158, screen), (120, 120));
        assert_eq!(overlay_position(1900, 1070, 126, 158, screen), (1900 - 126 - 20, 1070 - 158 - 20));
    }

    #[test]
    fn overlay_stays_on_the_cursors_monitor() {
        // a 1920x1080 monitor left of a 2560x1440 one, bottom-aligned (gap above the left one)
        let monitors = [(0, 360, 1920, 1080), (1920, 0, 2560, 1440)];
        let screen = (0, 0, 4480, 1440);
        // right edge of the left monitor: flips left instead of spilling onto the right one
        let m = monitor_at(&monitors, 1910, 800, screen);
        assert_eq!(m, monitors[0]);
        assert_eq!(overlay_position(1910, 800, 126, 158, m), (1910 - 126 - 20, 820));
        // just across the edge it sits on the right monitor
        let m = monitor_at(&monitors, 1920, 800, screen);
        assert_eq!(m, monitors[1]);
        assert_eq!(overlay_position(1920, 800, 126, 158, m), (1940, 820));
        // top of the left monitor: below the cursor, never up in the gap
        let m = monitor_at(&monitors, 500, 365, screen);
        assert_eq!(overlay_position(500, 365, 126, 158, m), (520, 385));
        // in the gap (pointer can't really go there): the nearest monitor
        assert_eq!(monitor_at(&monitors, 500, 100, screen), monitors[0]);
        // a wide overlay (long value) at the left edge of the right monitor
        assert_eq!(overlay_position(1925, 1430, 200, 158, monitors[1]), (1945, 1430 - 158 - 20));
        // no monitor list: the whole screen
        assert_eq!(monitor_at(&[], 5, 5, screen), screen);
        // a monitor spanning both (Xvfb keeps one for its whole screen): the smaller one wins
        let with_whole = [screen, monitors[0], monitors[1]];
        assert_eq!(monitor_at(&with_whole, 1910, 800, screen), monitors[0]);
        assert_eq!(monitor_at(&with_whole, 1920, 800, screen), monitors[1]);
    }
}
