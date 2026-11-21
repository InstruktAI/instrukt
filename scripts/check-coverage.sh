#!/bin/sh
# Check use-case coverage of this repository's tests against its feature
# specs (docs/project/policy/testing.md) with the freshly built instrukt,
# with native application roots and inherited HOME.
set -eu
cargo build --locked --quiet --bin instrukt
exec python3 scripts/with-runtime-homes.py "${CARGO_TARGET_DIR:-target}/debug/instrukt" specs coverage --check
