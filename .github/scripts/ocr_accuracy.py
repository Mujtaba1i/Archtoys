#!/usr/bin/env python3
"""Text Extractor's accuracy on screen-like text (a CI step).

Draws known text at screen sizes (11-32 px), dark on light and light on dark,
in English, German and Russian, has Archtoys read each image
(`archtoys --extract-file`, the same code as a real capture), and writes a
table to the run summary: how much of the text came back right, the language
it named, how sure it was, and how long it took.

English at 14 px and up must come back at least 90% right in both themes, or
the step fails. Arabic at 14 px and up below 90% is a warning on the run (a
sentence and a short line are checked: a short line was once read as
English garble), never a failure: the README says languages other than
English may not be read accurately yet. Everything else is reported, not a
gate (the other languages are downloaded on the first run,
which also tests that). Arabic is drawn with pango-view: ImageMagick's
label: doesn't join or order right-to-left letters.

    python3 .github/scripts/ocr_accuracy.py target/release/archtoys
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
    # short lines: named French as English, and English as Turkish, before v0.7.5
    ("French short", "French", "Mot de passe oublié ?", "DejaVu-Sans"),
    ("English short", "English", "Network connection lost", "DejaVu-Sans"),
]
# the cases that must pass the gate, and the ones that only warn below it
GATED = ("English",)
WARNED = ("Arabic",)
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
    # ARCHTOYS_OCR_DEBUG: what it tried (each language's score), shown for the problems
    env = dict(os.environ, ARCHTOYS_OCR_DEBUG="1")
    out = subprocess.run([binary, "--extract-file", path], capture_output=True, text=True, env=env)
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
    binary = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/release/archtoys")
    rows = []
    with tempfile.TemporaryDirectory() as tmp:
        for name, language, text, font in CASES:
            for size in SIZES:
                for theme, (bg, fg) in THEMES.items():
                    path = os.path.join(tmp, f"{name}-{size}-{theme}.png".replace(" ", "_"))
                    draw(path, text, font, size, bg, fg)
                    fields, got, ms, err = read(binary, path)
                    rows.append(dict(name=name, size=size, theme=theme, acc=accuracy(text, got), expected=language,
                                     named=fields.get("language", "—"), sure=fields.get("sure", "—"), ms=ms,
                                     got=got, debug=err, download="Downloading" in err))

    def gated(r):
        return r["size"] >= 14 and r["name"].startswith(GATED)

    def warned(r):
        return r["size"] >= 14 and r["name"].startswith(WARNED)

    failures = [r for r in rows if gated(r) and r["acc"] < GATE]
    warnings = [r for r in rows if warned(r) and r["acc"] < GATE]
    problems = [r for r in rows if r["acc"] < GATE or r["named"] != r["expected"]]

    # one line per text: the whole picture at a glance
    lines = [
        "### Text Extractor accuracy" + (" ❌" if failures else " ⚠️" if problems else " ✅"),
        "",
        "| Text | Lowest (14 px +) | Average | Language right | Median time | Rule |",
        "|---|---|---|---|---|---|",
    ]
    for name, *_ in CASES:
        mine = [r for r in rows if r["name"] == name]
        big = [r["acc"] for r in mine if r["size"] >= 14]
        named = sum(r["named"] == r["expected"] for r in mine)
        times = sorted(r["ms"] for r in mine if not r["download"])
        median = f"{times[len(times) // 2]:.0f} ms" if times else "—"
        status = ("gate" if name.startswith(GATED) else "warns" if name.startswith(WARNED) else "report")
        mark = "✅" if all(r["acc"] >= GATE and r["named"] == r["expected"] for r in mine) else ("❌" if status == "gate" and min(big) < GATE else "⚠️")
        lines.append(f"| {name} | {min(big):.0%} | {sum(r['acc'] for r in mine) / len(mine):.0%} | {named} of {len(mine)} | {median} | {mark} {status} |")
    lines += ["", f"Gate: English at 14 px and up, {GATE:.0%}. Arabic below it warns. Sizes {SIZES[0]}-{SIZES[-1]} px, light and dark."]
    downloads = [r for r in rows if r["download"]]
    if downloads:
        lines.append(f"The first non-English text also downloaded the languages ({downloads[0]['ms'] / 1000:.1f} s, once): left out of the median times.")

    # only what went wrong, with what it read and why
    if problems:
        lines += ["", f"#### {len(problems)} below {GATE:.0%} or named wrong", "",
                  "| Text | Size | Theme | Right | Named | Sure | Read |", "|---|---|---|---|---|---|---|"]
        for r in problems:
            read_as = r["got"].replace("\n", " ").replace("|", "\\|")[:60]
            lines.append(f"| {r['name']} | {r['size']} px | {r['theme']} | {r['acc']:.0%} | {r['named']} | {r['sure']}% | `{read_as}` |")
        lines += ["", "<details><summary>What it tried for these (each language's score)</summary>", "", "```"]
        for r in problems:
            lines.append(f"== {r['name']} {r['size']} px {r['theme']}")
            lines += [l for l in r["debug"].splitlines() if l.startswith("ocr:")]
        lines += ["```", "", "</details>"]

    lines += ["", "<details><summary>Every case</summary>", "",
              "| Text | Size | Theme | Right | Language (expected → named) | Sure | Time |", "|---|---|---|---|---|---|---|"]
    for r in rows:
        mark = "✅" if r["expected"] == r["named"] else "⚠️"
        dl = " (+ download)" if r["download"] else ""
        lines.append(f"| {r['name']} | {r['size']} px | {r['theme']} | {r['acc']:.0%} | {mark} {r['expected']} → {r['named']} | {r['sure']}% | {r['ms']:.0f} ms{dl} |")
    lines += ["", "</details>"]

    report = "\n".join(lines)
    print(report)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(report + "\n")
    for r in warnings:
        # a yellow annotation on the run, not a failure
        print(f"::warning title=Text Extractor accuracy::{r['name']} {r['size']}px {r['theme']}: {r['acc']:.0%} right, named {r['named']}")
    if failures:
        print("\nBelow the gate:\n" + "\n".join(f"{r['name']} {r['size']}px {r['theme']}: {r['acc']:.0%}, read {r['got']!r}" for r in failures), file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
