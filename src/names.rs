//! Color names.
//!
//! First the 148 standard CSS color names (so `#FF69B4` is "Hot Pink"), then
//! the closest of ~32,000 names from meodai/color-names (MIT, see
//! data/colornames-LICENSE.txt). "Closest" is measured in OKLab, where the
//! distance matches how different two colors look to people.
//!
//! The list is built into the app (no internet needed) and parsed once on
//! first use; a lookup takes well under a millisecond.

use crate::color::{rgb_to_oklab, Rgb};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const NAMES_CSV: &str = include_str!("../data/colornames.csv");

/// The standard CSS color names (one per color; "Gray"/"Cyan"/"Magenta"
/// for the colors that have two spellings).
const CSS_NAMES: &[(&str, Rgb)] = &[
    ("Alice Blue", (240, 248, 255)),
    ("Antique White", (250, 235, 215)),
    ("Aquamarine", (127, 255, 212)),
    ("Azure", (240, 255, 255)),
    ("Beige", (245, 245, 220)),
    ("Bisque", (255, 228, 196)),
    ("Black", (0, 0, 0)),
    ("Blanched Almond", (255, 235, 205)),
    ("Blue", (0, 0, 255)),
    ("Blue Violet", (138, 43, 226)),
    ("Brown", (165, 42, 42)),
    ("Burlywood", (222, 184, 135)),
    ("Cadet Blue", (95, 158, 160)),
    ("Chartreuse", (127, 255, 0)),
    ("Chocolate", (210, 105, 30)),
    ("Coral", (255, 127, 80)),
    ("Cornflower Blue", (100, 149, 237)),
    ("Cornsilk", (255, 248, 220)),
    ("Crimson", (220, 20, 60)),
    ("Cyan", (0, 255, 255)),
    ("Dark Blue", (0, 0, 139)),
    ("Dark Cyan", (0, 139, 139)),
    ("Dark Goldenrod", (184, 134, 11)),
    ("Dark Gray", (169, 169, 169)),
    ("Dark Green", (0, 100, 0)),
    ("Dark Khaki", (189, 183, 107)),
    ("Dark Magenta", (139, 0, 139)),
    ("Dark Olive Green", (85, 107, 47)),
    ("Dark Orange", (255, 140, 0)),
    ("Dark Orchid", (153, 50, 204)),
    ("Dark Red", (139, 0, 0)),
    ("Dark Salmon", (233, 150, 122)),
    ("Dark Sea Green", (143, 188, 143)),
    ("Dark Slate Blue", (72, 61, 139)),
    ("Dark Slate Gray", (47, 79, 79)),
    ("Dark Turquoise", (0, 206, 209)),
    ("Dark Violet", (148, 0, 211)),
    ("Deep Pink", (255, 20, 147)),
    ("Deep Sky Blue", (0, 191, 255)),
    ("Dim Gray", (105, 105, 105)),
    ("Dodger Blue", (30, 144, 255)),
    ("Firebrick", (178, 34, 34)),
    ("Floral White", (255, 250, 240)),
    ("Forest Green", (34, 139, 34)),
    ("Gainsboro", (220, 220, 220)),
    ("Ghost White", (248, 248, 255)),
    ("Gold", (255, 215, 0)),
    ("Goldenrod", (218, 165, 32)),
    ("Gray", (128, 128, 128)),
    ("Green", (0, 128, 0)),
    ("Green Yellow", (173, 255, 47)),
    ("Honeydew", (240, 255, 240)),
    ("Hot Pink", (255, 105, 180)),
    ("Indian Red", (205, 92, 92)),
    ("Indigo", (75, 0, 130)),
    ("Ivory", (255, 255, 240)),
    ("Khaki", (240, 230, 140)),
    ("Lavender", (230, 230, 250)),
    ("Lavender Blush", (255, 240, 245)),
    ("Lawn Green", (124, 252, 0)),
    ("Lemon Chiffon", (255, 250, 205)),
    ("Light Blue", (173, 216, 230)),
    ("Light Coral", (240, 128, 128)),
    ("Light Cyan", (224, 255, 255)),
    ("Light Goldenrod Yellow", (250, 250, 210)),
    ("Light Gray", (211, 211, 211)),
    ("Light Green", (144, 238, 144)),
    ("Light Pink", (255, 182, 193)),
    ("Light Salmon", (255, 160, 122)),
    ("Light Sea Green", (32, 178, 170)),
    ("Light Sky Blue", (135, 206, 250)),
    ("Light Slate Gray", (119, 136, 153)),
    ("Light Steel Blue", (176, 196, 222)),
    ("Light Yellow", (255, 255, 224)),
    ("Lime", (0, 255, 0)),
    ("Lime Green", (50, 205, 50)),
    ("Linen", (250, 240, 230)),
    ("Magenta", (255, 0, 255)),
    ("Maroon", (128, 0, 0)),
    ("Medium Aquamarine", (102, 205, 170)),
    ("Medium Blue", (0, 0, 205)),
    ("Medium Orchid", (186, 85, 211)),
    ("Medium Purple", (147, 112, 219)),
    ("Medium Sea Green", (60, 179, 113)),
    ("Medium Slate Blue", (123, 104, 238)),
    ("Medium Spring Green", (0, 250, 154)),
    ("Medium Turquoise", (72, 209, 204)),
    ("Medium Violet Red", (199, 21, 133)),
    ("Midnight Blue", (25, 25, 112)),
    ("Mint Cream", (245, 255, 250)),
    ("Misty Rose", (255, 228, 225)),
    ("Moccasin", (255, 228, 181)),
    ("Navajo White", (255, 222, 173)),
    ("Navy", (0, 0, 128)),
    ("Old Lace", (253, 245, 230)),
    ("Olive", (128, 128, 0)),
    ("Olive Drab", (107, 142, 35)),
    ("Orange", (255, 165, 0)),
    ("Orange Red", (255, 69, 0)),
    ("Orchid", (218, 112, 214)),
    ("Pale Goldenrod", (238, 232, 170)),
    ("Pale Green", (152, 251, 152)),
    ("Pale Turquoise", (175, 238, 238)),
    ("Pale Violet Red", (219, 112, 147)),
    ("Papaya Whip", (255, 239, 213)),
    ("Peach Puff", (255, 218, 185)),
    ("Peru", (205, 133, 63)),
    ("Pink", (255, 192, 203)),
    ("Plum", (221, 160, 221)),
    ("Powder Blue", (176, 224, 230)),
    ("Purple", (128, 0, 128)),
    ("Rebecca Purple", (102, 51, 153)),
    ("Red", (255, 0, 0)),
    ("Rosy Brown", (188, 143, 143)),
    ("Royal Blue", (65, 105, 225)),
    ("Saddle Brown", (139, 69, 19)),
    ("Salmon", (250, 128, 114)),
    ("Sandy Brown", (244, 164, 96)),
    ("Sea Green", (46, 139, 87)),
    ("Seashell", (255, 245, 238)),
    ("Sienna", (160, 82, 45)),
    ("Silver", (192, 192, 192)),
    ("Sky Blue", (135, 206, 235)),
    ("Slate Blue", (106, 90, 205)),
    ("Slate Gray", (112, 128, 144)),
    ("Snow", (255, 250, 250)),
    ("Spring Green", (0, 255, 127)),
    ("Steel Blue", (70, 130, 180)),
    ("Tan", (210, 180, 140)),
    ("Teal", (0, 128, 128)),
    ("Thistle", (216, 191, 216)),
    ("Tomato", (255, 99, 71)),
    ("Turquoise", (64, 224, 208)),
    ("Violet", (238, 130, 238)),
    ("Wheat", (245, 222, 179)),
    ("White", (255, 255, 255)),
    ("White Smoke", (245, 245, 245)),
    ("Yellow", (255, 255, 0)),
    ("Yellow Green", (154, 205, 50)),
];

