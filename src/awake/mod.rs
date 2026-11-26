//! Awake (v0.9.0, inspired by Vorssaint): keeps the computer awake for as
//! long as you choose. The desktop is asked through its portal (Inhibit:
//! "suspend", plus "idle" when the screen stays on), with logind as the
//! fallback. No password. Times follow the system's clock (12 or 24 hours).
//! When the time is up: a notification. The easter egg: while it's on, the
//! cup in the sidebar sometimes puffs steam for a moment.

pub mod clock;

use crate::{AppWindow, Awake, AwakePanel};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zbus::zvariant::{OwnedFd, OwnedObjectPath, Value};

/// The lengths on the page, in minutes (6 = until turned off, 7 = until a time).
pub const LENGTHS: [u64; 6] = [15, 30, 60, 120, 240, 480];
pub const FOREVER: i32 = 6;
pub const UNTIL: i32 = 7;

/// Holding one of these keeps the computer awake. Dropping it lets it sleep.
enum Inhibitor {
    // the request's handle, closed to let go (and released with the connection anyway)
    Portal(zbus::blocking::Connection, OwnedObjectPath),
    // logind lets go when the descriptor closes
    Logind(#[allow(dead_code)] OwnedFd),
}

impl Drop for Inhibitor {
    fn drop(&mut self) {
        if let Inhibitor::Portal(conn, handle) = self {
            let _ = conn.call_method(
                Some("org.freedesktop.portal.Desktop"),
                handle.as_str(),
                Some("org.freedesktop.portal.Request"),
                "Close",
                &(),
            );
        }
    }
}

/// Asks the desktop to stay awake: suspend always, idle (the screen) too when `screen_on`.
fn inhibit(screen_on: bool) -> Result<Inhibitor, String> {
    let portal = || -> Result<Inhibitor, String> {
        let conn = zbus::blocking::Connection::session().map_err(|e| e.to_string())?;
        let flags: u32 = if screen_on { 4 | 8 } else { 4 };
        let mut options: HashMap<&str, Value> = HashMap::new();
        options.insert("reason", Value::from("Archtoys Awake"));
        options.insert("handle_token", Value::from(crate::portal::new_token("archtoys_awake")));
        let reply = conn
            .call_method(
                Some("org.freedesktop.portal.Desktop"),
                "/org/freedesktop/portal/desktop",
                Some("org.freedesktop.portal.Inhibit"),
                "Inhibit",
                &("", flags, options),
            )
            .map_err(|e| e.to_string())?;
        let handle: OwnedObjectPath = reply.body().deserialize().map_err(|e| e.to_string())?;
        Ok(Inhibitor::Portal(conn, handle))
    };
    let logind = || -> Result<Inhibitor, String> {
        let conn = zbus::blocking::Connection::system().map_err(|e| e.to_string())?;
        let what = if screen_on { "sleep:idle" } else { "sleep" };
        let reply = conn
            .call_method(
                Some("org.freedesktop.login1"),
                "/org/freedesktop/login1",
                Some("org.freedesktop.login1.Manager"),
                "Inhibit",
                &(what, "Archtoys", "Awake keeps the computer awake", "block"),
            )
            .map_err(|e| e.to_string())?;
        let fd: OwnedFd = reply.body().deserialize().map_err(|e| e.to_string())?;
        Ok(Inhibitor::Logind(fd))
    };
    portal().or_else(|portal_err| {
        logind().map_err(|logind_err| format!("the desktop said no ({portal_err}), and so did logind ({logind_err})"))
    })
}

struct Session {
    _inhibitor: Inhibitor,
    choice: i32,
    started: Instant,
    // None: until turned off
    end: Option<Instant>,
    end_wall: Option<SystemTime>,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    static TICK: slint::Timer = slint::Timer::default();
    static PUFF: slint::Timer = slint::Timer::default();
    static PANEL: RefCell<Option<AwakePanel>> = const { RefCell::new(None) };
    static UI: RefCell<Option<slint::Weak<AppWindow>>> = const { RefCell::new(None) };
    static PERSIST: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

fn main_ui() -> Option<AppWindow> {
    UI.with(|u| u.borrow().as_ref().and_then(|w| w.upgrade()))
}

/// Runs `f` on the Awake global of the main window and of the tray panel.
fn each_global(f: impl Fn(&Awake)) {
    if let Some(ui) = main_ui() {
        f(&ui.global::<Awake>());
    }
    PANEL.with(|p| {
        if let Some(panel) = p.borrow().as_ref() {
            f(&panel.global::<Awake>());
        }
    });
}

/// "1:25:40", or "25:40" under an hour.
pub fn clock_text(left: Duration) -> String {
    let s = left.as_secs();
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Home's badge: "1:25 left", "25 min left".
pub fn short_text(left: Duration) -> String {
    let m = left.as_secs().div_ceil(60);
    if m >= 60 {
        format!("{}:{:02} left", m / 60, m % 60)
    } else {
        format!("{m} min left")
    }
}

/// Shows the session (or nothing) everywhere, and keeps the tray's line in step.
fn render() {
    let state = SESSION.with(|s| {
        s.borrow().as_ref().map(|s| {
            let now = Instant::now();
            match s.end {
                Some(end) => {
                    let left = end.saturating_duration_since(now);
                    let total = end.saturating_duration_since(s.started).max(Duration::from_secs(1));
                    let until = s.end_wall.map(|w| format!("Awake until {}", clock::time_text(w))).unwrap_or_default();
                    (s.choice, clock_text(left), short_text(left), until, left.as_secs_f32() / total.as_secs_f32())
                }
                // no countdown: words, the size of "Awake is off"
                None => (s.choice, "Staying awake".to_string(), "On".to_string(), "Until you turn it off".to_string(), 1.0),
            }
        })
    });
    each_global(|g| match &state {
        Some((choice, left, short, until, fraction)) => {
            g.set_running(true);
            g.set_choice(*choice);
            g.set_left(left.as_str().into());
            g.set_left_short(short.as_str().into());
            g.set_until(until.as_str().into());
            g.set_fraction(*fraction);
        }
        None => {
            g.set_running(false);
            g.set_choice(-1);
            g.set_puff(false);
        }
    });
    crate::tray::set_awake(state.as_ref().map(|(_, _, short, _, _)| short.clone()));
}

fn screen_on() -> bool {
    main_ui().map(|ui| ui.global::<Awake>().get_screen_on()).unwrap_or(true)
}

/// Starts (or restarts with a new length). `minutes` None: until turned off.
fn begin(choice: i32, end_wall: Option<SystemTime>) {
    let inhibitor = match inhibit(screen_on()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("awake: couldn't keep the computer awake: {e}");
            each_global(|g| g.set_error("Your desktop didn't let Archtoys keep the computer awake.".into()));
            return;
        }
    };
    each_global(|g| g.set_error("".into()));
    let now = Instant::now();
    let end = end_wall.map(|w| now + w.duration_since(SystemTime::now()).unwrap_or_default());
    SESSION.with(|s| *s.borrow_mut() = Some(Session { _inhibitor: inhibitor, choice, started: now, end, end_wall }));
    TICK.with(|t| t.start(slint::TimerMode::Repeated, Duration::from_secs(1), tick));
    schedule_puff();
    render();
}

fn stop() {
    SESSION.with(|s| *s.borrow_mut() = None);
    TICK.with(|t| t.stop());
    PUFF.with(|t| t.stop());
    render();
}

fn tick() {
    let done = SESSION.with(|s| s.borrow().as_ref().and_then(|s| s.end).is_some_and(|end| Instant::now() >= end));
    if done {
        stop();
        std::thread::spawn(notify_time_up);
    } else {
        render();
    }
}

/// The easter egg: now and then (every 3 to 15 minutes, at random), the
/// sidebar's cup puffs for under a second. Nothing announces it.
fn schedule_puff() {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as u64;
    let wait = Duration::from_secs(180 + nanos % 720);
    PUFF.with(|t| {
        t.start(slint::TimerMode::SingleShot, wait, || {
            if SESSION.with(|s| s.borrow().is_some()) {
                each_global(|g| g.set_puff(true));
                slint::Timer::single_shot(Duration::from_millis(900), || {
                    each_global(|g| g.set_puff(false));
                    schedule_puff();
                });
            }
        })
    });
}

/// "Time's up", through the desktop's notifications.
fn notify_time_up() {
    let Ok(conn) = zbus::blocking::Connection::session() else { return };
    let hints: HashMap<&str, Value> = HashMap::new();
    let _ = conn.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("Archtoys", 0u32, "archtoys", "Time's up", "Awake stopped. Your computer can sleep again.", Vec::<&str>::new(), hints, -1i32),
    );
}

fn start_choice(choice: i32) {
    let running = SESSION.with(|s| s.borrow().as_ref().map(|s| s.choice));
    // the chosen length clicked again stops it
    if running == Some(choice) {
        stop();
        return;
    }
    match choice {
        UNTIL => open_until(),
        FOREVER => begin(FOREVER, None),
        i if (0..6).contains(&i) => begin(i, Some(SystemTime::now() + Duration::from_secs(LENGTHS[i as usize] * 60))),
        _ => {}
    }
}

fn add(minutes: i32) {
    let changed = SESSION.with(|s| {
        let mut s = s.borrow_mut();
        let Some(session) = s.as_mut() else { return false };
        let (Some(end), Some(wall)) = (session.end, session.end_wall) else { return false };
        let now = Instant::now();
        let delta = Duration::from_secs(minutes.unsigned_abs() as u64 * 60);
        // never less than a minute left
        let new_end = if minutes >= 0 { end + delta } else { end.checked_sub(delta).unwrap_or(now).max(now + Duration::from_secs(60)) };
        let new_wall = if new_end >= end { wall + (new_end - end) } else { wall - (end - new_end) };
        session.end = Some(new_end);
        session.end_wall = Some(new_wall);
        true
    });
    if changed {
        render();
    }
}

/// The "Until..." dialog, starting an hour from now (on the next quarter hour).
fn open_until() {
    let Some(ui) = main_ui() else { return };
    crate::ui_state::show_window(&ui);
    if ui.get_page() != 4 {
        ui.invoke_go(4);
    }
    let soon = SystemTime::now() + Duration::from_secs(3600);
    let (h, m) = clock::hour_minute(soon);
    let m = (m.div_ceil(15) * 15) % 60;
    let g = ui.global::<Awake>();
    let twelve = g.get_twelve_hour();
    let (shown, pm) = clock::to_shown_hour(h, twelve);
    // "07" on a 24-hour clock, "7" with AM/PM
    g.set_until_hour(if twelve { shown.to_string() } else { format!("{shown:02}") }.into());
    g.set_until_minute(format!("{m:02}").into());
    g.set_until_pm(pm);
    until_changed();
    g.set_until_open(true);
}

/// The time the dialog's fields say, as the next such moment.
fn until_target() -> Option<SystemTime> {
    let ui = main_ui()?;
    let g = ui.global::<Awake>();
    let hour: u32 = g.get_until_hour().trim().parse().ok()?;
    let minute: u32 = g.get_until_minute().trim().parse().ok()?;
    let hour = clock::from_shown_hour(hour, g.get_twelve_hour(), g.get_until_pm())?;
    if minute > 59 {
        return None;
    }
    clock::next_time(hour, minute)
}

fn until_changed() {
    let Some(ui) = main_ui() else { return };
    let text = match until_target() {
        Some(target) => {
            let mins = target.duration_since(SystemTime::now()).unwrap_or_default().as_secs().div_ceil(60);
            match (mins / 60, mins % 60) {
                (0, m) => format!("Awake for {m} min."),
                (h, 0) => format!("Awake for {h} h."),
                (h, m) => format!("Awake for {h} h {m} min."),
            }
        }
        None => "Type a time, like 18:30.".to_string(),
    };
    ui.global::<Awake>().set_until_preview(text.into());
}

fn start_until() {
    let Some(target) = until_target() else { return };
    if let Some(ui) = main_ui() {
        ui.global::<Awake>().set_until_open(false);
    }
    begin(UNTIL, Some(target));
}

fn screen_changed(on: bool) {
    // both windows and the tray show the same switch
    each_global(|g| g.set_screen_on(on));
    crate::tray::set_awake_screen(on);
    PERSIST.with(|p| {
        if let Some(p) = p.borrow().as_ref() {
            p()
        }
    });
    // asked again with the new flags, keeping the time
    let current = SESSION.with(|s| s.borrow().as_ref().map(|s| (s.choice, s.end_wall)));
    if let Some((choice, wall)) = current {
        let started = SESSION.with(|s| s.borrow().as_ref().map(|s| s.started));
        SESSION.with(|s| *s.borrow_mut() = None);
        begin(choice, wall);
        if let Some(started) = started {
            SESSION.with(|s| {
                if let Some(s) = s.borrow_mut().as_mut() {
                    s.started = started;
                }
            });
        }
        render();
    }
}

/// Connects the callbacks of a window's Awake global (the main window's, or the tray panel's).
fn wire(g: &Awake, panel: bool) {
    g.on_start(start_choice);
    g.on_stop(stop);
    g.on_add(add);
    g.on_until_changed(until_changed);
    g.on_start_until(start_until);
    // the window whose switch moved already holds the new value
    g.on_screen_changed(move || {
        let on = if panel {
            panel_handle().map(|p| p.global::<Awake>().get_screen_on())
        } else {
            main_ui().map(|ui| ui.global::<Awake>().get_screen_on())
        };
        screen_changed(on.unwrap_or(true));
    });
    g.on_open_app(|| {
        close_panel();
        if let Some(ui) = main_ui() {
            crate::ui_state::show_window(&ui);
            if ui.get_page() != 4 {
                ui.invoke_go(4);
            }
        }
    });
}

fn panel_handle() -> Option<AwakePanel> {
    PANEL.with(|p| p.borrow().as_ref().map(|p| p.clone_strong()))
}

pub(crate) fn install(ui: &AppWindow, persist: impl Fn() + 'static) {
    UI.with(|u| *u.borrow_mut() = Some(ui.as_weak()));
    PERSIST.with(|p| *p.borrow_mut() = Some(Box::new(persist)));
    wire(&ui.global::<Awake>(), false);
    // the clock's format, asked once (it starts a program)
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let twelve = clock::system_uses_12_hours();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.global::<Awake>().set_twelve_hour(twelve);
            render();
        });
    });
}

