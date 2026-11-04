//! The color names and the search index, prebuilt into the app.
//!
//! `data/names-index.bin` holds fixed-size records (numbers, little-endian) and
//! `data/names-index.txt` every name and its search form. Both are read where
//! they are, inside the binary: nothing is parsed or copied at startup, and
//! they cost no memory until used.
//!
//! They are made from `data/colornames.csv` and the CSS names by `build` (only
//! in tests). `the_index_is_up_to_date` fails when they no longer match; then:
//!     cargo test --locked write_name_index -- --ignored
//!
//! Layout of the .bin: "ATNI", then the counts (u32): names, extra names,
//! items, words. Then the records:
//!   name  (20 bytes): OKLab L a b (f32 ×3), r g b, text offset (u32), length (u8)
//!         the first `names` are the CSV's, by lightness (for the nearest name);
//!         the extra ones are the CSS names the search uses
//!   item  (22 bytes): name index (u16), search-form offset (u32), length (u8),
//!         quality (u8), alphabetical rank (u16), OKLCH L C h (f32 ×3)
//!   word   (7 bytes): offset (u32), length (u8), how many names use it (u16); alphabetical

use crate::color::Rgb;

static BIN: &[u8] = include_bytes!("../data/names-index.bin");
static TEXT: &str = include_str!("../data/names-index.txt");

const HEADER: usize = 20;
const NAME: usize = 20;
const ITEM: usize = 22;
const WORD: usize = 7;

fn u32_at(o: usize) -> u32 {
    u32::from_le_bytes([BIN[o], BIN[o + 1], BIN[o + 2], BIN[o + 3]])
}
fn f32_at(o: usize) -> f32 {
    f32::from_bits(u32_at(o))
}
fn f32x3_at(o: usize) -> [f32; 3] {
    [f32_at(o), f32_at(o + 4), f32_at(o + 8)]
}
/// One record, bounds-checked once (searching reads every record per keystroke).
fn record<const N: usize>(o: usize) -> &'static [u8; N] {
    BIN[o..o + N].try_into().expect("names-index.bin is cut short")
}
fn le_f32(r: &[u8], i: usize) -> f32 {
    f32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]])
}
fn le_text(r: &[u8], i: usize) -> &'static str {
    let start = u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]) as usize;
    &TEXT[start..start + usize::from(r[i + 4])]
}
fn count(n: usize) -> usize {
    u32_at(4 + 4 * n) as usize
}

/// A color name from the names list (or a CSS name, for the search).
#[derive(Clone, Copy)]
pub(crate) struct Name {
    pub name: &'static str,
    pub rgb: Rgb,
    pub lab: [f32; 3],
}

/// How many names the nearest-name lookup uses (sorted by lightness).
pub(crate) fn name_count() -> usize {
    count(0)
}

fn name_offset(i: usize) -> usize {
    HEADER + i * NAME
}

/// OKLab lightness of name `i`: the nearest-name lookup's fast path.
#[inline]
pub(crate) fn lightness(i: usize) -> f32 {
    f32_at(name_offset(i))
}

#[inline]
pub(crate) fn name(i: usize) -> Name {
    let r: &[u8; NAME] = record(name_offset(i));
    Name {
        lab: [le_f32(r, 0), le_f32(r, 4), le_f32(r, 8)],
        rgb: (r[12], r[13], r[14]),
        name: le_text(r, 15),
    }
}

/// One search entry. Matching reads only the fields; the name, color and
/// OKLab come from the names table when asked for.
#[derive(Clone, Copy)]
pub(crate) struct Item {
    name_index: u16,
    /// The name as `search::normalize` writes it, e.g. "a l orange" for "À l’Orange".
    pub norm: &'static str,
    /// OKLCH: lightness, chroma, hue in degrees.
    pub lch: [f32; 3],
    /// Points for a good name: 5 if marked "good name", 3 if a CSS name, 2 if short.
    pub quality: u8,
    /// Position in alphabetical order, the last tie-breaker.
    pub rank: u32,
}

pub(crate) fn item_count() -> usize {
    count(2)
}

fn items_start() -> usize {
    HEADER + (count(0) + count(1)) * NAME
}

