//! System tray icon (StatusNotifierItem) with Open / Quit. Its dot takes the
//! selected color.

use crate::color::Rgb;
use crate::AppWindow;
use image::ImageFormat;
use ksni::blocking::Handle;
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};
use std::cell::RefCell;
use std::sync::OnceLock;

/// The tray icon without its dot (blue brackets, the A with the ring cut
/// around where the dot goes), 64 x 64. Made from packaging/archtoys-tray.svg.
const BASE_PNG: &[u8] = include_bytes!("../packaging/archtoys-tray-base-64.png");
/// Where the dot goes on that image, in pixels: centre and radius.
const DOT: (f32, f32, f32) = (40.54, 40.83, 2.8);
/// A thin dark outline, so a light dot (yellow) stays visible on a light panel.
const OUTLINE: f32 = 1.0;
const OUTLINE_RGB: Rgb = (0x14, 0x14, 0x18);

thread_local! {
    /// The running tray, to recolor its dot. UI thread only.
    static HANDLE: RefCell<Option<Handle<AppTray>>> = const { RefCell::new(None) };
}

/// Keeps the tray's handle so `set_dot` can reach it.
pub fn install(handle: Handle<AppTray>) {
    HANDLE.with(|h| *h.borrow_mut() = Some(handle));
}

/// Gives the tray dot the selected color.
pub fn set_dot(rgb: Rgb) {
    HANDLE.with(|h| {
        if let Some(handle) = h.borrow().as_ref() {
            handle.update(|tray| tray.dot = rgb);
        }
    });
}

fn base() -> Option<&'static (u32, u32, Vec<u8>)> {
    static BASE: OnceLock<Option<(u32, u32, Vec<u8>)>> = OnceLock::new();
    BASE.get_or_init(|| match image::load_from_memory_with_format(BASE_PNG, ImageFormat::Png) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            Some((rgba.width(), rgba.height(), rgba.into_raw()))
        }
        Err(err) => {
            eprintln!("tray: failed to decode the icon: {err:?}");
            None
        }
    })
    .as_ref()
}

/// The base image with the dot drawn on it, as ARGB (what the tray wants).
fn icon_with_dot(dot: Rgb) -> Vec<Icon> {
    let Some((width, height, rgba)) = base() else { return vec![] };
    let mut pixels = rgba.clone();
    let (cx, cy, r) = DOT;
    for y in 0..*height {
        for x in 0..*width {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            // coverage of the whole disc (dot + outline), and of the dot alone; 1 px soft edge
            let disc = (r + OUTLINE + 0.5 - d).clamp(0.0, 1.0);
            if disc <= 0.0 {
                continue;
            }
            let inner = (r + 0.5 - d).clamp(0.0, 1.0);
            let mix = |a: u8, b: u8| (a as f32 * inner + b as f32 * (1.0 - inner)).round() as u8;
            let color = [mix(dot.0, OUTLINE_RGB.0), mix(dot.1, OUTLINE_RGB.1), mix(dot.2, OUTLINE_RGB.2)];
            let i = ((y * width + x) * 4) as usize;
            let under = &mut pixels[i..i + 4];
            let alpha = under[3] as f32 / 255.0;
            let out_alpha = disc + alpha * (1.0 - disc);
            for c in 0..3 {
                let value = (color[c] as f32 * disc + under[c] as f32 * alpha * (1.0 - disc)) / out_alpha.max(1e-6);
                under[c] = value.round() as u8;
            }
            under[3] = (out_alpha * 255.0).round() as u8;
        }
    }
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.rotate_right(1); // rgba -> argb
    }
    vec![Icon { width: *width as i32, height: *height as i32, data: pixels }]
}

pub struct AppTray {
    pub ui: slint::Weak<AppWindow>,
    /// The selected color, shown as the dot.
    pub dot: Rgb,
}

impl Tray for AppTray {
    fn id(&self) -> String {
        "archtoys-color-picker".into()
    }

    fn title(&self) -> String {
        "Archtoys Color Picker".into()
    }

    // No themed icon name: the panel would draw the installed icon and never the
    // colored dot.
    fn icon_name(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        icon_with_dot(self.dot)
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.ui.upgrade_in_event_loop(|ui| {
            crate::ui_state::show_window(&ui);
        });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|this: &mut AppTray| {
                    let _ = this.ui.upgrade_in_event_loop(|ui: AppWindow| {
                        crate::ui_state::show_window(&ui);
                    });
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|_this: &mut AppTray| {
                    let _ = slint::invoke_from_event_loop(|| {
                        slint::quit_event_loop().ok();
                    });
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dot_takes_the_color_with_a_dark_outline() {
        let icons = icon_with_dot((255, 196, 0));
        let icon = &icons[0];
        let at = |x: usize, y: usize| {
            let i = (y * icon.width as usize + x) * 4;
            (icon.data[i], icon.data[i + 1], icon.data[i + 2], icon.data[i + 3]) // a, r, g, b
        };
        assert_eq!(at(40, 40), (255, 255, 196, 0)); // the centre is the color, opaque
        let (a, r, g, b) = at(40, 37); // 3.3 px up: in the outline
        assert!(a > 200 && r < 120 && g < 120 && b < 60, "{:?}", (a, r, g, b));
    }
}
