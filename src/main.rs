slint::include_modules!();

use arboard::{Clipboard, SetExtLinux};
use palette::{Srgb, Hsl, Hsv, FromColor, IntoColor};
use rand::Rng;
use std::rc::Rc;
use slint::{Color, ModelRc, VecModel, Model};
use std::thread;

fn calculate_shades(r: u8, g: u8, b: u8) -> (Color, Color, Color, Color) {
    let lighter_2 = Color::from_rgb_u8(
        ((r as f32 * 1.5).min(255.0)) as u8,
        ((g as f32 * 1.5).min(255.0)) as u8,
        ((b as f32 * 1.5).min(255.0)) as u8,
    );
    let lighter_1 = Color::from_rgb_u8(
        ((r as f32 * 1.2).min(255.0)) as u8,
        ((g as f32 * 1.2).min(255.0)) as u8,
        ((b as f32 * 1.2).min(255.0)) as u8,
    );
    let darker_1 = Color::from_rgb_u8(
        (r as f32 * 0.7) as u8,
        (g as f32 * 0.7) as u8,
        (b as f32 * 0.7) as u8,
    );
    let darker_2 = Color::from_rgb_u8(
        (r as f32 * 0.5) as u8,
        (g as f32 * 0.5) as u8,
        (b as f32 * 0.5) as u8,
    );
    (lighter_2, lighter_1, darker_1, darker_2)
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let ui_handle = ui.as_weak();

    let history_data: Rc<VecModel<Color>> = Rc::new(VecModel::from(vec![
        Color::from_rgb_u8(203, 182, 172),
        Color::from_rgb_u8(85, 85, 85),
    ]));
    ui.set_history_model(ModelRc::from(history_data.clone()));

    // Initialize shades for the default color
    let (r, g, b) = (203, 182, 172); // Default color cbb6ac
    let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
    ui.set_shade_lighter_2(lighter_2);
    ui.set_shade_lighter_1(lighter_1);
    ui.set_shade_darker_1(darker_1);
    ui.set_shade_darker_2(darker_2);

    let update_ui_colors = |ui: &AppWindow, r: u8, g: u8, b: u8| {
        ui.set_current_color(Color::from_rgb_u8(r, g, b));
        
        let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
        ui.set_shade_lighter_2(lighter_2);
        ui.set_shade_lighter_1(lighter_1);
        ui.set_shade_darker_1(darker_1);
        ui.set_shade_darker_2(darker_2);
        
        let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        let hsl: Hsl = Hsl::from_color(srgb);
        let hsv: Hsv = Hsv::from_color(srgb);
        
        ui.set_val_hex(format!("{:02x}{:02x}{:02x}", r, g, b).into());
        ui.set_val_rgb(format!("rgb({}, {}, {})", r, g, b).into());
        ui.set_val_hsl(format!("hsl({:.0}, {:.0}%, {:.0}%)", hsl.hue.into_degrees(), hsl.saturation * 100.0, hsl.lightness * 100.0).into());
        ui.set_val_hsv(format!("hsv({:.0}, {:.0}%, {:.0}%)", hsv.hue.into_degrees(), hsv.saturation * 100.0, hsv.value * 100.0).into());
    };

    let ui_weak = ui_handle.clone();
    let history_clone = history_data.clone();
    ui.on_pick_color(move || {
        let ui = ui_weak.unwrap();
        if ui.get_setting_minimize() {
            ui.window().hide().unwrap();
        }
        let mut rng = rand::thread_rng();
        let r: u8 = rng.gen();
        let g: u8 = rng.gen();
        let b: u8 = rng.gen();
        update_ui_colors(&ui, r, g, b);
        history_clone.insert(0, Color::from_rgb_u8(r, g, b));
        if ui.get_setting_autocopy() {
            let hex_to_copy = format!("{:02x}{:02x}{:02x}", r, g, b);
            thread::spawn(move || {
                if let Ok(mut clipboard) = Clipboard::new() {
                    let _ = clipboard.set().wait().text(hex_to_copy);
                }
            });
        }
        if ui.get_setting_minimize() {
             ui.window().show().unwrap();
        }
    });

    ui.on_copy_to_clipboard(move |text| {
        let text_to_copy = text.to_string();
        thread::spawn(move || {
            match Clipboard::new() {
                Ok(mut clipboard) => {
                    let _ = clipboard.set().wait().text(text_to_copy);
                }
                Err(e) => eprintln!("Clipboard error: {}", e),
            }
        });
    });

    let ui_weak = ui_handle.clone();
    let history_read = history_data.clone();
    ui.on_history_clicked(move |index| {
        let ui = ui_weak.unwrap();
        if let Some(col) = history_read.row_data(index as usize) {
             update_ui_colors(&ui, col.red(), col.green(), col.blue());
        }
    });

    let ui_weak = ui_handle.clone();
    let history_shades = history_data.clone();
    ui.on_shade_clicked(move |factor| {
        let ui = ui_weak.unwrap();
        let current = ui.get_current_color();
        let r = (current.red() as f32 * factor).clamp(0.0, 255.0) as u8;
        let g = (current.green() as f32 * factor).clamp(0.0, 255.0) as u8;
        let b = (current.blue() as f32 * factor).clamp(0.0, 255.0) as u8;
        history_shades.insert(0, Color::from_rgb_u8(r, g, b));
        update_ui_colors(&ui, r, g, b);
    });

    // Parse color from different formats
    fn parse_color(type_str: &str, value: &str) -> Option<(u8, u8, u8)> {
        match type_str {
            "HEX" => {
                let clean = value.trim_start_matches('#').trim();
                if clean.len() == 6 {
                    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
                    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
                    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "RGB" => {
                let clean = value.trim().trim_start_matches("rgb(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let r = parts[0].trim().parse::<u8>().ok()?;
                    let g = parts[1].trim().parse::<u8>().ok()?;
                    let b = parts[2].trim().parse::<u8>().ok()?;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "HSL" => {
                let clean = value.trim().trim_start_matches("hsl(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let h = parts[0].trim().parse::<f32>().ok()?;
                    let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    let l = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    
                    let hsl = Hsl::new(h, s, l);
                    let rgb: Srgb = hsl.into_color();
                    let r = (rgb.red * 255.0).round() as u8;
                    let g = (rgb.green * 255.0).round() as u8;
                    let b = (rgb.blue * 255.0).round() as u8;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            "HSV" => {
                let clean = value.trim().trim_start_matches("hsv(").trim_end_matches(')');
                let parts: Vec<&str> = clean.split(',').collect();
                if parts.len() == 3 {
                    let h = parts[0].trim().parse::<f32>().ok()?;
                    let s = parts[1].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    let v = parts[2].trim().trim_end_matches('%').parse::<f32>().ok()? / 100.0;
                    
                    let hsv = Hsv::new(h, s, v);
                    let rgb: Srgb = hsv.into_color();
                    let r = (rgb.red * 255.0).round() as u8;
                    let g = (rgb.green * 255.0).round() as u8;
                    let b = (rgb.blue * 255.0).round() as u8;
                    Some((r, g, b))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    let ui_weak = ui_handle.clone();
    ui.on_value_edited(move |type_str, value| {
        let ui = ui_weak.unwrap();
        if let Some((r, g, b)) = parse_color(&type_str.to_string(), &value.to_string()) {
            ui.set_current_color(Color::from_rgb_u8(r, g, b));
            
            let (lighter_2, lighter_1, darker_1, darker_2) = calculate_shades(r, g, b);
            ui.set_shade_lighter_2(lighter_2);
            ui.set_shade_lighter_1(lighter_1);
            ui.set_shade_darker_1(darker_1);
            ui.set_shade_darker_2(darker_2);
            
            let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            let hsl: Hsl = Hsl::from_color(srgb);
            let hsv: Hsv = Hsv::from_color(srgb);
            
            // Update other fields but not the one being edited
            if type_str != "HEX" {
                ui.set_val_hex(format!("{:02x}{:02x}{:02x}", r, g, b).into());
            }
            if type_str != "RGB" {
                ui.set_val_rgb(format!("rgb({}, {}, {})", r, g, b).into());
            }
            if type_str != "HSL" {
                ui.set_val_hsl(format!("hsl({:.0}, {:.0}%, {:.0}%)", hsl.hue.into_degrees(), hsl.saturation * 100.0, hsl.lightness * 100.0).into());
            }
            if type_str != "HSV" {
                ui.set_val_hsv(format!("hsv({:.0}, {:.0}%, {:.0}%)", hsv.hue.into_degrees(), hsv.saturation * 100.0, hsv.value * 100.0).into());
            }
        }
    });

    let ui_weak = ui_handle.clone();
    let history_accept = history_data.clone();
    ui.on_value_accepted(move |type_str, value| {
        let ui = ui_weak.unwrap();
        if let Some((r, g, b)) = parse_color(&type_str.to_string(), &value.to_string()) {
            history_accept.insert(0, Color::from_rgb_u8(r, g, b));
            update_ui_colors(&ui, r, g, b);
        }
    });

    ui.run()
}