#!/bin/sh
# Lints and unit-tests the crates the given paths belong to and every crate
# that depends on them (project/instrukt/policy/testing): clippy over all their
# targets, and their colocated unit tests. scripts/rust-crates.py selects them;
# a path the commit removed selects the crate it was in. Run it under the build
# lock (scripts/with-build-lock.sh), so what is already compiled is reused.
set -eu
packages=$(python3 "$(dirname "$0")/rust-crates.py" "$@")
all=$(printf '%s\n' "$packages" | sed -n 1p)
libs=$(printf '%s\n' "$packages" | sed -n 2p)
bins=$(printf '%s\n' "$packages" | sed -n 3p)
[ -n "$all" ] || exit 0
# shellcheck disable=SC2086 # one word per argument
cargo clippy $all --all-targets --locked -- -D warnings
if [ -n "$libs" ]; then
    # shellcheck disable=SC2086
    cargo test $libs --locked --lib
fi
if [ -n "$bins" ]; then
    # shellcheck disable=SC2086
    cargo test $bins --locked --bins
fi
