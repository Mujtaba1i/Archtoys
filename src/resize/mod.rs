//! Image Resizer (v0.9.0), a Micro-toy: resize and convert pictures from the
//! file manager's right-click menu (`archtoys --resize ACTION FILES`, which
//! a running copy gets over D-Bus). Turn on installs ImageMagick (with the
//! password, like Text Extractor's Tesseract) and adds the menus.

pub mod magick;
pub mod menus;

use crate::{AppWindow, ResizeCustom, ResizePanel, ResizeProgress, Resizer};
use magick::{Dest, Format, Info, Mode, Size, Spec, Tool, Unit};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A saved Custom... setup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub spec: Spec,
}

thread_local! {
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    static PERSIST: RefCell<Option<Rc<dyn Fn()>>> = const { RefCell::new(None) };
    static PRESETS: RefCell<Vec<Preset>> = const { RefCell::new(Vec::new()) };
    static PANEL: RefCell<Option<(ResizePanel, Vec<PathBuf>)>> = const { RefCell::new(None) };
    static CUSTOM: RefCell<Option<Custom>> = const { RefCell::new(None) };
    static JOBS: RefCell<Vec<ResizeProgress>> = const { RefCell::new(Vec::new()) };
    static CANCEL: RefCell<Option<Arc<AtomicBool>>> = const { RefCell::new(None) };
}

fn main_ui() -> Option<AppWindow> {
    UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade()))
}

fn persist() {
    if let Some(p) = PERSIST.with(|p| p.borrow().clone()) {
        p();
    }
}

pub fn presets() -> Vec<Preset> {
    PRESETS.with(|p| p.borrow().clone())
}

pub fn set_presets(list: Vec<Preset>) {
    PRESETS.with(|p| *p.borrow_mut() = list);
}

fn preset_names() -> Vec<String> {
    PRESETS.with(|p| p.borrow().iter().map(|p| p.name.clone()).collect())
}

