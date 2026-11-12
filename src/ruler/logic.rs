//! The Screen Ruler's arithmetic, kept apart from the windows so it can be
//! tested: how far a color reaches (the edges), the sizes as text, which pick
//! is bright enough for the lines, the icon's teeth and the preview drawing.

use crate::color::Rgb;

/// The four modes, in the toolbar's order (Ctrl+1 to Ctrl+4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Bounds,
    Spacing,
    Horizontal,
    Vertical,
}

impl Mode {
    pub(crate) const ALL: [Mode; 4] = [Mode::Bounds, Mode::Spacing, Mode::Horizontal, Mode::Vertical];

    pub(crate) fn index(self) -> i32 {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(1) as i32
    }

    pub(crate) fn from_index(i: i32) -> Mode {
        Self::ALL.get(i.max(0) as usize).copied().unwrap_or(Mode::Spacing)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Mode::Bounds => "Bounds",
            Mode::Spacing => "Spacing",
            Mode::Horizontal => "Horizontal",
            Mode::Vertical => "Vertical",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Mode> {
        Self::ALL.iter().copied().find(|m| m.name().eq_ignore_ascii_case(name))
    }
}

/// One measured shape, in screen pixels (a line has the other side 0).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub w: u32,
    pub h: u32,
}

/// The tolerance: how much a color may differ (on any of red, green, blue)
/// and still count as the same surface (30 by default, config.rs).
pub(crate) const TOLERANCE_STEP: i32 = 5;
pub(crate) const TOLERANCE_MAX: i32 = 255;

/// The most shapes kept on the screen at once (Bounds, Shift-click).
pub(crate) const MAX_SHAPES: usize = 3;

/// Shift-click may keep the box being drawn: there is room for it and the next one.
pub(crate) fn room_to_keep(kept: usize) -> bool {
    kept + 1 < MAX_SHAPES
}

fn similar(a: &[u8], b: &[u8], tolerance: i32) -> bool {
    (0..3).all(|c| (i32::from(a[c]) - i32::from(b[c])).abs() <= tolerance)
}

/// From pixel (x, y) of an RGBA image, the last pixel in each direction that
/// still looks like (x, y) and is `inside` (the screen it may measure on):
/// (left, right, top, bottom), inclusive.
pub(crate) fn reach(
    rgba: &[u8],
    width: u32,
    height: u32,
    (x, y): (i32, i32),
    tolerance: i32,
    inside: impl Fn(i32, i32) -> bool,
) -> (i32, i32, i32, i32) {
    let (w, h) = (width as i32, height as i32);
    let x = x.clamp(0, w - 1);
    let y = y.clamp(0, h - 1);
    let at = |px: i32, py: i32| {
        let i = ((py * w + px) * 4) as usize;
        &rgba[i..i + 4]
    };
    let start = at(x, y);
    let (mut left, mut right, mut top, mut bottom) = (x, x, y, y);
    while left > 0 && inside(left - 1, y) && similar(at(left - 1, y), start, tolerance) {
        left -= 1;
    }
    while right < w - 1 && inside(right + 1, y) && similar(at(right + 1, y), start, tolerance) {
        right += 1;
    }
    while top > 0 && inside(x, top - 1) && similar(at(x, top - 1), start, tolerance) {
        top -= 1;
    }
    while bottom < h - 1 && inside(x, bottom + 1) && similar(at(x, bottom + 1), start, tolerance) {
        bottom += 1;
    }
    (left, right, top, bottom)
}

/// The biggest a measure may be, in screen pixels: its own screen's size, or
/// with `cross` every screen's added up, across and down (side by side or
/// stacked, either way).
pub(crate) fn most(screens: &[(u32, u32)], i: usize, cross: bool) -> (u32, u32) {
    if cross {
        screens.iter().fold((0, 0), |(w, h), s| (w + s.0, h + s.1))
    } else {
        screens.get(i).copied().unwrap_or((u32::MAX, u32::MAX))
    }
}

