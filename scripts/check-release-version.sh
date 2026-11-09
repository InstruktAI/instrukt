#!/bin/sh
# Refuse publishing a tag whose binary and cockpit versions disagree.
set -eu
tag=${1:?Usage: check-release-version.sh vVERSION}
workspace=$(awk '/^\[workspace.package\]$/ {section=1; next} /^\[/ {section=0} section && /^version = / {gsub(/"/, "", $3); print $3}' Cargo.toml)
skill=$(awk '/^metadata:$/ {metadata=1; next} metadata && /^  version:/ {gsub(/"/, "", $2); print $2; exit}' skills/instrukt/SKILL.md)
if [ "$tag" != "v$workspace" ] || [ "$workspace" != "$skill" ]; then
    printf 'release version mismatch: tag=%s workspace=%s skill=%s\n' "$tag" "$workspace" "$skill" >&2
    exit 1
fi