fn strings(list: &[String]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(list.iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// Gives a window the main window's theme and colors (each window has its own `Skin`).
macro_rules! copy_skin {
    ($window:expr) => {
        if let Some(ui) = main_ui() {
            let from = ui.global::<crate::Skin>();
            let to = $window.global::<crate::Skin>();
            to.set_theme(from.get_theme());
            to.set_dark_t(from.get_dark_t());
            to.set_aurora(from.get_aurora());
            to.set_real_shadows(!crate::cpu_drawn());
        }
    };
}

/// The quick actions (see `menus::QUICK`) as what they do.
pub fn quick_spec(id: &str) -> Option<Spec> {
    let percent = |p: f32| Spec { size: Some(Size { w: p, h: p, unit: Unit::Percent, mode: Mode::Fit, lock: true }), ..Default::default() };
    let fit = |px: f32| Spec { size: Some(Size { w: px, h: px, unit: Unit::Px, mode: Mode::Fit, lock: false }), ..Default::default() };
    let to = |format: Format| Spec { format, ..Default::default() };
    Some(match id {
        "r25" => percent(25.0),
        "r50" => percent(50.0),
        "r150" => percent(150.0),
        "r200" => percent(200.0),
        "fit1920" => fit(1920.0),
        "fit1280" => fit(1280.0),
        "png" => to(Format::Png),
        "jpg" => to(Format::Jpg),
        "webp" => to(Format::Webp),
        "smaller" => Spec { smaller: true, ..Default::default() },
        _ => {
            let i: usize = id.strip_prefix("preset:")?.parse().ok()?;
            return PRESETS.with(|p| p.borrow().get(i).map(|p| p.spec.clone()));
        }
    })
}

// ---- Home: the switch and Turn on ----

pub fn install(ui: &AppWindow, persist_fn: impl Fn() + 'static) {
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
    PERSIST.with(|p| *p.borrow_mut() = Some(Rc::new(persist_fn)));
    let g = ui.global::<Resizer>();
    g.on_toggled(toggled);
    g.on_turn_on(turn_on);
    g.on_cancel_install(|| {
        CANCEL.with(|c| {
            if let Some(cancel) = c.borrow().as_ref() {
                cancel.store(true, Ordering::SeqCst);
            }
        })
    });
}

/// After the config is read: when it's on, write the menus again (the
/// program's path may have changed with an update or a move).
pub fn refresh_menus() {
    let on = main_ui().is_some_and(|ui| ui.global::<Resizer>().get_on());
    if on {
        let names = preset_names();
        std::thread::spawn(move || {
            if let Err(err) = menus::install(&names) {
                eprintln!("image resizer: couldn't update the file manager menus: {err}");
            }
        });
    }
}

fn toggled(on: bool) {
    let Some(ui) = main_ui() else { return };
    let g = ui.global::<Resizer>();
    if !on {
        menus::remove();
        g.set_on(false);
        persist();
        return;
    }
    // the switch stays off until it's really on
    g.set_on(false);
    if magick::find_tool().is_some() {
        enable();
    } else {
        g.set_dialog(1);
    }
}

fn enable() {
    let Some(ui) = main_ui() else { return };
    let g = ui.global::<Resizer>();
    match menus::install(&preset_names()) {
        Ok(_) => {
            g.set_on(true);
            g.set_dialog(0);
            persist();
        }
        Err(err) => {
            g.set_message(format!("Image Resizer couldn't be added to the right-click menu: {err}").into());
            g.set_dialog(3);
        }
    }
}

fn turn_on() {
    let Some(ui) = main_ui() else { return };
    let g = ui.global::<Resizer>();
    if magick::find_tool().is_some() {
        enable();
        return;
    }
    let distro = crate::ocr::install::detect();
    let Some(script) = crate::ocr::install::imagemagick_script(&distro) else {
        g.set_message(
            format!(
                "Archtoys can't install software on {}. Install ImageMagick from your software center, then turn Image Resizer on again.",
                distro.name()
            )
            .into(),
        );
        g.set_dialog(3);
        return;
    };
    let cancel = Arc::new(AtomicBool::new(false));
    CANCEL.with(|c| *c.borrow_mut() = Some(cancel.clone()));
    g.set_stage("Waiting for your password…".into());
    g.set_stats("".into());
    g.set_fraction(-1.0);
    g.set_dialog(2);
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let progress = weak.clone();
        let result = crate::ocr::install::run_script(&distro, &script, cancel, move |p| {
            let _ = progress.upgrade_in_event_loop(move |ui| {
                let g = ui.global::<Resizer>();
                g.set_fraction(p.fraction.unwrap_or(-1.0));
                g.set_stats(p.stats.into());
                g.set_stage(p.stage.into());
            });
        });
        let found = magick::find_tool().is_some();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            CANCEL.with(|c| c.borrow_mut().take());
            let g = ui.global::<Resizer>();
            match (result, found) {
                (Ok(()), true) => enable(),
                (Ok(()), false) => {
                    g.set_message("It was installed, but Archtoys can't find ImageMagick (magick or convert).".into());
                    g.set_dialog(3);
                }
                (Err(err), _) => {
                    g.set_message(err.into());
                    g.set_dialog(3);
                }
            }
        });
    });
}

// ---- From the menu ----

/// `archtoys --resize ACTION FILES` (here or from a later launch).
pub fn start(action: &str, files: Vec<String>) {
    start_paths(action, files.into_iter().map(PathBuf::from).collect());
}

fn start_paths(action: &str, files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    match action {
        "panel" => open_panel(files),
        "custom" => open_custom(files),
        id => match quick_spec(id) {
            Some(spec) => run_job(files, spec, Dest::Beside),
            None => eprintln!("image resizer: no action called {id}"),
        },
    }
}

fn count_text(n: usize) -> String {
    if n == 1 {
        "1 picture".into()
    } else {
        format!("{n} pictures")
    }
}

/// Where the mouse is, on X11 (the panel opens there). Wayland places it itself.
fn pointer() -> Option<(i32, i32)> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return None;
    }
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;
    Some((reply.root_x as i32, reply.root_y as i32))
}

