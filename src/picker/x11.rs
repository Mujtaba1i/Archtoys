//! X11 picker: live preview + magnifier that follows the cursor.
//!
//! Each frame we ask the X server for only the 11×11 pixels around the
//! cursor (instead of capturing the whole screen), which also works on every
//! monitor, because the root window spans all of them. The screen is only
//! redrawn when something actually changed.

use super::{
    center_rgb, finish_picker, freeze, magnifier_image, show_hover, PickOutcome, PickerContext,
    MAGNIFIER_RADIUS, PICKER_CANCELLED,
};
use crate::color::format_hex;
use crate::config::HistoryStore;
use crate::{AppWindow, PickerOverlay, PickerShieldWindow};
use device_query::{DeviceQuery, DeviceState, Keycode};
use slint::{Color, ComponentHandle, LogicalPosition};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    ConnectionExt as _, EventMask, GrabMode, GrabStatus, ImageFormat, ImageOrder, KeyButMask, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::{CURRENT_TIME, NONE};

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

/// Keeps other apps from receiving clicks while picking.
struct PointerGrab {
    conn: RustConnection,
}

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
        conn.flush().map_err(|err| format!("flush: {err}"))?;
        Ok(Self { conn })
    }
}

impl PointerGrab {
    /// Button presses delivered to the grab since the last call, as
    /// (left pressed, right pressed). Unlike polling the button state, this
    /// never misses a click, however short (e.g. a touchpad tap).
    fn pressed_buttons(&self) -> (bool, bool) {
        let (mut left, mut right) = (false, false);
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            if let Event::ButtonPress(press) = event {
                match press.detail {
                    1 => left = true,
                    3 => right = true,
                    _ => {}
                }
            }
        }
        (left, right)
    }
}

impl Drop for PointerGrab {
    fn drop(&mut self) {
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
}

impl Sampler {
    fn connect() -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|err| format!("connect: {err}"))?;
        let setup = conn.setup();
        let screen = setup.roots.get(screen_num).ok_or("no root screen")?;
        let bpp = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == screen.root_depth)
            .map(|f| f.bits_per_pixel)
            .unwrap_or(0);
        if bpp != 32 {
            return Err(format!("unsupported screen format ({bpp} bits per pixel)"));
        }
        Ok(Self {
            root: screen.root,
            width: screen.width_in_pixels as i32,
            height: screen.height_in_pixels as i32,
            lsb_first: setup.image_byte_order == ImageOrder::LSB_FIRST,
            conn,
        })
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

/// X11 32-bit ZPixmap → RGBA. LSB-first servers send B,G,R,x; MSB-first x,R,G,B.
fn zpixmap32_to_rgba(data: &[u8], pixels: usize, lsb_first: bool) -> Result<Vec<u8>, String> {
    if data.len() < pixels * 4 {
        return Err(format!("short image: {} bytes for {pixels} pixels", data.len()));
    }
    let mut out = Vec::with_capacity(pixels * 4);
    for px in data[..pixels * 4].chunks_exact(4) {
        if lsb_first {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        } else {
            out.extend_from_slice(&[px[1], px[2], px[3], 255]);
        }
    }
    Ok(out)
}

