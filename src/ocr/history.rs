//! Text Extractor's history: the text of the last 500 extractions (never the
//! screenshots), saved in `~/.local/share/archtoys/text-history.json` until
//! the user deletes an entry or clears it all.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) const MAX: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub(crate) struct Entry {
    pub text: String,
    /// "English"
    pub language: String,
    /// how sure Tesseract was, 0-100
    pub sure: u8,
    /// the text's height on screen, in pixels
    pub size: u32,
    pub words: u32,
    /// seconds since 1970 (UTC)
    pub time: u64,
}

fn path() -> PathBuf {
    super::lang::app_data_dir().join("text-history.json")
}

pub(crate) fn load() -> Vec<Entry> {
    let mut entries: Vec<Entry> =
        std::fs::read_to_string(path()).ok().and_then(|data| serde_json::from_str(&data).ok()).unwrap_or_default();
    entries.truncate(MAX);
    entries
}

pub(crate) fn save(entries: &[Entry]) {
    let path = path();
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("text history: couldn't make {}: {err}", parent.display());
            return;
        }
    }
    match serde_json::to_string(entries) {
        Ok(data) => {
            if let Err(err) = crate::config::write_atomic(&path, data.as_bytes()) {
                eprintln!("text history: couldn't save it: {err}");
            }
        }
        Err(err) => eprintln!("text history: {err}"),
    }
}

/// Adds the newest entry at the top, keeping the last 500.
pub(crate) fn push(entries: &mut Vec<Entry>, entry: Entry) {
    entries.insert(0, entry);
    entries.truncate(MAX);
}

/// The system's clock: 12 hours ("2:20 PM") or 24 ("14:20"). Read once at
/// startup, before any other thread runs (`init_clock`, main.rs).
static TWELVE_HOURS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Reads whether the system shows 12-hour times: the locale's time format
/// (what KDE's Region settings set), and on GNOME its own clock setting.
/// Call it first thing in main: setlocale isn't safe once threads run.
pub(crate) fn init_clock() {
    #[cfg(target_env = "gnu")]
    {
        extern "C" {
            fn setlocale(category: i32, locale: *const std::ffi::c_char) -> *mut std::ffi::c_char;
            fn nl_langinfo(item: i32) -> *const std::ffi::c_char;
        }
        const LC_TIME: i32 = 2;
        const T_FMT: i32 = 0x2002A;
        // SAFETY: called before any other thread exists; the strings are glibc's own, read at once.
        let format = unsafe {
            setlocale(LC_TIME, c"".as_ptr());
            let p = nl_langinfo(T_FMT);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
            }
        };
        TWELVE_HOURS.store(twelve_hour_format(&format), std::sync::atomic::Ordering::Relaxed);
    }
    if std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("GNOME")) {
        std::thread::spawn(|| {
            let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "clock-format"]).output();
            if let Some(out) = out.ok().filter(|o| o.status.success()) {
                let value = String::from_utf8_lossy(&out.stdout);
                TWELVE_HOURS.store(value.contains("12h"), std::sync::atomic::Ordering::Relaxed);
            }
        });
    }
}

/// A strftime time format that shows AM/PM ("%r", "%I:%M %p")?
fn twelve_hour_format(format: &str) -> bool {
    ["%r", "%I", "%l", "%p", "%P"].iter().any(|f| format.contains(f))
}

/// "14:20", or "2:20 PM" when the system uses 12-hour times.
fn clock(hh: u32, mm: u32) -> String {
    clock_as(hh, mm, TWELVE_HOURS.load(std::sync::atomic::Ordering::Relaxed))
}

