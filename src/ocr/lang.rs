//! Languages: which one a text is in, and the language data Tesseract needs.
//!
//! Archtoys keeps its own language folder (`~/.local/share/archtoys/tessdata`):
//! the system's languages are linked into it, and any other language is
//! downloaded there the first time it is seen (Tesseract's `tessdata_fast`
//! files, Apache-2.0, 1-4 MB each; the licence is saved next to them). No
//! password is needed for that.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the language files come from.
const TESSDATA_URL: &str = "https://github.com/tesseract-ocr/tessdata_fast/raw/main";
/// The bigger, more accurate data, read with when the fast data wasn't sure (v0.7.5).
const BEST_URL: &str = "https://github.com/tesseract-ocr/tessdata_best/raw/main";

/// Every language `tessdata_fast` has (its file names without `.traineddata`),
/// except Hebrew and Yiddish (written in Hebrew letters): Archtoys reads no
/// Hebrew, by choice (see `EXCLUDED`).
pub(crate) const KNOWN: &[&str] = &[
    "afr",
    "amh",
    "ara",
    "asm",
    "aze",
    "aze_cyrl",
    "bel",
    "ben",
    "bod",
    "bos",
    "bre",
    "bul",
    "cat",
    "ceb",
    "ces",
    "chi_sim",
    "chi_sim_vert",
    "chi_tra",
    "chi_tra_vert",
    "chr",
    "cos",
    "cym",
    "dan",
    "deu",
    "deu_latf",
    "div",
    "dzo",
    "ell",
    "eng",
    "enm",
    "epo",
    "equ",
    "est",
    "eus",
    "fao",
    "fas",
    "fil",
    "fin",
    "fra",
    "frk",
    "frm",
    "fry",
    "gla",
    "gle",
    "glg",
    "grc",
    "guj",
    "hat",
    "hin",
    "hrv",
    "hun",
    "hye",
    "iku",
    "ind",
    "isl",
    "ita",
    "ita_old",
    "jav",
    "jpn",
    "jpn_vert",
    "kan",
    "kat",
    "kat_old",
    "kaz",
    "khm",
    "kir",
    "kmr",
    "kor",
    "kor_vert",
    "lao",
    "lat",
    "lav",
    "lit",
    "ltz",
    "mal",
    "mar",
    "mkd",
    "mlt",
    "mon",
    "mri",
    "msa",
    "mya",
    "nep",
    "nld",
    "nor",
    "oci",
    "ori",
    "osd",
    "pan",
    "pol",
    "por",
    "pus",
    "que",
    "ron",
    "rus",
    "san",
    "sin",
    "slk",
    "slv",
    "snd",
    "spa",
    "spa_old",
    "sqi",
    "srp",
    "srp_latn",
    "sun",
    "swa",
    "swe",
    "syr",
    "tam",
    "tat",
    "tel",
    "tgk",
    "tha",
    "tir",
    "ton",
    "tur",
    "uig",
    "ukr",
    "urd",
    "uzb",
    "uzb_cyrl",
    "vie",
    "yor",
];

/// Languages Archtoys never reads, even when the system has their data:
/// Hebrew, and Yiddish (written in Hebrew letters).
pub(crate) const EXCLUDED: &[&str] = &["heb", "yid"];

/// A letter of the Hebrew alphabet (a reading never keeps one).
pub(crate) fn hebrew(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x05FF | 0xFB1D..=0xFB4F)
}

fn data_home() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local").join("share")
}

/// Archtoys' own folder, under `~/.local/share/archtoys`.
pub(crate) fn app_data_dir() -> PathBuf {
    data_home().join("archtoys")
}

pub(crate) fn tessdata_dir() -> PathBuf {
    app_data_dir().join("tessdata")
}

/// The best data has the same file names, so it has its own folder.
pub(crate) fn best_dir() -> PathBuf {
    app_data_dir().join("tessdata-best")
}

/// Is a language's best data there, or on its way?
pub(crate) enum Best {
    Ready,
    Downloading,
}

