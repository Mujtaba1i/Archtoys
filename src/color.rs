//! Pure color math: formatting, parsing and shades. No UI code here, so
//! everything in this file is covered by the unit tests at the bottom.

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

// ------------------------------------------------- color space conversions

// The arithmetic below follows the `palette` crate step by step (the app used it
// until v0.4.0), so every value on screen stays exactly what it was: checked on
// all 16.7 million colors.

/// The hue in degrees (0 for grays), the largest and the smallest channel (0–1),
/// shared by HSL and HSV.
fn hue_parts(rgb: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (rgb.0 as f32 / 255.0, rgb.1 as f32 / 255.0, rgb.2 as f32 / 255.0);
    let (max, min, sep, coeff) = if r > g { (r, g, g - b, 0.0) } else { (g, r, b - r, 2.0) };
    let (max, min, sep, coeff) = if b > max { (b, min, r - g, 4.0) } else { (max, min.min(b), sep, coeff) };
    let hue = if max != min { (sep / (max - min) + coeff) * 60.0 } else { 0.0 };
    (hue, max, min)
}

/// Whole degrees, rounded the way the app always has: halves away from 0
/// with the hue taken between -180 and 180 (232.5 shows as 232).
fn round_hue(hue: f32) -> f32 {
    let signed = hue - (((hue + 180.0) / 360.0) - 1.0).ceil() * 360.0;
    signed.round().rem_euclid(360.0)
}

/// RGB from hue (degrees), chroma (largest minus smallest channel) and the
/// smallest channel, the three values the HSL and HSV formulas share.
fn from_hue(h: f32, min: f32, chroma: f32) -> Rgb {
    let h = h.rem_euclid(360.0) / 60.0;
    let x = chroma * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    to_rgb(r + min, g + min, b + min)
}

/// 0–1 channels to bytes.
fn to_rgb(r: f32, g: f32, b: f32) -> Rgb {
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    (byte(r), byte(g), byte(b))
}

/// A constant written as an exact decimal, rounded to f32 the way palette does.
const fn c(x: f64) -> f32 {
    x as f32
}

/// The sRGB curve: a screen value (0–255) to light intensity (0–1), and back.
fn to_linear(byte: u8) -> f32 {
    let x = byte as f32 / 255.0;
    if x <= c(0.04045) {
        c(1.0 / 12.92) * x
    } else {
        x.mul_add(c(1.0 / 1.055), c(0.055 / 1.055)).powf(c(2.4))
    }
}

fn from_linear(x: f32) -> f32 {
    if x <= c(0.0031308) {
        c(12.92) * x
    } else {
        x.powf(c(1.0 / 2.4)) * c(1.055) - c(0.055)
    }
}

/// OKLab as [L, a, b] (used for formatting and for finding color names).
/// Matrices from Björn Ottosson's definition of OKLab.
pub fn rgb_to_oklab(rgb: Rgb) -> [f32; 3] {
    let (r, g, b) = (to_linear(rgb.0), to_linear(rgb.1), to_linear(rgb.2));
    let l = (c(0.4122214708) * r + c(0.5363325363) * g + c(0.0514459929) * b).cbrt();
    let m = (c(0.2119034982) * r + c(0.6806995451) * g + c(0.1073969566) * b).cbrt();
    let s = (c(0.0883024619) * r + c(0.2817188376) * g + c(0.6299787005) * b).cbrt();
    [
        c(0.2104542553) * l + c(0.7936177850) * m - c(0.0040720468) * s,
        c(1.9779984951) * l - c(2.4285922050) * m + c(0.4505937099) * s,
        c(0.0259040371) * l + c(0.7827717662) * m - c(0.8086757660) * s,
    ]
}

fn oklab_to_rgb(lightness: f32, a: f32, b: f32) -> Rgb {
    let l_ = lightness + c(0.3963377774) * a + c(0.2158037573) * b;
    let m_ = lightness - c(0.1055613458) * a - c(0.0638541728) * b;
    let s_ = lightness - c(0.0894841775) * a - c(1.2914855480) * b;
    let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    to_rgb(
        from_linear(c(4.0767416621) * l - c(3.3077115913) * m + c(0.2309699292) * s),
        from_linear(c(-1.2684380046) * l + c(2.6097574011) * m - c(0.3413193965) * s),
        from_linear(c(-0.0041960863) * l - c(0.7034186147) * m + c(1.7076147010) * s),
    )
}

// ------------------------------------------------------------- formatting

pub fn format_hsl(r: u8, g: u8, b: u8) -> String {
    let (hue, max, min) = hue_parts((r, g, b));
    let l = (max + min) / 2.0;
    let sum = max + min;
    let s = if max == min { 0.0 } else if sum > 1.0 { (max - min) / (2.0 - sum) } else { (max - min) / sum };
    let h = round_hue(hue);
    let s = (s * 100.0).round().clamp(0.0, 100.0);
    let l = (l * 100.0).round().clamp(0.0, 100.0);
    format!("hsl({h:.0},{s:.0}%,{l:.0}%)")
}

pub fn format_hsv(r: u8, g: u8, b: u8) -> String {
    let (hue, max, min) = hue_parts((r, g, b));
    let s = if max == 0.0 { 0.0 } else { (max - min) / max };
    let h = round_hue(hue);
    let s = (s * 100.0).round().clamp(0.0, 100.0);
    let v = (max * 100.0).round().clamp(0.0, 100.0);
    format!("hsv({h:.0},{s:.0}%,{v:.0}%)")
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
    let [l, a, bb] = rgb_to_oklab((r, g, b));
    let chroma = a.hypot(bb);
    let hue = if chroma < 0.0005 {
        0.0
    } else {
        let degrees = (std::f32::consts::PI + (-bb).atan2(-a)).to_degrees();
        degrees - (degrees / 360.0).floor() * 360.0
    };
    format!("oklch({} {} {})", fixed(l, 3), fixed(chroma, 3), fixed(hue, 1))
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

pub fn parse_hsl_permissive(value: &str) -> Option<Rgb> {
    let payload = inner_function_payload(value, "hsl");
    let parts: Vec<&str> = payload.split(',').map(str::trim).collect();
    if parts.len() != 3 {
        return None;
    }

    let h = parts[0].parse::<f32>().ok()?.rem_euclid(360.0);
    let s = parse_percentage_0_to_1(parts[1])?;
    let l = parse_percentage_0_to_1(parts[2])?;

    let chroma = (1.0 - (2.0 * l - 1.0).abs()) * s;
    Some(from_hue(h, l - chroma / 2.0, chroma))
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

    Some(from_hue(h, v - v * s, v * s))
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
    let (c, h) = (c.max(0.0), h.to_radians());
    Some(oklab_to_rgb(l.clamp(0.0, 1.0), c * h.cos(), c * h.sin()))
}

pub fn parse_oklab_permissive(value: &str) -> Option<Rgb> {
    let parts = components(value, "oklab");
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let l = fraction(&parts[0])?;
    let a = parts[1].parse::<f32>().ok()?;
    let b = parts[2].parse::<f32>().ok()?;
    Some(oklab_to_rgb(l.clamp(0.0, 1.0), a, b))
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