fn close_panel() {
    PANEL.with(|p| {
        if let Some((panel, _)) = p.borrow_mut().take() {
            let _ = panel.hide();
        }
    });
}

fn panel_files() -> Vec<PathBuf> {
    PANEL.with(|p| p.borrow().as_ref().map(|(_, f)| f.clone()).unwrap_or_default())
}

fn open_panel(files: Vec<PathBuf>) {
    close_panel();
    let Ok(panel) = ResizePanel::new() else { return };
    copy_skin!(panel);
    panel.set_count_text(count_text(files.len()).into());
    panel.set_presets(strings(&preset_names()));
    panel.on_closed(close_panel);
    panel.on_act(|id| {
        let files = panel_files();
        close_panel();
        start_paths(id.as_str(), files);
    });
    panel.on_act_preset(|i| {
        let files = panel_files();
        close_panel();
        start_paths(&format!("preset:{i}"), files);
    });
    crate::window::close_when_unfocused(panel.window(), Rc::new(close_panel));
    let _ = panel.show();
    if let Some((x, y)) = pointer() {
        panel.window().set_position(slint::PhysicalPosition::new((x - 30).max(0), (y - 20).max(0)));
    }
    PANEL.with(|p| *p.borrow_mut() = Some((panel, files)));
}

// ---- Custom... ----

struct Custom {
    window: ResizeCustom,
    files: Vec<PathBuf>,
    /// the first picture: the fields' starting size, the lock's shape, the units
    first: Option<Info>,
    folder: Option<PathBuf>,
    total_bytes: u64,
}

static ESTIMATE: AtomicU64 = AtomicU64::new(0);

const UNITS: [Unit; 4] = [Unit::Px, Unit::Percent, Unit::Cm, Unit::In];
const FORMATS: [Format; 4] = [Format::Keep, Format::Png, Format::Jpg, Format::Webp];
const MODES: [Mode; 3] = [Mode::Fit, Mode::Fill, Mode::Stretch];

/// A field's text: whole pixels, else up to 2 decimals without trailing zeros.
fn number_text(v: f32, unit: Unit) -> String {
    if unit == Unit::Px {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn parse_number(s: &str) -> f32 {
    s.trim().replace(',', ".").parse::<f32>().ok().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0)
}

pub fn bytes_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1_000_000.0 {
        format!("{:.1} MB", b / 1_000_000.0)
    } else {
        format!("{} KB", (b / 1000.0).round().max(1.0) as u64)
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// "beach.jpg and hills.jpg have metadata", "4 pictures have metadata".
pub fn metadata_text(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} has metadata"),
        [a, b] => format!("{a} and {b} have metadata"),
        more => format!("{} pictures have metadata", more.len()),
    }
}

fn with_custom<R>(f: impl FnOnce(&mut Custom) -> R) -> Option<R> {
    CUSTOM.with(|c| c.borrow_mut().as_mut().map(f))
}

fn spec_from(w: &ResizeCustom) -> Spec {
    let unit = UNITS[w.get_unit().clamp(0, 3) as usize];
    let (width, height) = (parse_number(&w.get_width_text()), parse_number(&w.get_height_text()));
    let size = (width > 0.0 || height > 0.0).then_some(Size {
        w: width,
        h: height,
        unit,
        mode: MODES[w.get_mode().clamp(0, 2) as usize],
        lock: w.get_lock(),
    });
    Spec {
        size,
        format: FORMATS[w.get_format().clamp(0, 3) as usize],
        quality: Some(w.get_quality().round().clamp(1.0, 100.0) as u8),
        smaller: false,
        strip: w.get_strip(),
    }
}

fn dest_from(c: &Custom) -> Dest {
    match c.window.get_dest() {
        1 => c.folder.clone().map(Dest::Folder).unwrap_or_default(),
        2 => Dest::Replace,
        _ => Dest::Beside,
    }
}

