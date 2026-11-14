//! Downloads for the font finder (the index and font files), with curl like
//! the language data: its progress (bytes, speed) for the search view, and
//! Stop. Through a temporary file, so a cut-off download leaves nothing.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// A small text file (latest.json).
pub(crate) fn text(url: &str) -> Result<String, String> {
    let out =
        Command::new("curl").args(["-fsSL", "--connect-timeout", "10", "--max-time", "20"]).arg(url).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("couldn't download {url}"))
    }
}

/// The file's size from the server, when it says.
fn size(url: &str) -> Option<u64> {
    let out = Command::new("curl").args(["-sIL", "--connect-timeout", "10", "--max-time", "15"]).arg(url).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok()))
        .next_back()
}

/// Downloads `url` to `dest`, telling `progress` (bytes so far, the size when
/// known, bytes per second) about every 100 ms. `stop` cancels it.
pub(crate) fn download(url: &str, dest: &Path, progress: &dyn Fn(u64, Option<u64>, f64), stop: &AtomicBool) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension("part");
    let total = size(url);
    let mut child = Command::new("curl")
        .args(["-fsSL", "--connect-timeout", "10", "--max-time", "600", "-o"])
        .arg(&tmp)
        .arg(url)
        .spawn()
        .map_err(|_| "curl isn't installed".to_string())?;
    let start = Instant::now();
    loop {
        let got = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        progress(got, total, got as f64 / start.elapsed().as_secs_f64().max(0.05));
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&tmp);
            return Err("stopped".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let got = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
            if !status.success() || got < 100 {
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("couldn't download {url}"));
            }
            return std::fs::rename(&tmp, dest).map_err(|e| e.to_string());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A file's SHA-256 (sha256sum is in every distribution's coreutils).
pub(crate) fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// A path in github.com/google/fonts as a download link ([ and ] escaped).
pub(crate) fn google_url(path: &str) -> String {
    format!("https://github.com/google/fonts/raw/main/{}", path.replace('[', "%5B").replace(']', "%5D").replace(' ', "%20"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn google_font_paths_become_links() {
        assert_eq!(
            super::google_url("ofl/roboto/Roboto[wdth,wght].ttf"),
            "https://github.com/google/fonts/raw/main/ofl/roboto/Roboto%5Bwdth,wght%5D.ttf"
        );
    }
}
