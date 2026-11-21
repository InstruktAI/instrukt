#!/bin/sh
# Check that this repository's committed instruction files are the rendering
# of its committed documentation, with isolated native roots, so no live state is
# read or written. A commit made without instrukt's pre-commit section leaves
# them stale, and `instrukt docs render --check` names each one
# (project/instrukt/spec/feature/publication/agent-entrypoints, "A commit made
# without the hook"). It runs the instrukt INSTRUKT_CHECK_BIN names; without
# it, it builds this checkout's instrukt and runs that, so CI checks with the
# binary of the commit under check.
set -eu
instrukt="${INSTRUKT_CHECK_BIN:-}"
if [ -z "$instrukt" ]; then
    cargo build --locked --quiet --bin instrukt
    target="${CARGO_TARGET_DIR:-target}"
    case "$target" in
    /*) ;;
    *) target="$(pwd)/$target" ;;
    esac
    instrukt="$target/debug/instrukt"
fi
exec python3 scripts/with-runtime-homes.py "$instrukt" docs render --check