fn home_short(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The SAVE TO line, the button's words and a new estimate.
fn custom_changed() {
    with_custom(|c| {
        let w = &c.window;
        let spec = spec_from(w);
        let text = match w.get_dest() {
            1 => c.folder.as_deref().map(home_short).unwrap_or_default(),
            2 => "Replace originals: they're overwritten".to_string(),
            _ => {
                let example = c.files.first().map(|first| {
                    let ext = match &c.first {
                        Some(info) => magick::out_format(&spec, info, first).1,
                        None => first.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default(),
                    };
                    file_name(&magick::out_path(&Dest::Beside, first, &ext, &|_| false))
                });
                format!("Next to the originals ({})", example.unwrap_or_default())
            }
        };
        w.set_dest_text(text.into());
        w.set_error("".into());
    });
    estimate();
}

/// "About 9.6 MB → 2.1 MB": the first picture is made for real (in the temp
/// folder), and the others are taken to shrink or grow the same way.
fn estimate() {
    let Some((first, info, spec, total, weak)) = with_custom(|c| {
        let w = &c.window;
        w.set_estimate_to("".into());
        Some((c.files.first()?.clone(), c.first.clone()?, spec_from(w), c.total_bytes, w.as_weak()))
    })
    .flatten() else {
        return;
    };
    let id = ESTIMATE.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        // only the last change counts: a slider sends many
        std::thread::sleep(Duration::from_millis(300));
        if ESTIMATE.load(Ordering::SeqCst) != id {
            return;
        }
        let Some(tool) = magick::tool() else { return };
        let (format, ext) = magick::out_format(&spec, &info, &first);
        let out = std::env::temp_dir().join(format!("archtoys-estimate-{}.{ext}", std::process::id()));
        let made = magick::convert(tool, &spec, &info, &first, &out, &format);
        let first_in = std::fs::metadata(&first).map(|m| m.len()).unwrap_or(0).max(1);
        let first_out = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        let _ = std::fs::remove_file(&out);
        let guess = (total as f64 * first_out as f64 / first_in as f64) as u64;
        let _ = weak.upgrade_in_event_loop(move |w| {
            if ESTIMATE.load(Ordering::SeqCst) == id {
                w.set_estimate_to(if made.is_ok() { bytes_text(guess) } else { "?".to_string() }.into());
            }
        });
    });
}

/// The fields in a new unit, the same size.
fn unit_picked(i: i32) {
    with_custom(|c| {
        let w = &c.window;
        let old = UNITS[w.get_unit().clamp(0, 3) as usize];
        let new = UNITS[i.clamp(0, 3) as usize];
        if let Some(info) = &c.first {
            for (text, along, set) in [
                (w.get_width_text(), info.w, &(|s: String| w.set_width_text(s.into())) as &dyn Fn(String)),
                (w.get_height_text(), info.h, &|s: String| w.set_height_text(s.into())),
            ] {
                let v = parse_number(&text);
                if v > 0.0 {
                    let px = magick::to_px(v, old, along, info.ppi);
                    set(number_text(magick::from_px(px.round() as u32, new, along, info.ppi), new));
                }
            }
        }
        w.set_unit(i);
    });
    custom_changed();
}

/// With the lock on, one field follows the other in the first picture's shape.
fn follow(width_edited: bool) {
    with_custom(|c| {
        let w = &c.window;
        let (Some(info), true) = (&c.first, w.get_lock()) else { return };
        let unit = UNITS[w.get_unit().clamp(0, 3) as usize];
        let (from, along_from, along_to) =
            if width_edited { (w.get_width_text(), info.w, info.h) } else { (w.get_height_text(), info.h, info.w) };
        let v = parse_number(&from);
        let text = if v <= 0.0 {
            String::new()
        } else if unit == Unit::Percent {
            number_text(v, unit)
        } else {
            let px = magick::to_px(v, unit, along_from, info.ppi) * along_to as f32 / along_from as f32;
            number_text(magick::from_px(px.round() as u32, unit, along_to, info.ppi), unit)
        };
        if width_edited {
            w.set_height_text(text.into());
        } else {
            w.set_width_text(text.into());
        }
    });
    custom_changed();
}

