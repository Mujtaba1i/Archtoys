//! Color search: type a name, a word, a typo or a color code, get named colors.
//!
//! * A color word ("red", "dark red") chooses colors by their real color, measured
//!   in OKLCH, not by the word in their name: Maroon is found for "red", and
//!   Old Gungeon Red (which is blue) is not.
//! * Any other word ("sky", "blue sky") matches names; every word must match.
//! * A color code ("#FF46A2", "rgb(255,70,162)") shows the nearest named colors.
//! * A typo is corrected only when nothing matches, and the result says so.
//!
//! Everything comes from the names already built into the app (`names.rs`), so
//! the search adds nothing to the download. The index is built once, in the
//! background at startup (`prepare`), so the first search is instant.

use crate::color::{parse_color, parse_hex_flexible, rgb_to_oklab, ColorField, Rgb};
use crate::names::{self, CSS_NAMES};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// What a search found.
#[derive(Debug, Default, PartialEq)]
pub struct Results {
    /// Best first. Indexes for `entry`.
    pub hits: Vec<u32>,
    /// "Showing results for pink", a hint when nothing matches, or None.
    pub note: Option<String>,
}

/// Name and color of a search hit.
pub fn entry(index: u32) -> (&'static str, Rgb) {
    let item = &pool().items[index as usize];
    (item.name, item.rgb)
}

/// Builds the index on a background thread, so the first search doesn't wait.
pub fn prepare() {
    std::thread::spawn(|| {
        pool();
    });
}

pub fn search(query: &str) -> Results {
    let query = query.trim();
    if query.is_empty() {
        return Results::default();
    }
    if let Some(code) = color_code(query) {
        return match code {
            Ok(rgb) => Results { hits: nearest(rgb, 60), note: None },
            Err(example) => Results {
                hits: vec![],
                note: Some(format!("Finish the color code, for example {example}")),
            },
        };
    }

    let text = normalize(query);
    if text.is_empty() {
        return Results::default(); // only punctuation: nothing to look for yet
    }
    let hits = matches(&text);
    if !hits.is_empty() {
        return Results { hits, note: None };
    }

    // Nothing matched. 1: a typo correction.
    if let Some(corrected) = correct(&text) {
        let hits = matches(&corrected);
        if !hits.is_empty() {
            return Results { hits, note: Some(format!("Showing results for {corrected}")) };
        }
    }
    // 2: a color word anywhere in the text.
    if let Some(word) = text.split(' ').find(|w| family(w).is_some()) {
        return Results { hits: matches(word), note: Some(format!("Showing results for {word}")) };
    }
    // 3: a hint.
    Results {
        hits: vec![],
        note: Some(format!(
            "No color matches '{query}'. Try a name like 'sky', a code like #FF46A2, or pick a color from your screen."
        )),
    }
}

// ------------------------------------------------------------------ the index

struct Item {
    name: &'static str,
    rgb: Rgb,
    /// The name as `normalize` writes it, e.g. "a l orange" for "À l’Orange".
    norm: String,
    /// Where each word of `norm` starts.
    starts: Vec<u16>,
    lab: [f32; 3],
    /// OKLCH: lightness, chroma, hue in degrees.
    lch: [f32; 3],
    /// Points for a good name: 5 if marked "good name", 3 if a CSS name, 2 if short.
    quality: u8,
    /// Position in alphabetical order, the last tie-breaker.
    rank: u32,
}

struct Pool {
    items: Vec<Item>,
    /// Every distinct word in the names, with how many names use it.
    words: HashMap<String, u32>,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(build_pool)
}

fn lch(lab: [f32; 3]) -> [f32; 3] {
    let hue = lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0);
    [lab[0], lab[1].hypot(lab[2]), hue]
}

