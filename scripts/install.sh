#!/bin/sh
# Install only a verified release binary, then atomically replace the old one.
set -eu
fail() { printf 'instrukt installer: %s\n' "$*" >&2; exit 1; }
tag=${INSTRUKT_VERSION:-}
while [ "$#" -gt 0 ]; do
    case $1 in
        --version) [ "$#" -ge 2 ] || fail '--version needs a tag'; tag=$2; shift 2 ;;
        -h|--help) echo 'Usage: install.sh [--version <tag>]'; exit 0 ;;
        *) fail "unknown argument: $1" ;;
    esac
done
os=$(uname -s)
arch=$(uname -m)
case "$os/$arch" in
    Darwin/arm64) target=aarch64-apple-darwin ;;
    Linux/x86_64) target=x86_64-unknown-linux-gnu ;;
    Linux/aarch64|Linux/arm64) target=aarch64-unknown-linux-gnu ;;
    *) fail "unsupported platform $os/$arch; supported: aarch64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu" ;;
esac
if command -v curl >/dev/null 2>&1; then downloader=curl
elif command -v wget >/dev/null 2>&1; then downloader=wget
else fail 'missing curl or wget'; fi
for tool in tar mktemp awk cp mv chmod mkdir rm; do command -v "$tool" >/dev/null 2>&1 || fail "missing $tool"; done
if command -v sha256sum >/dev/null 2>&1; then digest=sha256sum
elif command -v shasum >/dev/null 2>&1; then digest=shasum
else fail 'missing sha256sum or shasum'; fi
base=https://github.com/InstruktAI/instrukt/releases
fetch() {
    if [ "$downloader" = curl ]; then curl --proto '=https' --tlsv1.2 -fsSL "$1"
    else wget --https-only -qO- "$1"; fi
}
if [ -z "$tag" ]; then
    if [ "$downloader" = curl ]; then
        latest=$(curl --proto '=https' --tlsv1.2 -fsSL -o /dev/null -w '%{url_effective}' "$base/latest") || fail 'cannot resolve latest release'
    else
        latest=$(wget --https-only --spider --server-response "$base/latest" 2>&1 | awk '/^[[:space:]]*Location:/ {print $2}' | tail -n 1) || fail 'cannot resolve latest release'
    fi
    tag=${latest##*/}
fi
case $tag in v*) ;; *) tag=v$tag ;; esac
version=${tag#v}
case $version in ''|*[!0-9A-Za-z.+-]*) fail "invalid release tag $tag" ;; esac
[ "$version" != "$tag" ] || fail "invalid release tag $tag"
dir=${INSTRUKT_INSTALL_DIR:-${XDG_BIN_HOME:-$HOME/.local/bin}}
report() {
    printf 'instrukt %s installed at %s/instrukt\n' "$version" "$dir"
    # Print the literal shell variable for the caller.
    # shellcheck disable=SC2016
    case :${PATH:-}: in *:"$dir":*) ;; *) printf 'Add the directory to PATH: export PATH="%s:$PATH"\n' "$dir" ;; esac
    printf 'Next: instrukt -h\nnpx skills add InstruktAI/instrukt@%s --skill instrukt -g -y\n' "$tag"
}
if [ -x "$dir/instrukt" ] && [ "$("$dir/instrukt" --version 2>/dev/null || :)" = "instrukt $version" ]; then
    printf 'Already installed: instrukt %s\n' "$version"; report; exit 0
fi
umask 077
tmp=$(mktemp -d) || fail 'cannot create private temporary directory'
staged=
cleanup() { rm -rf "$tmp"; if [ -n "$staged" ]; then rm -f "$staged"; fi; }
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
name=instrukt-$tag-$target
asset=$name.tar.gz
fetch "$base/download/$tag/$asset" > "$tmp/$asset" || fail "download failed: $asset"
fetch "$base/download/$tag/SHA256SUMS" > "$tmp/SHA256SUMS" || fail "download failed: SHA256SUMS for $asset"
expected=$(awk -v file="$asset" '$2 == file {print $1; n++} END {if (n != 1) exit 1}' "$tmp/SHA256SUMS") || fail "checksum entry missing or duplicated: $asset"
if [ "$digest" = sha256sum ]; then actual=$(sha256sum "$tmp/$asset")
else actual=$(shasum -a 256 "$tmp/$asset"); fi
actual=${actual%% *}
[ "$expected" = "$actual" ] || fail "checksum failed: $asset"
# Check member names and types before extracting; links are not release files.
tar -tzf "$tmp/$asset" > "$tmp/members" || fail "archive listing failed: $asset"
awk -v root="$name" '
    $0 == root || $0 == root "/" {next}
    $0 == root "/instrukt" {binary++; next}
    $0 == root "/README.md" {readme++; next}
    {exit 1}
    END {if (binary != 1 || readme != 1) exit 1}
' "$tmp/members" || fail "unexpected archive layout: $asset"
tar -tvzf "$tmp/$asset" | awk 'substr($0,1,1) != "-" && substr($0,1,1) != "d" {exit 1}' || fail "archive contains links: $asset"
tar -xzf "$tmp/$asset" -C "$tmp" || fail "unpack failed: $asset"
binary=$tmp/$name/instrukt
[ -f "$binary" ] && [ -x "$binary" ] || fail "binary missing or not executable: $asset"
reported=$("$binary" --version) || fail "binary cannot run: $asset"
[ "$reported" = "instrukt $version" ] || fail "binary version mismatch: $asset (reported $reported)"
[ ! -d "$dir/instrukt" ] || fail "install destination is a directory: $dir/instrukt"
mkdir -p "$dir" || fail "cannot create install directory for $asset"
staged=$(mktemp "$dir/.instrukt.XXXXXX") || fail "cannot stage $asset"
if ! cp "$binary" "$staged" || ! chmod 755 "$staged"; then fail "cannot stage binary: $asset"; fi
mv -f "$staged" "$dir/instrukt" || fail "cannot replace binary: $asset"
staged=
report
