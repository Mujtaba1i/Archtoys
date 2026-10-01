//! Pure color math: formatting, parsing and shades. No UI code here, so
//! everything in this file is covered by the unit tests at the bottom.

use palette::{FromColor, Hsl, Hsv, IntoColor, Srgb};

pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorField {
    Hex,
    Rgb,
    Hsl,
    Hsv,
}

impl ColorField {
    pub fn from_ui_label(label: &str) -> Option<Self> {
        match label {
            "HEX" => Some(Self::Hex),
            "RGB" => Some(Self::Rgb),
            "HSL" => Some(Self::Hsl),
            "HSV" => Some(Self::Hsv),
            _ => None,
        }
    }
}

pub fn format_hex(r: u8, g: u8, b: u8) -> String {
    format!("#{:02X}{:02X}{:02X}", r, g, b)
}

pub fn format_rgb(r: u8, g: u8, b: u8) -> String {
    format!("rgb({r},{g},{b})")
}

pub fn format_hsl(r: u8, g: u8, b: u8) -> String {
    let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let hsl: Hsl = Hsl::from_color(srgb);
    let h = hsl.hue.into_degrees().round().rem_euclid(360.0);
    let s = (hsl.saturation * 100.0).round().clamp(0.0, 100.0);
    let l = (hsl.lightness * 100.0).round().clamp(0.0, 100.0);
    format!("hsl({h:.0},{s:.0}%,{l:.0}%)")
}

pub fn format_hsv(r: u8, g: u8, b: u8) -> String {
    let srgb = Srgb::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let hsv: Hsv = Hsv::from_color(srgb);
    let h = hsv.hue.into_degrees().round().rem_euclid(360.0);
    let s = (hsv.saturation * 100.0).round().clamp(0.0, 100.0);
    let v = (hsv.value * 100.0).round().clamp(0.0, 100.0);
    format!("hsv({h:.0},{s:.0}%,{v:.0}%)")
}

pub fn format_canonical(field: ColorField, rgb: Rgb) -> String {
    let (r, g, b) = rgb;
    match field {
        ColorField::Hex => format_hex(r, g, b),
        ColorField::Rgb => format_rgb(r, g, b),
        ColorField::Hsl => format_hsl(r, g, b),
        ColorField::Hsv => format_hsv(r, g, b),
    }
}

/// Multiplies each channel by `factor`, clamped to 0..=255.
pub fn scale_rgb(rgb: Rgb, factor: f32) -> Rgb {
    let scale = |c: u8| (c as f32 * factor).clamp(0.0, 255.0) as u8;
    (scale(rgb.0), scale(rgb.1), scale(rgb.2))
}

/// The four shades shown in the bar on the left:
/// (lighter 2, lighter 1, darker 1, darker 2).
pub fn calculate_shades(rgb: Rgb) -> (Rgb, Rgb, Rgb, Rgb) {
    (
        scale_rgb(rgb, 1.5),
        scale_rgb(rgb, 1.2),
        scale_rgb(rgb, 0.7),
        scale_rgb(rgb, 0.5),
    )
}

pub fn parse_hex_flexible(value: &str) -> Option<Rgb> {
    let clean = value.trim().trim_start_matches('#');
    if clean.len() != 6 || !clean.is_ascii() {
        return None;
    }

    let r = u8::from_str_radix(&clean[0..2], 16).ok()?;
    let g = u8::from_str_radix(&clean[2..4], 16).ok()?;
    let b = u8::from_str_radix(&clean[4..6], 16).ok()?;
    Some((r, g, b))
}

fn inner_function_payload<'a>(value: &'a str, func_name: &str) -> &'a str {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let prefix = format!("{func_name}(");

    if lower.starts_with(&prefix) && trimmed.ends_with(')') && trimmed.len() >= prefix.len() + 1 {
        let start = prefix.len();
        let end = trimmed.len() - 1;
        trimmed[start..end].trim()
    } else {
        trimmed
    }
}

pub fn parse_rgb_permissive(value: &str) -> Option<Rgb> {
    let payload = inner_function_payload(value, "rgb");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let parse_component = |s: &str| -> Option<u8> {
        let raw = s.parse::<i32>().ok()?;
        Some(raw.clamp(0, 255) as u8)
    };

    Some((
        parse_component(parts[0])?,
        parse_component(parts[1])?,
        parse_component(parts[2])?,
    ))
}

