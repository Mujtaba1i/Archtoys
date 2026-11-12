# Contributing to Archtoys

Thank you for helping. Bug reports, ideas and pull requests are all welcome. Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting a problem

The easiest way is **Settings → Report a problem** in the app. It opens a new GitHub issue with your version, distribution, desktop, screens and the picker and hotkey methods already filled in, so you only describe what went wrong. It never includes file paths, user names, colors or your history.

Running `archtoys` from a terminal prints which picker and hotkey method it chose and why it fell back, which helps too.

## Ideas

Which PowerToys tool do you miss on Linux? [Open an issue](https://github.com/Mujtaba1i/Archtoys/issues) and say so. Archtoys adds one toy or one main change per release, so a good idea may wait for its turn.

## Building and testing

You need Rust (the latest stable, [rustup](https://rustup.rs) if your distribution's is old) and a few libraries:

```bash
# Fedora
sudo dnf install rust cargo gcc pkgconf-pkg-config fontconfig-devel freetype-devel libX11-devel libxkbcommon-devel

# Arch
sudo pacman -S --needed rust pkgconf fontconfig libx11 libxkbcommon

# Debian / Ubuntu
sudo apt install cargo pkg-config libfontconfig1-dev libx11-dev libxcb-shm0-dev libxkbcommon-dev
```

Then:

```bash
cargo test --locked                  # the unit tests
cargo run --release --locked         # the app
cargo fmt --check                    # the code style CI checks
cargo clippy --locked --all-targets -- -D warnings
```

Always build with `--locked` and keep `Cargo.lock` in step, or CI fails.

After changing a `.slint` file, run `touch src/main.rs` before building: Cargo doesn't always notice the change.

### The end-to-end test

`.github/scripts/e2e_test.py` uses the app like a person would, on a virtual X11 screen, and checks the result in the settings file, the clipboard and the pixels. CI runs it on every push. To run it yourself (it needs `Xvfb`, `xdotool`, `xclip`, `openbox`, `python3-xlib` and `python3-pil`):

```bash
cargo build --release --locked
dbus-run-session -- python3 .github/scripts/e2e_test.py target/release/archtoys
```

`--shot` saves a screenshot of each screen. The click positions are for the default window, so a change to the layout may need new positions in `AT`.

## Sending a change

1. Fork the repository and make a branch.
2. Keep the change to one thing. A fix and a new feature are two pull requests.
3. Run the tests, `cargo fmt` and clippy (above).
4. Write the commit message for the people who use Archtoys: the release notes are made from it. Say what changed for them, not how the code changed.
5. Open a pull request. CI builds it, runs every check and compares its speed and memory with main.

New dependencies need a good reason: Archtoys is kept small on purpose. Check the licence is allowed by `deny.toml` (`cargo deny check licenses`).

Archtoys runs on Wayland and X11, on KDE Plasma, GNOME and others. If you can, try your change on more than one, and say in the pull request where you tested it.