fn fill_fields(w: &ResizeCustom, spec: &Spec, first: Option<&Info>) {
    match (&spec.size, first) {
        (Some(size), _) => {
            w.set_unit(UNITS.iter().position(|u| *u == size.unit).unwrap_or(0) as i32);
            w.set_width_text(if size.w > 0.0 { number_text(size.w, size.unit) } else { String::new() }.into());
            w.set_height_text(if size.h > 0.0 { number_text(size.h, size.unit) } else { String::new() }.into());
            w.set_mode(MODES.iter().position(|m| *m == size.mode).unwrap_or(0) as i32);
            w.set_lock(size.lock);
        }
        (None, Some(info)) => {
            w.set_unit(0);
            w.set_width_text(info.w.to_string().into());
            w.set_height_text(info.h.to_string().into());
        }
        (None, None) => {}
    }
    w.set_format(FORMATS.iter().position(|f| *f == spec.format).unwrap_or(0) as i32);
    w.set_quality(spec.quality.unwrap_or(85) as f32);
    w.set_strip(spec.strip);
}

fn choose_folder() {
    let Some(weak) = with_custom(|c| c.window.as_weak()) else { return };
    std::thread::spawn(move || {
        let chosen = pick_folder();
        let _ = weak.upgrade_in_event_loop(move |w| match chosen {
            Ok(Some(folder)) => {
                with_custom(|c| c.folder = Some(folder));
                w.set_dest(1);
                custom_changed();
            }
            Ok(None) => {}
            Err(err) => w.set_error(format!("The folder chooser didn't open: {err}").into()),
        });
    });
}

/// The desktop's own folder chooser (the FileChooser portal).
fn pick_folder() -> Result<Option<PathBuf>, String> {
    use zbus::zvariant::Value;
    let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
    let token = crate::portal::new_token("archtoys_folder");
    let mut options = crate::portal::options(&token);
    options.insert("directory", Value::from(true));
    options.insert("modal", Value::from(true));
    match crate::portal::call(&conn, "org.freedesktop.portal.FileChooser", "OpenFile", &("", "Save the pictures in", options), &token)? {
        crate::portal::Outcome::Success(results) => {
            let uris: Vec<String> = results.get("uris").and_then(|v| <Vec<String>>::try_from(v.clone()).ok()).unwrap_or_default();
            Ok(uris.first().and_then(|u| crate::portal::file_uri_to_path(u)))
        }
        crate::portal::Outcome::Cancelled => Ok(None),
    }
}

fn save_preset() {
    let Some((name, spec)) = with_custom(|c| (c.window.get_preset_name().trim().to_string(), spec_from(&c.window))) else { return };
    if name.is_empty() {
        return;
    }
    let index = PRESETS.with(|p| {
        let mut list = p.borrow_mut();
        match list.iter().position(|p| p.name == name) {
            Some(i) => {
                list[i].spec = spec;
                i
            }
            None => {
                list.push(Preset { name, spec });
                list.len() - 1
            }
        }
    });
    persist();
    refresh_menus();
    with_custom(|c| {
        c.window.set_presets(strings(&custom_preset_list()));
        c.window.set_preset(index as i32 + 1);
    });
}

fn custom_preset_list() -> Vec<String> {
    let names = preset_names();
    if names.is_empty() {
        return names;
    }
    std::iter::once("None".to_string()).chain(names).collect()
}

fn close_custom() {
    CUSTOM.with(|c| {
        if let Some(custom) = c.borrow_mut().take() {
            let _ = custom.window.hide();
        }
    });
}