fn close_panel() {
    PANEL.with(|p| {
        if let Some(panel) = p.borrow().as_ref() {
            let _ = panel.hide();
        }
    });
}

/// A click on the tray icon: Awake's small window, over the tray, or away
/// again when it's already open.
pub(crate) fn toggle_panel(x: i32, y: i32) {
    let open = PANEL.with(|p| p.borrow().as_ref().is_some_and(|p| p.window().is_visible()));
    if open {
        close_panel();
        return;
    }
    let made = PANEL.with(|p| p.borrow().is_some());
    if !made {
        let Ok(panel) = AwakePanel::new() else { return };
        wire(&panel.global::<Awake>(), true);
        panel.on_closed(close_panel);
        crate::window::close_when_unfocused(panel.window(), std::rc::Rc::new(close_panel));
        PANEL.with(|p| *p.borrow_mut() = Some(panel));
    }
    if let Some(panel) = panel_handle() {
        if let Some(ui) = main_ui() {
            let skin = ui.global::<crate::Skin>();
            let ps = panel.global::<crate::Skin>();
            ps.set_theme(skin.get_theme());
            ps.set_dark_t(skin.get_dark_t());
            ps.set_aurora(skin.get_aurora());
            ps.set_real_shadows(!crate::cpu_drawn());
            let g = ui.global::<Awake>();
            let pg = panel.global::<Awake>();
            pg.set_screen_on(g.get_screen_on());
            pg.set_twelve_hour(g.get_twelve_hour());
        }
        let _ = panel.show();
        // over the tray icon: the desktop's panel is usually at the bottom
        // (shortcut: X11 and KDE Wayland only, other Wayland desktops place it themselves)
        if x > 0 || y > 0 {
            let size = panel.window().size();
            let px = (x - size.width as i32 / 2).max(0);
            let py = if y > 400 { (y - size.height as i32 - 12).max(0) } else { y + 12 };
            panel.window().set_position(slint::PhysicalPosition::new(px, py));
        }
    }
    render();
}

/// The tray menu's choices.
pub(crate) fn tray_start(choice: i32) {
    start_choice(choice);
}

pub(crate) fn tray_stop() {
    stop();
}

pub(crate) fn tray_add(minutes: i32) {
    add(minutes);
}

pub(crate) fn tray_screen(on: bool) {
    screen_changed(on);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_left_reads_like_a_clock() {
        assert_eq!(clock_text(Duration::from_secs(5140)), "1:25:40");
        assert_eq!(clock_text(Duration::from_secs(1540)), "25:40");
        assert_eq!(clock_text(Duration::from_secs(9)), "0:09");
        assert_eq!(short_text(Duration::from_secs(5140)), "1:26 left");
        assert_eq!(short_text(Duration::from_secs(1500)), "25 min left");
        assert_eq!(short_text(Duration::from_secs(1)), "1 min left");
    }

    #[test]
    fn the_lengths_are_the_page_s() {
        assert_eq!(LENGTHS, [15, 30, 60, 120, 240, 480]);
    }
}
