//! How alike two pictures of text are. Word by word (screens hint the gaps
//! between words differently), each squeezed to the other's ink box and
//! softened a little (hinting snaps edges to whole pixels), then correlated.
//! Measured in the v0.7.5 prototype on text drawn by GTK's own stack: 60%
//! of the fonts first, 73% in the top 3, out of 1,481.

use image::{imageops, GrayImage};

/// Darker than this is ink.
const INK: u8 = 200;

/// The picture cut to its ink (None when there is none).
pub(crate) fn ink_box(img: &GrayImage) -> Option<GrayImage> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (x, y, p) in img.enumerate_pixels() {
        if p.0[0] < INK {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 <= x1 && y0 <= y1).then(|| imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image())
}

/// Pearson correlation of two pictures the same size (1 = the same, 0 = unrelated).
pub(crate) fn correlation(a: &GrayImage, b: &GrayImage) -> f32 {
    let n = a.as_raw().len().min(b.as_raw().len()) as f32;
    if n == 0.0 {
        return 0.0;
    }
    let (ma, mb) = (mean(a), mean(b));
    let (mut ab, mut aa, mut bb) = (0f32, 0f32, 0f32);
    for (&x, &y) in a.as_raw().iter().zip(b.as_raw()) {
        let (x, y) = (x as f32 - ma, y as f32 - mb);
        ab += x * y;
        aa += x * x;
        bb += y * y;
    }
    let d = (aa * bb).sqrt();
    if d > 0.0 {
        ab / d
    } else {
        0.0
    }
}

fn mean(img: &GrayImage) -> f32 {
    let raw = img.as_raw();
    raw.iter().map(|&v| v as f32).sum::<f32>() / raw.len().max(1) as f32
}

/// How alike a word of the selection (cut to its ink) and a drawn word look.
pub(crate) fn word_score(selection: &GrayImage, drawn: &GrayImage) -> f32 {
    let Some(drawn) = ink_box(drawn) else { return 0.0 };
    // compared at twice the selection's size: the drawing (made big) shrunk
    // smoothly, the selection grown, so a stroke half a pixel off on the
    // screen still overlaps (at 10-12 px, comparing pixel for pixel scored
    // the right font's own "notes" 0.49)
    let (w, h) = (selection.width() * 2, selection.height() * 2);
    let sigma = (h as f32 / 30.0).max(1.0);
    let grown = imageops::resize(selection, w, h, imageops::FilterType::Triangle);
    let squeezed = imageops::resize(&drawn, w, h, imageops::FilterType::Triangle);
    let alike = correlation(&imageops::blur(&grown, sigma), &imageops::blur(&squeezed, sigma));
    alike * proportions(selection, &drawn)
}

/// How close two words' proportions are (1 = the same width for their
/// height). Squeezed to the same box, a condensed font's word can look right
/// (French text came back "Sofia Sans Extra Condensed"): its proportions give it away.
fn proportions(a: &GrayImage, b: &GrayImage) -> f32 {
    let ratio = |img: &GrayImage| img.width() as f32 / img.height().max(1) as f32;
    let r = ratio(a) / ratio(b);
    // as is (squared and stronger lost more right answers than they won)
    r.min(1.0 / r)
}

/// A score from 0 to 1 as the "sure" percentage shown (a correlation of 0.6 or less is no match at all).
pub(crate) fn sure(score: f32) -> u8 {
    (((score - 0.6) / 0.4).clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(w: u32, h: u32, x0: u32, x1: u32) -> GrayImage {
        GrayImage::from_fn(w, h, |x, _| image::Luma([if (x0..x1).contains(&x) { 0 } else { 255 }]))
    }

    #[test]
    fn the_same_picture_scores_one_and_ink_is_found() {
        let a = bar(20, 10, 5, 9);
        assert!((correlation(&a, &a) - 1.0).abs() < 1e-5);
        let ink = ink_box(&a).unwrap();
        assert_eq!(ink.dimensions(), (4, 10));
        assert!(ink_box(&GrayImage::from_pixel(5, 5, image::Luma([255]))).is_none());
    }

    #[test]
    fn a_word_matches_itself_better_than_another() {
        // "letters" as vertical strokes: the same strokes drawn bigger, and others
        let strokes = |w: u32, h: u32, period: u32| {
            GrayImage::from_fn(w, h, |x, _| image::Luma([if (x * 30 / w).is_multiple_of(period) { 0 } else { 255 }]))
        };
        let selection = ink_box(&strokes(30, 12, 3)).unwrap();
        let same = strokes(60, 24, 3);
        let other = strokes(60, 24, 5);
        assert!(word_score(&selection, &same) > word_score(&selection, &other));
    }

    #[test]
    fn a_narrower_word_loses() {
        let strokes = |w: u32, h: u32| GrayImage::from_fn(w, h, |x, _| image::Luma([if (x * 30 / w).is_multiple_of(3) { 0 } else { 255 }]));
        let selection = ink_box(&strokes(30, 12)).unwrap();
        // the same strokes, at the same proportions, then squeezed to two thirds the width
        assert!(word_score(&selection, &strokes(60, 24)) > word_score(&selection, &strokes(40, 24)));
    }

    #[test]
    fn scores_become_a_percentage() {
        assert_eq!((sure(0.5), sure(0.8), sure(1.0)), (0, 50, 100));
    }
}