struct Entry {
    name: &'static str,
    lab: [f32; 3],
}

/// Sorted by lightness (OKLab L), so a lookup only has to check the names
/// whose lightness is close enough to possibly be the nearest.
fn entries() -> &'static [Entry] {
    static LIST: OnceLock<Vec<Entry>> = OnceLock::new();
    LIST.get_or_init(|| {
        let mut list = Vec::with_capacity(32_000);
        for line in NAMES_CSV.lines().skip(1) {
            // name,#rrggbb,good-name-flag   (names contain no commas)
            let mut parts = line.split(',');
            let (Some(name), Some(hex)) = (parts.next(), parts.next()) else { continue };
            if let Some(rgb) = crate::color::parse_hex_flexible(hex) {
                list.push(Entry { name, lab: rgb_to_oklab(rgb) });
            }
        }
        list.sort_by(|a, b| a.lab[0].total_cmp(&b.lab[0]));
        list
    })
}

fn distance(a: &[f32; 3], b: &[f32; 3]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

/// The nearest entry. Walks outwards from the entries with the same
/// lightness and stops once the lightness difference alone is already
/// bigger than the best distance found, so the answer is exact but most of
/// the list is never looked at.
fn nearest(lab: &[f32; 3]) -> Option<(&'static Entry, f32)> {
    let list = entries();
    let start = list.partition_point(|e| e.lab[0] < lab[0]);
    let mut best: Option<(&'static Entry, f32)> = None;
    let (mut lo, mut hi) = (start, start); // lo: next below (exclusive), hi: next above
    loop {
        let bound = best.map_or(f32::INFINITY, |(_, d)| d);
        let down = (lo > 0).then(|| (lab[0] - list[lo - 1].lab[0]).powi(2)).filter(|dl| *dl < bound);
        let up = (hi < list.len()).then(|| (list[hi].lab[0] - lab[0]).powi(2)).filter(|dl| *dl < bound);
        let index = match (down, up) {
            (None, None) => break,
            (Some(d), Some(u)) if d <= u => { lo -= 1; lo }
            (Some(_), None) => { lo -= 1; lo }
            _ => { hi += 1; hi - 1 }
        };
        let d = distance(&list[index].lab, lab);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((&list[index], d));
        }
    }
    best
}

/// The color's name: exact if one exists, otherwise the closest one.
/// Returns (name, exact).
pub fn name_of(rgb: Rgb) -> (&'static str, bool) {
    if let Some((name, _)) = CSS_NAMES.iter().find(|(_, c)| *c == rgb) {
        return (name, true);
    }
    // Hovering often stays on the same few colors: remember recent answers.
    static CACHE: Mutex<Option<HashMap<Rgb, (&'static str, bool)>>> = Mutex::new(None);
    if let Some(hit) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&rgb).copied()) {
        return hit;
    }
    let answer = match nearest(&rgb_to_oklab(rgb)) {
        Some((entry, d)) => (entry.name, d < 1e-10),
        None => ("", false),
    };
    let mut cache = CACHE.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if cache.len() > 20_000 {
        cache.clear();
    }
    cache.insert(rgb, answer);
    answer
}

