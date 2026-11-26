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
    dbus-run-session -- python3 .github/scripts/e2e_test.py target/release/archtoys --wayland

--wayland (v0.8.0) runs the same checks on KDE Plasma (Wayland): a headless
KWin instead of Xvfb, a mouse and keyboard through KWin's fake input
(.github/scripts/wayland-input.c, built in CI, on PATH as wayland-input),
windows found, moved and closed with KWin scripts, the clipboard through
wl-paste, and the screen handed to the app as a picture
(ARCHTOYS_TEST_SCREENSHOT: a headless KWin can't take screenshots). A check
that can't run there (reading pixels off the screen) is skipped, with why.

Needs: Xvfb, xdotool, xclip, python3-xlib, dbus (for a second launch), and for
the Text Extractor check tesseract, python3-pillow and openbox (the capture
goes full screen, which on X11 a window manager does; skipped without them).
It never touches your own session: its own X screen, profile and runtime
folder, so it also runs next to an Archtoys you're using. Positions are
for the default window (520 x 560) at 100%: after a layout change, run with
--shot to save a screenshot of each screen and update the positions below.
"""
import io
import json
import re
import os
import shutil
import subprocess
import sys
import tempfile
import time

try:  # X11 only (the Wayland run doesn't need it)
    from Xlib import X, display
except ImportError:
    X = display = None

BIN = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("-") else "target/release/archtoys")
SHOTS = "--shot" in sys.argv
WAYLAND = "--wayland" in sys.argv
DESKTOP = "KDE Plasma (Wayland)" if WAYLAND else "X11"
DISPLAY_NUM = ":95"
SCREEN = (1600, 1000)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

results = []  # (area, check, ok, detail)


# ---------------------------------------------------------------- plumbing
def run(*cmd, timeout=10, env=None, check=False):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, env=env, check=check)


def xdo(*args):
    if WAYLAND:
        return kde.xdo(*[str(a) for a in args])
    return run("xdotool", *[str(a) for a in args]).stdout.strip()


# ---------------------------------------------------------------- KDE Plasma (Wayland)
class KDE:
    """What xdotool, xclip and Xvfb do, on a headless KWin."""

    # Linux key codes (US layout, KWin's default)
    KEYS = {"escape": 1, "return": 28, "enter": 28, "backspace": 14, "tab": 15, "space": 57, "delete": 111,
            "left": 105, "right": 106, "up": 103, "down": 108, "home": 102, "end": 107,
            "ctrl": 29, "control": 29, "shift": 42, "alt": 56, "super": 125, "meta": 125,
            "minus": 12, "-": 12, "equal": 13, "=": 13, "plus": 13, "comma": 51, ",": 51, "period": 52, ".": 52,
            "slash": 53, "/": 53, "semicolon": 39, ";": 39, "apostrophe": 40, "'": 40, "grave": 41, "`": 41,
            "bracketleft": 26, "[": 26, "bracketright": 27, "]": 27, "backslash": 43, "\\": 43, " ": 57}
    SHIFTED = {"!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0",
               "_": "minus", "+": "equal", "{": "[", "}": "]", "|": "\\", ":": ";", "\"": "'", "~": "`", "<": ",",
               ">": ".", "?": "/"}
    BUTTONS = {"1": 272, "2": 274, "3": 273}

    def __init__(self):
        for i, c in enumerate("1234567890"):
            self.KEYS[c] = 2 + i
        for row, start in (("qwertyuiop", 16), ("asdfghjkl", 30), ("zxcvbnm", 44)):
            for i, c in enumerate(row):
                self.KEYS[c] = start + i
        for n in range(1, 11):
            self.KEYS[f"f{n}"] = 58 + n
        self.runtime = os.environ["XDG_RUNTIME_DIR"]
        self.log_path = os.path.join(self.runtime, "kwin.log")
        self.shot = os.path.join(self.runtime, "screen.png")
        env = dict(os.environ, KWIN_WAYLAND_NO_PERMISSION_CHECKS="1", QT_FORCE_STDERR_LOGGING="1",
                   QT_LOGGING_RULES="*.debug=true")
        env.pop("DISPLAY", None)
        # no XWayland: it crashes in a headless KWin without a GPU (Archtoys
        # copies through Wayland's own clipboard, so it isn't needed)
        self.kwin = subprocess.Popen(["kwin_wayland", "--virtual", "--no-lockscreen", "--socket", "wayland-e2e",
                                      "--width", str(SCREEN[0]), "--height", str(SCREEN[1])],
                                     stdout=open(self.log_path, "w"), stderr=subprocess.STDOUT, env=env)
        wait_for(lambda: os.path.exists(os.path.join(self.runtime, "wayland-e2e")), 15)
        self.xdisplay = None
        out = run("kwin_wayland", "--version").stdout
        self.major = int(re.search(r"(\d+)\.", out).group(1)) if re.search(r"(\d+)\.", out) else 0
        # the whole path: each app run has its own runtime folder (the one-copy lock)
        os.environ.update(WAYLAND_DISPLAY=os.path.join(self.runtime, "wayland-e2e"), XDG_SESSION_TYPE="wayland",
                          XDG_CURRENT_DESKTOP="KDE")
        os.environ.pop("DISPLAY", None)
        if self.xdisplay:
            os.environ["DISPLAY"] = self.xdisplay
        # the services D-Bus starts (the desktop portal and its KDE half, for
        # the hotkeys) must know they're in a KDE Wayland session
        run("dbus-update-activation-environment", f"WAYLAND_DISPLAY={os.environ['WAYLAND_DISPLAY']}",
            "XDG_CURRENT_DESKTOP=KDE", "XDG_SESSION_TYPE=wayland", f"XDG_RUNTIME_DIR={self.runtime}")
        # one helper for the whole run: each start adds a pointer and keyboard,
        # and the app's own pointer is made again (a click right then is lost)
        self.input = subprocess.Popen(["wayland-input"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.pos = (0, 0)
        self.scripts = 0
        # where each window is, asked of KWin at most every few seconds: a
        # click would otherwise ask every time (the run took 12 minutes)
        self.places = {}
        self.overlays = []
        self.draw()
        self.send(f"move 5 5")
        time.sleep(1.0)

    def close(self):
        self.input.terminate()
        self.kwin.terminate()

    def send(self, line):
        self.input.stdin.write(line + "\n")
        self.input.stdin.flush()
        self.input.stdout.readline()  # "ok": KWin has it

    def script(self, js):
        """Runs a KWin script; returns what it printed (one line, tagged)."""
        self.scripts += 1
        tag = f"E2E{os.getpid()}x{self.scripts}"
        path = os.path.join(self.runtime, f"{tag}.js")
        with open(path, "w") as f:
            f.write(f"function OUT(s) {{ print('{tag} ' + s); }}\n" + js)
        # dbus-send: qdbus starts Qt for each call
        bus = ["dbus-send", "--session", "--print-reply", "--dest=org.kde.KWin", "/Scripting"]
        run(*bus, "org.kde.kwin.Scripting.loadScript", f"string:{path}", f"string:{tag}")
        run(*bus, "org.kde.kwin.Scripting.start")

        def printed():
            with open(self.log_path, "rb") as f:
                for line in f.read().decode("utf-8", "replace").splitlines():
                    if f"{tag} " in line:
                        return line.split(f"{tag} ", 1)[1]
            return None
        return wait_for(printed, 5, step=0.02) or ""

    WINDOWS = "(workspace.windowList ? workspace.windowList() : workspace.clientList())"

    def find(self, wid):
        return f"const w = {self.WINDOWS}.find(c => String(c.internalId) === '{wid}');"

    def place(self, wid):
        """The window's corner: known for a few seconds, else asked again."""
        known = self.places.get(wid)
        if known and time.time() - known[1] < 4:
            return known[0]
        g = self.geometry(wid)
        if g:
            self.places[wid] = ((g[0], g[1]), time.time())
            return g[0], g[1]
        return 0, 0

    def geometry(self, wid):
        # the app's own surface (the buffer), without KWin's title bar: what the X11 run measures
        got = self.script(self.find(wid) + " const g = w ? (w.clientGeometry || w.bufferGeometry || w.frameGeometry) : null;"
                          " OUT(g ? [g.x, g.y, g.width, g.height].join(' ') : 'none');")
        return tuple(int(float(v)) for v in got.split()) if got and got != "none" else None

    def key_codes(self, combo):
        codes = []
        for name in combo.split("+") if combo != "+" else ["+"]:
            low = name.lower()
            if low in self.KEYS:
                codes.append(self.KEYS[low])
            elif name in self.SHIFTED:
                codes += [self.KEYS["shift"], self.KEYS[self.SHIFTED[name]]]
            elif len(name) == 1 and name.isupper():
                codes += [self.KEYS["shift"], self.KEYS[low]]
            else:
                raise ValueError(f"no key code for {name!r}")
        return codes

    def press(self, codes):
        for c in codes:
            self.send(f"key {c} 1")
        for c in reversed(codes):
            self.send(f"key {c} 0")
        time.sleep(0.05)

    def xdo(self, cmd, *args):
        repeat, delay = 1, 0.0
        if "--repeat" in args:
            i = args.index("--repeat")
            repeat, args = int(args[i + 1]), args[:i] + args[i + 2:]
        if "--delay" in args:
            i = args.index("--delay")
            delay, args = int(args[i + 1]) / 1000, args[:i] + args[i + 2:]
        visible = "--onlyvisible" in args
        args = [a for a in args if a not in ("--clearmodifiers", "--sync", "--onlyvisible")]
        if repeat > 1:
            for n in range(repeat):
                self.xdo(cmd, *args)
                if n < repeat - 1:
                    time.sleep(delay)
            return ""
        if cmd == "mousemove":
            if args[0] == "--window":
                wx, wy = self.place(args[1])
                x, y = wx + int(args[2]), wy + int(args[3])
            else:
                x, y = int(args[0]), int(args[1])
            # a step on the way first, so the move is seen as one even when it stays put
            self.send(f"move {x + 1} {y}")
            self.send(f"move {x} {y}")
            self.pos = (x, y)
        elif cmd in ("click", "mousedown", "mouseup"):
            button = args[0]
            if button in ("4", "5"):  # the wheel, up or down: one event, as far as a notch scrolls on X11
                self.send(f"axis 0 {-120 if button == '4' else 120}")
            else:
                code = self.BUTTONS[button]
                if cmd in ("click", "mousedown"):
                    self.send(f"button {code} 1")
                if cmd in ("click", "mouseup"):
                    self.send(f"button {code} 0")
        elif cmd == "key":
            for combo in args:
                self.press(self.key_codes(combo))
        elif cmd in ("keydown", "keyup"):
            for c in self.key_codes(args[0]):
                self.send(f"key {c} {1 if cmd == 'keydown' else 0}")
        elif cmd == "type":
            text = args[-1]
            for ch in text:
                self.press(self.key_codes(ch))
        elif cmd == "search":
            self.places.clear()
            pattern = args[args.index("--name") + 1]
            # Archtoys' own windows without the desktop's frame: their contents start at the window's corner, as on Xvfb
            shown = " && !c.minimized" if visible else ""
            js = (f"const re = new RegExp({json.dumps(pattern)}); const ws = {self.WINDOWS}.filter(c => re.test(c.caption){shown});"
                  " OUT(ws.map(c => String(c.internalId)).join(','));")
            return "\n".join(i for i in self.script(js).split(",") if i)
        elif cmd == "windowfocus":
            self.script(self.find(args[0]) + " if (w) workspace.activeClient = w; OUT('ok');")
        elif cmd == "windowmove":
            # not needed here (every position is asked of KWin), and on KWin 6
            # setting the geometry also holds the window at that size
            pass
        elif cmd == "getwindowgeometry":
            g = self.geometry(args[0]) or (0, 0, 0, 0)
            return f"Window {args[0]}\n  Position: {g[0]},{g[1]} (screen: 0)\n  Geometry: {g[2]}x{g[3]}"
        else:
            raise ValueError(f"no Wayland way to do xdotool {cmd}")
        return ""

    def close_window(self, wid):
        self.places.clear()
        self.script(self.find(wid) + " if (w) w.closeWindow(); OUT('ok');")

    def draw(self):
        """The screen the app is handed when it takes a screenshot: the same squares, strip and text as on Xvfb."""
        from PIL import Image, ImageDraw
        img = Image.new("RGB", SCREEN, (0x40, 0x48, 0x50))
        d = ImageDraw.Draw(img)
        for name, rgb in SWATCHES.items():
            x, y = SWATCH_AT[name]
            d.rectangle([x, y, x + 119, y + 119], fill=rgb)
        for half, rgb in enumerate(STRIP):
            d.rectangle([STRIP_AT[0] + half * 100, STRIP_AT[1], STRIP_AT[0] + half * 100 + 99, STRIP_AT[1] + 59], fill=rgb)
        for overlay, at in self.overlays:
            img.paste(overlay, at)
        img.save(self.shot)


kde = None


def test(area, name):
    """Decorator: runs the check now, records ok / failed / skipped, never stops."""
    def wrap(fn):
        try:
            outcome = fn()
            if outcome is None:
                outcome = (True, "")
            ok, detail = outcome if isinstance(outcome, tuple) else (bool(outcome), "")
            ok = bool(ok)  # a check may hand back what it waited for (a list of windows, a color)
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


def not_on_kwin6(why):
    """A check a headless KWin 6 fails for a reason still to be found: skipped, with why.
    E2E_TRY_KWIN6=1 runs them anyway (v0.8.1: the sidebar's ruler passes again; the window
    keeping KWin's first size isn't Archtoys' window script, tried without it)."""
    if WAYLAND and kde.major >= 6 and not os.environ.get("E2E_TRY_KWIN6"):
        raise Skip(f"headless KWin 6: {why} (Plasma 5 and X11 pass it)")


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
                        PATH=self.bin_dir + ":" + os.environ["PATH"], ARCHTOYS_HOTKEY_BACKEND="x11",
                        # no real update check: a newer release's bubble would cover the bar
                        ARCHTOYS_UPDATE_API="file:///nonexistent/release.json")
        self.env.pop("WAYLAND_DISPLAY", None)
        if WAYLAND:
            # KDE Plasma: the app's own Wayland window and the desktop's shortcuts; the screen it sees is kde.shot
            self.env.update(XDG_SESSION_TYPE="wayland", XDG_CURRENT_DESKTOP="KDE", WAYLAND_DISPLAY=os.environ["WAYLAND_DISPLAY"],
                            ARCHTOYS_TEST_SCREENSHOT=kde.shot)
            self.env.pop("ARCHTOYS_HOTKEY_BACKEND", None)
            if kde.xdisplay:
                self.env["DISPLAY"] = kde.xdisplay
            else:
                self.env.pop("DISPLAY", None)
        # its own profile: GitHub's runners set these, and the app would save outside HOME
        for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"):
            self.env.pop(name, None)
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
        self.come_back()
        # relative to the window itself: a window manager's title bar doesn't shift it
        xdo("mousemove", "--window", self.win, x, y)
        time.sleep(0.4)  # rest on it first, like a hand: some buttons react to the hover
        xdo("click", button)
        time.sleep(wait)

    def key(self, *keys, wait=0.4):
        self.come_back()
        self.focus()
        for k in keys:
            xdo("key", "--clearmodifiers", k)
            time.sleep(0.08)
        time.sleep(wait)

    def type(self, text, wait=0.5):
        self.come_back()
        self.focus()
        xdo("type", "--delay", "40", text)
        time.sleep(wait)

    def pixel(self, x, y):
        wx, wy, _, _ = self.geometry()
        return screen_pixel(wx + x, wy + y)

    def shot(self, name):
        if SHOTS and not WAYLAND:
            path = os.path.join(REPO, f"e2e-{name}.png")
            run("import", "-display", DISPLAY_NUM, "-window", "root", path)
            print(f"  (saved {path})")

    def step_aside(self):
        """Wayland: the main window minimized before a frozen screen opens. On
        KDE Archtoys fades it instead of hiding it, and a headless KWin never
        gives back the drawing buffers of a window covered by another (it
        composites nothing), so the app would wait on its next redraw. The
        pick itself is what's checked; it starts the way a hotkey or a desktop
        shortcut starts it, with the window out of the way."""
        if WAYLAND:
            kde.script(kde.find(self.win) + " if (w) w.minimized = true; OUT('ok');")
            kde.places.clear()
            self.aside = True
            time.sleep(0.5)

    def come_back(self):
        """Wayland: the window stepped aside comes back (the user would click it in the task bar)."""
        if WAYLAND and getattr(self, "aside", False):
            self.aside = False
            kde.script(kde.find(self.win) + " if (w) { w.minimized = false; workspace.activeClient = w; } OUT('ok');")
            kde.places.clear()
            time.sleep(0.5)

    def desktop_keys(self):
        """Wayland: did the desktop give Archtoys its hotkeys? Plasma 5.27's
        portal can't tell which app asks unless it was started from the app
        menu, so it assigns no keys (it says "")."""
        out = self.output()
        return not WAYLAND or ('desktop shortcut for pick-color is ""' not in out and "so no hotkeys" not in out)

    def start_toy(self, keys, flag):
        """A toy by its hotkey, or (Wayland without hotkeys) the way a desktop
        shortcut starts it: `archtoys --measure` and so on."""
        self.step_aside()
        if self.desktop_keys():
            xdo("key", "--clearmodifiers", keys)
        else:
            subprocess.run([BIN, flag], env=self.env, timeout=10)

    def start_pick(self):
        """The Pick button (X11), or `archtoys --pick` with the window aside (Wayland, see step_aside)."""
        if WAYLAND:
            self.step_aside()
            subprocess.run([BIN, "--pick"], env=self.env, timeout=10)
            time.sleep(1.0)
        else:
            self.click(*AT["pick"], wait=1.0)

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
    if WAYLAND:
        raise Skip("a headless KWin can't read pixels off the screen (no screenshots without a GPU)")
    global _x
    if _x is None:
        _x = display.Display(DISPLAY_NUM)
    raw = _x.screen().root.get_image(x, y, 1, 1, X.ZPixmap, 0xFFFFFFFF).data
    return raw[2], raw[1], raw[0]  # BGRX


