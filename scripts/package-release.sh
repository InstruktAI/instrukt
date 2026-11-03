#!/bin/sh
# Package a built release binary for one target into
# instrukt-<tag>-<target>.tar.gz in the current directory.
# Usage: scripts/package-release.sh <tag> <target>
set -eu
tag="$1"
target="$2"
name="instrukt-${tag}-${target}"
mkdir "$name"
cp "target/${target}/release/instrukt" README.md "$name/"
tar -czf "$name.tar.gz" "$name"
rm -r "$name"