impl Item {
    pub(crate) fn name(&self) -> &'static str {
        name(usize::from(self.name_index)).name
    }
    pub(crate) fn rgb(&self) -> Rgb {
        name(usize::from(self.name_index)).rgb
    }
    pub(crate) fn lab(&self) -> [f32; 3] {
        f32x3_at(name_offset(usize::from(self.name_index)))
    }
}

pub(crate) fn item(i: usize) -> Item {
    item_at(items_start() + i * ITEM)
}

#[inline]
fn item_at(o: usize) -> Item {
    let r: &[u8; ITEM] = record(o);
    Item {
        name_index: u16::from_le_bytes([r[0], r[1]]),
        norm: le_text(r, 2),
        quality: r[7],
        rank: u32::from(u16::from_le_bytes([r[8], r[9]])),
        lch: [le_f32(r, 10), le_f32(r, 14), le_f32(r, 18)],
    }
}

pub(crate) fn items() -> impl Iterator<Item = Item> {
    let start = items_start();
    (0..item_count()).map(move |i| item_at(start + i * ITEM))
}

fn words_start() -> usize {
    items_start() + item_count() * ITEM
}

/// Every distinct word in the search forms, alphabetical, with how many names use it.
pub(crate) fn words() -> impl Iterator<Item = (&'static str, u32)> {
    let start = words_start();
    (0..count(3)).map(move |i| word_at(start + i * WORD))
}

fn word(i: usize) -> (&'static str, u32) {
    word_at(words_start() + i * WORD)
}

#[inline]
fn word_at(o: usize) -> (&'static str, u32) {
    let r: &[u8; WORD] = record(o);
    (le_text(r, 0), u32::from(u16::from_le_bytes([r[5], r[6]])))
}