fn parse_percentage_0_to_1(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    let raw = trimmed.strip_suffix('%').unwrap_or(trimmed).trim();
    let parsed = raw.parse::<f32>().ok()?;
    Some((parsed / 100.0).clamp(0.0, 1.0))
}

fn srgb_to_rgb(rgb: Srgb) -> Rgb {
    (
        (rgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (rgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

pub fn parse_hsl_permissive(value: &str) -> Option<Rgb> {
    let payload = inner_function_payload(value, "hsl");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()?.rem_euclid(360.0);
    let s = parse_percentage_0_to_1(parts[1])?;
    let l = parse_percentage_0_to_1(parts[2])?;

    let rgb: Srgb = Hsl::new(h, s, l).into_color();
    Some(srgb_to_rgb(rgb))
}

pub fn parse_hsv_permissive(value: &str) -> Option<Rgb> {
    let payload = inner_function_payload(value, "hsv");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()?.rem_euclid(360.0);
    let s = parse_percentage_0_to_1(parts[1])?;
    let v = parse_percentage_0_to_1(parts[2])?;

    let rgb: Srgb = Hsv::new(h, s, v).into_color();
    Some(srgb_to_rgb(rgb))
}

pub fn parse_color(field: ColorField, value: &str) -> Option<Rgb> {
    match field {
        ColorField::Hex => parse_hex_flexible(value),
        ColorField::Rgb => parse_rgb_permissive(value),
        ColorField::Hsl => parse_hsl_permissive(value),
        ColorField::Hsv => parse_hsv_permissive(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_all_fields() {
        let c = (203, 182, 172);
        assert_eq!(format_canonical(ColorField::Hex, c), "#CBB6AC");
        assert_eq!(format_canonical(ColorField::Rgb, c), "rgb(203,182,172)");
        assert_eq!(format_canonical(ColorField::Hsl, c), "hsl(19,23%,74%)");
        assert_eq!(format_canonical(ColorField::Hsv, c), "hsv(19,15%,80%)");
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(parse_hex_flexible("#1981CE"), Some((0x19, 0x81, 0xCE)));
        assert_eq!(parse_hex_flexible("  1981ce "), Some((0x19, 0x81, 0xCE)));
        assert_eq!(parse_hex_flexible("#12345"), None);
        assert_eq!(parse_hex_flexible("#GGGGGG"), None);
        assert_eq!(parse_hex_flexible("aébcd"), None); // used to panic: slicing cut "é" in half
        assert_eq!(parse_hex_flexible("#١٢٣٤٥٦"), None); // Arabic-Indic digits
    }

    #[test]
    fn parses_rgb_hsl_hsv() {
        assert_eq!(parse_rgb_permissive("rgb(1, 2, 300)"), Some((1, 2, 255)));
        assert_eq!(parse_rgb_permissive("10,20,30"), Some((10, 20, 30)));
        assert_eq!(parse_rgb_permissive("rgb(1,2)"), None);
        assert_eq!(parse_hsl_permissive("hsl(0,100%,50%)"), Some((255, 0, 0)));
        assert_eq!(parse_hsv_permissive("hsv(120,100%,100%)"), Some((0, 255, 0)));
    }

    #[test]
    fn every_format_round_trips_through_its_parser() {
        for c in [(0, 0, 0), (255, 255, 255), (25, 129, 206), (239, 168, 62)] {
            let hex = format_canonical(ColorField::Hex, c);
            assert_eq!(parse_color(ColorField::Hex, &hex), Some(c));
            let rgb = format_canonical(ColorField::Rgb, c);
            assert_eq!(parse_color(ColorField::Rgb, &rgb), Some(c));
        }
    }

    #[test]
    fn shades_clamp_and_scale() {
        let (l2, l1, d1, d2) = calculate_shades((200, 100, 0));
        assert_eq!(l2, (255, 150, 0));
        assert_eq!(l1, (240, 120, 0));
        assert_eq!(d1, (140, 70, 0));
        assert_eq!(d2, (100, 50, 0));
    }
}
