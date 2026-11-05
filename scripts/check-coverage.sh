#!/bin/sh
# Check use-case coverage of this repository's tests against its feature
# specs (docs/local/policy/testing.md) with the freshly built instrukt,
# under a throwaway HOME so no live home is read or written.
set -eu
cargo build --locked --quiet --bin instrukt
home="$(mktemp -d)"
trap 'rm -rf "$home"' EXIT
HOME="$home" "${CARGO_TARGET_DIR:-target}/debug/instrukt" specs coverage --check