/// The best data being downloaded in the background (one download each).
static BEST_DOWNLOADS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// A language's best data: there already, downloaded now if that takes no
/// longer than `wait`, else downloading in the background for next time.
/// The time is judged from the file's size and the speed measured over the
/// first moment, not a fixed timeout: a fast line finishes a big file, a slow
/// one isn't waited on for minutes.
pub(crate) fn get_best(code: &str, wait: std::time::Duration) -> Result<Best, String> {
    let dir = best_dir();
    if has(&dir, code) {
        return Ok(Best::Ready);
    }
    if !KNOWN.contains(&code) {
        return Err(format!("no language data for {code}"));
    }
    if BEST_DOWNLOADS.lock().is_ok_and(|d| d.iter().any(|c| c == code)) {
        return Ok(Best::Downloading);
    }
    std::fs::create_dir_all(&dir).map_err(|err| format!("couldn't make {}: {err}", dir.display()))?;
    let licence = dir.join("LICENSE-tessdata_best.txt");
    if !licence.exists() {
        let _ = fetch("https://raw.githubusercontent.com/tesseract-ocr/tessdata_best/main/LICENSE", &licence);
    }
    let url = format!("{BEST_URL}/{code}.traineddata");
    let dest = file_for(&dir, code);
    let tmp = dest.with_extension("part");
    let size = content_length(&url);
    let Ok(mut child) =
        Command::new("curl").args(["-fsSL", "--connect-timeout", "10", "--max-time", "900", "-o"]).arg(&tmp).arg(&url).spawn()
    else {
        // no curl: wget, in the background (it can't tell how far it got)
        let (code, tmp, dest, url) = (code.to_string(), tmp.clone(), dest.clone(), url.clone());
        background(code, move || {
            Command::new("wget").args(["-q", "-T", "20", "-O"]).arg(&tmp).arg(&url).status().is_ok_and(|s| s.success())
                && finish_part(&tmp, &dest)
        });
        return Ok(Best::Downloading);
    };
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|err| err.to_string())? {
            return if status.success() && finish_part(&tmp, &dest) {
                Ok(Best::Ready)
            } else {
                let _ = std::fs::remove_file(&tmp);
                Err(format!("couldn't download {url}"))
            };
        }
        let elapsed = start.elapsed();
        let got = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
        let too_slow = match size {
            Some(total) if elapsed.as_millis() >= 1000 && got > 0 => expected_time(total, got, elapsed) > wait,
            _ => elapsed > wait,
        };
        if too_slow {
            let (code, tmp, dest) = (code.to_string(), tmp.clone(), dest.clone());
            background(code, move || child.wait().is_ok_and(|s| s.success()) && finish_part(&tmp, &dest));
            return Ok(Best::Downloading);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// How long a whole download takes at the speed so far.
fn expected_time(total: u64, got: u64, elapsed: std::time::Duration) -> std::time::Duration {
    elapsed.mul_f64(total as f64 / got.max(1) as f64)
}

/// The file's size, from the server's headers (None when it doesn't say).
fn content_length(url: &str) -> Option<u64> {
    let out = Command::new("curl").args(["-sIL", "--connect-timeout", "10", "--max-time", "15"]).arg(url).output().ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse().ok()))
        .next_back() // after redirects: the last response's
}

/// A download that went on in the background finishes there.
fn background(code: String, job: impl FnOnce() -> bool + Send + 'static) {
    if let Ok(mut d) = BEST_DOWNLOADS.lock() {
        d.push(code.clone());
    }
    std::thread::spawn(move || {
        if !job() {
            eprintln!("text extractor: couldn't download the best data for {code}");
        }
        if let Ok(mut d) = BEST_DOWNLOADS.lock() {
            d.retain(|c| *c != code);
        }
    });
}

/// A finished download's temporary file becomes the real one (a cut-off one is thrown away).
fn finish_part(tmp: &Path, dest: &Path) -> bool {
    if std::fs::metadata(tmp).map(|m| m.len()).unwrap_or(0) < 100 {
        let _ = std::fs::remove_file(tmp);
        return false;
    }
    std::fs::rename(tmp, dest).is_ok()
}

fn file_for(dir: &Path, code: &str) -> PathBuf {
    dir.join(format!("{code}.traineddata"))
}

pub(crate) fn has(dir: &Path, code: &str) -> bool {
    file_for(dir, code).exists()
}

