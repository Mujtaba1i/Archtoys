"""Adds a supporter to SUPPORTERS.md (the "Add a supporter" Action).

Started by the Cloudflare Worker for each public one-time tip on Ko-fi
(support/kofi-worker.js), or by hand from the Actions tab. Input comes from
environment variables, never the command line:
  NAME, MESSAGE  as the supporter wrote them (cleaned here again)
  ID             Ko-fi's message id, so a tip Ko-fi sends twice is listed once
  TEST           "true" for Ko-fi's "Send a test": nobody is added
Writes a plain-words line to the run summary, and changed=true / name=... to
$GITHUB_OUTPUT when the file changed (the workflow then commits it).
"""
import os
import re
import sys

FILE = "SUPPORTERS.md"
MARK = "<!-- New supporters are added below this line"
MAX_NAME, MAX_MESSAGE = 60, 200
ID_OK = re.compile(r"^[A-Za-z0-9_-]{1,80}$")


def clean(value, limit):
    """One plain line: no control characters, spaces squeezed, at most `limit` characters."""
    text = re.sub(r"[\x00-\x1f\x7f\u2028\u2029]+", " ", value or "")
    text = re.sub(r"\s+", " ", text).strip()
    return text[:limit].strip()


def markdown_safe(text):
    """Shown as typed: no links, images, HTML or formatting can sneak in."""
    return re.sub(r"([\\`*_{}\[\]()<>#+!|~])", r"\\\1", text)


def add(content, name, message="", kofi_id="", test=False):
    """Returns (new content or None, what happened in plain words)."""
    name, message = clean(name, MAX_NAME), clean(message, MAX_MESSAGE)
    if test:
        return None, "Ko-fi test received: the chain Ko-fi → Cloudflare → GitHub works. Nobody was added."
    if not name:
        return None, "No name was given, so nobody was added."
    kofi_id = kofi_id if ID_OK.match(kofi_id or "") else ""
    if kofi_id and f"<!-- kofi:{kofi_id} -->" in content:
        return None, f"{name} is already listed (Ko-fi sent this tip twice). Nothing changed."
    if MARK not in content:
        return None, f"{FILE} has lost its marker line ({MARK} …), so {name} wasn't added. Put the line back."
    line = f"- **{markdown_safe(name)}**"
    if message:
        line += f": “{markdown_safe(message)}”"
    if kofi_id:
        line += f" <!-- kofi:{kofi_id} -->"
    return content.rstrip("\n") + "\n" + line + "\n", f"Added {name} to {FILE}. Thank you, {name}!"


def main():
    env = os.environ
    with open(FILE, encoding="utf-8") as f:
        content = f.read()
    new, note = add(content, env.get("NAME", ""), env.get("MESSAGE", ""), env.get("ID", ""),
                    env.get("TEST", "").lower() == "true")
    print(note)
    with open(env.get("GITHUB_STEP_SUMMARY", os.devnull), "a", encoding="utf-8") as f:
        f.write(f"### Add a supporter\n\n{note}\n")
    if new is not None:
        with open(FILE, "w", encoding="utf-8") as f:
            f.write(new)
        with open(env.get("GITHUB_OUTPUT", os.devnull), "a", encoding="utf-8") as f:
            f.write(f"changed=true\nname={clean(env.get('NAME', ''), MAX_NAME)}\n")
    # a missing marker is a real failure: the run goes red so it gets noticed
    return 1 if "lost its marker" in note else 0


if __name__ == "__main__":
    sys.exit(main())
