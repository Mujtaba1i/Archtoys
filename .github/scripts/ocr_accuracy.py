#!/usr/bin/env python3
"""Text Extractor's accuracy on screen-like text (a CI step).

Draws known text at screen sizes (11-32 px), dark on light and light on dark,
in English, German and Russian, has Archtoys read each image
(`archtoys --extract-file`, the same code as a real capture), and writes a
table to the run summary: how much of the text came back right, the language
it named, how sure it was, and how long it took.

English and Arabic at 14 px and up must come back at least 90% right in both
themes, or the step fails (Arabic: a short line read as English garble once,
so a sentence and a short line are both checked). Everything else is
reported, not a gate (the other languages are downloaded on the first run,
which also tests that). Arabic is drawn with pango-view: ImageMagick's
label: doesn't join or order right-to-left letters.

    python3 .github/scripts/ocr_accuracy.py target/debug/archtoys
"""
import os
import subprocess
import sys
import tempfile
import time

CASES = [
    # (name, language Archtoys should name, text, font)
    ("English", "English", "The quick brown fox jumps over the lazy dog. Pack my box with five dozen jugs.", "DejaVu-Sans"),
    ("English serif", "English", "Archtoys picks any pixel on your screen and copies the text in it.", "Liberation-Serif"),
    ("German", "German", "Die Rechnung ist bis zum Ende des Monats zu bezahlen, sonst kommen Gebühren dazu.", "DejaVu-Sans"),
    ("Russian", "Russian", "Съешь же ещё этих мягких французских булок, да выпей чаю.", "DejaVu-Sans"),
    ("Arabic", "Arabic", "يقرأ مستخرج النصوص أي نص على الشاشة حتى في الصور والفيديو", "Noto Sans Arabic"),
    ("Arabic short", "Arabic", "هذا نص عربي قصير", "Noto Sans Arabic"),
]
# the cases that must pass the gate
GATED = ("English", "Arabic")
SIZES = [11, 14, 18, 24, 32]
THEMES = {"light": ("#ffffff", "#16181d"), "dark": ("#1d1f25", "#e6e8ec")}
GATE = 0.90


def distance(a, b):
    """Levenshtein distance."""
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


def accuracy(expected, got):
    e = " ".join(expected.split())
    g = " ".join(got.split())
    if not e:
        return 1.0
    return max(0.0, 1.0 - distance(e, g) / len(e))


def draw(path, text, font, size, background, foreground):
    if any("\u0600" <= c <= "\u06ff" for c in text):
        # 72 dpi: the size is in pixels, like the other cases
        subprocess.run(
            ["pango-view", "-q", "--dpi=72", f"--font={font} {size}", f"--background={background}",
             f"--foreground={foreground}", "--margin=12", f"--text={text}", "-o", path],
            check=True,
        )
        return
    convert = "magick" if subprocess.run(["which", "magick"], capture_output=True).returncode == 0 else "convert"
    subprocess.run(
        [convert, "-background", background, "-fill", foreground, "-font", font, "-pointsize", str(size),
         "-bordercolor", background, f"label:{text}", "-border", "12", path],
        check=True,
    )


def read(binary, path):
    started = time.perf_counter()
    out = subprocess.run([binary, "--extract-file", path], capture_output=True, text=True)
    ms = (time.perf_counter() - started) * 1000
    fields, text = {}, ""
    if "---\n" in out.stdout:
        head, text = out.stdout.split("---\n", 1)
        for line in head.splitlines():
            if ": " in line:
                k, v = line.split(": ", 1)
                fields[k] = v
    return fields, text.strip(), ms, out.stderr.strip()


def main():
    binary = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/debug/archtoys")
    rows, failures = [], []
    with tempfile.TemporaryDirectory() as tmp:
        for name, language, text, font in CASES:
            for size in SIZES:
                for theme, (bg, fg) in THEMES.items():
                    path = os.path.join(tmp, f"{name}-{size}-{theme}.png".replace(" ", "_"))
                    draw(path, text, font, size, bg, fg)
                    fields, got, ms, err = read(binary, path)
                    acc = accuracy(text, got)
                    rows.append((name, size, theme, acc, language, fields.get("language", "—"), fields.get("sure", "—"), ms))
                    if name.startswith(GATED) and size >= 14 and acc < GATE:
                        failures.append(f"{name} {size}px {theme}: {acc:.0%} right, read: {got!r} {err}")

    lines = [
        "### Text Extractor accuracy",
        "",
        "| Text | Size | Theme | Right | Language (expected → named) | Sure | Time |",
        "|---|---|---|---|---|---|---|",
    ]
    for name, size, theme, acc, expected, named, sure, ms in rows:
        mark = "✅" if expected == named else "⚠️"
        lines.append(f"| {name} | {size} px | {theme} | {acc:.0%} | {mark} {expected} → {named} | {sure}% | {ms:.0f} ms |")
    gated = [r[3] for r in rows if r[0].startswith(GATED) and r[1] >= 14]
    lines += ["", f"English and Arabic at 14 px and up: lowest {min(gated):.0%} (gate {GATE:.0%})."]
    named_right = sum(1 for r in rows if r[4] == r[5])
    lines.append(f"Language named right: {named_right} of {len(rows)}.")
    report = "\n".join(lines)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(report + "\n")
    if failures:
        print("\nBelow the gate:\n" + "\n".join(failures), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