/// What the NAME row shows: "Hot Pink", or "≈ Hot Pink" if only close.
pub fn display_name(rgb: Rgb) -> String {
    match name_of(rgb) {
        ("", _) => String::new(),
        (name, true) => name.to_string(),
        (name, false) => format!("≈ {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_names_win_for_exact_colors() {
        assert_eq!(name_of((255, 105, 180)), ("Hot Pink", true));
        assert_eq!(name_of((255, 0, 0)), ("Red", true));
        assert_eq!(name_of((128, 128, 128)), ("Gray", true));
        assert_eq!(CSS_NAMES.len(), 139); // 148 names, 9 colors have two spellings
    }

    #[test]
    fn list_is_loaded_and_exact_matches_are_found() {
        assert!(entries().len() > 30_000);
        assert_eq!(name_of((0xa5, 0x93, 0x44)), ("18th Century Green", true));
    }

    /// The fast search must give exactly what checking every name gives.
    #[test]
    fn fast_search_matches_checking_every_name() {
        let mut seed: u32 = 12345;
        for _ in 0..3000 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let rgb = ((seed >> 8) as u8, (seed >> 16) as u8, (seed >> 24) as u8);
            let lab = rgb_to_oklab(rgb);
            let brute = entries()
                .iter()
                .map(|e| (e, distance(&e.lab, &lab)))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .unwrap();
            let fast = nearest(&lab).unwrap();
            assert!((fast.1 - brute.1).abs() < 1e-9, "{rgb:?}: {} vs {}", fast.0.name, brute.0.name);
        }
    }

    #[test]
    fn other_colors_get_the_closest_name() {
        let (name, exact) = name_of((0xff, 0x46, 0xa2));
        assert!(!exact);
        assert!(!name.is_empty());
        assert!(display_name((0xff, 0x46, 0xa2)).starts_with("≈ "));
    }
}
