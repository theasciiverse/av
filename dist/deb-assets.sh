#!/bin/sh
# Makes the generated files the .deb takes from target/deb-assets: the icon
# as PNGs (needs rsvg-convert, Debian's librsvg2-bin), the shell
# completions, the changelog (dist/debian-changelog.py, from ROADMAP.md)
# and the copyright file made by dist/debian-copyright.py.
# Usage: dist/deb-assets.sh <av binary> <copyright>
set -eu
av=$1 copyright=$2
out=target/deb-assets
mkdir -p "$out"
for size in 48 128 256; do
    mkdir -p "$out/$size"
    rsvg-convert -w "$size" -h "$size" dist/asciiverse.svg -o "$out/$size/asciiverse.png"
done
"$av" completions bash > "$out/av.bash"
"$av" completions zsh > "$out/_av"
"$av" completions fish > "$out/av.fish"
cp "$copyright" "$out/copyright"
python3 dist/debian-changelog.py > "$out/changelog"
