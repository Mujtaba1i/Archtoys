#!/usr/bin/env python3
"""Archtoys end to end, the way a person uses it (a CI step).

Runs the app on a virtual X11 screen (Xvfb) with a fresh profile and uses it
with a real mouse and keyboard (xdotool): picks colors from colored squares,
clicks shades, history, copy buttons, searches, opens the editor, the formats
panel, Settings and the sidebar, moves between the pages, extracts text with
Text Extractor, and opens Report a problem. Each check is verified by what a
user would get (the color in config.json, the clipboard, the pixels on
screen, files written), never by the app reporting on itself.

Every check runs even when an earlier one failed; the summary (stdout and the
run's summary page) lists each one, and the step fails if any failed.

    dbus-run-session -- python3 .github/scripts/e2e_test.py target/release/archtoys

Needs: Xvfb, xdotool, xclip, python3-xlib, dbus (for a second launch), and for
the Text Extractor check tesseract, python3-pillow and openbox (the capture
goes full screen, which on X11 a window manager does; skipped without them).
It never touches your own session: its own X screen, profile and runtime
folder, so it also runs next to an Archtoys you're using. Positions are
for the default window (520 x 560) at 100%: after a layout change, run with
--shot to save a screenshot of each screen and update the positions below.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

from Xlib import X, display

BIN = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else "target/release/archtoys")
SHOTS = "--shot" in sys.argv
DISPLAY_NUM = ":95"
SCREEN = (1600, 1000)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

results = []  # (area, check, ok, detail)


# ---------------------------------------------------------------- plumbing
def run(*cmd, timeout=10, env=None, check=False):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env, check=check)


def xdo(*args):
    return run("xdotool", *[str(a) for a in args]).stdout.strip()


def test(area, name):
    """Decorator: runs the check now, records ok / failed / skipped, never stops."""
    def wrap(fn):
        try:
            outcome = fn()
            if outcome is None:
                outcome = (True, "")
            ok, detail = outcome if isinstance(outcome, tuple) else (bool(outcome), "")
        except Skip as skip:
            ok, detail = None, str(skip)
        except Exception as err:  # a failed check must not stop the others
            ok, detail = False, f"{type(err).__name__}: {err}"
        results.append((area, name, ok, detail))
        mark = {True: "PASS", False: "FAIL", None: "SKIP"}[ok]
        print(f"[{mark}] {area}: {name}" + (f" ({detail})" if detail else ""), flush=True)
        return fn
    return wrap


class Skip(Exception):
    pass


def wait_for(predicate, timeout=5.0, step=0.1):
    end = time.time() + timeout
    last = None
    while time.time() < end:
        last = predicate()
        if last:
            return last
        time.sleep(step)
    return last


class App:
    """One Archtoys run with its own profile (HOME)."""

    def __init__(self, config=None, extra_env=None, args=()):
        self.home = tempfile.mkdtemp(prefix="archtoys-e2e-")
        os.makedirs(self.config_dir, exist_ok=True)
        if config is not None:
            with open(self.config_path, "w") as f:
                json.dump(config, f)
        # Report a problem opens a browser: a fake xdg-open keeps the link
        self.bin_dir = os.path.join(self.home, "bin")
        os.makedirs(self.bin_dir)
        with open(os.path.join(self.bin_dir, "xdg-open"), "w") as f:
            f.write(f'#!/bin/sh\nprintf "%s\\n" "$1" >> "{self.home}/opened.txt"\n')
        os.chmod(os.path.join(self.bin_dir, "xdg-open"), 0o755)
        # its own runtime folder: on your own computer, the Archtoys you're running holds the
        # one-copy lock in the real one, and this copy would just hand over to it and quit
        runtime = tempfile.mkdtemp(prefix="archtoys-e2e-run-")
        os.chmod(runtime, 0o700)
        self.env = dict(os.environ, HOME=self.home, DISPLAY=DISPLAY_NUM, XDG_SESSION_TYPE="x11",
                        XDG_RUNTIME_DIR=runtime, XDG_CURRENT_DESKTOP="",
                        PATH=self.bin_dir + ":" + os.environ["PATH"], ARCHTOYS_HOTKEY_BACKEND="x11")
        self.env.pop("WAYLAND_DISPLAY", None)
        self.env.update(extra_env or {})
        self.log = open(os.path.join(self.home, "app.log"), "w")
        self.proc = subprocess.Popen([BIN, *args], env=self.env, stdout=self.log, stderr=subprocess.STDOUT)
        self.win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys$").split("\n")[0], 10)
        if self.win:
            xdo("windowmove", self.win, 0, 0)
            self.focus()
            time.sleep(1.0)

    def focus(self):
        """The keyboard to the window (a desktop does it on a click; Xvfb has no desktop)."""
        xdo("windowfocus", "--sync", self.win)

    config_dir = property(lambda self: os.path.join(self.home, ".config", "archtoys-color-picker"))
    config_path = property(lambda self: os.path.join(self.config_dir, "config.json"))

    def config(self):
        try:
            with open(self.config_path) as f:
                return json.load(f)
        except (OSError, ValueError):
            return {}

    def output(self):
        self.log.flush()
        with open(os.path.join(self.home, "app.log")) as f:
            return f.read()

    def geometry(self):
        out = xdo("getwindowgeometry", self.win)
        pos = out.split("Position: ")[1].split(" ")[0].split(",")
        size = out.split("Geometry: ")[1].split("x")
        return int(pos[0]), int(pos[1]), int(size[0]), int(size[1])

    def click(self, x, y, button=1, wait=0.6):
        # relative to the window itself: a window manager's title bar doesn't shift it
        xdo("mousemove", "--window", self.win, x, y)
        time.sleep(0.4)  # rest on it first, like a hand: some buttons react to the hover
        xdo("click", button)
        time.sleep(wait)

    def key(self, *keys, wait=0.4):
        self.focus()
        for k in keys:
            xdo("key", "--clearmodifiers", k)
            time.sleep(0.08)
        time.sleep(wait)

    def type(self, text, wait=0.5):
        self.focus()
        xdo("type", "--delay", "40", text)
        time.sleep(wait)

    def pixel(self, x, y):
        wx, wy, _, _ = self.geometry()
        return screen_pixel(wx + x, wy + y)

    def shot(self, name):
        if SHOTS:
            path = os.path.join(REPO, f"e2e-{name}.png")
            run("import", "-display", DISPLAY_NUM, "-window", "root", path)
            print(f"  (saved {path})")

    def running(self):
        return self.proc.poll() is None

    def quit(self):
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self.log.close()


_x = None


def screen_pixel(x, y):
    global _x
    if _x is None:
        _x = display.Display(DISPLAY_NUM)
    raw = _x.screen().root.get_image(x, y, 1, 1, X.ZPixmap, 0xFFFFFFFF).data
    return raw[2], raw[1], raw[0]  # BGRX


def clipboard():
    try:
        return run("xclip", "-display", DISPLAY_NUM, "-o", "-selection", "clipboard", timeout=3).stdout
    except subprocess.TimeoutExpired:
        return ""


def close_window(win):
    """The window's close button, the way a desktop sends it (WM_DELETE_WINDOW)."""
    from Xlib import protocol
    d = display.Display(DISPLAY_NUM)
    w = d.create_resource_object("window", int(win))
    proto, delete = d.intern_atom("WM_PROTOCOLS"), d.intern_atom("WM_DELETE_WINDOW")
    w.send_event(protocol.event.ClientMessage(window=w, client_type=proto, data=(32, [delete, X.CurrentTime, 0, 0, 0])),
                 event_mask=X.NoEventMask)
    d.flush()
    d.close()