fn clock_as(hh: u32, mm: u32, twelve: bool) -> String {
    if twelve {
        let h = if hh.is_multiple_of(12) { 12 } else { hh % 12 };
        format!("{h}:{mm:02} {}", if hh < 12 { "AM" } else { "PM" })
    } else {
        format!("{hh}:{mm:02}")
    }
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// (year, month 1-12, day, hour, minute) in the computer's local time.
fn local_parts(unix: u64) -> (i64, u32, u32, u32, u32) {
    #[cfg(target_env = "gnu")]
    {
        // glibc's struct tm on 64-bit Linux
        #[repr(C)]
        struct Tm {
            sec: i32,
            min: i32,
            hour: i32,
            mday: i32,
            mon: i32,
            year: i32,
            wday: i32,
            yday: i32,
            isdst: i32,
            gmtoff: i64,
            zone: *const std::ffi::c_char,
        }
        extern "C" {
            fn localtime_r(time: *const i64, result: *mut Tm) -> *mut Tm;
        }
        let t = unix as i64;
        let mut tm =
            Tm { sec: 0, min: 0, hour: 0, mday: 0, mon: 0, year: 0, wday: 0, yday: 0, isdst: 0, gmtoff: 0, zone: std::ptr::null() };
        // SAFETY: both pointers are valid for the call; localtime_r only writes `tm`.
        if !unsafe { localtime_r(&t, &mut tm) }.is_null() {
            return (tm.year as i64 + 1900, tm.mon as u32 + 1, tm.mday as u32, tm.hour as u32, tm.min as u32);
        }
    }
    utc_parts(unix)
}

/// The same in UTC (Howard Hinnant's days-to-civil).
fn utc_parts(unix: u64) -> (i64, u32, u32, u32, u32) {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, (secs / 3600) as u32, ((secs % 3600) / 60) as u32)
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// "Today 9:41", "Yesterday 18:20", "3 Nov", or "3 Nov 2025" in another year.
pub(crate) fn when_label(time: u64, now: u64) -> String {
    label_from(local_parts(time), local_parts(now), local_parts(now.saturating_sub(86_400)))
}

fn label_from(t: (i64, u32, u32, u32, u32), today: (i64, u32, u32, u32, u32), yesterday: (i64, u32, u32, u32, u32)) -> String {
    let (y, m, d, hh, mm) = t;
    if (y, m, d) == (today.0, today.1, today.2) {
        format!("Today {}", clock(hh, mm))
    } else if (y, m, d) == (yesterday.0, yesterday.1, yesterday.2) {
        format!("Yesterday {}", clock(hh, mm))
    } else if y == today.0 {
        format!("{d} {}", MONTHS[(m - 1) as usize])
    } else {
        format!("{d} {} {y}", MONTHS[(m - 1) as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_follow_the_system_clock() {
        assert_eq!(clock_as(14, 20, false), "14:20");
        assert_eq!(clock_as(14, 20, true), "2:20 PM");
        assert_eq!(clock_as(0, 5, true), "12:05 AM");
        assert_eq!(clock_as(12, 0, true), "12:00 PM");
        // en_US says "%r", en_GB "%T", de_DE "%T", the C locale "%H:%M:%S"
        assert!(twelve_hour_format("%r") && twelve_hour_format("%I:%M:%S %p"));
        assert!(!twelve_hour_format("%T") && !twelve_hour_format("%H:%M:%S"));
    }

    #[test]
    fn keeps_the_last_500_newest_first() {
        let mut entries = Vec::new();
        for i in 0..520 {
            push(&mut entries, Entry { text: format!("text {i}"), ..Default::default() });
        }
        assert_eq!(entries.len(), MAX);
        assert_eq!(entries[0].text, "text 519");
        assert_eq!(entries[MAX - 1].text, "text 20");
    }

    #[test]
    fn old_or_damaged_files_still_load() {
        let partial: Vec<Entry> = serde_json::from_str(r#"[{"text":"hi"}]"#).unwrap();
        assert_eq!(partial[0].text, "hi");
        assert_eq!(partial[0].language, "");
        assert!(serde_json::from_str::<Vec<Entry>>("not json").is_err()); // load() then starts empty
    }

    #[test]
    fn utc_dates_are_right() {
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0));
        assert_eq!(utc_parts(1_793_867_000), (2026, 11, 5, 8, 23)); // 2026-11-05 08:23:20 UTC
        assert_eq!(utc_parts(951_782_400), (2000, 2, 29, 0, 0)); // a leap day
    }

    #[test]
    fn labels_read_well() {
        let today = (2026, 11, 5, 12, 0);
        let yesterday = (2026, 11, 4, 12, 0);
        assert_eq!(label_from((2026, 11, 5, 9, 41), today, yesterday), "Today 9:41");
        assert_eq!(label_from((2026, 11, 4, 18, 5), today, yesterday), "Yesterday 18:05");
        assert_eq!(label_from((2026, 11, 3, 8, 0), today, yesterday), "3 Nov");
        assert_eq!(label_from((2025, 12, 31, 8, 0), today, yesterday), "31 Dec 2025");
    }
}