/// Makes the language folder and links every language the system has into it.
pub(crate) fn prepare_dir() -> Result<PathBuf, String> {
    let dir = tessdata_dir();
    std::fs::create_dir_all(&dir).map_err(|err| format!("couldn't make {}: {err}", dir.display()))?;
    if let Some(system) = super::engine::system_tessdata() {
        if let Ok(entries) = std::fs::read_dir(&system) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let target = dir.join(&name);
                let file = name.to_string_lossy();
                let excluded = EXCLUDED.iter().any(|c| file == format!("{c}.traineddata"));
                // a link left by a removed language is replaced
                if file.ends_with(".traineddata") && !excluded && !target.exists() {
                    let _ = std::fs::remove_file(&target);
                    let _ = std::os::unix::fs::symlink(entry.path(), &target);
                }
            }
        }
    }
    Ok(dir)
}

/// Downloads `url` to `dest` with curl or wget (one of them is on every
/// desktop), through a temporary file so a cut-off download leaves nothing.
fn fetch(url: &str, dest: &Path) -> Result<(), String> {
    let tmp = dest.with_extension("part");
    let curl = Command::new("curl").args(["-fsSL", "--connect-timeout", "10", "--max-time", "300", "-o"]).arg(&tmp).arg(url).status();
    let ok = match curl {
        Ok(status) => status.success(),
        Err(_) => Command::new("wget")
            .args(["-q", "-T", "20", "-O"])
            .arg(&tmp)
            .arg(url)
            .status()
            .map_err(|_| "neither curl nor wget is installed".to_string())?
            .success(),
    };
    let size = std::fs::metadata(&tmp).map(|m| m.len()).unwrap_or(0);
    if !ok || size < 100 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("couldn't download {url}"));
    }
    std::fs::rename(&tmp, dest).map_err(|err| format!("couldn't save {}: {err}", dest.display()))
}

/// Downloads a language into Archtoys' folder (and the licence, once).
pub(crate) fn download(dir: &Path, code: &str) -> Result<(), String> {
    if !KNOWN.contains(&code) {
        return Err(format!("no language data for {code}"));
    }
    let licence = dir.join("LICENSE-tessdata_fast.txt");
    if !licence.exists() {
        let _ = fetch("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/main/LICENSE", &licence);
    }
    fetch(&format!("{TESSDATA_URL}/{code}.traineddata"), &file_for(dir, code))
}

/// Downloads several languages at once (the licence first, once), each in
/// its own thread. Returns the ones that couldn't be downloaded.
pub(crate) fn download_all(dir: &Path, codes: &[&'static str]) -> Vec<&'static str> {
    let licence = dir.join("LICENSE-tessdata_fast.txt");
    if !licence.exists() {
        let _ = fetch("https://raw.githubusercontent.com/tesseract-ocr/tessdata_fast/main/LICENSE", &licence);
    }
    std::thread::scope(|scope| {
        let jobs: Vec<_> = codes.iter().map(|&code| (code, scope.spawn(move || download(dir, code)))).collect();
        jobs.into_iter().filter_map(|(code, job)| (!matches!(job.join(), Ok(Ok(())))).then_some(code)).collect()
    })
}

/// The languages Archtoys downloaded (not the system's linked ones), with
/// their total size in bytes.
pub(crate) fn downloaded() -> (Vec<String>, u64) {
    let mut codes = Vec::new();
    let mut bytes = 0;
    // the fast data and the best data (a language in both is listed once)
    for entries in [tessdata_dir(), best_dir()].iter().filter_map(|dir| std::fs::read_dir(dir).ok()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_link = entry.file_type().is_ok_and(|t| t.is_symlink());
            let Some(code) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".traineddata")) else {
                continue;
            };
            if !is_link {
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                codes.push(code.to_string());
            }
        }
    }
    codes.sort();
    codes.dedup();
    (codes, bytes)
}

/// Deletes the downloaded languages; they come back the next time they're needed.
pub(crate) fn delete_downloaded() {
    for code in downloaded().0 {
        let _ = std::fs::remove_file(file_for(&tessdata_dir(), &code));
        let _ = std::fs::remove_file(file_for(&best_dir(), &code));
    }
}

/// whatlang's language → Tesseract's language code (mostly the same ISO
/// 639-3 code), or None when Tesseract has no data for it.
pub(crate) fn from_whatlang(lang: whatlang::Lang) -> Option<&'static str> {
    let code = match lang.code() {
        "cmn" => "chi_sim",
        "nob" => "nor",
        "pes" => "fas",
        "zlm" => "msa",
        "tgl" => "fil",
        code => code,
    };
    KNOWN.iter().find(|&&k| k == code).copied()
}

