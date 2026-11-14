//! Text drawn in a font file the way a screen draws it (hinted, gray), so it
//! can be compared with the selection. swash (already used by Slint) does the
//! drawing. Bold comes from the font's weight axis when it has one.

use image::GrayImage;
use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;
use swash::FontRef;

/// One font, loaded once and drawn many times.
pub(crate) struct Face {
    data: Vec<u8>,
    /// the font's weight axis set to Regular (400) and to Bold (700), when it
    /// has one: a variable font's default isn't always Regular (Montserrat's
    /// and Raleway's is their thinnest)
    regular_coord: Option<Vec<i16>>,
    bold_coord: Option<Vec<i16>>,
}

impl Face {
    pub(crate) fn open(path: &std::path::Path) -> Option<Face> {
        let data = std::fs::read(path).ok()?;
        let font = FontRef::from_index(&data, 0)?;
        // a font without Latin letters isn't one the text could be in
        let charmap = font.charmap();
        if "aeoAEO".chars().any(|c| charmap.map(c) == 0) {
            return None;
        }
        let vars: Vec<_> = font.variations().collect();
        let wght = swash::tag_from_bytes(b"wght");
        let at = |weight: f32| -> Option<Vec<i16>> {
            vars.iter()
                .any(|v| v.tag() == wght)
                .then(|| vars.iter().map(|v| if v.tag() == wght { v.normalize(weight) } else { 0 }).collect())
        };
        Some(Face { regular_coord: at(400.0), bold_coord: at(700.0), data })
    }

    pub(crate) fn has_weight_axis(&self) -> bool {
        self.bold_coord.is_some()
    }

    /// `text` drawn at `px` pixels per em, dark on white, with a 2 px margin.
    pub(crate) fn draw(&self, text: &str, px: f32, bold: bool) -> Option<GrayImage> {
        let font = FontRef::from_index(&self.data, 0)?;
        let none: Vec<i16> = Vec::new();
        let coords: &[i16] = match (bold, &self.bold_coord, &self.regular_coord) {
            (true, Some(c), _) | (false, _, Some(c)) => c,
            _ => &none,
        };
        let metrics = font.metrics(coords).scale(px);
        let glyph_metrics = font.glyph_metrics(coords).scale(px);
        let charmap = font.charmap();
        let mut context = ScaleContext::new();
        let mut builder = context.builder(font).size(px).hint(false);
        if !coords.is_empty() {
            builder = builder.normalized_coords(coords.iter());
        }
        let mut scaler = builder.build();
        let mut render = Render::new(&[Source::Outline]);
        render.format(Format::Alpha);
        let pad = 2i32;
        let ascent = metrics.ascent.ceil() as i32;
        let height = (metrics.ascent + metrics.descent).ceil() as i32 + 2 * pad;
        let mut pen = pad as f32;
        let mut glyphs = Vec::new();
        for c in text.chars() {
            let id = charmap.map(c);
            if let Some(image) = render.render(&mut scaler, id) {
                glyphs.push((pen, image));
            }
            pen += glyph_metrics.advance_width(id);
        }
        let width = (pen.ceil() as i32 + 2 * pad).max(1);
        let mut out = GrayImage::from_pixel(width as u32, height.max(1) as u32, image::Luma([255]));
        for (x, image) in glyphs {
            let p = image.placement;
            let (left, top) = (x.round() as i32 + p.left, pad + ascent - p.top);
            for row in 0..p.height as i32 {
                for col in 0..p.width as i32 {
                    let (ox, oy) = (left + col, top + row);
                    if ox < 0 || oy < 0 || ox >= width || oy >= height {
                        continue;
                    }
                    let alpha = image.data[(row * p.width as i32 + col) as usize];
                    let px = out.get_pixel_mut(ox as u32, oy as u32);
                    px.0[0] = px.0[0].min(255 - alpha);
                }
            }
        }
        Some(out)
    }
}
