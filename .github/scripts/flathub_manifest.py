"""The Flatpak manifest for Flathub: the archtoys module built from the release's
git tag and commit, not from this checkout.

    python3 flathub_manifest.py MANIFEST TAG COMMIT > flathub-manifest.yml

In this repo the archtoys module's first source is `type: dir` (local builds:
flatpak-builder from a checkout). Flathub builds from git, so that source
becomes `type: git` with url, tag and commit. If it is already a git source,
only its tag and commit change. Every other line (comments included) stays as
it is. Line-based on purpose: a YAML library would drop the comments.
"""

import re
import sys

URL = "https://github.com/Mujtaba1i/Archtoys.git"


def source_block(lines, start):
    """The lines of the list item starting at `start`: until the next item at
    the same indent, or a line indented less."""
    indent = len(lines[start]) - len(lines[start].lstrip())
    end = start + 1
    while end < len(lines):
        line = lines[end]
        if line.strip() and len(line) - len(line.lstrip()) <= indent:
            break
        end += 1
    return indent, end


def rewrite(text, tag, commit):
    if not re.fullmatch(r"v\d+\.\d+\.\d+", tag):
        raise ValueError(f"not a release tag: {tag!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError(f"not a full commit hash: {commit!r}")
    lines = text.split("\n")
    module = next((i for i, l in enumerate(lines) if l.strip() == "- name: archtoys"), None)
    if module is None:
        raise ValueError("no archtoys module in the manifest")
    sources = next((i for i in range(module + 1, len(lines)) if lines[i].strip() == "sources:"), None)
    if sources is None:
        raise ValueError("the archtoys module has no sources")
    first = sources + 1
    while first < len(lines) and not lines[first].strip():
        first += 1
    item = lines[first].strip()
    indent, end = source_block(lines, first)
    pad = " " * indent
    if item == "- type: git":
        block = lines[first:end]
        for key, value in (("tag", tag), ("commit", commit)):
            at = [i for i, l in enumerate(block) if l.strip().startswith(f"{key}:")]
            if not at:
                raise ValueError(f"the git source has no {key}")
            line = block[at[0]]
            block[at[0]] = line[: len(line) - len(line.lstrip())] + f"{key}: {value}"
        new = block
    elif item == "- type: dir":
        new = [f"{pad}- type: git", f"{pad}  url: {URL}", f"{pad}  tag: {tag}", f"{pad}  commit: {commit}"]
    else:
        raise ValueError(f"the archtoys module's first source is {item!r}, not a dir or git source")
    return "\n".join(lines[:first] + new + lines[end:])


def main(argv):
    if len(argv) != 4:
        print(__doc__.strip().split("\n\n")[1], file=sys.stderr)
        return 2
    with open(argv[1], encoding="utf-8") as f:
        text = f.read()
    try:
        sys.stdout.write(rewrite(text, argv[2], argv[3]))
    except ValueError as err:
        print(f"flathub_manifest: {err}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
