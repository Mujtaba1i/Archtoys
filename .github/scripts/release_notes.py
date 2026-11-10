"""Splits the commits since the last release for the release notes.

Reads "author<TAB>subject" lines (oldest first) on stdin and prints one list:
  release_notes.py changes     what users read (one line each)
  release_notes.py technical   Dependabot's updates, for the folded panel
Left out of both: "Release vX" commits, "Add supporter: ..." commits (the
SUPPORTERS.md Action) and empty subjects. Each subject once, in order.
"""
import re
import sys

UPDATE_BOTS = {"dependabot[bot]"}
SKIP = re.compile(r"^(Release v[0-9]|Add supporter\b)")


def split(lines):
    changes, technical = [], []
    for line in lines:
        author, _, subject = line.rstrip("\n").partition("\t")
        subject = subject.strip()
        if not subject or SKIP.match(subject):
            continue
        target = technical if author.strip() in UPDATE_BOTS else changes
        if subject not in target:
            target.append(subject)
    return changes, technical


def main(which):
    changes, technical = split(sys.stdin)
    print("\n".join(changes if which == "changes" else technical))


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "changes")