/// A size no bigger than `most` (rounding never makes it a pixel over).
pub(crate) fn capped(s: Size, most: (u32, u32)) -> Size {
    Size { w: s.w.min(most.0), h: s.h.min(most.1) }
}

/// Screenshot pixels → screen pixels (the screenshot can have more pixels than the screen).
pub(crate) fn screen_px(image_px: i32, step: i32) -> u32 {
    (image_px.max(0) as f32 / step.max(1) as f32).round() as u32
}

/// "360 × 94", "100" (a line), without the unit.
pub(crate) fn numbers(mode: Mode, s: Size) -> String {
    match mode {
        Mode::Horizontal => format!("{}", s.w),
        Mode::Vertical => format!("{}", s.h),
        _ => format!("{} × {}", s.w, s.h),
    }
}

/// The units the measurements are shown and copied in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unit {
    Px,
    Cm,
    In,
}

impl Unit {
    pub(crate) fn from_index(i: i32) -> Unit {
        match i {
            1 => Unit::Cm,
            2 => Unit::In,
            _ => Unit::Px,
        }
    }
}

fn physical(px: u32, mm_per_px: f32, unit: Unit) -> String {
    let mm = px as f32 * mm_per_px;
    match unit {
        Unit::In => format!("{:.2}", mm / 25.4),
        _ => format!("{:.1}", mm / 10.0),
    }
}

/// The size as it is copied and shown: "360 × 94 px", or in cm or inches when
/// the screen reports its size (`mm_per_px`), else in px.
pub(crate) fn text(mode: Mode, s: Size, unit: Unit, mm_per_px: Option<f32>) -> String {
    let Some(k) = mm_per_px.filter(|_| unit != Unit::Px) else {
        return format!("{} px", numbers(mode, s));
    };
    let u = if unit == Unit::In { "in" } else { "cm" };
    match mode {
        Mode::Horizontal => format!("{} {u}", physical(s.w, k, unit)),
        Mode::Vertical => format!("{} {u}", physical(s.h, k, unit)),
        _ => format!("{} × {} {u}", physical(s.w, k, unit), physical(s.h, k, unit)),
    }
}

/// Several shapes on one line, the unit once: "300 × 150, 200 × 200 px".
pub(crate) fn text_all(mode: Mode, shapes: &[Size], unit: Unit, mm_per_px: Option<f32>) -> String {
    let each: Vec<String> = shapes.iter().map(|s| text(mode, *s, unit, mm_per_px)).collect();
    let unit_word = each.first().and_then(|t| t.rsplit(' ').next()).unwrap_or("px").to_string();
    let bare: Vec<String> = each.iter().map(|t| t.trim_end_matches(&unit_word).trim_end().to_string()).collect();
    format!("{} {unit_word}", bare.join(", "))
}

/// "≈ 9.5 × 2.5 cm on this screen" under the big px size (when the screen reports its size).
pub(crate) fn physical_line(mode: Mode, s: Size, mm_per_px: Option<f32>) -> String {
    match mm_per_px {
        Some(k) => format!("≈ {} on this screen", text(mode, s, Unit::Cm, Some(k))),
        None => String::new(),
    }
}

/// A pick the lines can take (the easter egg): colorful enough, and neither
/// too dark nor too light to show on the screen. OKLCH chroma and lightness.
pub(crate) fn is_bright(rgb: Rgb) -> bool {
    let [l, a, b] = crate::color::rgb_to_oklab(rgb);
    let chroma = (a * a + b * b).sqrt();
    chroma >= 0.1 && (0.45..=0.85).contains(&l)
}

/// Archtoys orange: the lines' color before any bright pick.
pub(crate) const DEFAULT_LINE: Rgb = (0xFC, 0x4C, 0x02);

/// The lines' color: the newest bright color in the history, else orange.
pub(crate) fn line_color(history_newest_first: &[Rgb]) -> Rgb {
    history_newest_first.iter().copied().find(|c| is_bright(*c)).unwrap_or(DEFAULT_LINE)
}

