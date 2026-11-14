#!/usr/bin/env python3
"""The font finder's accuracy (a CI step that only warns, never fails).

Draws short lines with pango-view (what GTK apps draw text with: slight
hinting, gray smoothing) in fonts the runner has, Regular and Bold, at a few
sizes, has Archtoys find each one (`archtoys --find-font`, the same code as
Find font in a capture, the fonts on this computer only: no download), and
writes a table to the run summary: the family first, in the top 3, the weight
right, and how long it took. Below 50% in the top 3, a warning on the run.
The font finder is a best guess by design (the app says so the first time).

    python3 .github/scripts/font_accuracy.py target/release/archtoys
"""
import os
import subprocess
import sys
import tempfile
import time

FONTS = ["DejaVu Sans", "DejaVu Serif", "Liberation Sans", "Liberation Serif", "Liberation Mono", "Noto Sans", "Noto Serif"]
LINES = ["Release notes for this week", "Your order has shipped", "Open the settings file"]
SIZES = [13, 16, 22]
WARN_BELOW = 50  # % in the top 3


def draw(font, bold, px, text, path):
    weight = " Bold" if bold else ""
    r = subprocess.run(["pango-view", "--no-display", f"--font={font}{weight} {px}px", "--background=white", "--foreground=black",
                        "--margin=10", "--hinting=slight", "--antialias=gray", f"--text={text}", "-o", path],
                       capture_output=True, env=dict(os.environ, LANG="C.UTF-8"))
    return r.returncode == 0


def guesses(binary, path):
    env = dict(os.environ, ARCHTOYS_FONTS_OFFLINE="1", HOME=tempfile.mkdtemp())
    for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME"):
        env.pop(name, None)
    t = time.time()
    out = subprocess.run([binary, "--find-font", path], capture_output=True, text=True, env=env, timeout=120).stdout
    took = time.time() - t
    found = []
    for line in out.splitlines():
        if line.startswith("guess: "):
            name = line[len("guess: "):].split(" | ")[0].strip()
            found.append((name.removesuffix(" Bold"), name.endswith(" Bold")))
    return found, took


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else "target/release/archtoys"
    rows, first, top3, weight, total, times = [], 0, 0, 0, 0, []
    path = os.path.join(tempfile.mkdtemp(), "line.png")
    for i, font in enumerate(FONTS):
        for bold in (False, True):
            px = SIZES[(i + bold) % len(SIZES)]
            text = LINES[(i + 2 * bold) % len(LINES)]
            if not draw(font, bold, px, text, path):
                continue
            found, took = guesses(binary, path)
            times.append(took)
            families = [f for f, _ in found[:3]]
            ok1 = bool(found) and found[0][0] == font
            ok3 = font in families
            okw = ok3 and found[families.index(font)][1] == bold
            first += ok1
            top3 += ok3
            weight += okw
            total += 1
            shown = ", ".join(f"{f}{' Bold' if b else ''}" for f, b in found[:3]) or "nothing"
            mark = "✅" if ok1 else ("🟡" if ok3 else "❌")
            rows.append(f"| {font}{' Bold' if bold else ''} | {px} px | {mark} | {shown} | {took:.2f} s |")
    if not total:
        print("pango-view drew nothing: is pango1.0-tools installed?")
        return 0
    pct = lambda n: round(100 * n / total)
    times.sort()
    summary = [
        "### Font finder (the fonts on the computer, warns only)",
        "",
        f"First: **{pct(first)}%** · in the top 3: **{pct(top3)}%** · weight right (when found): {pct(weight)}% · "
        f"{total} lines, median {times[len(times) // 2]:.2f} s",
        "",
        "| Drawn in | Size | | Archtoys' guesses (top 3) | Time |",
        "|---|---|---|---|---|",
        *rows,
        "",
        "✅ first, 🟡 in the top 3, ❌ not in it. Lines drawn by pango (GTK's text), slight hinting, gray smoothing.",
    ]
    text = "\n".join(summary)
    print(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as f:
            f.write(text + "\n")
    if pct(top3) < WARN_BELOW:
        print(f"::warning title=Font finder::only {pct(top3)}% of the lines had their font in the top 3 (warns below {WARN_BELOW}%)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
