#!/bin/sh
# Makes the generated files the .deb takes from target/deb-assets: the icon
# as PNGs (needs rsvg-convert, Debian's librsvg2-bin) and the shell
# completions. Usage: dist/deb-assets.sh <av binary> <THIRD-PARTY-LICENSES.txt>
set -eu
av=$1 licenses=$2
out=target/deb-assets
mkdir -p "$out"
for size in 48 128 256; do
    mkdir -p "$out/$size"
    rsvg-convert -w "$size" -h "$size" dist/asciiverse.svg -o "$out/$size/asciiverse.png"
done
"$av" completions bash > "$out/av.bash"
"$av" completions zsh > "$out/_av"
"$av" completions fish > "$out/av.fish"
cp "$licenses" "$out/THIRD-PARTY-LICENSES.txt"

# Debian wants a changelog; the real notes are the GitHub Release's
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
cat > "$out/changelog" <<END
av ($version-1) unstable; urgency=medium

  * Version $version: https://github.com/theasciiverse/av/releases/tag/v$version

 -- David Sandquist <sandquist@gmail.com>  $(date -R)
END
