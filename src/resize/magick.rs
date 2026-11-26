//! What Image Resizer asks ImageMagick to do: the command for one picture,
//! where the new picture goes, and how big it comes out. ImageMagick 7 is
//! `magick`, 6 (Debian, Ubuntu) is `convert` and `identify`.

use crate::launch::outside_command;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Unit {
    #[default]
    Px,
    Percent,
    Cm,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Mode {
    /// all of it, inside the size
    #[default]
    Fit,
    /// fills the size, edges cut
    Fill,
    /// fills the size, squashed
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Format {
    #[default]
    Keep,
    Png,
    Jpg,
    Webp,
}

/// The size asked for. 0 for width or height = "whatever keeps the shape".
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f32,
    pub h: f32,
    pub unit: Unit,
    pub mode: Mode,
    /// the proportions lock (only the Custom window uses it)
    #[serde(default)]
    pub lock: bool,
}

/// What to do to every picture of a job. Saved as a preset (without where to save).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Spec {
    /// None = the same size
    pub size: Option<Size>,
    pub format: Format,
    /// JPG and WebP; None = what ImageMagick keeps or picks
    pub quality: Option<u8>,
    /// "Make smaller"
    #[serde(default)]
    pub smaller: bool,
    /// remove the metadata (GPS, camera, date, program, preview copy)
    #[serde(default)]
    pub strip: bool,
}

/// Where the new pictures go.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Dest {
    /// next to the original, with -resized added
    #[default]
    Beside,
    Folder(PathBuf),
    Replace,
}

/// "Make smaller" brings bigger pictures down to this long side.
pub const SMALLER_SIDE: u32 = 2560;
pub const SMALLER_QUALITY: u8 = 80;

/// What `identify` says about a picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    /// ImageMagick's name for the format: "JPEG", "PNG", "WEBP", "GIF"
    pub format: String,
    pub w: u32,
    pub h: u32,
    /// pixels per inch, when the picture says
    pub ppi: Option<f32>,
    /// it has EXIF, XMP or IPTC
    pub metadata: bool,
    /// as the picture is shown (EXIF turned it a quarter)
    pub turned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    /// ImageMagick 7
    Magick,
    /// ImageMagick 6
    Convert,
}

impl Tool {
    fn command(self, identify: bool) -> Command {
        match (self, identify) {
            (Tool::Magick, false) => outside_command("magick"),
            (Tool::Magick, true) => {
                let mut c = outside_command("magick");
                c.arg("identify");
                c
            }
            (Tool::Convert, false) => outside_command("convert"),
            (Tool::Convert, true) => outside_command("identify"),
        }
    }
}

fn runs(program: &str) -> bool {
    outside_command(program)
        .arg("-version")
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("ImageMagick"))
        .unwrap_or(false)
}

/// The installed ImageMagick, looked for again each time (it may have just been installed).
pub fn find_tool() -> Option<Tool> {
    if runs("magick") {
        Some(Tool::Magick)
    } else if runs("convert") {
        Some(Tool::Convert)
    } else {
        None
    }
}

/// `find_tool`, asked once per run of a job.
pub fn tool() -> Option<Tool> {
    static TOOL: OnceLock<Option<Tool>> = OnceLock::new();
    *TOOL.get_or_init(find_tool)
}

/// A name ImageMagick would misread ("pic[1].jpg" means its frame 1) goes in through stdin.
fn needs_stdin(path: &Path) -> bool {
    path.to_string_lossy().contains(['[', ']'])
}

/// Asks `identify` about a picture. Err: not a picture ImageMagick can read.
pub fn probe(tool: Tool, path: &Path) -> Result<Info, String> {
    let mut cmd = tool.command(true);
    cmd.args(["-format", "%m|%w|%h|%x|%U|%[orientation]|%[profiles]\\n"]);
    let stdin = needs_stdin(path);
    if stdin {
        cmd.arg("-");
        cmd.stdin(std::fs::File::open(path).map_err(|e| e.to_string())?);
    } else {
        cmd.arg(path);
    }
    let out = cmd.output().map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&out.stdout);
    // IM 6 warns about %[profiles] on a picture with none, and still answers
    parse_probe(text.lines().next().unwrap_or("")).ok_or_else(|| "not a picture".to_string())
}

