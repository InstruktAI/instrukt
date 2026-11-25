#!/bin/sh
# The Rust half of CI's Linux build job (project/instrukt/policy/testing):
# workspace clippy and the doctests. The nextest archive of every test binary
# that the shard jobs run (scripts/ci-ubuntu-shard.sh) is built by the
# workflow's archive job, so the shards do not wait for these checks; the unit
# and functional tests run once, from that archive, and not here. Document,
# configuration and coverage validation stay in the workflow's own steps, so
# a change that selects no Rust never enters this script.
#   scripts/ci-ubuntu-build.sh
set -eu
if [ "$#" -ne 0 ]; then
    echo "usage: scripts/ci-ubuntu-build.sh" >&2
    exit 2
fi
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked --doc
