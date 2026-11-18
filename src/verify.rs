//! Every download is checked before it's used (v0.8.0): a file that was
//! changed, swapped or cut off on the way is thrown away.
//!
//! Files from a GitHub repository (Text Extractor's languages, Google Fonts)
//! are downloaded from its newest version, and checked against the hash
//! GitHub's API gives for that file: git's own ID of its content, the SHA-1
//! of "blob SIZE\0" + the bytes. (GitHub publishes no SHA-256 for files in a
//! repository; git's SHA-1 is hardened against the known collision attacks.)
//! The font index carries its own SHA-256 (in latest.json).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// What a downloaded file must hash to.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expected {
    /// its SHA-256 (the font index)
    Sha256(String),
    /// git's ID of its content (a file in a GitHub repository)
    GitBlob(String),
}

impl Expected {
    /// The file's content hashes to what was expected.
    pub(crate) fn matches(&self, path: &Path) -> bool {
        match self {
            Expected::Sha256(sha) => sha256(path).as_deref() == Some(sha.as_str()),
            Expected::GitBlob(sha) => git_blob_id(path).as_deref() == Some(sha.as_str()),
        }
    }
}

/// A file's SHA-256 (sha256sum is in every distribution's coreutils).
pub(crate) fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// git's ID of a file's content (what `git hash-object` prints), with
/// sha1sum from coreutils: git itself may not be installed.
pub(crate) fn git_blob_id(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    let mut child = Command::new("sha1sum").stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().ok()?;
    {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(format!("blob {size}\0").as_bytes()).ok()?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = file.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            stdin.write_all(&buf[..n]).ok()?;
        }
    }
    let out = child.wait_with_output().ok()?;
    String::from_utf8_lossy(&out.stdout).split_whitespace().next().map(str::to_string)
}

/// The hashes GitHub gives for the files of a folder, asked once per folder
/// while the app runs (one request lists them all, and GitHub allows an app
/// without an account 60 requests an hour).
static LISTINGS: Mutex<Option<HashMap<String, HashMap<String, String>>>> = Mutex::new(None);

/// What `file` in folder `dir` of `repo` ("tesseract-ocr/tessdata_fast", "",
/// "eng.traineddata") must hash to, from GitHub's API.
pub(crate) fn github_file(repo: &str, dir: &str, file: &str) -> Result<Expected, String> {
    let key = format!("{repo}/{dir}");
    let cached = LISTINGS.lock().ok().and_then(|l| l.as_ref().and_then(|l| l.get(&key).cloned()));
    let listing = match cached {
        Some(listing) => listing,
        None => {
            let url = format!("https://api.github.com/repos/{repo}/contents/{dir}");
            let out = Command::new("curl")
                .args(["-fsSL", "--connect-timeout", "10", "--max-time", "30", "-H", "Accept: application/vnd.github+json"])
                .arg(&url)
                .output()
                .map_err(|_| "curl isn't installed".to_string())?;
            if !out.status.success() {
                return Err("GitHub didn't give the file's checksum (offline, or asked too often: try again in a while)".into());
            }
            let listing = parse_listing(&String::from_utf8_lossy(&out.stdout))?;
            if let Ok(mut l) = LISTINGS.lock() {
                l.get_or_insert_with(HashMap::new).insert(key, listing.clone());
            }
            listing
        }
    };
    listing.get(file).map(|sha| Expected::GitBlob(sha.clone())).ok_or(format!("GitHub has no {file} in {repo}"))
}

/// A GitHub contents listing → each file's name and git blob ID.
fn parse_listing(json: &str) -> Result<HashMap<String, String>, String> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(json).map_err(|e| format!("GitHub's file list: {e}"))?;
    Ok(entries
        .iter()
        .filter(|e| e["type"] == "file")
        .filter_map(|e| Some((e["name"].as_str()?.to_string(), e["sha"].as_str().filter(|s| s.len() == 40)?.to_string())))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_blob_ids_are_gits() {
        // the same as `printf abc | git hash-object --stdin` and an empty file's
        let dir = std::env::temp_dir().join(format!("archtoys-blob-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (abc, empty) = (dir.join("abc"), dir.join("empty"));
        std::fs::write(&abc, b"abc").unwrap();
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(git_blob_id(&abc).as_deref(), Some("f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f"));
        assert_eq!(git_blob_id(&empty).as_deref(), Some("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"));
        assert!(Expected::GitBlob("f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f".into()).matches(&abc));
        assert!(!Expected::GitBlob("f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f".into()).matches(&empty));
        assert!(Expected::Sha256("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into()).matches(&abc));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_github_listing_gives_each_files_id() {
        let json = r#"[
            {"name": "eng.traineddata", "type": "file", "sha": "bbef4675053b5b468cdb477053e28b1c698ba08e"},
            {"name": "script", "type": "dir", "sha": "0000000000000000000000000000000000000000"},
            {"name": "odd", "type": "file", "sha": "short"}
        ]"#;
        let listing = parse_listing(json).unwrap();
        assert_eq!(listing.get("eng.traineddata").map(String::as_str), Some("bbef4675053b5b468cdb477053e28b1c698ba08e"));
        assert_eq!(listing.len(), 1);
        assert!(parse_listing(r#"{"message": "API rate limit exceeded"}"#).is_err());
    }
}

#[cfg(test)]
mod hunt_tests {
    #[test]
    fn a_strange_github_answer_never_panics() {
        for text in crate::hunt::texts() {
            let _ = super::parse_listing(&text);
            let _ = super::parse_listing(&format!("[{{\"name\": {}, \"type\": \"file\", \"sha\": 5}}]", serde_json::Value::String(text)));
        }
    }
}
