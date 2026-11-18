#!/bin/sh
# Check this repository's committed project layer under a throwaway HOME, so
# no live home is read or written. It runs the instrukt INSTRUKT_CHECK_BIN
# names: the commit check passes the one scripts/instrukt-for-checks chooses,
# which is this checkout's development build when no crate file is newer than
# it, else the installed instrukt, and every-file runs pass a build of the
# revision under check. Without INSTRUKT_CHECK_BIN it builds this checkout's
# instrukt and runs that. A project layer committed at an older schema than
# the program's, as a commit that bypassed instrukt's pre-commit section
# leaves it, is a finding
# (project/instrukt/spec/feature/configuration/owned-files). The throwaway
# machine layer selects the runtime the project's modules need.
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
home="$(mktemp -d)"
trap 'rm -rf "$home"' EXIT
unset XDG_STATE_HOME INSTRUKT_HOME
# The machine layer is written from outside the repository: a configuration
# writing verb run inside it would convert the very layer under check.
(
    cd "$home"
    HOME="$home" "$instrukt" config init --global >/dev/null
    revision="$(HOME="$home" "$instrukt" config get --global --revision)"
    HOME="$home" "$instrukt" config patch --global --revision "$revision" --yaml 'runner: {runtime: t3}'
)
HOME="$home" "$instrukt" config validate