pub fn parse_probe(line: &str) -> Option<Info> {
    let mut parts = line.trim().split('|');
    let format = parts.next()?.trim().to_string();
    let w: u32 = parts.next()?.trim().parse().ok()?;
    let h: u32 = parts.next()?.trim().parse().ok()?;
    if format.is_empty() || w == 0 || h == 0 {
        return None;
    }
    let res: f32 = parts.next().and_then(|r| r.split_whitespace().next()?.parse().ok()).unwrap_or(0.0);
    let units = parts.next().unwrap_or("");
    let ppi = match units {
        u if u.contains("PixelsPerCentimeter") && res > 0.0 => Some(res * 2.54),
        u if u.contains("PixelsPerInch") && res > 0.0 => Some(res),
        _ => None,
    };
    let orientation = parts.next().unwrap_or("");
    // RightTop, LeftBottom, ...: shown turned a quarter, so width and height swap
    let turned = matches!(orientation, "RightTop" | "LeftTop" | "RightBottom" | "LeftBottom");
    let profiles = parts.next().unwrap_or("").to_ascii_lowercase();
    let metadata = ["exif", "xmp", "iptc", "8bim"].iter().any(|p| profiles.contains(p));
    let (w, h) = if turned { (h, w) } else { (w, h) };
    Some(Info { format, w, h, ppi, metadata, turned })
}

/// Pixels per inch when the picture doesn't say. shortcut: one fixed value, a setting if people ask.
pub const DEFAULT_PPI: f32 = 96.0;

/// The size in pixels a value in `unit` means for this picture (`along` = its pixels that way).
pub fn to_px(value: f32, unit: Unit, along: u32, ppi: Option<f32>) -> f32 {
    match unit {
        Unit::Px => value,
        Unit::Percent => value / 100.0 * along as f32,
        Unit::Cm => value / 2.54 * ppi.unwrap_or(DEFAULT_PPI),
        Unit::In => value * ppi.unwrap_or(DEFAULT_PPI),
    }
}

/// The picture's size in `unit` (what the Custom fields show for it).
pub fn from_px(px: u32, unit: Unit, along: u32, ppi: Option<f32>) -> f32 {
    match unit {
        Unit::Px => px as f32,
        Unit::Percent => px as f32 / along.max(1) as f32 * 100.0,
        Unit::Cm => px as f32 / ppi.unwrap_or(DEFAULT_PPI) * 2.54,
        Unit::In => px as f32 / ppi.unwrap_or(DEFAULT_PPI),
    }
}

/// The size box in whole pixels for this picture: (w, h), 0 = keep the shape.
fn box_px(size: &Size, info: &Info) -> (u32, u32) {
    let w = to_px(size.w, size.unit, info.w, info.ppi).round().max(0.0) as u32;
    let h = to_px(size.h, size.unit, info.h, info.ppi).round().max(0.0) as u32;
    (w, h)
}

/// How big the new picture comes out.
pub fn out_size(spec: &Spec, info: &Info) -> (u32, u32) {
    let (w0, h0) = (info.w as f64, info.h as f64);
    let fit = |bw: f64, bh: f64| -> (u32, u32) {
        let scale = match (bw > 0.0, bh > 0.0) {
            (true, true) => (bw / w0).min(bh / h0),
            (true, false) => bw / w0,
            (false, true) => bh / h0,
            (false, false) => 1.0,
        };
        (((w0 * scale).round() as u32).max(1), ((h0 * scale).round() as u32).max(1))
    };
    let (w, h) = match &spec.size {
        None => (info.w, info.h),
        Some(size) => {
            let (bw, bh) = box_px(size, info);
            match size.mode {
                _ if bw == 0 || bh == 0 => fit(bw as f64, bh as f64),
                Mode::Fit => fit(bw as f64, bh as f64),
                Mode::Fill | Mode::Stretch => (bw.max(1), bh.max(1)),
            }
        }
    };
    if spec.smaller && w.max(h) > SMALLER_SIDE {
        let s = SMALLER_SIDE as f64 / w.max(h) as f64;
        return (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1));
    }
    (w, h)
}

/// The format the new picture is written in, as ImageMagick names it, and its file extension.
pub fn out_format(spec: &Spec, info: &Info, input: &Path) -> (String, String) {
    match spec.format {
        Format::Png => ("PNG".into(), "png".into()),
        Format::Jpg => ("JPEG".into(), "jpg".into()),
        Format::Webp => ("WEBP".into(), "webp".into()),
        Format::Keep => {
            let ext = input.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| info.format.to_ascii_lowercase());
            (info.format.clone(), ext)
        }
    }
}

fn takes_quality(format: &str) -> bool {
    matches!(format, "JPEG" | "JPG" | "WEBP")
}

/// The quality the new picture is saved at (JPG and WebP only).
pub fn out_quality(spec: &Spec, format: &str) -> Option<u8> {
    if !takes_quality(format) {
        return None;
    }
    spec.quality.or(spec.smaller.then_some(SMALLER_QUALITY))
}