/// Short Latin-script text whatlang isn't sure of (a button, a title, a few
/// words: it guessed Afrikaans for "Fichier enregistré" and French for
/// "Network connection lost"): the language its letters and common words
/// show. Letters only a few languages use count most, then the little words
/// every sentence has, then a nudge for the user's own languages. A tie goes
/// to whatlang among the tied ones. No evidence, or English ahead: English.
/// Before (v0.7.4) a short line kept the language its reading happened to
/// win with (English, or Turkish or Vietnamese, whose data reads accents).
/// `read_with`: the data that read it. Turkish's turns è into ö and ¿ into ç,
/// Vietnamese's sprinkles accents: those letters aren't evidence then (only
/// the ones its own language alone has, ş ğ ı or đ ơ ư, still are).
pub(crate) fn name_latin(text: &str, user: &[&str], read_with: &str) -> &'static str {
    // (code, letters only it and a few others use, its most common little words)
    const EVIDENCE: &[(&str, &str, &[&str])] = &[
        (
            "eng",
            "",
            &[
                "the", "a", "an", "to", "of", "and", "in", "on", "for", "your", "you", "is", "are", "with", "this", "that", "here", "more",
                "it", "be", "not", "all", "my", "has", "have", "off", "or", "by", "from", "at",
            ],
        ),
        (
            "fra",
            "éçèêëœàâîïôûù",
            &[
                "le", "la", "les", "de", "du", "des", "un", "une", "et", "pour", "ici", "votre", "vos", "est", "pas", "sur", "avec",
                "dans", "tout", "au", "aux", "ce", "cette", "mon", "ma", "mes", "ou", "par", "qui", "que", "en", "ne", "il", "elle",
                "nous", "vous",
            ],
        ),
        (
            "spa",
            "ñ¿¡áíóú",
            &[
                "el", "la", "los", "las", "de", "del", "un", "una", "y", "para", "por", "tu", "su", "más", "con", "en", "es", "no", "al",
                "que", "mi", "se", "lo", "como", "este", "esta",
            ],
        ),
        (
            "por",
            "ãõçáâêôàóú",
            &[
                "o", "os", "as", "de", "do", "da", "dos", "das", "um", "uma", "e", "para", "por", "você", "seu", "sua", "com", "em", "não",
                "ao", "que", "no", "na", "se", "mais",
            ],
        ),
        (
            "deu",
            "äöüß",
            &[
                "der", "die", "das", "den", "dem", "des", "und", "ist", "nicht", "mit", "für", "zu", "ein", "eine", "auf", "im", "von",
                "sie", "es", "ich", "du", "ihr", "wir", "sind", "oder", "auch", "noch", "bitte",
            ],
        ),
        (
            "ita",
            "àèìòù",
            &[
                "il", "lo", "la", "gli", "le", "di", "del", "della", "un", "una", "e", "per", "con", "non", "che", "è", "in", "su", "al",
                "sono", "questo", "tuo",
            ],
        ),
        (
            "nld",
            "ĳ",
            &["de", "het", "een", "en", "van", "is", "niet", "met", "voor", "op", "je", "jouw", "uw", "zijn", "dat", "naar", "ook"],
        ),
        ("pol", "ąćęłńśźż", &["i", "w", "z", "na", "do", "nie", "się", "jest", "to", "że", "od", "po", "jak", "dla", "twoje"]),
        ("tur", "şğıİŞĞ", &["ve", "bir", "bu", "için", "ile", "da", "de", "mi", "ne", "çok", "daha", "olarak"]),
        (
            "vie",
            "đơưạảấầẩẫậắằẳẵặẹẻẽếềểễệỉịọỏốồổỗộớờởỡợụủứừửữựỳỵỷỹĐ",
            &["và", "của", "là", "có", "không", "cho", "các", "những", "một", "được", "này", "bạn"],
        ),
    ];
    let invented = match read_with {
        "tur" => "çöü",
        "vie" => "àáâãèéêìíòóôõùúý",
        _ => "",
    };
    let lower: String = text.to_lowercase().chars().filter(|c| !invented.contains(*c)).collect();
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let scores: Vec<(&'static str, u32)> = EVIDENCE
        .iter()
        .map(|&(code, letters, common)| {
            let letter_hits = lower.chars().filter(|c| letters.contains(*c)).count() as u32;
            let word_hits = words.iter().filter(|w| common.contains(w)).count() as u32;
            (code, 3 * letter_hits.min(2) + 2 * word_hits + u32::from(user.contains(&code) && letter_hits + word_hits > 0))
        })
        .collect();
    let best = scores.iter().map(|s| s.1).max().unwrap_or(0);
    let english = scores[0].1;
    if best == 0 || english >= best {
        return "eng";
    }
    let tied: Vec<&'static str> = scores.iter().filter(|s| s.1 == best).map(|s| s.0).collect();
    if tied.len() == 1 {
        return tied[0];
    }
    let allow: Vec<whatlang::Lang> = tied.iter().filter_map(|c| whatlang::Lang::from_code(*c)).collect();
    whatlang::Detector::with_allowlist(allow)
        .detect_lang(text)
        .and_then(|l| tied.iter().copied().find(|c| Some(*c) == from_whatlang(l)))
        .unwrap_or(tied[0])
}

