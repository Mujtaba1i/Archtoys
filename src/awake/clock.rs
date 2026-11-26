//! Local time for Awake: "4:30 PM" or "16:30" the way the system shows
//! times, and the next moment a given hour and minute comes round.

use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Does the system show times with AM and PM? Asks `locale` for the time
/// format (it follows LC_TIME, which the desktop's region setting sets).
pub fn system_uses_12_hours() -> bool {
    Command::new("locale").arg("t_fmt").output().ok().map(|o| uses_12_hours(&String::from_utf8_lossy(&o.stdout))).unwrap_or(false)
}

/// A strftime time format with %p (AM/PM), %r (12-hour clock) or %I (12-hour hour).
pub fn uses_12_hours(t_fmt: &str) -> bool {
    t_fmt.contains("%p") || t_fmt.contains("%r") || t_fmt.contains("%I") || t_fmt.contains("%l")
}

fn local_tm(at: SystemTime) -> Option<libc::tm> {
    let secs = at.duration_since(UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    // SAFETY: localtime_r writes only into `tm`, which outlives the call
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        (!libc::localtime_r(&secs, &mut tm).is_null()).then_some(tm)
    }
}

/// The local hour (0 to 23) and minute.
pub fn hour_minute(at: SystemTime) -> (u32, u32) {
    local_tm(at).map(|tm| (tm.tm_hour as u32, tm.tm_min as u32)).unwrap_or((0, 0))
}

/// "4:30 PM" on a 12-hour clock, else "16:30".
pub fn time_text(at: SystemTime) -> String {
    let (h, m) = hour_minute(at);
    format_time(h, m, crate::awake::clock::system_12h_cached())
}

pub fn format_time(hour: u32, minute: u32, twelve: bool) -> String {
    if twelve {
        let (shown, pm) = to_shown_hour(hour, true);
        format!("{shown}:{minute:02} {}", if pm { "PM" } else { "AM" })
    } else {
        format!("{hour:02}:{minute:02}")
    }
}

/// The hour as shown: 0-23, or 1-12 and whether it's PM.
pub fn to_shown_hour(hour: u32, twelve: bool) -> (u32, bool) {
    if !twelve {
        return (hour, false);
    }
    let pm = hour >= 12;
    let h = hour % 12;
    (if h == 0 { 12 } else { h }, pm)
}

/// Back from the shown hour to 0-23. None for an hour that can't be.
pub fn from_shown_hour(shown: u32, twelve: bool, pm: bool) -> Option<u32> {
    if twelve {
        if !(1..=12).contains(&shown) {
            return None;
        }
        Some(shown % 12 + if pm { 12 } else { 0 })
    } else {
        (shown < 24).then_some(shown)
    }
}

/// The next time the clock shows `hour:minute` (today, or tomorrow when that's past).
pub fn next_time(hour: u32, minute: u32) -> Option<SystemTime> {
    let now = SystemTime::now();
    let mut tm = local_tm(now)?;
    tm.tm_hour = hour as i32;
    tm.tm_min = minute as i32;
    tm.tm_sec = 0;
    tm.tm_isdst = -1;
    // SAFETY: mktime reads and normalises `tm`, nothing else
    let at = unsafe { libc::mktime(&mut tm) };
    if at < 0 {
        return None;
    }
    let mut target = UNIX_EPOCH + Duration::from_secs(at as u64);
    if target <= now {
        target += Duration::from_secs(24 * 3600);
    }
    Some(target)
}

/// The answer of `system_uses_12_hours`, asked once.
pub fn system_12h_cached() -> bool {
    static TWELVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TWELVE.get_or_init(system_uses_12_hours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_time_format_says_which_clock() {
        assert!(uses_12_hours("%r"));
        assert!(uses_12_hours("%I:%M:%S %p"));
        assert!(!uses_12_hours("%T"));
        assert!(!uses_12_hours("%H:%M:%S"));
    }

    #[test]
    fn hours_go_both_ways_on_a_12_hour_clock() {
        assert_eq!(to_shown_hour(0, true), (12, false));
        assert_eq!(to_shown_hour(12, true), (12, true));
        assert_eq!(to_shown_hour(16, true), (4, true));
        assert_eq!(to_shown_hour(16, false), (16, false));
        assert_eq!(from_shown_hour(12, true, false), Some(0));
        assert_eq!(from_shown_hour(12, true, true), Some(12));
        assert_eq!(from_shown_hour(4, true, true), Some(16));
        assert_eq!(from_shown_hour(13, true, true), None);
        assert_eq!(from_shown_hour(23, false, false), Some(23));
        assert_eq!(from_shown_hour(24, false, false), None);
    }

    #[test]
    fn times_read_like_the_system_s() {
        assert_eq!(format_time(16, 30, true), "4:30 PM");
        assert_eq!(format_time(0, 5, true), "12:05 AM");
        assert_eq!(format_time(16, 30, false), "16:30");
        assert_eq!(format_time(9, 5, false), "09:05");
    }

    #[test]
    fn the_next_time_is_always_ahead_and_within_a_day() {
        let now = SystemTime::now();
        let (h, m) = hour_minute(now);
        let next = next_time(h, m).unwrap();
        let ahead = next.duration_since(now).unwrap();
        assert!(ahead > Duration::ZERO && ahead <= Duration::from_secs(24 * 3600), "{ahead:?}");
    }
}