/// ImageMagick's arguments between the input and the output.
pub fn operations(spec: &Spec, info: &Info, format: &str) -> Vec<String> {
    let mut a: Vec<String> = vec!["-auto-orient".into()];
    if spec.strip {
        // keeps the color profile (ICC): without it colors can shift
        a.extend(["+profile".into(), "!icc,*".into()]);
    }
    let (w, h) = out_size(spec, info);
    if (w, h) != (info.w, info.h) {
        a.extend(["-filter".into(), "Lanczos".into()]);
        let fill = spec.size.as_ref().is_some_and(|s| s.mode == Mode::Fill && s.w > 0.0 && s.h > 0.0);
        if fill && !spec.smaller {
            // cover the box, then cut what sticks out, from the middle
            let (bw, bh) = (w, h);
            let scale = (bw as f64 / info.w as f64).max(bh as f64 / info.h as f64);
            let (cw, ch) = (((info.w as f64 * scale).ceil() as u32).max(bw), ((info.h as f64 * scale).ceil() as u32).max(bh));
            a.extend(["-resize".into(), format!("{cw}x{ch}!")]);
            a.extend(["-gravity".into(), "center".into(), "-extent".into(), format!("{bw}x{bh}")]);
        } else {
            a.extend(["-resize".into(), format!("{w}x{h}!")]);
        }
    }
    if format == "JPEG" {
        // JPG has no see-through parts: white behind them, not black
        a.extend(["-background".into(), "white".into(), "-alpha".into(), "remove".into(), "-alpha".into(), "off".into()]);
    }
    if let Some(q) = out_quality(spec, format) {
        a.extend(["-quality".into(), q.to_string()]);
    }
    if spec.smaller && format == "PNG" {
        a.extend(["-define".into(), "png:compression-level=9".into()]);
    }
    a
}

/// `stem-resized.ext`, or `stem-resized-2.ext` when that's taken.
fn free_name(dir: &Path, stem: &str, suffix: &str, ext: &str, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let mut n = 1;
    loop {
        let name = if n == 1 { format!("{stem}{suffix}.{ext}") } else { format!("{stem}{suffix}-{n}.{ext}") };
        let path = dir.join(name);
        if !taken(&path) {
            return path;
        }
        n += 1;
    }
}

/// Where the new picture goes. Never over another file, except the original with Replace.
pub fn out_path(dest: &Dest, input: &Path, ext: &str, taken: &dyn Fn(&Path) -> bool) -> PathBuf {
    let dir = input.parent().unwrap_or(Path::new("."));
    let stem = input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "picture".into());
    match dest {
        Dest::Beside => free_name(dir, &stem, "-resized", ext, taken),
        Dest::Folder(folder) => free_name(folder, &stem, "", ext, &|p| p == input || taken(p)),
        Dest::Replace => {
            let same = input.with_extension(ext);
            if same == input || !taken(&same) {
                same
            } else {
                // a different file already has the new name: don't overwrite it
                free_name(dir, &stem, "", ext, taken)
            }
        }
    }
}