/// The language's name in English ("German"), for the history and the toolbar.
pub(crate) fn display_name(code: &str) -> String {
    let special = match code {
        "chi_sim" => Some("Chinese (simplified)"),
        "chi_tra" => Some("Chinese (traditional)"),
        "fas" => Some("Persian"),
        "nor" => Some("Norwegian"),
        "msa" => Some("Malay"),
        "fil" => Some("Filipino"),
        "osd" => Some("Unknown"),
        "bod" => Some("Tibetan"),
        "sin" => Some("Sinhala"),
        _ => None,
    };
    if let Some(name) = special {
        return name.to_string();
    }
    whatlang::Lang::from_code(code).map(|l| l.eng_name().to_string()).unwrap_or_else(|| code.to_string())
}

/// The language the desktop is set to ("de_DE.UTF-8" → "deu"), when Tesseract knows it.
pub(crate) fn system_language() -> Option<&'static str> {
    let value = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")?;
    let two = value.split(['_', '.', '@']).next()?.to_ascii_lowercase();
    let code = match two.as_str() {
        "en" => "eng",
        "de" => "deu",
        "fr" => "fra",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "nld",
        "ar" => "ara",
        "ru" => "rus",
        "uk" => "ukr",
        "tr" => "tur",
        "pl" => "pol",
        "cs" => "ces",
        "sk" => "slk",
        "zh" => "chi_sim",
        "ja" => "jpn",
        "ko" => "kor",
        "hi" => "hin",
        "el" => "ell",
        "sv" => "swe",
        "da" => "dan",
        "fi" => "fin",
        "nb" | "no" | "nn" => "nor",
        "hu" => "hun",
        "ro" => "ron",
        "fa" => "fas",
        "vi" => "vie",
        "id" => "ind",
        "th" => "tha",
        "bg" => "bul",
        "hr" => "hrv",
        "sr" => "srp",
        "ca" => "cat",
        _ => return None,
    };
    Some(code)
}

/// The languages this user reads, besides English: the desktop's language,
/// the keyboard layouts and the input methods (Chinese, Japanese, Korean).
/// Tesseract's script detection is unreliable on a few words of Korean,
/// Chinese or Japanese, so the user's own languages are read first.
pub(crate) fn user_languages() -> &'static [&'static str] {
    static LANGS: std::sync::OnceLock<Vec<&'static str>> = std::sync::OnceLock::new();
    LANGS.get_or_init(|| {
        let config = |path: &str| std::fs::read_to_string(config_home().join(path)).unwrap_or_default();
        let run = |cmd: &str, args: &[&str]| {
            Command::new(cmd).args(args).output().map(|out| String::from_utf8_lossy(&out.stdout).into_owned()).unwrap_or_default()
        };
        let mut names = Vec::new();
        names.extend(kde_layouts(&config("kxkbrc")));
        names.extend(fcitx5_methods(&config("fcitx5/profile")));
        if std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_lowercase().contains("gnome") {
            names.extend(gnome_sources(&run("gsettings", &["get", "org.gnome.desktop.input-sources", "sources"])));
        }
        if names.is_empty() {
            names.extend(setxkbmap_layouts(&run("setxkbmap", &["-query"])));
        }
        let mut langs: Vec<&'static str> = system_language().into_iter().collect();
        for code in names.iter().filter_map(|n| from_input(n)) {
            if code != "eng" && !langs.contains(&code) {
                langs.push(code);
            }
        }
        langs.retain(|c| *c != "eng");
        langs
    })
}

fn config_home() -> PathBuf {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".config"),
    }
}