/// The ruler icon's teeth (the other easter egg): one per cm of the last
/// measurement's longest side, at least 4 (the icon as drawn) and at most 7.
pub(crate) fn teeth(longest_px: u32, mm_per_px: Option<f32>) -> u32 {
    // without the screen's size: about 38 px per cm at 100% (96 dots per inch)
    let mm = longest_px as f32 * mm_per_px.unwrap_or(25.4 / 96.0);
    ((mm / 10.0).round() as u32).clamp(4, 7)
}

/// The icon itself: the ruler's outline and its teeth, big and small in turn
/// starting with a big one, spread evenly (an 18-unit square like the other icons).
pub(crate) fn icon_svg(teeth: u32, stroke: f32) -> String {
    let n = teeth.max(1);
    let mut path = String::new();
    for i in 1..=n {
        let x = 1.5 + 15.0 * i as f32 / (n + 1) as f32;
        let len = if i % 2 == 1 { 3.0 } else { 2.0 };
        path.push_str(&format!("M{x:.2} 5.5v{len}"));
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 18 18\" width=\"18\" height=\"18\" fill=\"none\" stroke=\"#000\" \
         stroke-width=\"{stroke}\" stroke-linecap=\"round\"><rect x=\"1.5\" y=\"5.5\" width=\"15\" height=\"7\" rx=\"1\"/><path d=\"{path}\"/></svg>"
    )
}

/// A straight piece of line, in some drawing's own units (a bar, an end cap,
/// one dash of an outline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bar {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The outline of a box as dashes, `thick` wide.
pub(crate) fn dashes(x: f32, y: f32, w: f32, h: f32, thick: f32, dash: f32, gap: f32) -> Vec<Bar> {
    let mut out = Vec::new();
    let mut run = |len: f32, mut put: Box<dyn FnMut(f32, f32) -> Bar>| {
        let mut at = 0.0;
        while at < len {
            let d = dash.min(len - at);
            out.push(put(at, d));
            at += dash + gap;
        }
    };
    run(w, Box::new(move |at, d| Bar { x: x + at, y, w: d, h: thick }));
    run(w, Box::new(move |at, d| Bar { x: x + at, y: y + h - thick, w: d, h: thick }));
    run(h, Box::new(move |at, d| Bar { x, y: y + at, w: thick, h: d }));
    run(h, Box::new(move |at, d| Bar { x: x + w - thick, y: y + at, w: thick, h: d }));
    out
}

/// How the n-th of `count` shapes is drawn: the newest is always solid; with
/// two the first is dashed; with three the first is dashed and see-through,
/// the second dashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    Solid,
    Dashed,
    Faint,
}

pub(crate) fn style(index: usize, count: usize) -> Style {
    if index + 1 >= count {
        Style::Solid
    } else if count >= 3 && index == 0 {
        Style::Faint
    } else {
        Style::Dashed
    }
}

/// One shape of the preview on the ruler's page, in the preview's pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PreviewShape {
    /// the lines (a cross, one line, or a box's outline as one solid rectangle)
    pub bars: Vec<Bar>,
    /// a box to fill and outline (Bounds), else None
    pub frame: Option<Bar>,
    pub style: Style,
    /// where its label sits (its center x, its top y), and the text
    pub label_x: f32,
    pub label_y: f32,
    pub label: String,
}

const CAP: f32 = 14.0;
const LINE: f32 = 2.0;