def clipboard():
    if WAYLAND:
        try:
            return run("wl-paste", "--no-newline", timeout=3).stdout
        except subprocess.TimeoutExpired:
            return ""
    try:
        return run("xclip", "-display", DISPLAY_NUM, "-o", "-selection", "clipboard", timeout=3).stdout
    except subprocess.TimeoutExpired:
        return ""


def close_window(win):
    """The window's close button, the way a desktop sends it (WM_DELETE_WINDOW)."""
    if WAYLAND:
        return kde.close_window(win)
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
    if WAYLAND:
        kde.draw()
        return kde
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
    if WAYLAND:
        global kde
        kde = KDE()
        try:
            checks()
        finally:
            kde.close()
        return report()
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
    "home_picker_card": (127, 121),  # the Color Picker row (v0.8.0)
    "home_micro_switch": (460, 470),  # Screenshot to clipboard's switch (below Awake since v0.9.0)
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
    "rail_awake": (30, 248),
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
        if not app.desktop_keys():
            raise Skip("the desktop assigned no keys: Plasma 5.27's portal can't tell which app asks when it isn't started from the app menu")
        app.step_aside()
        xdo("key", "--clearmodifiers", "super+ctrl+c")
    else:
        app.start_pick()
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

    @test("Home", "the Color Picker row opens the Color Picker")
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
        app.start_pick()
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
        app.start_pick()
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
        not_on_kwin6("the window keeps the size KWin first gave it")
        app.click(*AT["menu"], wait=1.0)
        _, _, w, _ = app.geometry()
        return app.config().get("sidebar_mode") == 1 and w == 580, f"mode {app.config().get('sidebar_mode')}, width {w}"

    @test("Sidebar", "a second click soon after pins all of it (+178)")
    def _():
        not_on_kwin6("the window keeps the size KWin first gave it")
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
        not_on_kwin6("the window keeps the size KWin first gave it")
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
        not_on_kwin6("the window isn't shown again")
        run(BIN, env=app.env, timeout=20)
        win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys$"), 5)
        copies = running_copies()
        return bool(win) and app.running() and copies == 1, f"window {bool(win)}, copies {copies}"
    app.quit()

    # ---- pages and Settings (the icons pinned) ------------------------------
    # Wayland: Archtoys owning the hotkeys, as on X11, so Settings is laid out
    # the same (with the desktop's, each hotkey card says so in two lines)
    APP_KEYS = {"ARCHTOYS_HOTKEY_BACKEND": "x11"} if WAYLAND else None
    app = App(dict(PICKER, sidebar_mode=1), extra_env=APP_KEYS)

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
    # (on KDE Plasma, KWin is the window manager)
    wm = kde if WAYLAND else subprocess.Popen(["openbox"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) \
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
        app.start_toy("ctrl+super+t", "--extract")
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
    micro_checks(wm)
    if wm and not WAYLAND:
        wm.terminate()


def micro_checks(wm):
    """The Micro-toys (v0.8.0): Screenshot to clipboard's switch on Home and its hotkey."""
    app = App({"last_page": 0})

    def captures():
        return app.output().count("freeze picker: ready")

    @test("Micro-toys", "off at first: Ctrl+Super+S does nothing")
    def _():
        if not app.desktop_keys():
            raise Skip("the desktop assigned no keys, so no hotkey can show it does nothing")
        before = captures()
        xdo("key", "ctrl+super+s")
        time.sleep(2.5)
        opened = xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$").split()
        if opened:
            xdo("windowfocus", "--sync", opened[0])
            xdo("key", "Escape")
        return not opened and captures() == before, f"a capture opened: {bool(opened)}"

    @test("Micro-toys", "its switch on Home turns it on (saved), then Ctrl+Super+S and a drag copy exactly that area as a picture")
    def _():
        if not wm:
            raise Skip("openbox isn't installed (the capture needs a window manager to go full screen)")
        app.click(*AT["home_micro_switch"], wait=1.0)
        if not wait_for(lambda: app.config().get("micro_screenshot") is True, 4):
            return False, f"the switch didn't turn it on (micro_screenshot {app.config().get('micro_screenshot')})"
        app.start_toy("ctrl+super+s", "--screenshot")
        time.sleep(2.5)
        xdo("mousemove", 100, 100)
        xdo("mousedown", 1)
        xdo("mousemove", 300, 200)
        xdo("mousemove", 420, 260)
        xdo("mouseup", 1)
        time.sleep(2)
        paste = ["wl-paste", "--type", "image/png"] if WAYLAND else ["xclip", "-display", DISPLAY_NUM, "-o", "-selection", "clipboard", "-t", "image/png"]
        out = subprocess.run(paste, capture_output=True, timeout=5).stdout
        if not out.startswith(b"\x89PNG"):
            return False, f"no picture on the clipboard ({len(out)} bytes): " + app.output()[-300:]
        from PIL import Image
        size = Image.open(io.BytesIO(out)).size
        return size == (320, 160), f"picture {size[0]}x{size[1]} (dragged 320x160)"

    @test("Image Resizer", "archtoys --resize r50 makes a half-size copy next to the picture, and skips what isn't one")
    def _():
        if not (shutil.which("convert") or shutil.which("magick")):
            raise Skip("ImageMagick isn't installed")
        from PIL import Image
        folder = tempfile.mkdtemp(prefix="archtoys-resize-")
        Image.new("RGB", (640, 480), (200, 40, 90)).save(os.path.join(folder, "pic.png"))
        with open(os.path.join(folder, "notes.txt"), "w") as f:
            f.write("not a picture")
        # as a file manager starts it: names relative to the folder, the running copy does the work
        subprocess.run([BIN, "--resize", "r50", "pic.png", "notes.txt"], cwd=folder, env=app.env, timeout=10)
        made = os.path.join(folder, "pic-resized.png")
        if not wait_for(lambda: os.path.exists(made), 15):
            return False, f"no pic-resized.png: {sorted(os.listdir(folder))} " + app.output()[-300:]
        time.sleep(0.5)
        size = Image.open(made).size
        others = sorted(n for n in os.listdir(folder) if n not in ("pic.png", "notes.txt", "pic-resized.png"))
        return size == (320, 240) and not others, f"pic-resized.png {size[0]}x{size[1]} (want 320x240), other files {others}"

    @test("Awake", "the sidebar's cup opens its page")
    def _():
        app.click(*AT["menu"], wait=1.0)  # pins the icons
        app.click(*AT["rail_awake"])
        return bool(wait_for(lambda: app.config().get("last_page") == 4, 3)), f"last_page {app.config().get('last_page')}"
    app.quit()


def ruler_checks(wm):
    """The Screen Ruler on the frozen screen (full screen, so it needs the window manager too)."""
    if WAYLAND and kde.major >= 6:
        results.append(("Screen Ruler", "all checks", None, "skipped on Wayland (Plasma 6)"))
        return
    # (Wayland: Archtoys owning the hotkeys, so its Settings section is laid out as on X11)
    app = App({"last_page": 1}, extra_env={"ARCHTOYS_HOTKEY_BACKEND": "x11"} if WAYLAND else None)
    history = os.path.join(app.home, ".local", "share", "archtoys", "ruler-history.json")

    def entries():
        try:
            with open(history) as f:
                return json.load(f)
        except (OSError, ValueError):
            return []

    def open_ruler():
        app.start_toy("ctrl+super+m", "--measure")
        win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$").split(), 8)
        if not win and WAYLAND:  # the last one may still have been closing: once more
            app.start_toy("ctrl+super+m", "--measure")
            win = wait_for(lambda: xdo("search", "--onlyvisible", "--name", "^Archtoys Picker$").split(), 8)
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
        saved = (wait_for(lambda: entries()[:1], 3) or [])  # written just after the copy
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
        not_on_kwin6("Fedora's fonts lay Settings out a little differently, so a click can miss the switch")
        app.click(*AT["rail_settings"], wait=1.2)
        app.shot("ruler-settings")
        app.click(*AT["s_ruler_cm"])
        return bool(wait_for(lambda: app.config().get("ruler_units") == 1, 3)), f"ruler_units {app.config().get('ruler_units')}"

    @test("Screen Ruler", "the − and + buttons change the tolerance by 5 (Settings: Change the tolerance with)")
    def _():
        not_on_kwin6("Fedora's fonts lay Settings out a little differently, so a click can miss the switch")
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


class Overlay:
    """Text drawn into the picture the app is handed (Wayland); close() takes it away."""

    def __init__(self, img, at):
        self.item = (img, at)
        kde.overlays.append(self.item)
        kde.draw()

    def close(self):
        kde.overlays.remove(self.item)
        kde.draw()


def show_text(text, x, y):
    """Text on the screen to extract: drawn once with Pillow, kept by the X server."""
    from PIL import Image, ImageDraw, ImageFont
    font_file = run("fc-match", "-f", "%{file}", "DejaVu Sans").stdout.strip()
    font = ImageFont.truetype(font_file, 30)
    w, h = 620, 70
    img = Image.new("RGB", (w, h), "white")
    ImageDraw.Draw(img).text((14, 14), text, fill="black", font=font)
    if WAYLAND:
        return Overlay(img, (x, y))
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
    mark = {True: "✅", False: "❌", None: "⏭️"}

    def row(r):
        area, name, ok, detail = r
        return f"| {area} | {name} | {mark[ok]} | {detail.replace('|', '/')} |"

    table = ["| Area | Check | | Details |", "|---|---|---|---|"]
    lines = [f"### Archtoys, used like a person would ({DESKTOP}): {head}", "",
             f"{len(passed)} passed, {len(failed)} failed, {len(skipped)} skipped. Every check runs even after a failure.", ""]
    # what failed, in the open; what passed or was skipped, folded away
    if failed:
        lines += ["**Failed**", "", *table, *[row(r) for r in failed], ""]
    rest = [r for r in results if r[2] is not False]
    if rest:
        lines += [f"<details><summary>{len(passed)} passed, {len(skipped)} skipped (click to see them)</summary>", "",
                  *table, *[row(r) for r in rest], "", "</details>"]
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
