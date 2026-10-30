#!/usr/bin/env python3
"""The .deb's changelog, in Debian's format, from ROADMAP.md: every
"## <version>: built" section is one entry, its items the changes, dated
by the version's git tag (or now, for the version being built before it
is tagged). Newest first.

Usage: dist/debian-changelog.py > changelog   (from the repository root)
"""

import email.utils
import re
import subprocess
import textwrap
from pathlib import Path

MAINTAINER = "David Sandquist <sandquist@gmail.com>"

roadmap = Path("ROADMAP.md").read_text()
current = re.search(r'^version = "([^"]+)"', Path("Cargo.toml").read_text(), re.M).group(1)


def date_of(version):
    tag = subprocess.run(["git", "log", "-1", "--format=%aD", f"v{version}"], capture_output=True, text=True)
    return tag.stdout.strip() if tag.returncode == 0 and tag.stdout.strip() else email.utils.formatdate(localtime=True)


entries = []  # (version, [items])
for m in re.finditer(r"^## (\S+): built\n(.*?)(?=^## |\Z)", roadmap, re.M | re.S):
    items = []
    for line in m.group(2).splitlines():
        if line.startswith("- "):
            items.append(re.sub(r"^- (\[x\] )?", "", line))
        elif line.startswith("  ") and items:
            items[-1] += " " + line.strip()
    entries.append((m.group(1), [i.replace("`", "") for i in items]))
if current not in [v for v, _ in entries]:
    entries.append((current, [f"Version {current}."]))

out = []
for version, items in reversed(entries):
    out.append(f"av ({version}-1) unstable; urgency=medium")
    out.append("")
    for item in items:
        out += textwrap.wrap(item, 76, initial_indent="  * ", subsequent_indent="    ")
    out.append("")
    out.append(f" -- {MAINTAINER}  {date_of(version)}")
    out.append("")
print("\n".join(out).rstrip("\n"))
