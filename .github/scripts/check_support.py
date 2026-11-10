"""Buy me a coffee: every button points at the same support page, and the page
works both ways (forwarding to Ko-fi, or saying support isn't set up).

The address lives in src/launch.rs (SUPPORT_URL); the README button,
.github/FUNDING.yml and the AppStream metainfo must use the same one, and
docs/support/index.html is the page GitHub Pages serves there.
Prints what's wrong in plain words; exit 1 if anything is.
"""
import re
import sys

problems = []


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


launch = read("src/launch.rs")
m = re.search(r'pub const SUPPORT_URL: &str = "([^"]+)";', launch)
if not m:
    sys.exit("src/launch.rs has no SUPPORT_URL: the app's Buy me a coffee button has nowhere to go.")
url = m.group(1)
print(f"The support page: {url}")

for path, what in [("README.md", "the README's Buy me a coffee button"),
                   (".github/FUNDING.yml", "GitHub's Sponsor button (FUNDING.yml)"),
                   ("packaging/io.github.Mujtaba1i.Archtoys.metainfo.xml", "the software centers' Donate link (metainfo)")]:
    if url not in read(path):
        problems.append(f"{what} doesn't point at {url}. Use the same address as SUPPORT_URL in src/launch.rs.")

if not re.fullmatch(r"https://[a-z0-9-]+\.github\.io/[^/]+/support/", url):
    problems.append(f"{url} isn't a GitHub Pages address ending in /support/, where docs/support/index.html is served.")

page = read("docs/support/index.html")
link = re.search(r'<a id="kofi" href="([^"]*)"', page)
if not link:
    problems.append('docs/support/index.html lost its <a id="kofi" href="..."> line: the page has no link to forward to.')
elif link.group(1) and not re.fullmatch(r"https://ko-fi\.com/[A-Za-z0-9_]+/?", link.group(1)):
    problems.append(f'docs/support/index.html links to "{link.group(1)}": it must be a ko-fi.com page, or empty to pause support.')
else:
    print(f"It forwards to: {link.group(1) or '(nothing: support is paused, the page says so)'}")
if "isn’t set up right now" not in page:
    problems.append("docs/support/index.html lost its \"isn’t set up right now\" message, shown when the Ko-fi link is empty.")

for p in problems:
    print(f"::error title=Buy me a coffee::{p}")
    print(p)
if problems:
    sys.exit(1)
print("Every Buy me a coffee button points at the same page, and the page works.")