# Colored squares on the screen to pick from, painted into the screen's
# background (the X server keeps it painted). Not windows of their own: a
# window manager keeps those above a full-screen window, and the mouse over
# a square would go to the square, not to the frozen screen. Away from the
# Archtoys window.
SWATCHES = {"blue": (0x33, 0x66, 0xCC), "red": (0xE0, 0x40, 0x30), "green": (0x20, 0xA0, 0x60), "yellow": (0xF0, 0xC0, 0x20)}
SWATCH_AT = {name: (1000 + i * 140, 700) for i, name in enumerate(SWATCHES)}  # top left, 120 x 120
# For the ruler's tolerance: a 200 x 60 strip of two grays 20 apart (each half
# 100 wide), on the background (#404850, 64 and 84 away from them).
# the ruler's toolbar with − and + buttons (frozen screen, centered at the top)
RULER_MINUS, RULER_PLUS = (893, 46), (1019, 46)
STRIP_AT, STRIP = (200, 880), ((0x80, 0x80, 0x80), (0x94, 0x94, 0x94))


def show_swatches():
    d = display.Display(DISPLAY_NUM)
    screen = d.screen()
    background = screen.root.create_pixmap(SCREEN[0], SCREEN[1], screen.root_depth)
    gc = background.create_gc(foreground=0x404850)
    background.fill_rectangle(gc, 0, 0, SCREEN[0], SCREEN[1])
    for name, (r, g, b) in SWATCHES.items():
        gc.change(foreground=(r << 16) | (g << 8) | b)
        background.fill_rectangle(gc, *SWATCH_AT[name], 120, 120)
    for half, (r, g, b) in enumerate(STRIP):
        gc.change(foreground=(r << 16) | (g << 8) | b)
        background.fill_rectangle(gc, STRIP_AT[0] + half * 100, STRIP_AT[1], 100, 60)
    screen.root.change_attributes(background_pixmap=background)
    screen.root.clear_area(0, 0, 0, 0)
    d.flush()
    return d  # keep the connection: closing it would free the background


