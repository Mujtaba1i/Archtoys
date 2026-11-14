//! The Google Fonts index (built monthly in github.com/Mujtaba1i/archtoys-font-index):
//! each Latin family's letters as small gray thumbnails. The selected words
//! are "written" with every family's thumbnails to pick the few worth
//! downloading. It's read as a stream, one family at a time, so a search
//! never holds the whole index in memory.

use super::matcher::{correlation, ink_box};
use super::net;
use image::{imageops, GrayImage};
use std::io::Read;
use std::path::PathBuf;

const LATEST: &str = "https://raw.githubusercontent.com/Mujtaba1i/archtoys-font-index/main/latest.json";
const BASE: &str = "https://raw.githubusercontent.com/Mujtaba1i/archtoys-font-index/main";

/// One Google Fonts family in the index.
#[derive(Clone, Debug)]
pub(crate) struct Family {
    pub name: String,
    /// its file's path in github.com/google/fonts ("ofl/roboto/Roboto[wdth,wght].ttf")
    pub path: String,
    /// 0 sans serif, 1 serif, 2 display, 3 handwriting, 4 monospace
    pub category: u8,
    /// 0 OFL, 1 Apache, 2 UFL
    pub licence: u8,
    pub designer: String,
}

fn dir() -> PathBuf {
    crate::ocr::lang::app_data_dir().join("font-index")
}

/// The newest index on this computer, whatever month.
fn cached() -> Option<PathBuf> {
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(dir()).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "gz")).collect();
    files.sort();
    files.pop()
}

/// The index: this month's, downloaded (and its SHA-256 checked) when it's
/// new, else the one on this computer (offline, or GitHub unreachable).
pub(crate) fn get(progress: &dyn Fn(u64, Option<u64>, f64), stop: &std::sync::atomic::AtomicBool) -> Result<PathBuf, String> {
    // tests and CI: an index file on disk
    if let Some(path) = std::env::var_os("ARCHTOYS_FONT_INDEX").map(PathBuf::from).filter(|p| p.exists()) {
        return Ok(path);
    }
    let latest = net::text(LATEST).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).map_err(|e| e.to_string()));
    let latest = match latest {
        Ok(v) => v,
        Err(err) => return cached().ok_or(format!("couldn't get the font index ({err})")),
    };
    let (Some(file), Some(sha)) = (latest["file"].as_str(), latest["sha256"].as_str()) else {
        return cached().ok_or("the font index's latest.json is incomplete".into());
    };
    if latest["format"].as_u64() != Some(1) {
        return cached().ok_or("the font index is a newer format: update Archtoys".into());
    }
    let dest = dir().join(file.replace('/', "-"));
    if dest.exists() && net::sha256(&dest).as_deref() == Some(sha) {
        return Ok(dest);
    }
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    net::download(&format!("{BASE}/{file}"), &dest, progress, stop)?;
    if net::sha256(&dest).as_deref() != Some(sha) {
        let _ = std::fs::remove_file(&dest);
        return Err("the font index didn't download right (its SHA-256 is wrong)".into());
    }
    // the older months aren't needed any more
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for entry in entries.flatten().filter(|e| e.path() != dest) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(dest)
}

fn byte(r: &mut impl Read) -> Result<u8, String> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b).map_err(|e| e.to_string())?;
    Ok(b[0])
}