fn open_custom(files: Vec<PathBuf>) {
    close_custom();
    let Ok(w) = ResizeCustom::new() else { return };
    copy_skin!(w);
    let names: Vec<String> = files.iter().map(|f| file_name(f)).collect();
    let shown = if names.len() > 3 { format!("{}, ...", names[..3].join(", ")) } else { names.join(", ") };
    w.set_files_text(format!("{}: {shown}", count_text(files.len())).into());
    w.set_count(files.len() as i32);
    w.set_presets(strings(&custom_preset_list()));
    w.on_unit_picked(unit_picked);
    w.on_width_edited(|| follow(true));
    w.on_height_edited(|| follow(false));
    w.on_lock_toggled(|| {
        with_custom(|c| c.window.set_lock(!c.window.get_lock()));
        follow(true);
    });
    w.on_changed(custom_changed);
    w.on_preset_picked(|i| {
        if i > 0 {
            if let Some(p) = PRESETS.with(|p| p.borrow().get(i as usize - 1).cloned()) {
                with_custom(|c| fill_fields(&c.window, &p.spec, c.first.as_ref()));
                custom_changed();
            }
        }
    });
    w.on_choose_folder(choose_folder);
    w.on_save_preset(save_preset);
    w.on_cancel(close_custom);
    w.on_start(|| {
        let Some((files, spec, dest)) = with_custom(|c| (c.files.clone(), spec_from(&c.window), dest_from(c))) else { return };
        close_custom();
        run_job(files, spec, dest);
    });
    w.window().on_close_requested(|| {
        close_custom();
        slint::CloseRequestResponse::HideWindow
    });
    let total_bytes = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    w.set_estimate_from(bytes_text(total_bytes).into());
    w.set_dest_text("Next to the originals".into());
    let _ = w.show();
    let weak = w.as_weak();
    CUSTOM.with(|c| *c.borrow_mut() = Some(Custom { window: w, files: files.clone(), first: None, folder: None, total_bytes }));

    // what the pictures are: the first sets the fields, all say if they have metadata
    std::thread::spawn(move || {
        let Some(tool) = magick::tool() else {
            let _ =
                weak.upgrade_in_event_loop(|w| w.set_error("ImageMagick isn't installed. Turn Image Resizer on again in Archtoys.".into()));
            return;
        };
        let infos: Vec<(String, Option<Info>)> = files.iter().map(|f| (file_name(f), magick::probe(tool, f).ok())).collect();
        let _ = weak.upgrade_in_event_loop(move |w| {
            let first = infos.iter().find_map(|(_, i)| i.clone());
            let with_meta: Vec<String> =
                infos.iter().filter(|(_, i)| i.as_ref().is_some_and(|i| i.metadata)).map(|(n, _)| n.clone()).collect();
            w.set_meta_text(metadata_text(&with_meta).into());
            fill_fields(&w, &Spec::default(), first.as_ref());
            with_custom(|c| c.first = first);
            custom_changed();
        });
    });
}

// ---- The job: progress, then a summary ----

#[derive(Default)]
struct Tally {
    done: usize,
    skipped: Vec<String>,
    bytes_in: u64,
    bytes_out: u64,
    folder: Option<PathBuf>,
    stopped: bool,
}

/// "Resizing" or "Converting": what the job mostly does.
fn verbs(spec: &Spec) -> (&'static str, &'static str) {
    if spec.size.is_none() && !spec.smaller && spec.format != Format::Keep {
        ("Converting", "converted")
    } else if spec.size.is_none() && spec.smaller {
        ("Making smaller", "made smaller")
    } else {
        ("Resizing", "resized")
    }
}

pub fn summary_heading(spec: &Spec, total: usize, done: usize, skipped: usize, stopped: bool) -> String {
    let (_, past) = verbs(spec);
    if stopped {
        return format!("Stopped: {done} of {total} {past}");
    }
    match skipped {
        0 => format!("{done} {past}"),
        k => format!("{done} {past}, {k} skipped"),
    }
}

fn skipped_text(skipped: &[String]) -> String {
    let mut lines: Vec<String> = skipped.iter().take(4).cloned().collect();
    if skipped.len() > 4 {
        lines.push(format!("and {} more", skipped.len() - 4));
    }
    lines.join("\n")
}