def swatch_center(name):
    x, y = SWATCH_AT[name]
    return x + 60, y + 60


def hexcolor(rgb):
    return "#%02X%02X%02X" % tuple(rgb)


def last_color(app):
    c = app.config().get("last_color")
    return tuple(c) if c else None


# ---------------------------------------------------------------- the checks
def main():
    if not os.access(BIN, os.X_OK):
        print(f"no binary at {BIN}")
        return 2
    xvfb = subprocess.Popen(["Xvfb", DISPLAY_NUM, "-screen", "0", f"{SCREEN[0]}x{SCREEN[1]}x24", "-nolisten", "tcp"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.environ["DISPLAY"] = DISPLAY_NUM
    wait_for(lambda: run("xdotool", "getdisplaygeometry").returncode == 0, 10)
    keep = show_swatches()
    try:
        checks()
    finally:
        keep.close()
        xvfb.terminate()
    return report()


# Where things are, in the default window (520 x 560, the sidebar closed).
# Pinned icons ("rail", window 540 wide) for the pages and Settings.
AT = {
    "menu": (30, 20),  # on the sidebar's icon column (v0.7.3)
    "home_picker_card": (127, 175),
    "pick": (66, 66),
    "history": [(143, 66), (175, 66), (207, 66), (239, 66)],
    "search": (442, 66),
    "formats": (484, 66),
    "card_copy": (475, 128),
    "card_value": (120, 208),
    "rgb_copy": (232, 291),
    "shade_spot": [(40, 436), (137, 436), (330, 436), (427, 436)],  # tiles 1, 2, 4, 5, above their text
    "pen": (231, 442),
    "editor_square_light": (60, 312),
    "editor_use": (383, 524),
    "formats_first_off": (280, 334),  # CMYK's switch
    "text_extract": (97, 70),
    "rail_home": (30, 64),
    "rail_picker": (30, 110),
    "rail_text": (30, 156),
    "rail_ruler": (30, 202),
    "rail_settings": (30, 532),
    # Settings (rail): at the top
    "s_autocopy": (486, 226),
    "s_minimize": (486, 301),
    "s_show_name": (486, 376),
    "s_gradient": (464, 460),
    # Settings (rail): scrolled to the end (v0.7.3: Buy me a coffee above Report)
    "s_autostart": (486, 104),
    "s_tray": (486, 179),
    "s_light": (343, 251),
    "s_dark": (402, 251),
    "s_coffee": (467, 407),
    "s_report": (470, 500),
    # Settings opened from the Screen Ruler (its section first)
    "s_ruler_cm": (484, 395),
    "s_ruler_buttons": (480, 236),
}
PICKER = {"last_page": 1, "last_color": [45, 92, 118],
          "history": [[45, 92, 118], [224, 64, 48], [32, 160, 96], [240, 192, 32]]}


def changed(app, field, before, timeout=4):
    return wait_for(lambda: app.config().get(field) != before, timeout)


def pick_from(app, swatch, how="click"):
    """Pick (button or hotkey), then a click on a colored square."""
    if how == "hotkey":
        xdo("key", "--clearmodifiers", "super+ctrl+c")
    else:
        app.click(*AT["pick"], wait=1.0)
    time.sleep(0.8)
    xdo("mousemove", *swatch_center(swatch))
    time.sleep(0.5)
    xdo("click", 1)


def checks():
    # ---- the build ------------------------------------------------------
    @test("Version", "--version and -v print Cargo.toml's version")
    def _():
        with open(os.path.join(REPO, "Cargo.toml")) as f:
            want = next(l.split('"')[1] for l in f if l.startswith("version"))
        got = [run(BIN, flag).stdout.strip() for flag in ("--version", "-v")]
        return got == [f"archtoys {want}"] * 2, f"{got}, Cargo.toml {want}"

    # ---- first start: Home ----------------------------------------------
    app = App()

    @test("Start", "the window opens (fresh profile, 520 x 560)")
    def _():
        if not app.win:
            return False, "no window titled Archtoys:\n" + app.output()[-300:]
        _, _, w, h = app.geometry()
        return (w, h) == (520, 560), f"{w}x{h}"

    @test("Home", "the Color Picker card opens the Color Picker")
    def _():
        app.shot("home")
        app.click(*AT["home_picker_card"])
        return wait_for(lambda: app.config().get("last_page") == 1, 4), f"last_page {app.config().get('last_page')}"
    app.quit()

    # ---- the Color Picker -------------------------------------------------
    app = App(PICKER)
    app.shot("picker")

    @test("Pick", "Pick, then a click on the screen picks that color")
    def _():
        pick_from(app, "blue")
        got = wait_for(lambda: last_color(app) == SWATCHES["blue"] and last_color(app), 5)
        hist = app.config().get("history", [[]])[0]
        return got == SWATCHES["blue"] and tuple(hist) == SWATCHES["blue"], f"color {last_color(app)}, history[0] {hist}"

    @test("Pick", "the arrow keys move the pick one pixel, Enter picks")
    def _():
        app.click(*AT["pick"], wait=1.0)
        x, y = SWATCH_AT["red"]
        xdo("mousemove", x + 119, y + 60)  # red's last column
        time.sleep(0.6)
        for _ in range(25):  # over the gap, into green (20 px apart)
            xdo("key", "Right")
            time.sleep(0.03)
        xdo("key", "Return")
        got = wait_for(lambda: last_color(app) == SWATCHES["green"] and last_color(app), 5)
        return got == SWATCHES["green"], f"color {last_color(app)}"

    @test("Pick", "Esc cancels a pick (the color stays)")
    def _():
        before = last_color(app)
        app.click(*AT["pick"], wait=1.0)
        xdo("mousemove", *swatch_center("yellow"))
        time.sleep(0.6)
        xdo("key", "Escape")
        time.sleep(1.5)
        return last_color(app) == before, f"before {before}, after {last_color(app)}"

    @test("Pick", "the hotkey Ctrl+Super+C starts a pick")
    def _():
        pick_from(app, "yellow", how="hotkey")
        got = wait_for(lambda: last_color(app) == SWATCHES["yellow"] and last_color(app), 5)
        return got == SWATCHES["yellow"], f"color {last_color(app)}"

    @test("History", "a click on a history color selects it")
    def _():
        xdo("mousemove", 300, 540)
        time.sleep(0.4)
        want = app.pixel(*AT["history"][2])
        app.click(*AT["history"][2])
        got = wait_for(lambda: last_color(app) == want and last_color(app), 4)
        return got == want, f"bubble {hexcolor(want)}, color {last_color(app)}"

    @test("Shades", "a click on a shade selects it")
    def _():
        xdo("mousemove", 300, 540)
        time.sleep(0.5)
        want = app.pixel(*AT["shade_spot"][0])
        app.click(*AT["shade_spot"][0])
        got = wait_for(lambda: last_color(app) == want and last_color(app), 4)
        return got == want, f"shade {hexcolor(want)}, color {last_color(app)}"

    @test("Copy", "the color card's copy button copies the color (HEX)")
    def _():
        app.click(*AT["card_copy"])
        want = hexcolor(last_color(app))
        got = wait_for(lambda: clipboard().strip() == want and want, 3)
        return got == want, f"clipboard {clipboard()!r}, color {want}"

    @test("Type", "typing a color on the card selects it")
    def _():
        app.click(*AT["card_value"])
        app.key("ctrl+a")
        app.type("#FF46A2")
        app.key("Return")
        got = wait_for(lambda: last_color(app) == (255, 70, 162) and last_color(app), 4)
        return got == (255, 70, 162), f"color {last_color(app)}"

    @test("Copy", "a format box's copy button copies its full form")
    def _():
        want = "rgb(%d,%d,%d)" % last_color(app)
        app.click(*AT["rgb_copy"])
        got = wait_for(lambda: clipboard().strip() == want and True, 3)
        return bool(got), f"clipboard {clipboard()!r}, color {want}"

    @test("Search", "Ctrl+F, a color word and Enter select a color of that family")
    def _():
        before = last_color(app)
        app.key("ctrl+f")
        app.type("green", wait=1.0)
        app.key("Return", wait=1.0)
        got = wait_for(lambda: last_color(app) != before and last_color(app), 4)
        ok = bool(got) and got[1] > got[0] + 30 and got[1] > got[2]
        return ok, f"before {before}, after {got}"

    @test("Editor", "the pen opens the editor; a click in its square and Use change the color")
    def _():
        before = last_color(app)
        app.click(*AT["pen"], wait=1.2)
        app.shot("editor")
        app.click(*AT["editor_square_light"])
        app.click(*AT["editor_use"], wait=1.0)
        got = wait_for(lambda: last_color(app) != before and last_color(app), 4)
        return bool(got) and sum(got) > sum(before), f"before {before}, after {got}"

    @test("Formats", "a switch in the formats panel turns a format on")
    def _():
        app.click(*AT["formats"], wait=1.0)
        app.shot("formats")
        app.click(*AT["formats_first_off"], wait=0.8)
        formats = app.config().get("formats", [])
        cmyk = next((f for f in formats if f.get("key") == "CMYK"), {})
        app.click(150, 540, wait=1.0)  # a click beside the panel closes it
        return cmyk.get("enabled") is True, f"CMYK {cmyk}"

    # ---- the sidebar ------------------------------------------------------
    @test("Sidebar", "the menu button pins the icons (the window grows by 60)")
    def _():
        app.click(*AT["menu"], wait=1.0)
        _, _, w, _ = app.geometry()
        return app.config().get("sidebar_mode") == 1 and w == 580, f"mode {app.config().get('sidebar_mode')}, width {w}"

    @test("Sidebar", "a second click soon after pins all of it (+178)")
    def _():
        app.click(*AT["menu"], wait=1.0)  # closes it
        wx, wy, _, _ = app.geometry()
        xdo("mousemove", wx + AT["menu"][0], wy + AT["menu"][1])
        time.sleep(0.4)
        xdo("click", "--repeat", "2", "--delay", "120", 1)
        time.sleep(1.0)
        _, _, w, _ = app.geometry()
        return app.config().get("sidebar_mode") == 2 and w == 698, f"mode {app.config().get('sidebar_mode')}, width {w}"

    @test("Sidebar", "one more click closes it (the window gives the width back)")
    def _():
        app.click(*AT["menu"], wait=1.0)
        _, _, w, _ = app.geometry()
        return app.config().get("sidebar_mode") == 0 and w == 520, f"mode {app.config().get('sidebar_mode')}, width {w}"

    # ---- the window: closed to the tray and opened again --------------------
    @test("Window", "the close button keeps Archtoys running in the tray")
    def _():
        close_window(app.win)
        time.sleep(1.5)
        visible = xdo("search", "--onlyvisible", "--name", "^Archtoys$")
        return app.running() and not visible, f"running {app.running()}, window visible {bool(visible)}"

    @test("Window", "a second launch shows it again (one copy runs)")
    def _():
        run(BIN, env=app.env, timeout=20)
        win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys$"), 5)
        copies = running_copies()
        return bool(win) and app.running() and copies == 1, f"window {bool(win)}, copies {copies}"
    app.quit()

    # ---- pages and Settings (the icons pinned) ------------------------------
    app = App(dict(PICKER, sidebar_mode=1))

    @test("Pages", "the sidebar's icons open Text Extractor, Home and the Color Picker")
    def _():
        got = []
        for where, page in (("rail_text", 2), ("rail_home", 0), ("rail_picker", 1)):
            app.click(*AT[where])
            got.append(wait_for(lambda: app.config().get("last_page") == page, 3) and page)
        return got == [2, 0, 1], f"pages {got}"

    app.click(*AT["rail_settings"], wait=1.2)
    app.shot("settings")
    for name, field, want in (("Auto copy", "setting_autocopy", True), ("Minimize on pick", "setting_minimize", True),
                              ("Show color name", "show_name", False)):
        @test("Settings", f"the {name} switch changes the setting")
        def _(name=name, field=field, want=want):
            app.click(*AT["s_" + {"setting_autocopy": "autocopy", "setting_minimize": "minimize", "show_name": "show_name"}[field]])
            got = wait_for(lambda: app.config().get(field) == want, 3)
            return bool(got), f"{field} {app.config().get(field)}"

    @test("Settings", "Color card style: Gradient")
    def _():
        app.click(*AT["s_gradient"])
        return bool(wait_for(lambda: app.config().get("card_gradient") is True, 3)), f"card_gradient {app.config().get('card_gradient')}"

    # to the end of Settings
    wx, wy, _, _ = app.geometry()
    xdo("mousemove", wx + 300, wy + 300)
    for _ in range(40):
        xdo("click", 5)
    time.sleep(1.0)
    app.shot("settings-end")

    @test("Settings", "Theme: Dark, then Light (the window follows)")
    def _():
        app.click(*AT["s_dark"], wait=1.0)
        dark = app.config().get("theme"), app.pixel(300, 50)
        app.click(*AT["s_light"], wait=1.0)
        light = app.config().get("theme"), app.pixel(300, 50)
        ok = dark[0] == "dark" and light[0] == "light" and sum(dark[1]) < sum(light[1])
        return ok, f"dark {dark}, light {light}"

    @test("Settings", "Start on login writes the autostart entry, and off removes it")
    def _():
        entry = os.path.join(app.home, ".config", "autostart", "archtoys.desktop")
        app.click(*AT["s_autostart"])
        on = wait_for(lambda: os.path.exists(entry), 3)
        app.click(*AT["s_autostart"])
        off = wait_for(lambda: not os.path.exists(entry), 3)
        return bool(on and off), f"written {bool(on)}, removed {bool(off)}"

    @test("Settings", "Minimize to tray on close can be turned off")
    def _():
        app.click(*AT["s_tray"])
        return bool(wait_for(lambda: app.config().get("setting_minimize_tray") is False, 3)), \
            f"setting_minimize_tray {app.config().get('setting_minimize_tray')}"

    @test("Support", "Buy me a coffee opens the support page")
    def _():
        app.click(*AT["s_coffee"], wait=1.5)
        try:
            with open(os.path.join(app.home, "opened.txt")) as f:
                opened = f.read().split()
        except OSError:
            return False, "no browser was opened"
        want = "https://mujtaba1i.github.io/Archtoys/support/"
        return want in opened, f"opened {opened[-1] if opened else 'nothing'}"

    @test("Report", "Report a problem opens a filled-in GitHub issue")
    def _():
        app.click(*AT["s_report"], wait=1.5)
        try:
            with open(os.path.join(app.home, "opened.txt")) as f:
                url = f.read()
        except OSError:
            return False, "no browser was opened"
        need = ["/issues/new?body=", "Archtoys%3A%20", "Screens%3A"]
        if app.output().strip():  # it printed something: the report carries it
            need.append("Recent%20messages")
        missing = [n for n in need if n not in url]
        return not missing, ("missing " + ", ".join(missing)) if missing else f"{len(url)} characters"
    app.quit()

    # ---- Text Extractor -----------------------------------------------------
    have_tesseract = shutil.which("tesseract") is not None
    # the capture goes full screen, which on X11 a window manager does: a small one
    wm = subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) \
        if shutil.which("openbox") else None
    time.sleep(1)
    app = App({"last_page": 2, "text_enabled": True}) if have_tesseract else None
    words = "Archtoys reads this text"

    @test("Text Extractor", "drag over text on screen, Enter copies it (and it goes to the history)")
    def _():
        if not app:
            raise Skip("tesseract isn't installed")
        if not wm:
            raise Skip("openbox isn't installed (the capture needs a window manager to go full screen)")
        keep = show_text(words, 900, 120)
        app.shot("text")
        # the hotkey, not a click: the window manager's title bar moves the
        # window's contents by its own height (22 px with Ubuntu's openbox)
        xdo("key", "ctrl+super+t")
        time.sleep(2.5)
        xdo("mousemove", 890, 110)
        xdo("mousedown", 1)
        xdo("mousemove", 1200, 150)
        xdo("mousemove", 1520, 200)
        xdo("mouseup", 1)
        time.sleep(4)
        app.shot("text-toolbar")
        capture = xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$").split()
        if not capture:
            return False, "the capture didn't open (or closed): " + app.output()[-300:]
        xdo("windowfocus", "--sync", capture[0])
        xdo("key", "Return")
        got = wait_for(lambda: words.lower() in " ".join(clipboard().split()).lower(), 5)
        keep.close()
        history = os.path.join(app.home, ".local", "share", "archtoys", "text-history.json")
        saved = os.path.exists(history) and words.lower() in open(history).read().lower()
        return bool(got) and saved, f"clipboard {clipboard()!r}, history saved {saved}"
    if app:
        app.quit()

    ruler_checks(wm)
    if wm:
        wm.terminate()


def ruler_checks(wm):
    """The Screen Ruler on the frozen screen (full screen, so it needs the window manager too)."""
    app = App({"last_page": 1})  # the sidebar check opens the ruler's page itself
    history = os.path.join(app.home, ".local", "share", "archtoys", "ruler-history.json")

    def entries():
        try:
            with open(history) as f:
                return json.load(f)
        except (OSError, ValueError):
            return []

    def open_ruler():
        xdo("key", "--clearmodifiers", "ctrl+super+m")
        win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$").split(), 5)
        time.sleep(1.0)
        if win:
            xdo("windowfocus", "--sync", win[0])
        return win

    def need_wm():
        if not wm:
            raise Skip("openbox isn't installed (the ruler needs a window manager to go full screen)")

    @test("Screen Ruler", "the sidebar's ruler opens its page")
    def _():
        app.click(*AT["menu"], wait=1.0)  # pins the icons
        app.click(*AT["rail_ruler"])
        app.shot("ruler-page")
        return bool(wait_for(lambda: app.config().get("last_page") == 3, 3)), f"last_page {app.config().get('last_page')}"

    @test("Screen Ruler", "Ctrl+Super+M, then a click inside a square copies its size (Spacing)")
    def _():
        need_wm()
        if not open_ruler():
            return False, "the ruler didn't open: " + app.output()[-300:]
        xdo("mousemove", *swatch_center("blue"))
        time.sleep(0.8)
        app.shot("ruler-spacing")
        xdo("click", 1)
        got = wait_for(lambda: clipboard().strip() == "120 × 120 px", 4)
        saved = entries()[:1]
        ok = bool(got) and saved and saved[0].get("mode") == "Spacing" and saved[0].get("shapes") == [[120, 120]]
        return ok, f"clipboard {clipboard()!r}, history {saved}"

    @test("Screen Ruler", "Ctrl+3 measures across (Horizontal), Enter copies the width")
    def _():
        need_wm()
        if not open_ruler():
            return False, "the ruler didn't open"
        xdo("mousemove", *swatch_center("red"))
        time.sleep(0.5)
        xdo("key", "ctrl+3")
        time.sleep(0.5)
        xdo("key", "Return")
        got = wait_for(lambda: clipboard().strip() == "120 px", 4)
        return bool(got) and entries()[0].get("mode") == "Horizontal", f"clipboard {clipboard()!r}, newest {entries()[:1]}"

    @test("Screen Ruler", "Ctrl+4 measures up and down (Vertical): on an empty screen, exactly its height")
    def _():
        need_wm()
        if not open_ruler():
            return False, "the ruler didn't open"
        xdo("mousemove", SCREEN[0] - 20, SCREEN[1] // 2)  # right of the squares: nothing above or below
        time.sleep(0.5)
        xdo("key", "ctrl+4")
        time.sleep(0.5)
        xdo("key", "Return")
        want = f"{SCREEN[1]} px"
        got = wait_for(lambda: clipboard().strip() == want, 4)
        return bool(got) and entries()[0].get("mode") == "Vertical", f"clipboard {clipboard()!r}, want {want!r}, newest {entries()[:1]}"

    # (each scroll notch is 5, but winit 0.30 counts a fake X11 wheel click
    # like xdotool's twice: 3 notches down from 30 give 15 or 0, 11 up give
    # 70 or 110, and the measurements are the same either way)
    @test("Screen Ruler", "the scroll wheel changes the tolerance: lower splits the strip of two grays, higher reaches the whole screen")
    def _():
        need_wm()
        got = []
        for scroll, notches, want in ((None, 0, "200 px"), (5, 3, "100 px"), (4, 11, f"{SCREEN[0]} px")):
            if not open_ruler():
                return False, "the ruler didn't open"
            xdo("key", "ctrl+3")
            xdo("mousemove", STRIP_AT[0] + 50, STRIP_AT[1] + 30)  # the darker half
            time.sleep(0.5)
            for _ in range(notches):  # each notch 5 (down: less, up: more)
                xdo("click", scroll)
                time.sleep(0.15)
            time.sleep(0.4)
            if scroll == 5:
                app.shot("ruler-tolerance")
            xdo("key", "Return")
            got.append((wait_for(lambda: clipboard().strip() == want and want, 4) or clipboard().strip(), want))
            wait_for(lambda: not xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$"), 4)
        saved = wait_for(lambda: app.config().get("ruler_tolerance") in (70, 110), 3)
        return all(g == w for g, w in got) and bool(saved), f"{got}, saved tolerance {app.config().get('ruler_tolerance')}"

    @test("Screen Ruler", "Esc closes it without copying")
    def _():
        need_wm()
        before = len(entries())
        if not open_ruler():
            return False, "the ruler didn't open"
        xdo("mousemove", *swatch_center("green"))
        time.sleep(0.5)
        xdo("key", "Escape")
        closed = wait_for(lambda: not xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$"), 4)
        return bool(closed) and len(entries()) == before, f"closed {bool(closed)}, history {before} then {len(entries())}"

    @test("Screen Ruler", "Bounds: Shift-click keeps a box, 3 at most, Ctrl+C copies them all")
    def _():
        need_wm()
        if not open_ruler():
            return False, "the ruler didn't open"
        xdo("key", "ctrl+1")
        time.sleep(0.5)
        for i, (w, h) in enumerate([(100, 50), (80, 40), (60, 30), (40, 20)]):
            x, y = 200 + i * 150, 400
            xdo("mousemove", x, y)
            xdo("mousedown", 1)
            xdo("mousemove", x + w // 2, y + h // 2)
            xdo("mousemove", x + w, y + h)
            xdo("mouseup", 1)
            time.sleep(0.3)
            if i < 3:  # keep it (the third doesn't fit: 2 kept + the one drawn)
                xdo("mousemove", 1500, 950)
                xdo("keydown", "shift")
                xdo("click", 1)
                xdo("keyup", "shift")
                time.sleep(0.3)
        app.shot("ruler-bounds")
        xdo("key", "ctrl+c")
        want = "100 × 50, 80 × 40, 40 × 20 px"
        got = wait_for(lambda: clipboard().strip() == want, 4)
        return bool(got), f"clipboard {clipboard()!r}, want {want!r}"

    @test("Screen Ruler", "Settings: Units cm is saved")
    def _():
        app.click(*AT["rail_settings"], wait=1.2)
        app.shot("ruler-settings")
        app.click(*AT["s_ruler_cm"])
        return bool(wait_for(lambda: app.config().get("ruler_units") == 1, 3)), f"ruler_units {app.config().get('ruler_units')}"

    @test("Screen Ruler", "the − and + buttons change the tolerance by 5 (Settings: Change the tolerance with)")
    def _():
        need_wm()
        app.click(*AT["s_ruler_buttons"])
        if not wait_for(lambda: app.config().get("ruler_buttons") is True, 3):
            return False, f"ruler_buttons {app.config().get('ruler_buttons')}"
        before = app.config().get("ruler_tolerance")
        if not open_ruler():
            return False, "the ruler didn't open"
        xdo("key", "ctrl+3")
        app.shot("ruler-buttons")
        for _ in range(2):
            xdo("mousemove", *RULER_PLUS)
            time.sleep(0.3)
            xdo("click", 1)
            time.sleep(0.3)
        xdo("mousemove", *RULER_MINUS)
        time.sleep(0.3)
        xdo("click", 1)
        time.sleep(0.3)
        xdo("key", "Escape")
        after = wait_for(lambda: app.config().get("ruler_tolerance") != before and app.config().get("ruler_tolerance"), 4)
        return after == before + 5, f"tolerance {before} then {after} (want {before + 5}: +5 +5 -5)"
    app.quit()


def running_copies():
    """Archtoys processes that are running (an ended one not yet reaped, a zombie, isn't)."""
    count = 0
    for pid in os.listdir("/proc"):
        try:
            with open(f"/proc/{pid}/stat") as f:
                stat = f.read()
        except OSError:
            continue
        name, state = stat[stat.index("(") + 1:stat.rindex(")")], stat[stat.rindex(")") + 2]
        if name == os.path.basename(BIN)[:15] and state != "Z":
            count += 1
    return count


def show_text(text, x, y):
    """Text on the screen to extract: drawn once with Pillow, kept by the X server."""
    from PIL import Image, ImageDraw, ImageFont
    font_file = run("fc-match", "-f", "%{file}", "DejaVu Sans").stdout.strip()
    font = ImageFont.truetype(font_file, 30)
    w, h = 620, 70
    img = Image.new("RGB", (w, h), "white")
    ImageDraw.Draw(img).text((14, 14), text, fill="black", font=font)
    d = display.Display(DISPLAY_NUM)
    screen = d.screen()
    pixmap = screen.root.create_pixmap(w, h, screen.root_depth)
    gc = pixmap.create_gc()
    raw = img.convert("RGBX").tobytes("raw", "BGRX")
    for row in range(h):  # one row per request (requests have a size limit)
        pixmap.put_image(gc, 0, row, w, 1, X.ZPixmap, screen.root_depth, 0, raw[row * w * 4:(row + 1) * w * 4])
    win = screen.root.create_window(x, y, w, h, 0, screen.root_depth, X.InputOutput, X.CopyFromParent,
                                    background_pixmap=pixmap, override_redirect=True)
    win.map()
    d.flush()
    return d


def report():
    failed = [r for r in results if r[2] is False]
    passed = [r for r in results if r[2] is True]
    skipped = [r for r in results if r[2] is None]
    head = "✅ passed" if not failed else f"❌ {len(failed)} failed"
    lines = [f"### Archtoys, used like a person would (X11): {head}", "",
             f"{len(passed)} passed, {len(failed)} failed, {len(skipped)} skipped. Every check runs even after a failure.", "",
             "| Area | Check | | Details |", "|---|---|---|---|"]
    for area, name, ok, detail in results:
        mark = {True: "✅", False: "❌", None: "⏭️"}[ok]
        lines.append(f"| {area} | {name} | {mark} | {detail.replace('|', '/')} |")
    text = "\n".join(lines)
    print("\n" + text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a") as f:
            f.write(text + "\n\n")
    for area, name, ok, detail in failed:
        print(f"::error title=Archtoys end to end, {area}::{name}: {detail}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
