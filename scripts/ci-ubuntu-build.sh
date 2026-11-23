#!/bin/sh
# The Rust half of CI's Linux build job (project/instrukt/policy/testing):
# workspace clippy, the nextest archive of every test binary that the shard
# jobs run (scripts/ci-ubuntu-shard.sh), and the doctests. The unit and
# functional tests run once, from the archive, and not here. Document,
# configuration and coverage validation stay in the workflow's own steps, so
# a change that selects no Rust never enters this script.
#   scripts/ci-ubuntu-build.sh <archive-path>
set -eu
if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    echo "usage: scripts/ci-ubuntu-build.sh <archive-path>" >&2
    exit 2
fi
archive=$1
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest archive --workspace --locked --archive-file "$archive"
cargo test --workspace --locked --doc
if [ ! -s "$archive" ]; then
    echo "::error::the test archive $archive is missing or empty" >&2
    exit 1
fi