/// KDE: `LayoutList=us,ara` in kxkbrc.
fn kde_layouts(text: &str) -> Vec<String> {
    text.lines()
        .find_map(|l| l.trim().strip_prefix("LayoutList="))
        .map(|list| list.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default()
}

/// fcitx5: `Name=keyboard-us`, `Name=pinyin`, `Name=hangul` in its profile.
fn fcitx5_methods(text: &str) -> Vec<String> {
    text.lines().filter_map(|l| l.trim().strip_prefix("Name=")).map(str::to_string).collect()
}

/// GNOME: `[('xkb', 'us'), ('xkb', 'ara'), ('ibus', 'hangul')]`.
fn gnome_sources(text: &str) -> Vec<String> {
    text.split('(').skip(1).filter_map(|item| item.split('\'').nth(3).map(str::to_string)).collect()
}

/// X11: `layout:     us,ara` from `setxkbmap -query`.
fn setxkbmap_layouts(text: &str) -> Vec<String> {
    kde_layouts(&text.replace("layout:", "LayoutList=").replace(' ', ""))
}

/// A keyboard layout ("ara", "de", "keyboard-de", "xkb:il::heb") or an input
/// method ("hangul", "pinyin", "mozc") → its Tesseract language.
fn from_input(name: &str) -> Option<&'static str> {
    let name = name.trim().to_ascii_lowercase();
    let name = name.strip_prefix("keyboard-").unwrap_or(&name);
    // ibus: "xkb:us::eng"; a layout's variant: "de(nodeadkeys)"
    let name = name.strip_prefix("xkb:").map(|rest| rest.split(':').next().unwrap_or("")).unwrap_or(name);
    let name = name.split(['(', '+', '-']).next().unwrap_or("");
    Some(match name {
        "hangul" => "kor",
        "pinyin" | "libpinyin" | "rime" | "shuangpin" | "wubi" | "sunpinyin" | "googlepinyin" | "cn" => "chi_sim",
        "chewing" | "cangjie" | "quick" | "tw" | "zhuyin" => "chi_tra",
        "mozc" | "anthy" | "kkc" | "skk" | "jp" => "jpn",
        "kr" => "kor",
        "us" | "gb" | "uk" | "en" => "eng",
        "ara" | "ar" | "eg" | "iq" | "ma" | "dz" | "sy" => "ara",
        "de" | "at" | "ch" => "deu",
        "fr" | "be" => "fra",
        "es" | "latam" => "spa",
        "it" => "ita",
        "pt" | "br" => "por",
        "nl" => "nld",
        "ru" => "rus",
        "ua" => "ukr",
        "by" => "bel",
        "bg" => "bul",
        "rs" => "srp",
        "gr" => "ell",
        "tr" => "tur",
        "pl" => "pol",
        "cz" => "ces",
        "sk" => "slk",
        "hu" => "hun",
        "ro" => "ron",
        "se" => "swe",
        "no" => "nor",
        "dk" => "dan",
        "fi" => "fin",
        "ir" => "fas",
        "pk" => "urd",
        "in" => "hin",
        "th" => "tha",
        "vn" => "vie",
        "id" => "ind",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_download_is_judged_by_its_size_and_speed() {
        let s = std::time::Duration::from_secs;
        // 15 MB, 1.5 MB in the first second: 10 s in all (too long for an 8 s wait)
        assert_eq!(expected_time(15_000_000, 1_500_000, s(1)), s(10));
        // the same file on a fast line: half a second
        assert_eq!(expected_time(15_000_000, 30_000_000, s(1)), std::time::Duration::from_millis(500));
    }

    #[test]
    fn short_latin_lines_are_named_by_their_letters_and_little_words() {
        // what Tesseract really read (v0.7.5 tests: 16 px, two fonts, both themes)
        let cases = [
            ("eng", "Open the file"),
            ("eng", "Settings saved"),
            ("eng", "Network connection lost"), // whatlang: French 36%
            ("eng", "Forgot password?"),        // whatlang: Afrikaans
            ("eng", "Download complete"),
            ("eng", "Last updated yesterday"),
            ("eng", "Add to cart"),
            ("fra", "Fichier enregistré"), // whatlang: Afrikaans
            ("fra", "Mot de passe oublié ?"),
            ("fra", "Cliquez ici pour continuer"),
            ("fra", "Paramétres du compte"), // read in English: è came back é
            ("fra", "Télécharger le fichier"),
            ("spa", "¿Olvidaste tu contraseña?"),
            ("spa", "Configuración de la cuenta"), // was named Vietnamese
            ("deu", "Einstellungen öffnen"),
            ("tur", "Şifremi unuttum"),
            ("vie", "Đã lưu tệp tin"),
        ];
        for (want, text) in cases {
            assert_eq!(name_latin(text, &[], "eng"), want, "{text:?}");
        }
        // no evidence at all: English, whatever reading won (Turkish read it best, v0.7.4 said Turkish)
        assert_eq!(name_latin("Dosya kaydedildi", &[], "eng"), "eng");
        // a user's own language tips a close call ("en la" is French and Spanish alike)
        assert_eq!(name_latin("en la", &["spa"], "eng"), "spa");
        assert_eq!(name_latin("en la", &["fra"], "eng"), "fra");
        // read with Turkish's data, which made è an ö and ¿ a ç (it named them German and Portuguese)
        assert_eq!(name_latin("Paramötres du compte", &[], "tur"), "fra");
        assert_eq!(name_latin("çOlvidaste tu contrasefia?", &[], "tur"), "spa");
        // but Turkish's own letters still say Turkish
        assert_eq!(name_latin("Şifremi unuttum", &[], "tur"), "tur");
    }

    use super::*;

    #[test]
    fn whatlang_languages_map_to_tesseract_codes() {
        assert_eq!(from_whatlang(whatlang::Lang::Eng), Some("eng"));
        assert_eq!(from_whatlang(whatlang::Lang::Deu), Some("deu"));
        assert_eq!(from_whatlang(whatlang::Lang::Cmn), Some("chi_sim"));
        assert_eq!(from_whatlang(whatlang::Lang::Pes), Some("fas"));
        assert_eq!(from_whatlang(whatlang::Lang::Ara), Some("ara"));
        assert_eq!(display_name("deu"), "German");
        assert_eq!(display_name("chi_sim"), "Chinese (simplified)");
        assert_eq!(KNOWN.len(), 124);
        assert!(EXCLUDED.iter().all(|c| !KNOWN.contains(c))); // no Hebrew, by choice
        assert!(hebrew('ש') && hebrew('\u{FB2A}') && !hebrew('a') && !hebrew('ع'));
    }

    #[test]
    fn the_users_languages_come_from_layouts_and_input_methods() {
        assert_eq!(kde_layouts("[Layout]\nLayoutList=us,ara\nUse=true\n"), vec!["us", "ara"]);
        assert_eq!(
            fcitx5_methods("[Groups/0/Items/0]\nName=keyboard-us\n[Groups/0/Items/1]\nName=hangul\n"),
            vec!["keyboard-us", "hangul"]
        );
        assert_eq!(gnome_sources("[('xkb', 'us'), ('xkb', 'gr'), ('ibus', 'mozc-jp')]"), vec!["us", "gr", "mozc-jp"]);
        assert_eq!(setxkbmap_layouts("rules:      evdev\nlayout:     us,de\n"), vec!["us", "de"]);
        let langs: Vec<_> = ["keyboard-us", "hangul", "gr", "mozc-jp", "xkb:ara::ara", "de(nodeadkeys)", "pinyin", "klingon"]
            .iter()
            .map(|n| from_input(n))
            .collect();
        assert_eq!(langs, [Some("eng"), Some("kor"), Some("ell"), Some("jpn"), Some("ara"), Some("deu"), Some("chi_sim"), None]);
    }

    #[test]
    #[ignore = "prints whatlang's confidence for sample texts"]
    fn whatlang_confidence_report() {
        for text in [
            "Die Rechnung ist bis zum Ende des Monats zu bezahlen, sonst kommen Gebuhren dazu.",
            "The quick brown fox jumps over the lazy dog. Pack my box with five dozen jugs.",
            "ERROR: connection refused (port 5432)",
            "Rechnung Nr. 2026-114",
        ] {
            let info = whatlang::detect(text);
            println!("{text:?} -> {:?}", info.map(|i| (i.lang(), i.confidence(), i.is_reliable())));
        }
    }

    #[test]
    fn whatlang_tells_common_languages_apart() {
        let german = "Die Rechnung ist bis zum Ende des Monats zu bezahlen, sonst kommen Mahngebühren dazu.";
        let lang = whatlang::detect(german).map(|i| i.lang());
        assert_eq!(lang.and_then(from_whatlang), Some("deu"));
    }
}
