#!/bin/sh
# Finishes a .deb from cargo deb the way debhelper would: drops the empty
# Depends field it writes for a static binary, adds DEBIAN/md5sums (which
# dpkg --verify checks), and repacks with root as the owner.
# Usage: dist/deb-finish.sh <file.deb>
set -eu
deb=$1
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
dpkg-deb -R "$deb" "$dir/root"
sed -i '/^Depends: *$/d' "$dir/root/DEBIAN/control"
(cd "$dir/root" && find . -path ./DEBIAN -prune -o -type f -print | sed 's|^\./||' | LC_ALL=C sort | xargs md5sum > DEBIAN/md5sums)
dpkg-deb --root-owner-group -Zxz -b "$dir/root" "$deb" >/dev/null