fn saved_text(dest: &Dest, t: &Tally) -> String {
    if t.done == 0 {
        return String::new();
    }
    let place = match dest {
        Dest::Beside => "Saved next to the originals, with -resized added.".to_string(),
        Dest::Folder(f) => format!("Saved in {}.", home_short(f)),
        Dest::Replace => "The originals were replaced.".to_string(),
    };
    format!("{place} {} became {}.", bytes_text(t.bytes_in), bytes_text(t.bytes_out))
}

fn format_label(format: &str, quality: Option<u8>) -> String {
    let name = match format {
        "JPEG" => "JPG",
        "WEBP" => "WebP",
        other => other,
    };
    match quality {
        Some(q) => format!("{name} {q}%"),
        None => name.to_string(),
    }
}

/// One picture. Err: why it was skipped.
fn one(tool: Tool, spec: &Spec, dest: &Dest, input: &Path, on_start: &dyn Fn(String)) -> Result<(u64, u64, PathBuf), String> {
    if input.is_dir() {
        return Err("a folder".into());
    }
    let info = magick::probe(tool, input)?;
    let (format, ext) = magick::out_format(spec, &info, input);
    let output = magick::out_path(dest, input, &ext, &|p| p.exists());
    let (w, h) = magick::out_size(spec, &info);
    on_start(format!("{} · {w} × {h} · {}", file_name(input), format_label(&format, magick::out_quality(spec, &format))));
    let bytes_in = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    if let Dest::Folder(f) = dest {
        std::fs::create_dir_all(f).map_err(|e| e.to_string())?;
    }
    magick::convert(tool, spec, &info, input, &output, &format)?;
    if *dest == Dest::Replace && output != input {
        // a new format: the new picture took the original's place
        let _ = std::fs::remove_file(input);
    }
    let bytes_out = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok((bytes_in, bytes_out, output))
}

fn open_folder(folder: &Path) {
    let mut command = crate::launch::outside_command("xdg-open");
    command.arg(folder);
    if let Err(err) = crate::launch::spawn_and_reap(command) {
        eprintln!("image resizer: couldn't open the folder: {err}");
    }
}