/// The preview: one shape fills a 236 × 99 box (its label in the corner);
/// several share one scale, side by side in a 546 × 150 box, each label under
/// its shape. Very thin or tiny shapes keep a smallest visible size.
pub(crate) fn preview(mode: Mode, shapes: &[Size]) -> (f32, f32, Vec<PreviewShape>) {
    if shapes.len() <= 1 {
        let s = shapes.first().copied().unwrap_or(Size { w: 0, h: 0 });
        let (pw, ph) = (236.0, 99.0);
        let (aw, ah) = (pw - 60.0, ph - 40.0);
        let scale = fit_scale(&[s], mode, aw, ah);
        let (w, h) = (draw_len(s.w, scale, mode != Mode::Vertical), draw_len(s.h, scale, mode != Mode::Horizontal));
        let (cx, cy) = (pw / 2.0 - 4.0, ph / 2.0 - 8.0);
        let shape = shape_at(mode, cx, cy, w, h, Style::Solid, numbers(mode, s), pw - 6.0, ph - 6.0);
        return (pw, ph, vec![shape]);
    }
    let (pw, ph) = (546.0, 150.0);
    let col = pw / shapes.len() as f32;
    let (aw, ah) = (col - 40.0, 100.0);
    let scale = fit_scale(shapes, mode, aw, ah);
    let out = shapes
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (w, h) = (draw_len(s.w, scale, true), draw_len(s.h, scale, true));
            let cx = col * i as f32 + col / 2.0;
            shape_at(mode, cx, 12.0 + 50.0, w, h, style(i, shapes.len()), numbers(mode, *s), cx, 120.0)
        })
        .collect();
    (pw, ph, out)
}

fn fit_scale(shapes: &[Size], mode: Mode, aw: f32, ah: f32) -> f32 {
    let mw = shapes.iter().map(|s| s.w).max().unwrap_or(0).max(1) as f32;
    let mh = shapes.iter().map(|s| s.h).max().unwrap_or(0).max(1) as f32;
    match mode {
        Mode::Horizontal => aw / mw,
        Mode::Vertical => ah / mh,
        _ => (aw / mw).min(ah / mh),
    }
}

