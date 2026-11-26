#!/bin/sh
# Check this repository's committed project layer with isolated native roots and inherited HOME. It runs the instrukt INSTRUKT_CHECK_BIN
# names: the commit check passes the one scripts/instrukt-for-checks chooses,
# which is this checkout's development build when no crate file is newer than
# it, else the installed instrukt, and every-file runs pass a build of the
# revision under check. Without INSTRUKT_CHECK_BIN it builds this checkout's
# instrukt and runs that. A project layer committed at an older schema than
# the program's, as a commit that bypassed instrukt's pre-commit section
# leaves it, is validated as converted in memory, its pending conversion a
# notice on stderr; one missing its schema version or at a newer one fails
# (project/instrukt/spec/feature/configuration/owned-files). The throwaway
# machine layer selects the runtime the project's modules need, and names an
# event transport for its event responses (validation only reads the setting).
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
# Keep all machine-layer commands in one selected-root lifetime.
# shellcheck disable=SC2016 # Variables expand in the delegated shell.
exec python3 scripts/with-runtime-homes.py sh -eu -c '
    binary=$1
    repository=$PWD
    cd "$(dirname "$INSTRUKT_HOME")"
    "$binary" config init --global >/dev/null
    revision=$("$binary" config get --global --revision)
    "$binary" config patch --global --revision "$revision" --yaml "runner: {runtime: t3}
redis: {cluster: {url: redis://127.0.0.1:6379}}"
    cd "$repository"
    "$binary" config validate
' check-config "$instrukt"
