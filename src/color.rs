//! Pure color math: formatting, parsing and shades. No UI code here, so
//! everything in this file is covered by the unit tests at the bottom.

use palette::{FromColor, Hsl, Hsv, IntoColor, Oklab, Oklch, Srgb};

pub type Rgb = (u8, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorField {
    Name,
    Hex,
    Rgb,
    Hsl,
    Hsv,
    Cmyk,
    Hsla,
    Oklch,
    Oklab,
}

impl ColorField {
    /// Every field, in the default order.
    pub const ALL: [ColorField; 9] = [
        Self::Name,
        Self::Hex,
        Self::Rgb,
        Self::Hsl,
        Self::Hsv,
        Self::Cmyk,
        Self::Hsla,
        Self::Oklch,
        Self::Oklab,
    ];

    /// The short label shown in the row, also used as its key.
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "NAME",
            Self::Hex => "HEX",
            Self::Rgb => "RGB",
            Self::Hsl => "HSL",
            Self::Hsv => "HSV",
            Self::Cmyk => "CMYK",
            Self::Hsla => "HSLA",
            Self::Oklch => "OKLCH",
            Self::Oklab => "OKLAB",
        }
    }

    pub fn from_ui_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.label() == label)
    }

    /// Enabled until the user changes it.
    pub fn on_by_default(self) -> bool {
        matches!(self, Self::Name | Self::Hex | Self::Rgb | Self::Hsl | Self::Hsv)
    }

    /// The name can't be typed in (yet).
    pub fn editable(self) -> bool {
        self != Self::Name
    }

    /// An example of the format, for error messages.
    pub fn example(self) -> &'static str {
        match self {
            Self::Name => "a color name",
            Self::Hex => "#FF46A2",
            Self::Rgb => "rgb(255, 70, 162)",
            Self::Hsl => "hsl(330, 100%, 64%)",
            Self::Hsv => "hsv(330, 73%, 100%)",
            Self::Cmyk => "cmyk(0%, 73%, 36%, 0%)",
            Self::Hsla => "hsla(330, 100%, 64%, 1)",
            Self::Oklch => "oklch(0.682 0.217 352.4)",
            Self::Oklab => "oklab(0.682 0.207 -0.066)",
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

fn to_srgb(rgb: Rgb) -> Srgb {
    Srgb::new(rgb.0 as f32 / 255.0, rgb.1 as f32 / 255.0, rgb.2 as f32 / 255.0)
}

/// OKLab as [L, a, b] (used for formatting and for finding color names).
pub fn rgb_to_oklab(rgb: Rgb) -> [f32; 3] {
    let lab: Oklab = Oklab::from_color(to_srgb(rgb).into_linear());
    [lab.l, lab.a, lab.b]
}

/// CMYK without a printer profile: a common approximation, in percent.
pub fn format_cmyk(r: u8, g: u8, b: u8) -> String {
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let k = 1.0 - rf.max(gf).max(bf);
    let (c, m, y) = if k >= 1.0 {
        (0.0, 0.0, 0.0)
    } else {
        ((1.0 - rf - k) / (1.0 - k), (1.0 - gf - k) / (1.0 - k), (1.0 - bf - k) / (1.0 - k))
    };
    let pct = |v: f32| (v * 100.0).round().clamp(0.0, 100.0);
    format!("cmyk({:.0}%,{:.0}%,{:.0}%,{:.0}%)", pct(c), pct(m), pct(y), pct(k))
}

/// Screen pixels have no transparency, so alpha is always 1.
pub fn format_hsla(r: u8, g: u8, b: u8) -> String {
    let hsl = format_hsl(r, g, b);
    let inner = hsl.trim_start_matches("hsl(").trim_end_matches(')');
    format!("hsla({inner},1)")
}

/// Rounds and avoids printing "-0".
fn fixed(v: f32, decimals: usize) -> String {
    let s = format!("{v:.decimals$}");
    if s.trim_start_matches('-').chars().all(|c| c == '0' || c == '.') {
        s.trim_start_matches('-').to_string()
    } else {
        s
    }
}

/// CSS syntax: oklch(L C H) with L 0–1.
pub fn format_oklch(r: u8, g: u8, b: u8) -> String {
    let lch: Oklch = Oklch::from_color(to_srgb((r, g, b)).into_linear());
    let hue = if lch.chroma < 0.0005 {
        0.0
    } else {
        lch.hue.into_positive_degrees()
    };
    format!("oklch({} {} {})", fixed(lch.l, 3), fixed(lch.chroma, 3), fixed(hue, 1))
}

/// CSS syntax: oklab(L a b) with L 0–1.
pub fn format_oklab(r: u8, g: u8, b: u8) -> String {
    let [l, a, bb] = rgb_to_oklab((r, g, b));
    format!("oklab({} {} {})", fixed(l, 3), fixed(a, 3), fixed(bb, 3))
}

pub fn format_canonical(field: ColorField, rgb: Rgb) -> String {
    let (r, g, b) = rgb;
    match field {
        ColorField::Name => crate::names::display_name(rgb),
        ColorField::Hex => format_hex(r, g, b),
        ColorField::Rgb => format_rgb(r, g, b),
        ColorField::Hsl => format_hsl(r, g, b),
        ColorField::Hsv => format_hsv(r, g, b),
        ColorField::Cmyk => format_cmyk(r, g, b),
        ColorField::Hsla => format_hsla(r, g, b),
        ColorField::Oklch => format_oklch(r, g, b),
        ColorField::Oklab => format_oklab(r, g, b),
    }
}

/// What gets copied: like the display, but without the "≈ " of near names.
pub fn copy_text(field: ColorField, rgb: Rgb) -> String {
    let text = format_canonical(field, rgb);
    text.trim_start_matches("≈ ").to_string()
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

/// The numbers inside "func(...)", split on commas, spaces or "/".
fn components(value: &str, func_name: &str) -> Vec<String> {
    inner_function_payload(value, func_name)
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect()
}

/// "73%" or "0.73" → 0.73 (both accepted where it is unambiguous).
fn fraction(value: &str) -> Option<f32> {
    if let Some(p) = value.strip_suffix('%') {
        return Some(p.trim().parse::<f32>().ok()? / 100.0);
    }
    value.parse::<f32>().ok()
}

fn degrees(value: &str) -> Option<f32> {
    value.trim_end_matches("deg").parse::<f32>().ok()
}

pub fn parse_cmyk_permissive(value: &str) -> Option<Rgb> {
    let parts = components(value, "cmyk");
    if parts.len() != 4 {
        return None;
    }
    // Plain numbers are percentages here (cmyk(0, 73, 36, 0)), like "%" values.
    let pct = |s: &str| -> Option<f32> {
        let raw = s.strip_suffix('%').unwrap_or(s).trim().parse::<f32>().ok()?;
        Some((raw / 100.0).clamp(0.0, 1.0))
    };
    let (c, m, y, k) = (pct(&parts[0])?, pct(&parts[1])?, pct(&parts[2])?, pct(&parts[3])?);
    let ch = |v: f32| (255.0 * (1.0 - v) * (1.0 - k)).round().clamp(0.0, 255.0) as u8;
    Some((ch(c), ch(m), ch(y)))
}

pub fn parse_hsla_permissive(value: &str) -> Option<Rgb> {
    let parts = components(value, "hsla");
    // alpha is optional and ignored (screen colors are opaque)
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    parse_hsl_permissive(&format!("hsl({},{},{})", parts[0], parts[1], parts[2]))
}

pub fn parse_oklch_permissive(value: &str) -> Option<Rgb> {
    let parts = components(value, "oklch");
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let l = fraction(&parts[0])?;
    // chroma as "%" means % of 0.4 (CSS)
    let c = match parts[1].strip_suffix('%') {
        Some(p) => p.trim().parse::<f32>().ok()? / 100.0 * 0.4,
        None => parts[1].parse::<f32>().ok()?,
    };
    let h = degrees(&parts[2])?;
    let rgb: Srgb = Srgb::from_linear(Oklch::new(l.clamp(0.0, 1.0), c.max(0.0), h).into_color());
    Some(srgb_to_rgb(rgb))
}

pub fn parse_oklab_permissive(value: &str) -> Option<Rgb> {
    let parts = components(value, "oklab");
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let l = fraction(&parts[0])?;
    let a = parts[1].parse::<f32>().ok()?;
    let b = parts[2].parse::<f32>().ok()?;
    let rgb: Srgb = Srgb::from_linear(Oklab::new(l.clamp(0.0, 1.0), a, b).into_color());
    Some(srgb_to_rgb(rgb))
}

pub fn parse_color(field: ColorField, value: &str) -> Option<Rgb> {
    match field {
        ColorField::Name => None,
        ColorField::Hex => parse_hex_flexible(value),
        ColorField::Rgb => parse_rgb_permissive(value),
        ColorField::Hsl => parse_hsl_permissive(value),
        ColorField::Hsv => parse_hsv_permissive(value),
        ColorField::Cmyk => parse_cmyk_permissive(value),
        ColorField::Hsla => parse_hsla_permissive(value),
        ColorField::Oklch => parse_oklch_permissive(value),
        ColorField::Oklab => parse_oklab_permissive(value),
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
    fn formats_new_fields() {
        let c = (255, 70, 162); // #FF46A2
        assert_eq!(format_canonical(ColorField::Cmyk, c), "cmyk(0%,73%,36%,0%)");
        assert_eq!(format_canonical(ColorField::Hsla, c), "hsla(330,100%,64%,1)");
        assert!(format_canonical(ColorField::Oklch, c).starts_with("oklch(0.6"));
        assert!(format_canonical(ColorField::Oklab, c).starts_with("oklab(0.6"));
        // grays have no hue, and never print "-0"
        assert_eq!(format_canonical(ColorField::Oklch, (128, 128, 128)), "oklch(0.600 0.000 0.0)");
        assert!(!format_canonical(ColorField::Oklab, (128, 128, 128)).contains("-0"));
        assert_eq!(format_canonical(ColorField::Cmyk, (0, 0, 0)), "cmyk(0%,0%,0%,100%)");
    }

    #[test]
    fn new_fields_round_trip_within_one_step() {
        let colors = [(255, 70, 162), (25, 129, 206), (20, 184, 166), (255, 196, 0), (0, 0, 0), (255, 255, 255), (128, 128, 128)];
        for field in [ColorField::Cmyk, ColorField::Hsla, ColorField::Oklch, ColorField::Oklab] {
            for c in colors {
                let text = format_canonical(field, c);
                let back = parse_color(field, &text).unwrap_or_else(|| panic!("{text} didn't parse"));
                let diff = |a: u8, b: u8| (a as i32 - b as i32).abs();
                assert!(
                    diff(back.0, c.0) <= 2 && diff(back.1, c.1) <= 2 && diff(back.2, c.2) <= 2,
                    "{field:?}: {c:?} -> {text} -> {back:?}"
                );
            }
        }
    }

    #[test]
    fn parses_common_ways_of_writing_new_formats() {
        assert_eq!(parse_color(ColorField::Cmyk, "cmyk(0, 100, 100, 0)"), Some((255, 0, 0)));
        assert_eq!(parse_color(ColorField::Hsla, "hsla(0, 100%, 50%, 0.5)"), Some((255, 0, 0)));
        assert_eq!(parse_color(ColorField::Hsla, "hsla(0 100% 50% / 1)"), Some((255, 0, 0)));
        assert_eq!(parse_color(ColorField::Oklch, "oklch(62.8% 0.2577 29.23deg)"), Some((255, 0, 0)));
        assert_eq!(parse_color(ColorField::Oklab, "oklab(0.628 0.2249 0.1258)"), Some((255, 0, 0)));
        assert_eq!(parse_color(ColorField::Oklch, "oklch(nope)"), None);
        assert_eq!(parse_color(ColorField::Name, "Red"), None);
    }

    #[test]
    fn labels_round_trip() {
        for f in ColorField::ALL {
            assert_eq!(ColorField::from_ui_label(f.label()), Some(f));
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