/// A side drawn at `scale`, never thinner than can be seen.
fn draw_len(px: u32, scale: f32, shown: bool) -> f32 {
    if shown {
        (px as f32 * scale).max(6.0)
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
fn shape_at(mode: Mode, cx: f32, cy: f32, w: f32, h: f32, style: Style, label: String, label_x: f32, label_y: f32) -> PreviewShape {
    let (x0, y0) = (cx - w / 2.0, cy - h / 2.0);
    let mut bars = Vec::new();
    let mut frame = None;
    match mode {
        Mode::Bounds => frame = Some(Bar { x: x0, y: y0, w, h }),
        Mode::Spacing | Mode::Horizontal | Mode::Vertical => {
            if mode != Mode::Vertical {
                bars.push(Bar { x: x0, y: cy - LINE / 2.0, w, h: LINE });
                bars.push(Bar { x: x0 - LINE / 2.0, y: cy - CAP / 2.0, w: LINE, h: CAP });
                bars.push(Bar { x: x0 + w - LINE / 2.0, y: cy - CAP / 2.0, w: LINE, h: CAP });
            }
            if mode != Mode::Horizontal {
                bars.push(Bar { x: cx - LINE / 2.0, y: y0, w: LINE, h });
                bars.push(Bar { x: cx - CAP / 2.0, y: y0 - LINE / 2.0, w: CAP, h: LINE });
                bars.push(Bar { x: cx - CAP / 2.0, y: y0 + h - LINE / 2.0, w: CAP, h: LINE });
            }
        }
    }
    PreviewShape { bars, frame, style, label_x, label_y, label }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An image of `w` x `h` filled with `bg`, with a rectangle of `fg`.
    fn image(w: u32, h: u32, bg: Rgb, rect: (u32, u32, u32, u32), fg: Rgb) -> Vec<u8> {
        let mut px = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let inside = x >= rect.0 && x < rect.0 + rect.2 && y >= rect.1 && y < rect.1 + rect.3;
                let c = if inside { fg } else { bg };
                px.extend([c.0, c.1, c.2, 255]);
            }
        }
        px
    }

    #[test]
    fn the_edges_are_where_the_color_changes() {
        // a 100 x 40 gray box at (50, 30) on white
        let img = image(300, 200, (255, 255, 255), (50, 30, 100, 40), (128, 128, 128));
        assert_eq!(reach(&img, 300, 200, (80, 45), 30, all), (50, 149, 30, 69));
        // from the white around it: up to the box, and the image's edges
        assert_eq!(reach(&img, 300, 200, (10, 45), 30, all), (0, 49, 0, 199));
        // a color close enough counts as the same surface (tolerance), until it doesn't
        let soft = image(300, 200, (200, 200, 200), (100, 0, 50, 200), (215, 200, 200));
        assert_eq!(reach(&soft, 300, 200, (10, 10), 30, all).1, 299);
        assert_eq!(reach(&soft, 300, 200, (10, 10), 10, all).1, 99);
    }

    fn all(_: i32, _: i32) -> bool {
        true
    }

    #[test]
    fn a_measure_stays_on_its_screen() {
        // two screens side by side, 100 and 200 wide, all white: the lines stop at the screen's edge
        let img = image(300, 200, (255, 255, 255), (0, 0, 0, 0), (0, 0, 0));
        let left = |x: i32, _: i32| x < 100;
        let right = |x: i32, _: i32| x >= 100;
        assert_eq!(reach(&img, 300, 200, (50, 50), 30, left), (0, 99, 0, 199));
        assert_eq!(reach(&img, 300, 200, (150, 50), 30, right), (100, 299, 0, 199));
        // across screens: on to the other one
        assert_eq!(reach(&img, 300, 200, (50, 50), 30, all), (0, 299, 0, 199));
        // a gap between screens of different sizes isn't a screen: it stops there
        // (the left screen is only 150 high, so below it is a gap)
        let not_gap = |x: i32, y: i32| x >= 100 || y < 150;
        assert_eq!(reach(&img, 300, 200, (150, 180), 30, not_gap).0, 100);
    }

    #[test]
    fn three_shapes_at_most() {
        // 2 kept and the one being drawn: a 3rd Shift-click is refused ("3 is the most")
        assert!(room_to_keep(0));
        assert!(room_to_keep(1));
        assert!(!room_to_keep(2));
    }

    #[test]
    fn a_measure_is_never_bigger_than_the_screens() {
        let screens = [(1920, 1080), (3840, 2160)];
        assert_eq!(most(&screens, 0, false), (1920, 1080));
        assert_eq!(most(&screens, 1, false), (3840, 2160));
        // across screens: never more than both added up, either way (side by side or stacked)
        assert_eq!(most(&screens, 0, true), (5760, 3240));
        assert_eq!(capped(Size { w: 1921, h: 1081 }, most(&screens, 0, false)), Size { w: 1920, h: 1080 });
        assert_eq!(capped(Size { w: 2500, h: 1500 }, most(&screens, 0, true)), Size { w: 2500, h: 1500 });
        assert_eq!(capped(Size { w: 6000, h: 4000 }, most(&screens, 1, true)), Size { w: 5760, h: 3240 });
    }

    #[test]
    fn sizes_as_text_in_every_unit() {
        let s = Size { w: 360, h: 94 };
        assert_eq!(text(Mode::Spacing, s, Unit::Px, None), "360 × 94 px");
        assert_eq!(text(Mode::Horizontal, s, Unit::Px, None), "360 px");
        assert_eq!(text(Mode::Vertical, s, Unit::Px, None), "94 px");
        // a screen without its size: px whatever the setting
        assert_eq!(text(Mode::Spacing, s, Unit::Cm, None), "360 × 94 px");
        // 0.264 mm per pixel (a 96 dpi screen)
        assert_eq!(text(Mode::Spacing, s, Unit::Cm, Some(0.264)), "9.5 × 2.5 cm");
        assert_eq!(text(Mode::Spacing, s, Unit::In, Some(0.264)), "3.74 × 0.98 in");
        assert_eq!(physical_line(Mode::Spacing, s, Some(0.264)), "≈ 9.5 × 2.5 cm on this screen");
        assert_eq!(physical_line(Mode::Spacing, s, None), "");
        let shapes = [Size { w: 300, h: 150 }, Size { w: 200, h: 200 }, Size { w: 120, h: 60 }];
        assert_eq!(text_all(Mode::Bounds, &shapes, Unit::Px, None), "300 × 150, 200 × 200, 120 × 60 px");
        assert_eq!(text_all(Mode::Bounds, &shapes[..1], Unit::Cm, Some(0.264)), "7.9 × 4.0 cm");
    }

    #[test]
    fn the_lines_take_the_last_bright_pick() {
        // the examples on the mock-up
        assert!(is_bright((0xE0, 0x40, 0x30)));
        assert!(is_bright((0x20, 0xA0, 0x60)));
        assert!(!is_bright((0x1F, 0x23, 0x28)));
        assert!(!is_bright((0xEE, 0xF1, 0xF4)));
        assert_eq!(line_color(&[(0x1F, 0x23, 0x28), (0x20, 0xA0, 0x60), (0xE0, 0x40, 0x30)]), (0x20, 0xA0, 0x60));
        assert_eq!(line_color(&[(0x1F, 0x23, 0x28)]), DEFAULT_LINE);
    }

    #[test]
    fn the_icon_has_a_tooth_per_cm_between_4_and_7() {
        // estimated at 96 dpi: 38 px is about 1 cm
        assert_eq!(teeth(30, None), 4);
        assert_eq!(teeth(190, None), 5);
        assert_eq!(teeth(227, None), 6);
        assert_eq!(teeth(265, None), 7);
        assert_eq!(teeth(950, None), 7);
        // a screen that reports its size: 0.5 mm per pixel, 120 px = 6 cm
        assert_eq!(teeth(120, Some(0.5)), 6);
        // big, small, big... : 4 teeth is the icon as drawn (3, 2, 3, 2 long)
        let svg = icon_svg(4, 1.4);
        assert!(svg.contains("M4.50 5.5v3M7.50 5.5v2M10.50 5.5v3M13.50 5.5v2"), "{svg}");
    }

    #[test]
    fn several_shapes_are_drawn_in_order() {
        assert_eq!(style(0, 1), Style::Solid);
        assert_eq!((style(0, 2), style(1, 2)), (Style::Dashed, Style::Solid));
        assert_eq!((style(0, 3), style(1, 3), style(2, 3)), (Style::Faint, Style::Dashed, Style::Solid));
        // one shape: the small preview; several: the wide one, one scale, labels under each
        let (w, h, one) = preview(Mode::Bounds, &[Size { w: 300, h: 150 }]);
        assert_eq!((w, h, one.len()), (236.0, 99.0, 1));
        let f = one[0].frame.unwrap();
        assert!((f.w / f.h - 2.0).abs() < 0.01, "to scale: {f:?}");
        let (w, _, three) = preview(Mode::Bounds, &[Size { w: 300, h: 150 }, Size { w: 200, h: 200 }, Size { w: 120, h: 60 }]);
        assert_eq!((w, three.len()), (546.0, 3));
        let (a, b) = (three[0].frame.unwrap(), three[2].frame.unwrap());
        assert!((a.w / b.w - 2.5).abs() < 0.01, "one scale for all: {a:?} {b:?}");
        assert!(three.iter().all(|s| s.label_y > s.frame.unwrap().y + s.frame.unwrap().h));
        // a 3 x 400 px line still shows
        let (_, _, thin) = preview(Mode::Bounds, &[Size { w: 3, h: 400 }]);
        assert!(thin[0].frame.unwrap().w >= 6.0);
    }

    #[test]
    fn dashes_go_round_the_box() {
        let d = dashes(0.0, 0.0, 20.0, 10.0, 2.0, 6.0, 4.0);
        // top and bottom: 20 / (6 + 4) = 2 each, the sides: 1 each... rounded up
        assert_eq!(d.iter().filter(|b| b.h == 2.0).count(), 4);
        assert!(d.iter().all(|b| b.x >= 0.0 && b.x + b.w <= 20.0 && b.y >= 0.0 && b.y + b.h <= 10.0));
    }
}
