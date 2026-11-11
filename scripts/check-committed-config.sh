#!/bin/sh
# Check this repository's committed project layer with the freshly built
# instrukt, under a throwaway HOME so no live home is read or written. A
# project layer committed at an older schema than the binary's, as a commit
# that bypassed instrukt's pre-commit section leaves it, is a finding
# (docs/project/spec/feature/configuration/owned-files.md). The throwaway
# machine layer selects the runtime the project's modules need.
set -eu
cargo build --locked --quiet --bin instrukt
target="${CARGO_TARGET_DIR:-target}"
case "$target" in
/*) ;;
*) target="$(pwd)/$target" ;;
esac
instrukt="$target/debug/instrukt"
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
