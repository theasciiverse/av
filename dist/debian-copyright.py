#!/usr/bin/env python3
"""The .deb's /usr/share/doc/av/copyright, in Debian's machine-readable
format (https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/).

av is one static binary, so every crate built into it is listed: the
license it is used under (the first of about.toml's accepted licenses
that the crate offers), its own license expression when that is a choice,
and its authors. The full text of each license follows once.

Usage: cargo about generate --format json --target <triple>... | dist/debian-copyright.py
"""

import json
import re
import sys
from pathlib import Path

# SPDX's own text of each license (from the spdx crate's src/text/licenses),
# used ahead of a copy found in some crate
canonical = Path(__file__).parent / "licenses"
# what Debian ships in /usr/share/common-licenses is pointed to, not copied
COMMON = {"Apache-2.0": "the Apache License, Version 2.0", "MPL-2.0": "the Mozilla Public License, Version 2.0"}

about = json.load(sys.stdin)


def field(name, lines):
    """A field whose value runs over several lines, continued with a space."""
    lines = [l for l in lines if l] or ["unknown"]
    return [f"{name}: {lines[0]}"] + [f" {l}" for l in lines[1:]]


def text_block(text):
    """Free text inside a field: every line indented, blank lines as ' .'."""
    out = []
    for line in text.strip("\n").splitlines():
        line = line.rstrip()
        out.append(" " + line if line else " .")
    return out


crates = {}  # (name, version) -> (license used, crate)
texts = {}  # license id -> its text, without copyright lines
for lic in about["licenses"]:
    for used in lic["used_by"]:
        c = used["crate"]
        if c["name"] == "av":
            continue
        crates[(c["name"], c["version"])] = (lic["id"], c)
    if lic["id"] in texts:
        continue
    if lic["id"] in COMMON:
        texts[lic["id"]] = (
            f"On Debian systems, the full text of {COMMON[lic['id']]}\n"
            f"can be found in /usr/share/common-licenses/{lic['id']}."
        )
        continue
    own = canonical / f"{lic['id']}.txt"
    lines = (own.read_text() if own.exists() else lic["text"]).strip("\n").splitlines()
    # the holders are in each Files paragraph, so no copyright lines here
    lines = [l for l in lines if not re.match(r"\s*Copyright\s*(\(c\)|©|<|\d)", l, re.I)]
    # a license file written as Markdown: no heading marks, one blank line at a time
    text = re.sub(r"^#+\s*", "", "\n".join(lines), flags=re.M)
    texts[lic["id"]] = re.sub(r"\n\s*\n(\s*\n)+", "\n\n", text).strip("\n")

out = [
    "Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/",
    "Upstream-Name: av",
    "Upstream-Contact: David Sandquist <sandquist@gmail.com>",
    "Source: https://github.com/theasciiverse/av",
    "Comment: av is one static binary, built from its own source and from the",
    " Rust crates listed below, all of which are in it. Each crate is listed",
    " with the license it is used under; its own license expression follows",
    " when it offers a choice.",
    "",
    "Files: *",
    "Copyright: 2026 David Sandquist",
    "License: MIT",
]
for (name, version), (used, c) in sorted(crates.items()):
    out.append("")
    out.append(f"Files: vendor/{name}-{version}/*")
    authors = [re.sub(r"\s+", " ", a).strip() for a in c.get("authors") or []]
    out += field("Copyright", authors or [f"the {name} developers"])
    out.append(f"License: {used}")
    if c.get("license") and c["license"] != used:
        out.append(f"Comment: {name} is offered under {c['license']}; used under {used}.")

for lid in sorted(texts):
    out.append("")
    out.append(f"License: {lid}")
    out += text_block(texts[lid]) if texts[lid].strip() else []

print("\n".join(out))