fn run_job(files: Vec<PathBuf>, spec: Spec, dest: Dest) {
    let Ok(w) = ResizeProgress::new() else { return };
    copy_skin!(w);
    let total = files.len();
    let (doing, _) = verbs(&spec);
    w.set_heading(format!("{doing} 1 of {total}").into());
    w.set_fraction(0.0);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_button = stop.clone();
    w.on_stop(move || stop_button.store(true, Ordering::SeqCst));
    let weak = w.as_weak();
    let close = move || {
        if let Some(w) = weak.upgrade() {
            let _ = w.hide();
            JOBS.with(|j| j.borrow_mut().retain(|x| x.as_weak().upgrade().is_some_and(|x| x.window().is_visible())));
        }
    };
    w.on_dismiss(close.clone());
    w.window().on_close_requested({
        let stop = stop.clone();
        move || {
            stop.store(true, Ordering::SeqCst);
            close();
            slint::CloseRequestResponse::HideWindow
        }
    });
    let folder_slot: Arc<std::sync::Mutex<Option<PathBuf>>> = Arc::default();
    let slot = folder_slot.clone();
    w.on_open_folder(move || {
        if let Some(f) = slot.lock().unwrap().as_ref() {
            open_folder(f);
        }
    });
    let _ = w.show();
    let weak = w.as_weak();
    JOBS.with(|j| j.borrow_mut().push(w));

    std::thread::spawn(move || {
        let mut t = Tally::default();
        let Some(tool) = magick::tool() else {
            let _ = weak.upgrade_in_event_loop(|w| {
                w.set_heading("ImageMagick isn't installed".into());
                w.set_saved("Turn Image Resizer on again in Archtoys (Home, Micro-toys), and it installs it.".into());
                w.set_done(true);
            });
            return;
        };
        for (i, file) in files.iter().enumerate() {
            if stop.load(Ordering::SeqCst) {
                t.stopped = true;
                break;
            }
            let progress = weak.clone();
            let heading = format!("{doing} {} of {total}", i + 1);
            let fraction = i as f32 / total as f32;
            let on_start = move |line: String| {
                let heading = heading.clone();
                let _ = progress.upgrade_in_event_loop(move |w| {
                    w.set_heading(heading.into());
                    w.set_line(line.into());
                    w.set_fraction(fraction);
                });
            };
            match one(tool, &spec, &dest, file, &on_start) {
                Ok((bytes_in, bytes_out, output)) => {
                    t.done += 1;
                    t.bytes_in += bytes_in;
                    t.bytes_out += bytes_out;
                    if t.folder.is_none() {
                        t.folder = output.parent().map(Path::to_path_buf);
                    }
                }
                Err(why) => t.skipped.push(format!("{}: {why}", file_name(file))),
            }
        }
        let heading = summary_heading(&spec, total, t.done, t.skipped.len(), t.stopped);
        let skipped = skipped_text(&t.skipped);
        let saved = saved_text(&dest, &t);
        let folder = t.folder.clone();
        let _ = weak.upgrade_in_event_loop(move |w| {
            w.set_fraction(1.0);
            w.set_heading(heading.into());
            w.set_skipped(skipped.into());
            w.set_saved(saved.into());
            w.set_can_open(folder.is_some());
            *folder_slot.lock().unwrap() = folder;
            w.set_done(true);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_actions_do_what_they_say() {
        let half = quick_spec("r50").unwrap().size.unwrap();
        assert_eq!((half.w, half.unit), (50.0, Unit::Percent));
        assert_eq!(quick_spec("webp").unwrap().format, Format::Webp);
        assert!(quick_spec("webp").unwrap().size.is_none());
        assert!(quick_spec("smaller").unwrap().smaller);
        assert!(quick_spec("preset:3").is_none());
        assert!(quick_spec("nothing").is_none());
        // every menu entry but Custom... is a quick action
        for (id, _) in menus::QUICK.iter().flatten() {
            assert_eq!(quick_spec(id).is_some(), *id != "custom", "{id}");
        }
    }

    #[test]
    fn the_summary_reads_like_the_mockup() {
        let resize = quick_spec("r50").unwrap();
        assert_eq!(summary_heading(&resize, 13, 12, 1, false), "12 resized, 1 skipped");
        assert_eq!(summary_heading(&quick_spec("png").unwrap(), 3, 3, 0, false), "3 converted");
        assert_eq!(summary_heading(&resize, 12, 7, 0, true), "Stopped: 7 of 12 resized");
        let t = Tally { done: 12, bytes_in: 38_200_000, bytes_out: 7_900_000, ..Default::default() };
        assert_eq!(saved_text(&Dest::Beside, &t), "Saved next to the originals, with -resized added. 38.2 MB became 7.9 MB.");
        assert_eq!(skipped_text(&vec!["a: x".to_string(); 6]), "a: x\na: x\na: x\na: x\nand 2 more");
    }

    #[test]
    fn metadata_line_names_up_to_two() {
        assert_eq!(metadata_text(&["beach.jpg".into(), "hills.jpg".into()]), "beach.jpg and hills.jpg have metadata");
        assert_eq!(metadata_text(&["a.jpg".into()]), "a.jpg has metadata");
        assert_eq!(metadata_text(&["a".into(), "b".into(), "c".into()]), "3 pictures have metadata");
        assert_eq!(metadata_text(&[]), "");
    }

    #[test]
    fn numbers_in_the_fields() {
        assert_eq!(number_text(1920.4, Unit::Px), "1920");
        assert_eq!(number_text(12.50, Unit::Cm), "12.5");
        assert_eq!(number_text(50.0, Unit::Percent), "50");
        assert_eq!(parse_number(" 12,5 "), 12.5);
        assert_eq!(parse_number("abc"), 0.0);
        assert_eq!(bytes_text(9_600_000), "9.6 MB");
        assert_eq!(bytes_text(320_400), "320 KB");
    }
}