/// The names list plus the CSS names. A CSS name keeps its CSS color (Crimson is
/// #DC143C, as the color card says), the same name with the same color is kept
/// once, and the same name with different colors keeps every color.
fn build_pool() -> Pool {
    let csv = names::entries();
    let good: HashSet<String> = csv.iter().filter(|e| e.good).map(|e| normalize(e.name)).collect();
    let css: HashMap<String, Rgb> = CSS_NAMES.iter().map(|(n, rgb)| (normalize(n), *rgb)).collect();

    let mut items = Vec::with_capacity(csv.len() + CSS_NAMES.len());
    let mut seen: HashSet<(String, Rgb)> = HashSet::new();
    let mut add = |name: &'static str, rgb: Rgb, is_css: bool| {
        let norm = normalize(name);
        if !seen.insert((norm.clone(), rgb)) {
            return;
        }
        let lab = rgb_to_oklab(rgb);
        let quality = if good.contains(&norm) { 5 } else { 0 }
            + if is_css { 3 } else { 0 }
            + if norm.len() <= 10 { 2 } else { 0 };
        let starts = std::iter::once(0)
            .chain(norm.match_indices(' ').map(|(i, _)| i + 1))
            .map(|i| i as u16)
            .collect();
        items.push(Item { name, rgb, norm, starts, lab, lch: lch(lab), quality, rank: 0 });
    };
    for (name, rgb) in CSS_NAMES {
        add(name, *rgb, true);
    }
    for e in csv {
        if !css.contains_key(&normalize(e.name)) {
            add(e.name, e.rgb, false);
        }
    }

    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| items[a].norm.cmp(&items[b].norm));
    for (rank, i) in order.into_iter().enumerate() {
        items[i].rank = rank as u32;
    }

    let mut words: HashMap<String, u32> = HashMap::new();
    for item in &items {
        for word in item.norm.split(' ') {
            *words.entry(word.to_string()).or_default() += 1;
        }
    }
    Pool { items, words }
}

// -------------------------------------------------------------- normalizing

/// Accented letters in the names, without their accents.
fn fold(ch: char) -> Option<&'static str> {
    Some(match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ǎ' => "a",
        'ç' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => "e",
        'ğ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' | 'ǐ' => "i",
        'ł' => "l",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ő' => "o",
        'š' => "s",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ǔ' | 'ǜ' => "u",
        'ý' | 'ÿ' => "y",
        'ż' => "z",
        'ß' => "ss",
        'œ' => "oe",
        'æ' => "ae",
        '²' | '₂' => "2",
        'ⅱ' => "ii",
        '№' => "no",
        _ => return None,
    })
}

/// Lowercase, no accents, punctuation as spaces, and one spelling for words
/// with two ("grey" is "gray", "colour" is "color"). Used for both what you type
/// and the names, so "a l orange" finds "À l’Orange".
pub fn normalize(text: &str) -> String {
    let mut spaced = String::with_capacity(text.len());
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            spaced.push(ch);
        } else if let Some(plain) = fold(ch) {
            spaced.push_str(plain);
        } else if ch.is_alphanumeric() {
            spaced.push(ch); // other scripts stay as they are
        } else {
            spaced.push(' ');
        }
    }
    let words: Vec<String> = spaced
        .split_whitespace()
        .map(|w| {
            if let Some(rest) = w.strip_prefix("grey") {
                format!("gray{rest}")
            } else if let Some(rest) = w.strip_prefix("colour") {
                format!("color{rest}")
            } else {
                w.to_string()
            }
        })
        .collect();
    words.join(" ")
}

// ------------------------------------------------------------ color codes

/// `Some(Ok(rgb))` for a color code, `Some(Err(example))` for a code that isn't
/// finished yet, None for plain text. Hex needs the '#', so words like "cafe"
/// stay words.
fn color_code(query: &str) -> Option<Result<Rgb, &'static str>> {
    if query.starts_with('#') {
        return Some(parse_hex_flexible(query).ok_or(ColorField::Hex.example()));
    }
    let lower = query.to_ascii_lowercase();
    let open = lower.find('(')?;
    let prefix = lower[..open].trim();
    let field = ColorField::ALL
        .into_iter()
        .find(|f| f.label().eq_ignore_ascii_case(prefix))?;
    Some(parse_color(field, query).ok_or(field.example()))
}

