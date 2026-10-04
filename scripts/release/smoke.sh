#!/bin/sh
# Exercise the executable that was extracted from the archive.
set -eu
binary=$1
version=$2
test "$("$binary" --version)" = "paircomp $version"
"$binary" --help > /build/help.txt
grep -q -- '--full-digest' /build/help.txt
printf 'a\nb\n' > /build/smoke-file
printf 'y\n' | "$binary" --color never /build/smoke-file > /build/match.txt
grep -q 'Files match' /build/match.txt
# Identical first lines, then a differing byte on the second line.
status=0
printf 'n\n2\ny\ny\n2\nn\n' | "$binary" --color never /build/smoke-file > /build/differ.txt || status=$?
test "$status" = 1
grep -q 'First divergence: line 2, byte 1' /build/differ.txt
