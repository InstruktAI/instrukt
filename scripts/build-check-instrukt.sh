#!/bin/sh
# Builds this checkout's instrukt and copies it to the given path, both inside
# one hold of the shared build lock, so no other checkout's clean or build can
# remove or replace the binary between the build and the copy. The copy lives
# outside the target directory, so it outlives the lock.
#   scripts/with-build-lock.sh scripts/build-check-instrukt.sh <path>
set -eu
if [ -z "${INSTRUKT_BUILD_LOCKED:-}" ]; then
    echo "build-check-instrukt.sh runs under scripts/with-build-lock.sh" >&2
    exit 2
fi
cargo build --locked --bin instrukt
cp "${CARGO_TARGET_DIR:-target}/debug/instrukt" "$1"