/// The `count` named colors that look closest (OKLab distance).
fn nearest(rgb: Rgb, count: usize) -> Vec<u32> {
    let lab = rgb_to_oklab(rgb);
    let mut scored: Vec<(f32, u32)> = pool()
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let d = (item.lab[0] - lab[0]).powi(2) + (item.lab[1] - lab[1]).powi(2) + (item.lab[2] - lab[2]).powi(2);
            (d, i as u32)
        })
        .collect();
    let count = count.min(scored.len());
    scored.select_nth_unstable_by(count.saturating_sub(1), |a, b| a.0.total_cmp(&b.0));
    scored.truncate(count);
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    scored.into_iter().map(|(_, i)| i).collect()
}

// ---------------------------------------------------------- color families

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Magenta,
    Pink,
    Brown,
    Gray,
    Black,
    White,
}

fn family(word: &str) -> Option<Family> {
    Some(match word {
        "red" => Family::Red,
        "orange" => Family::Orange,
        "yellow" => Family::Yellow,
        "green" => Family::Green,
        "cyan" => Family::Cyan,
        "blue" => Family::Blue,
        "purple" => Family::Purple,
        "magenta" => Family::Magenta,
        "pink" => Family::Pink,
        "brown" => Family::Brown,
        "gray" => Family::Gray, // "grey" too: normalize spells it "gray"
        "black" => Family::Black,
        "white" => Family::White,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shade {
    Dark,
    Light,
}

fn shade(word: &str) -> Option<Shade> {
    match word {
        "dark" | "deep" => Some(Shade::Dark),
        "light" | "pale" => Some(Shade::Light),
        _ => None,
    }
}

/// 0 at `from`, 1 at `to` (falling when `from` > `to`), straight in between.
fn ramp(x: f32, from: f32, to: f32) -> f32 {
    ((x - from) / (to - from)).clamp(0.0, 1.0)
}

/// 1 for a hue between `lo` and `hi` degrees (it may wrap past 360), fading to
/// 0 over `fade_lo` degrees below and `fade_hi` above.
fn hue_band(h: f32, lo: f32, hi: f32, fade_lo: f32, fade_hi: f32) -> f32 {
    let width = (hi - lo).rem_euclid(360.0);
    let offset = (h - lo).rem_euclid(360.0);
    if offset <= width {
        return 1.0;
    }
    let above = offset - width;
    let below = 360.0 - offset;
    (1.0 - above / fade_hi).max(1.0 - below / fade_lo).max(0.0)
}

fn pink_fit([l, c, h]: [f32; 3]) -> f32 {
    hue_band(h, 330.0, 20.0, 12.0, 12.0) * ramp(c, 0.03, 0.06) * ramp(l, 0.55, 0.68)
}

fn brown_fit([l, c, h]: [f32; 3]) -> f32 {
    hue_band(h, 40.0, 80.0, 20.0, 20.0) * ramp(c, 0.02, 0.05) * ramp(l, 0.62, 0.50) * ramp(l, 0.12, 0.22)
}

/// How well a color belongs to a family, from 0 to 1. At 0.6 or more it is
/// shown, from 0.2 it sits at the bottom of the list, below 0.2 it is hidden.
/// Eight families are hues; pink is a light red or magenta, brown a dark
/// orange, and gray, black and white have little color, so lightness decides.
/// The constants were tuned by eye on the real list.
fn fit(family: Family, lch: [f32; 3]) -> f32 {
    let [l, c, h] = lch;
    let hue = |lo, hi, fade_lo, fade_hi| {
        hue_band(h, lo, hi, fade_lo, fade_hi) * ramp(c, 0.03, 0.08) * ramp(l, 0.12, 0.25)
    };
    match family {
        Family::Red => hue(14.0, 36.0, 34.0, 26.0) * (1.0 - 0.6 * pink_fit(lch)),
        Family::Orange => hue(45.0, 78.0, 14.0, 14.0) * (1.0 - 0.6 * brown_fit(lch)),
        Family::Yellow => hue(88.0, 115.0, 15.0, 18.0),
        Family::Green => hue(120.0, 165.0, 15.0, 15.0),
        Family::Cyan => hue(172.0, 215.0, 12.0, 15.0),
        Family::Blue => hue(220.0, 275.0, 15.0, 15.0),
        Family::Purple => hue(282.0, 332.0, 15.0, 12.0),
        Family::Magenta => hue(318.0, 350.0, 12.0, 10.0) * (1.0 - 0.4 * pink_fit(lch)),
        Family::Pink => pink_fit(lch),
        Family::Brown => brown_fit(lch),
        Family::Gray => ramp(c, 0.05, 0.02) * ramp(l, 0.2, 0.3) * ramp(l, 0.93, 0.86),
        Family::Black => ramp(l, 0.30, 0.18) * ramp(c, 0.10, 0.05),
        Family::White => ramp(l, 0.85, 0.93) * ramp(c, 0.06, 0.03),
    }
}

fn shade_fit(shade: Shade, l: f32) -> f32 {
    match shade {
        Shade::Dark => ramp(l, 0.60, 0.45),
        Shade::Light => ramp(l, 0.60, 0.72),
    }
}

const SHOWN: f32 = 0.6;
const BOTTOM: f32 = 0.2;

// ------------------------------------------------------------- matching

/// `str::contains` sets up a searcher on every call; names are so short that
/// a plain scan is several times faster.
fn contains(name: &str, needle: &str) -> bool {
    let (name, needle) = (name.as_bytes(), needle.as_bytes());
    match needle.len() {
        0 => true,
        1 => name.contains(&needle[0]),
        n => name.windows(n).any(|w| w == needle),
    }
}

/// 24 if `needle` starts a word of the name, 12 if it is somewhere else in it, 0 if not.
fn word_fit(item: &Item, needle: &str) -> u8 {
    if item.starts.iter().any(|&s| item.norm[s as usize..].starts_with(needle)) {
        24
    } else if contains(&item.norm, needle) {
        12
    } else {
        0
    }
}

/// Name points: the name starts with the text 30, a word of it does 24, it
/// contains the text 12. With several words, every word must match (in any
/// order) and the weakest one counts, unless the whole text does better.
fn name_fit(item: &Item, text: &str, words: &[&str]) -> u8 {
    if item.norm.starts_with(text) {
        return 30;
    }
    if words.len() < 2 {
        return word_fit(item, text);
    }
    let mut weakest = 24;
    for w in words {
        match word_fit(item, w) {
            0 => return 0,
            fit => weakest = weakest.min(fit),
        }
    }
    weakest.max(word_fit(item, text))
}

/// Normal matching, with no typo correction. `text` is already normalized.
fn matches(text: &str) -> Vec<u32> {
    let mut words: Vec<&str> = text.split(' ').collect();
    let families: Vec<Family> = words.iter().filter_map(|w| family(w)).collect();
    let shades: Vec<Shade> = words.iter().filter_map(|w| shade(w)).collect();
    // "red" or "dark red": choose by color. Anything else: by name.
    let by_color = (families.len() == 1 && shades.len() <= 1 && families.len() + shades.len() == words.len())
        .then(|| (families[0], shades.first().copied()));
    // Longest word first: it is the rarest, so most names are ruled out at once.
    words.sort_by_key(|w| std::cmp::Reverse(w.len()));

    // Sort key, packed into one number so sorting thousands of hits stays fast:
    // exact name first, then tier, more points, shorter name, alphabetical.
    let mut found: Vec<(u64, u32)> = Vec::with_capacity(4096);
    for (i, item) in pool().items.iter().enumerate() {
        let exact = item.norm == text;
        let (tier, points) = match by_color {
            Some((family, shade)) => {
                let color = fit(family, item.lch) * shade.map_or(1.0, |s| shade_fit(s, item.lch[0]));
                if color < BOTTOM && !exact {
                    continue; // hidden means hidden, unless typed in full
                }
                let names = name_fit(item, text, &words);
                (u8::from(color < SHOWN), color * 60.0 + f32::from(names) + f32::from(item.quality))
            }
            None => {
                let names = name_fit(item, text, &words);
                if names == 0 {
                    continue;
                }
                (0, f32::from(names + item.quality))
            }
        };
        let key = (u64::from(!exact) << 63)
            | (u64::from(tier) << 62)
            | ((16_383 - (points * 100.0) as u64).min(16_383) << 48) // points are at most 100
            | ((item.norm.len().min(255) as u64) << 40)
            | u64::from(item.rank);
        found.push((key, i as u32));
    }
    found.sort_unstable_by_key(|f| f.0);
    found.into_iter().map(|f| f.1).collect()
}

// ----------------------------------------------------------------- typos

/// Edits between two words, where a swapped pair of letters counts as one
/// ("pnik" is one slip from "pink"): optimal string alignment distance, on
/// bytes (normalized names are plain letters). Stops early once the answer
/// is sure to be more than `limit`, and then returns `limit + 1`.
fn edit_distance(a: &str, b: &str, limit: usize) -> usize {
    const MAX: usize = 63; // longer than any word in the names
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len().abs_diff(b.len()) > limit || a.len() > MAX || b.len() > MAX {
        return limit + 1;
    }
    // three rows on the stack (this runs for thousands of words per keystroke):
    // two back (for swaps), the previous one, the current one
    let mut back = [0usize; MAX + 1];
    let mut prev = [0usize; MAX + 1];
    let mut cur = [0usize; MAX + 1];
    for (j, cell) in prev.iter_mut().enumerate().take(b.len() + 1) {
        *cell = j;
    }
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(back[j - 2] + 1);
            }
            cur[j] = best;
            row_min = row_min.min(best);
        }
        if row_min > limit {
            return limit + 1;
        }
        std::mem::swap(&mut back, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()].min(limit + 1)
}

/// Words of 4 to 7 letters may have 1 mistake, longer words 2, shorter none.
fn allowed_mistakes(word: &str) -> usize {
    match word.chars().count() {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

/// The text with each unknown word replaced by the closest word in the names,
/// or None if nothing could be corrected. Real words are never changed.
fn correct(text: &str) -> Option<String> {
    let words = &pool().words;
    let mut changed = false;
    let mut out = vec![];
    for word in text.split(' ') {
        if words.contains_key(word) {
            out.push(word.to_string());
            continue;
        }
        let limit = allowed_mistakes(word);
        if limit == 0 {
            return None;
        }
        let best = words
            .iter()
            .filter(|(w, _)| w.len().abs_diff(word.len()) <= limit)
            .map(|(w, count)| (edit_distance(word, w, limit), *count, w))
            .filter(|(d, _, _)| *d <= limit)
            .min_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)).then(a.2.cmp(b.2)));
        let (_, _, w) = best?;
        out.push(w.clone());
        changed = true;
    }
    changed.then(|| out.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(results: &Results) -> Vec<&'static str> {
        results.hits.iter().map(|&i| entry(i).0).collect()
    }

    fn position(results: &Results, name: &str) -> Option<usize> {
        names(results).iter().position(|n| *n == name)
    }

    fn item(name: &str) -> &'static Item {
        pool().items.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("{name} is not in the list"))
    }

    #[test]
    fn normalizing_folds_accents_punctuation_and_spellings() {
        assert_eq!(normalize("À l’Orange"), "a l orange");
        assert_eq!(normalize("  April Fool's   Red "), "april fool s red");
        assert_eq!(normalize("Battleship Grey"), "battleship gray");
        assert_eq!(normalize("Greyish COLOUR"), "grayish color");
        assert_eq!(normalize("Straße"), "strasse");
    }

    #[test]
    fn every_name_folds_to_plain_letters() {
        for item in &pool().items {
            assert!(
                item.norm.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == ' ' || c == 'я'),
                "{} -> {}",
                item.name,
                item.norm
            );
        }
    }

    #[test]
    fn the_pool_is_the_built_in_names_with_no_double_entries() {
        let items = &pool().items;
        assert!(items.len() > 31_900, "{}", items.len());
        let mut seen = HashSet::new();
        for i in items {
            assert!(seen.insert((i.norm.clone(), i.rgb)), "{} twice", i.name);
        }
        // CSS names keep their CSS color, and the missing ones are added.
        assert_eq!(item("Crimson").rgb, (220, 20, 60));
        assert_eq!(item("Deep Pink").rgb, (255, 20, 147));
    }

    #[test]
    fn red_shows_reds_by_color_and_hides_the_rest() {
        let r = search("red");
        assert_eq!(names(&r)[0], "Red");
        for shown in ["Maroon", "Dark Red", "Crimson"] {
            assert!(position(&r, shown).is_some(), "{shown} missing");
        }
        for hidden in ["April Fool’s Red", "Old Gungeon Red", "Orange"] {
            assert!(position(&r, hidden).is_none(), "{hidden} shown");
        }
        // A name match ranks above a color without one.
        assert!(position(&r, "Dark Red") < position(&r, "Maroon"));
    }

    #[test]
    fn blue_shows_blues_only() {
        let r = search("blue");
        assert_eq!(names(&r)[0], "Blue");
        let old = position(&r, "Old Gungeon Red").expect("Old Gungeon Red missing");
        assert!(old > 0);
        for &i in &r.hits {
            let lch = pool().items[i as usize].lch;
            for other in [Family::Red, Family::Green, Family::Gray] {
                assert!(fit(other, lch) < SHOWN, "{} is {other:?}", entry(i).0);
            }
        }
    }

    #[test]
    fn a_full_name_always_comes_first_even_when_hidden() {
        assert_eq!(names(&search("April Fool's Red"))[0], "April Fool’s Red");
        assert_eq!(names(&search("a l orange"))[0], "À l’Orange");
    }

    #[test]
    fn dark_red_shows_dark_reds_above_light_ones() {
        let r = search("dark red");
        assert!(position(&r, "Maroon").is_some() && position(&r, "Dark Red").is_some());
        assert!(position(&r, "Red").is_none(), "bright Red shown for dark red");
        for &i in r.hits.iter().take(30) {
            assert!(pool().items[i as usize].lch[0] < 0.6, "{} is not dark", entry(i).0);
        }
    }

    #[test]
    fn typos_are_corrected_only_when_nothing_matches() {
        let r = search("pnik");
        assert_eq!(r.note.as_deref(), Some("Showing results for pink"));
        assert!(position(&r, "Pink").is_some());
        assert!(position(&r, "Pine").is_none() && position(&r, "Pie").is_none());
        for (typo, word) in [("ornage", "orange"), ("purlpe", "purple"), ("magneta", "magenta"), ("tourquoise", "turquoise")] {
            assert_eq!(search(typo).note, Some(format!("Showing results for {word}")), "{typo}");
        }
    }

    #[test]
    fn real_words_are_never_corrected() {
        let r = search("bleu");
        assert_eq!(r.note, None);
        assert!(position(&r, "Bleu de France").is_some());
        assert!(position(&r, "Blue").is_none());
    }

    #[test]
    fn words_match_in_any_order() {
        let r = search("blue sky");
        assert!(position(&r, "Sky Blue").is_some_and(|p| p < 10), "{:?}", &names(&r)[..10.min(r.hits.len())]);
    }

    #[test]
    fn grey_and_gray_are_the_same() {
        assert_eq!(search("grey"), search("gray"));
        let r = search("grey");
        assert!(position(&r, "Slate Gray").is_some() && position(&r, "Battleship Grey").is_some());
    }

    #[test]
    fn color_codes_show_the_nearest_names() {
        let hex = search("#FF46A2");
        assert!(position(&hex, "Barbie Pink").is_some_and(|p| p < 3), "{:?}", &names(&hex)[..5]);
        assert_eq!(search("rgb(255,70,162)"), hex);
        assert!(color_code("cafe").is_none()); // a word, not a hex code
        assert_eq!(search("rgb(255,70").note.as_deref(), Some("Finish the color code, for example rgb(255, 70, 162)"));
    }

    #[test]
    fn gibberish_ends_with_a_hint() {
        let r = search("xqzvw");
        assert!(r.hits.is_empty());
        assert!(r.note.unwrap().starts_with("No color matches 'xqzvw'. Try a name like 'sky'"));
        assert_eq!(search("   "), Results::default());
        assert_eq!(search("!!! '"), Results::default());
    }

    #[test]
    fn a_color_word_anywhere_is_the_next_step() {
        let r = search("my favourite blue zzzq");
        assert_eq!(r.note.as_deref(), Some("Showing results for blue"));
        assert_eq!(names(&r)[0], "Blue");
    }

    #[test]
    fn swapped_letters_are_one_mistake() {
        assert_eq!(edit_distance("pnik", "pink", 2), 1);
        assert_eq!(edit_distance("ornage", "orange", 2), 1);
        assert_eq!(edit_distance("pnik", "pine", 2), 2);
        assert_eq!(edit_distance("tourquoise", "turquoise", 2), 1);
        assert_eq!(edit_distance("", "abc", 5), 3);
        assert_eq!(edit_distance("abcdef", "uvwxyz", 1), 2); // gave up early: more than 1
        assert_eq!(allowed_mistakes("red"), 0);
        assert_eq!(allowed_mistakes("tourquoise"), 2);
    }

    /// Run with: cargo test --release search::tests::speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn speed() {
        pool();
        let mut worst = std::time::Duration::ZERO;
        for query in ["red", "dark red", "blue sky", "pnik", "a l orange", "xqzvw", "#FF46A2", "turquoise", "midnight b"] {
            for end in 1..=query.len() {
                let start = std::time::Instant::now();
                let r = search(&query[..end]);
                let took = start.elapsed();
                worst = worst.max(took);
                println!("{:>8.3} ms  {:>5} hits  {:?}", took.as_secs_f64() * 1e3, r.hits.len(), &query[..end]);
            }
        }
        println!("worst: {:.3} ms", worst.as_secs_f64() * 1e3);
        assert!(worst < std::time::Duration::from_millis(2));
    }

    /// Each family's best colors and tier sizes, to judge the thresholds by eye.
    /// Run with: cargo test --release search::tests::family_report -- --ignored --nocapture
    #[test]
    #[ignore]
    fn family_report() {
        for word in ["red", "orange", "yellow", "green", "cyan", "blue", "purple", "magenta", "pink", "brown", "gray", "black", "white", "dark red", "pale blue"] {
            let r = search(word);
            let (shown, bottom): (Vec<u32>, Vec<u32>) = r.hits.iter().partition(|&&i| {
                let item = &pool().items[i as usize];
                let text = normalize(word);
                let ws: Vec<&str> = text.split(' ').collect();
                let fam = ws.iter().find_map(|w| family(w)).unwrap();
                let sh = ws.iter().find_map(|w| shade(w));
                fit(fam, item.lch) * sh.map_or(1.0, |s| shade_fit(s, item.lch[0])) >= SHOWN || item.norm == text
            });
            let show = |list: &[u32]| {
                list.iter().map(|&i| {
                    let (n, (r, g, b)) = entry(i);
                    format!("{n}=#{r:02X}{g:02X}{b:02X}")
                }).collect::<Vec<_>>().join(" | ")
            };
            println!("REPORT\t{word}\t{}\t{}\t{}\t{}", shown.len(), bottom.len(), show(&shown[..shown.len().min(40)]), show(&bottom[..bottom.len().min(15)]));
        }
        for name in ["Chocolate Red", "Hot Pink", "Brown", "Rosy Brown", "Slate Gray", "Indian Red", "Olive", "Teal", "Indigo", "Coral"] {
            let it = item(name);
            let fits: Vec<String> = [Family::Red, Family::Orange, Family::Pink, Family::Brown, Family::Blue, Family::Purple, Family::Magenta, Family::Gray, Family::Green, Family::Cyan, Family::Yellow]
                .iter().map(|f| format!("{f:?}={:.2}", fit(*f, it.lch))).collect();
            println!("EDGE\t{name}\t#{:02X}{:02X}{:02X}\t{}", it.rgb.0, it.rgb.1, it.rgb.2, fits.join(" "));
        }
    }
}