fn overlay_position(x: i32, y: i32, screen_w: i32, screen_h: i32) -> (i32, i32) {
    let mut pos_x = x + OVERLAY_OFFSET_X;
    let mut pos_y = y + OVERLAY_OFFSET_Y;
    if pos_x + OVERLAY_WIDTH > screen_w {
        pos_x = x - OVERLAY_WIDTH - OVERLAY_OFFSET_X;
    }
    if pos_y + OVERLAY_HEIGHT > screen_h {
        pos_y = y - OVERLAY_HEIGHT - OVERLAY_OFFSET_Y;
    }
    (
        pos_x.clamp(0, (screen_w - OVERLAY_WIDTH).max(0)),
        pos_y.clamp(0, (screen_h - OVERLAY_HEIGHT).max(0)),
    )
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

fn run_live(
    ui_weak: &slint::Weak<AppWindow>,
    overlay_weak: &slint::Weak<PickerOverlay>,
) -> Result<PickOutcome, String> {
    let grab = PointerGrab::acquire()
        .map_err(|err| eprintln!("x11 pointer grab warning: {err}"))
        .ok();
    let sampler = Sampler::connect()?;
    let keyboard = DeviceState::new();

    // Ignore a button that is still held from the click that started picking.
    let (_, _, mut prev_left, mut prev_right) = sampler.pointer()?;
    let mut last: Option<(i32, i32, Vec<u8>)> = None;

    // The newest hover state, and whether the UI already has an update
    // queued. If the UI is busy we just replace the data instead of queuing
    // another update, so updates can't pile up and lag behind the mouse.
    type Hover = (crate::color::Rgb, Vec<u8>, i32, i32);
    let newest: Arc<Mutex<Option<Hover>>> = Arc::new(Mutex::new(None));
    let queued = Arc::new(AtomicBool::new(false));

    loop {
        if PICKER_CANCELLED.load(Ordering::SeqCst) {
            return Ok(PickOutcome::Cancelled);
        }
        if keyboard.get_keys().contains(&Keycode::Escape) {
            return Ok(PickOutcome::Cancelled);
        }

        let (x, y, left, right) = sampler.pointer()?;
        let area = sampler.area(x, y, MAGNIFIER_RADIUS)?;
        let rgb = center_rgb(&area, MAGNIFIER_RADIUS);

        // Clicks: events from the grab (reliable), plus the button state as
        // a backup in case the grab failed.
        let (clicked_left, clicked_right) = grab.as_ref().map(|g| g.pressed_buttons()).unwrap_or((false, false));
        if clicked_left || (left && !prev_left) {
            return Ok(PickOutcome::Picked(rgb));
        }
        if clicked_right || (right && !prev_right) {
            return Ok(PickOutcome::Cancelled);
        }
        prev_left = left;
        prev_right = right;

        // Redraw only when the position or the pixels changed.
        let changed = last
            .as_ref()
            .map(|(lx, ly, la)| *lx != x || *ly != y || *la != area)
            .unwrap_or(true);
        if changed {
            let (pos_x, pos_y) = overlay_position(x, y, sampler.width, sampler.height);
            *newest.lock().unwrap() = Some((rgb, area.clone(), pos_x, pos_y));
            if queued.swap(true, Ordering::SeqCst) {
                last = Some((x, y, area));
                thread::sleep(Duration::from_millis(16));
                continue; // the queued update will pick up the newest data
            }
            let ui_weak = ui_weak.clone();
            let overlay_weak = overlay_weak.clone();
            let newest = newest.clone();
            let queued = queued.clone();
            let _ = slint::invoke_from_event_loop(move || {
                queued.store(false, Ordering::SeqCst);
                let Some((rgb, area_for_ui, pos_x, pos_y)) = newest.lock().unwrap().take() else {
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
                    overlay.set_preview_hex(format_hex(rgb.0, rgb.1, rgb.2).into());
                    let scale = overlay.window().scale_factor();
                    overlay
                        .window()
                        .set_position(LogicalPosition::new(pos_x as f32 / scale, pos_y as f32 / scale));
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
pub(super) fn start_freeze(
    ui_weak: slint::Weak<AppWindow>,
    history_store: HistoryStore,
    context: PickerContext,
    hid_window: bool,
) {
    thread::spawn(move || {
        if hid_window {
            thread::sleep(Duration::from_millis(250));
        }
        let shot = capture_screen_to_png().and_then(|path| freeze::load_png(&path));
        let _ = slint::invoke_from_event_loop(move || match shot {
            Ok(shot) => {
                let (ui2, history2) = (ui_weak.clone(), history_store.clone());
                freeze::show(ui_weak, history_store, context, shot, Box::new(move || start(ui2, history2, context)));
            }
            Err(err) => {
                eprintln!("freeze picker: {err}; using the live picker");
                start(ui_weak, history_store, context);
            }
        });
    });
}

/// Saves the whole screen as a temporary PNG, like the portal does on Wayland.
fn capture_screen_to_png() -> Result<PathBuf, String> {
    let sampler = Sampler::connect()?;
    let rgba = sampler.rect(0, 0, sampler.width, sampler.height)?;
    let path = std::env::temp_dir().join(format!("archtoys-freeze-{}.png", std::process::id()));
    image::save_buffer(&path, &rgba, sampler.width as u32, sampler.height as u32, image::ColorType::Rgba8)
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
        assert_eq!(overlay_position(100, 100, 1920, 1080), (120, 120));
        assert_eq!(overlay_position(1900, 1070, 1920, 1080), (1900 - 126 - 20, 1070 - 158 - 20));
    }
}