fn bytes(r: &mut impl Read, n: usize) -> Result<Vec<u8>, String> {
    let mut v = vec![0u8; n];
    r.read_exact(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

fn u16le(r: &mut impl Read) -> Result<u16, String> {
    Ok(u16::from_le_bytes([byte(r)?, byte(r)?]))
}

/// A word written with a family's thumbnails (characters it has no picture for become spaces).
fn write(glyphs: &[(u8, Vec<u8>)], space: u8, h: u32, word: &str) -> GrayImage {
    let parts: Vec<(u32, &[u8])> = word
        .chars()
        .map(|c| match (c as u32).checked_sub(33).and_then(|i| glyphs.get(i as usize)) {
            Some((w, data)) => (*w as u32, data.as_slice()),
            None => (space as u32, &[][..]),
        })
        .collect();
    let width: u32 = parts.iter().map(|p| p.0).sum::<u32>().max(1);
    let mut out = GrayImage::from_pixel(width, h, image::Luma([255]));
    let mut x0 = 0;
    for (w, data) in parts {
        if !data.is_empty() {
            for y in 0..h {
                for x in 0..w {
                    out.put_pixel(x0 + x, y, image::Luma([data[(y * w + x) as usize]]));
                }
            }
        }
        x0 += w;
    }
    out
}

/// Darker strokes, as Bold would have (each pixel takes its darker left neighbour).
fn embolden(img: &GrayImage) -> GrayImage {
    GrayImage::from_fn(img.width(), img.height(), |x, y| {
        let here = img.get_pixel(x, y).0[0];
        let left = if x > 0 { img.get_pixel(x - 1, y).0[0] } else { 255 };
        image::Luma([here.min(left)])
    })
}

/// How alike a font's thumbnails write the words and the selection's words
/// look (each cut to its ink, weighted by width): as drawn, and emboldened (a
/// Bold guess for a font with only a Regular here). Every candidate, local
/// or from the index, is judged by this one measure, so their scores compare.
/// (Drawn at the screen's size instead, pixel for pixel, the right font
/// scored lower than look-alikes: at 10-12 px a stroke half a pixel off counts
/// as a different shape. Thumbnails are drawn big and shrunk smoothly.)
pub(crate) fn score(glyphs: &[(u8, Vec<u8>)], space: u8, h: u32, words: &[(GrayImage, String)]) -> (f32, f32) {
    let soft = |img: &GrayImage| imageops::blur(img, 0.8);
    let (mut regular, mut bold, mut weight) = (0f32, 0f32, 0f32);
    for (selection, word) in words {
        let Some(strip) = ink_box(&write(glyphs, space, h, word)) else { continue };
        let small = soft(&imageops::resize(selection, strip.width(), strip.height(), imageops::FilterType::Triangle));
        regular += correlation(&small, &soft(&strip)) * selection.width() as f32;
        bold += correlation(&small, &soft(&embolden(&strip))) * selection.width() as f32;
        weight += selection.width() as f32;
    }
    if weight > 0.0 {
        (regular / weight, bold / weight)
    } else {
        (0.0, 0.0)
    }
}

/// The `keep` families whose thumbnails write the words most like the
/// selection's, surest first: (score, Bold looks closer, the family).
pub(crate) fn shortlist(path: &std::path::Path, words: &[(GrayImage, String)], keep: usize) -> Result<Vec<(f32, bool, Family)>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut r = std::io::BufReader::new(flate2::read::GzDecoder::new(file));
    if bytes(&mut r, 4)? != b"ATFI" || byte(&mut r)? != 1 {
        return Err("not a font index".into());
    }
    let h = byte(&mut r)? as u32;
    let _ = u16le(&mut r)?;
    let count = u32::from_le_bytes(bytes(&mut r, 4)?.try_into().unwrap_or([0; 4]));
    let mut best: Vec<(f32, bool, Family)> = Vec::new();
    for _ in 0..count {
        let n = byte(&mut r)? as usize;
        let name = String::from_utf8_lossy(&bytes(&mut r, n)?).into_owned();
        let n = u16le(&mut r)? as usize;
        let font_path = String::from_utf8_lossy(&bytes(&mut r, n)?).into_owned();
        let _flags = byte(&mut r)?; // bit 0: a weight axis (the font file says so too)
        let category = byte(&mut r)?;
        let licence = byte(&mut r)?;
        let n = byte(&mut r)? as usize;
        let designer = String::from_utf8_lossy(&bytes(&mut r, n)?).into_owned();
        let space = byte(&mut r)?;
        let mut glyphs = Vec::with_capacity(94);
        for _ in 0..94 {
            let w = byte(&mut r)?;
            glyphs.push((w, bytes(&mut r, w as usize * h as usize)?));
        }
        if super::local::other_script(&name) {
            continue; // made for another script (Noto Sans Symbols...): it carries Noto Sans's Latin
        }
        let (regular, bold) = score(&glyphs, space, h, words);
        let s = regular.max(bold);
        if best.len() < keep || s > best.last().map_or(f32::MIN, |b| b.0) {
            let family = Family { name, path: font_path, category, licence, designer };
            let at = best.partition_point(|b| b.0 >= s);
            best.insert(at, (s, bold > regular, family));
            best.truncate(keep);
        }
    }
    Ok(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_written_with_thumbnails_and_spaces() {
        // "!" is 2 wide and black, anything else is a space 3 wide
        let mut glyphs = vec![(2u8, vec![0u8; 2 * 4])];
        glyphs.extend((1..94).map(|_| (1u8, vec![255u8; 4])));
        let img = write(&glyphs, 3, 4, "! !");
        assert_eq!(img.dimensions(), (2 + 3 + 2, 4));
        assert_eq!((img.get_pixel(0, 0).0[0], img.get_pixel(3, 0).0[0], img.get_pixel(6, 3).0[0]), (0, 255, 0));
    }

    #[test]
    fn bold_darkens_to_the_right() {
        let img = GrayImage::from_fn(4, 1, |x, _| image::Luma([if x == 1 { 0 } else { 255 }]));
        let b = embolden(&img);
        assert_eq!((b.get_pixel(1, 0).0[0], b.get_pixel(2, 0).0[0], b.get_pixel(3, 0).0[0]), (0, 0, 255));
    }
}