/// Makes the new picture. Written to a hidden file first, then renamed, so
/// a stop or a failure never leaves half a picture (or half an original).
pub fn convert(tool: Tool, spec: &Spec, info: &Info, input: &Path, output: &Path, format: &str) -> Result<(), String> {
    let dir = output.parent().unwrap_or(Path::new("."));
    let temp = dir.join(format!(".{}.archtoys-part", output.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
    let mut cmd = tool.command(false);
    if needs_stdin(input) {
        cmd.arg("-");
        cmd.stdin(std::fs::File::open(input).map_err(|e| e.to_string())?);
    } else {
        cmd.arg(input);
    }
    cmd.args(operations(spec, info, format));
    // the format named in front: the hidden name's extension says nothing
    cmd.arg(format!("{format}:{}", temp.display()));
    let out = cmd.output().map_err(|e| format!("ImageMagick didn't start: {e}"))?;
    if !out.status.success() || !temp.exists() {
        let _ = std::fs::remove_file(&temp);
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("ImageMagick couldn't save it");
        return Err(last.to_string());
    }
    std::fs::rename(&temp, output).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("couldn't save it: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo() -> Info {
        Info { format: "JPEG".into(), w: 1600, h: 900, ppi: Some(72.0), metadata: true, turned: false }
    }

    fn sized(w: f32, h: f32, unit: Unit, mode: Mode) -> Spec {
        Spec { size: Some(Size { w, h, unit, mode, lock: false }), ..Default::default() }
    }

    #[test]
    fn identify_s_line_is_read() {
        let info = parse_probe("JPEG|4000|3000|72 PixelsPerInch|PixelsPerInch|RightTop|exif,xmp").unwrap();
        assert_eq!((info.w, info.h, info.turned, info.metadata), (3000, 4000, true, true));
        assert_eq!(info.ppi, Some(72.0));
        let png = parse_probe("PNG|800|600|0|Undefined|Undefined|").unwrap();
        assert_eq!((png.metadata, png.ppi), (false, None));
        let cm = parse_probe("PNG|800|600|28.35|PixelsPerCentimeter|TopLeft|").unwrap();
        assert!((cm.ppi.unwrap() - 72.0).abs() < 0.1);
        assert!(parse_probe("").is_none());
        assert!(parse_probe("TXT|0|0").is_none());
    }

    #[test]
    fn the_new_size_follows_fit_fill_and_stretch() {
        // the plan's example: 1600 x 900 into 1000 x 1000
        assert_eq!(out_size(&sized(1000.0, 1000.0, Unit::Px, Mode::Fit), &photo()), (1000, 563));
        assert_eq!(out_size(&sized(1000.0, 1000.0, Unit::Px, Mode::Fill), &photo()), (1000, 1000));
        assert_eq!(out_size(&sized(1000.0, 1000.0, Unit::Px, Mode::Stretch), &photo()), (1000, 1000));
        // fit in 1920 makes a smaller picture bigger (no "never bigger" option)
        assert_eq!(out_size(&sized(1920.0, 1920.0, Unit::Px, Mode::Fit), &photo()), (1920, 1080));
        assert_eq!(out_size(&sized(50.0, 50.0, Unit::Percent, Mode::Fit), &photo()), (800, 450));
        // one side empty keeps the shape
        assert_eq!(out_size(&sized(800.0, 0.0, Unit::Px, Mode::Stretch), &photo()), (800, 450));
        // 2.54 cm at 72 ppi = 72 px
        assert_eq!(out_size(&sized(2.54, 0.0, Unit::Cm, Mode::Fit), &photo()), (72, 41));
        assert_eq!(out_size(&Spec::default(), &photo()), (1600, 900));
    }

    #[test]
    fn make_smaller_caps_the_long_side_and_the_quality() {
        let big = Info { w: 6000, h: 4000, ..photo() };
        let spec = Spec { smaller: true, ..Default::default() };
        assert_eq!(out_size(&spec, &big), (2560, 1707));
        assert_eq!(out_size(&spec, &photo()), (1600, 900));
        assert_eq!(out_quality(&spec, "JPEG"), Some(80));
        assert_eq!(out_quality(&spec, "PNG"), None);
        assert!(operations(&spec, &photo(), "PNG").contains(&"png:compression-level=9".to_string()));
    }

    #[test]
    fn the_command_orients_resizes_and_keeps_the_color_profile() {
        let mut spec = sized(50.0, 50.0, Unit::Percent, Mode::Fit);
        spec.strip = true;
        let ops = operations(&spec, &photo(), "JPEG").join(" ");
        assert!(ops.starts_with("-auto-orient +profile !icc,* -filter Lanczos -resize 800x450!"), "{ops}");
        assert!(ops.contains("-alpha remove"));
        // fill covers the box, then cuts from the middle
        let fill = operations(&sized(1000.0, 1000.0, Unit::Px, Mode::Fill), &photo(), "PNG").join(" ");
        assert!(fill.contains("-resize 1778x1000! -gravity center -extent 1000x1000"), "{fill}");
        // the same size and format: nothing resized
        assert_eq!(operations(&Spec::default(), &photo(), "PNG"), vec!["-auto-orient"]);
    }

    #[test]
    fn new_pictures_never_overwrite_another_file() {
        let input = Path::new("/p/beach.jpg");
        let none = |_: &Path| false;
        assert_eq!(out_path(&Dest::Beside, input, "jpg", &none), Path::new("/p/beach-resized.jpg"));
        let first_taken = |p: &Path| p == Path::new("/p/beach-resized.jpg");
        assert_eq!(out_path(&Dest::Beside, input, "jpg", &first_taken), Path::new("/p/beach-resized-2.jpg"));
        assert_eq!(out_path(&Dest::Replace, input, "jpg", &|_| true), Path::new("/p/beach.jpg"));
        assert_eq!(out_path(&Dest::Replace, input, "png", &none), Path::new("/p/beach.png"));
        // beach.png is another picture: not overwritten
        let png_taken = |p: &Path| p == Path::new("/p/beach.png");
        assert_eq!(out_path(&Dest::Replace, input, "png", &png_taken), Path::new("/p/beach-2.png"));
        let folder = Dest::Folder(PathBuf::from("/out"));
        assert_eq!(out_path(&folder, input, "webp", &none), Path::new("/out/beach.webp"));
        // the chosen folder is the picture's own: never the original
        let same = Dest::Folder(PathBuf::from("/p"));
        assert_eq!(out_path(&same, input, "jpg", &none), Path::new("/p/beach-2.jpg"));
    }

    #[test]
    fn units_go_both_ways() {
        for unit in [Unit::Px, Unit::Percent, Unit::Cm, Unit::In] {
            let v = from_px(800, unit, 1600, Some(300.0));
            assert!((to_px(v, unit, 1600, Some(300.0)) - 800.0).abs() < 0.01, "{unit:?}");
        }
    }
}
