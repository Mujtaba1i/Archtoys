//! System tray icon (StatusNotifierItem) with Open / Quit.

use crate::AppWindow;
use image::{GenericImageView, ImageFormat};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Tray};
use slint::ComponentHandle;
use std::sync::OnceLock;

fn tray_icon_pixmap() -> Vec<Icon> {
    static ICON: OnceLock<Vec<Icon>> = OnceLock::new();
    ICON.get_or_init(|| {
        let bytes = include_bytes!("../packaging/archtoys-64.png");
        let img = match image::load_from_memory_with_format(bytes, ImageFormat::Png) {
            Ok(img) => img,
            Err(err) => {
                eprintln!("tray: failed to decode embedded icon: {err:?}");
                return vec![];
            }
        };
        let (width, height) = img.dimensions();
        let mut data = img.into_rgba8().into_vec();
        for pixel in data.chunks_exact_mut(4) {
            pixel.rotate_right(1); // rgba -> argb
        }
        vec![Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    })
    .clone()
}

pub struct AppTray {
    pub ui: slint::Weak<AppWindow>,
}

impl Tray for AppTray {
    fn id(&self) -> String {
        "archtoys-color-picker".into()
    }

    fn title(&self) -> String {
        "Archtoys Color Picker".into()
    }

    fn icon_name(&self) -> String {
        "archtoys".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        tray_icon_pixmap()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.ui.upgrade_in_event_loop(|ui| {
            ui.window().show().ok();
        });
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open".into(),
                activate: Box::new(|this: &mut AppTray| {
                    let _ = this.ui.upgrade_in_event_loop(|ui: AppWindow| {
                        ui.window().show().ok();
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