pub(crate) fn is_word(w: &str) -> bool {
    let (mut lo, mut hi) = (0, count(3));
    while lo < hi {
        let mid = (lo + hi) / 2;
        match word(mid).0.cmp(w) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

/// Makes the two files from the CSV and the CSS names (what the app did at
/// every start before v0.6.2).
#[cfg(test)]
fn build() -> (Vec<u8>, String) {
    use crate::color::rgb_to_oklab;
    use crate::names::CSS_NAMES;
    use crate::search::normalize;
    use std::collections::{HashMap, HashSet};

    const CSV: &str = include_str!("../data/colornames.csv");
    struct Row {
        name: &'static str,
        rgb: Rgb,
        lab: [f32; 3],
        good: bool,
    }
    let mut csv = vec![];
    for line in CSV.lines().skip(1) {
        // name,#rrggbb,good-name flag   (names contain no commas)
        let mut parts = line.split(',');
        let (Some(name), Some(hex)) = (parts.next(), parts.next()) else { continue };
        let good = parts.next().is_some_and(|flag| flag.trim() == "x");
        if let Some(rgb) = crate::color::parse_hex_flexible(hex) {
            csv.push(Row { name, rgb, lab: rgb_to_oklab(rgb), good });
        }
    }
    csv.sort_by(|a, b| a.lab[0].total_cmp(&b.lab[0]));

    // The search pool: the CSV names plus the CSS names. A CSS name keeps its CSS
    // color (Crimson is #DC143C, as the color card says), the same name with the
    // same color is kept once, and the same name with different colors keeps
    // every color.
    let good: HashSet<String> = csv.iter().filter(|e| e.good).map(|e| normalize(e.name)).collect();
    let css: HashSet<String> = CSS_NAMES.iter().map(|(n, _)| normalize(n)).collect();
    struct Pooled {
        name_index: usize,
        norm: String,
        lch: [f32; 3],
        quality: u8,
    }
    let mut pool: Vec<Pooled> = vec![];
    let mut seen: HashSet<(String, Rgb)> = HashSet::new();
    let mut add = |name_index: usize, name: &str, rgb: Rgb, is_css: bool| {
        let norm = normalize(name);
        if !seen.insert((norm.clone(), rgb)) {
            return;
        }
        let lab = rgb_to_oklab(rgb);
        let hue = lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0);
        let quality = if good.contains(&norm) { 5 } else { 0 }
            + if is_css { 3 } else { 0 }
            + if norm.len() <= 10 { 2 } else { 0 };
        pool.push(Pooled { name_index, norm, lch: [lab[0], lab[1].hypot(lab[2]), hue], quality });
    };
    for (k, (name, rgb)) in CSS_NAMES.iter().enumerate() {
        add(csv.len() + k, name, *rgb, true);
    }
    for (i, e) in csv.iter().enumerate() {
        if !css.contains(&normalize(e.name)) {
            add(i, e.name, e.rgb, false);
        }
    }
    let mut order: Vec<usize> = (0..pool.len()).collect();
    order.sort_by(|&a, &b| pool[a].norm.cmp(&pool[b].norm));
    let mut rank = vec![0usize; pool.len()];
    for (r, i) in order.into_iter().enumerate() {
        rank[i] = r;
    }

    let mut bin = b"ATNI".to_vec();
    let mut text = String::new();
    let mut put_text = |bin: &mut Vec<u8>, s: &str| {
        bin.extend((text.len() as u32).to_le_bytes());
        bin.push(u8::try_from(s.len()).expect("a name longer than 255 bytes"));
        text.push_str(s);
    };
    let names: Vec<(&str, Rgb, [f32; 3])> = csv
        .iter()
        .map(|e| (e.name, e.rgb, e.lab))
        .chain(CSS_NAMES.iter().map(|(n, rgb)| (*n, *rgb, rgb_to_oklab(*rgb))))
        .collect();

    // the words, and where each first appears in a search form
    let mut word_counts: HashMap<String, (u32, usize)> = HashMap::new();
    {
        let mut at = names.iter().map(|n| n.0.len()).sum::<usize>(); // the search forms come after the names
        for p in &pool {
            let mut start = at;
            for w in p.norm.split(' ') {
                word_counts.entry(w.to_string()).or_insert((0, start)).0 += 1;
                start += w.len() + 1;
            }
            at += p.norm.len();
        }
    }
    let mut words: Vec<(String, u32, usize)> = word_counts.into_iter().map(|(w, (c, o))| (w, c, o)).collect();
    words.sort();

    for n in [csv.len(), CSS_NAMES.len(), pool.len(), words.len()] {
        bin.extend((n as u32).to_le_bytes());
    }
    for (name, rgb, lab) in &names {
        for v in lab {
            bin.extend(v.to_le_bytes());
        }
        bin.extend([rgb.0, rgb.1, rgb.2]);
        put_text(&mut bin, name);
    }
    for (p, r) in pool.iter().zip(&rank) {
        bin.extend(u16::try_from(p.name_index).expect("too many names").to_le_bytes());
        put_text(&mut bin, &p.norm);
        bin.push(p.quality);
        bin.extend(u16::try_from(*r).expect("too many names").to_le_bytes());
        for v in p.lch {
            bin.extend(v.to_le_bytes());
        }
    }
    for (w, c, o) in &words {
        bin.extend((*o as u32).to_le_bytes());
        bin.push(u8::try_from(w.len()).unwrap());
        bin.extend(u16::try_from(*c).unwrap_or(u16::MAX).to_le_bytes());
    }
    (bin, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_is_up_to_date() {
        let (bin, text) = build();
        assert!(
            bin == BIN && text == TEXT,
            "data/names-index.bin/.txt don't match data/colornames.csv and the CSS names (or how they're \
             made changed): cargo test --locked write_name_index -- --ignored"
        );
    }

    /// Writes data/names-index.bin and .txt.
    #[test]
    #[ignore]
    fn write_name_index() {
        let (bin, text) = build();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("data");
        std::fs::write(dir.join("names-index.bin"), bin).unwrap();
        std::fs::write(dir.join("names-index.txt"), text).unwrap();
    }

    #[test]
    fn reads_back_what_was_written() {
        assert!(name_count() > 30_000);
        let red = (0..item_count()).map(item).find(|i| i.name() == "Red").unwrap();
        assert_eq!((red.rgb(), red.norm), ((255, 0, 0), "red"));
        assert!(is_word("barbie") && !is_word("barbi"));
        assert!(words().all(|(w, n)| !w.is_empty() && !w.contains(' ') && n > 0));
        // the whole index lies inside the file
        assert_eq!(words_start() + count(3) * WORD, BIN.len());
    }
}
