#!/bin/sh
# Runs one partition of the Linux test suite from the archive the build job
# made (scripts/ci-ubuntu-build.sh), for CI's shard matrix
# (project/instrukt/policy/testing). The index is zero-based and the total is
# the number of shard jobs, both as the matrix reports them, so the partitions
# run cannot disagree with the number of jobs. The archive is extracted into
# the checkout, which puts the program at the path its tests were compiled to
# find. Only after the partition passed does the script print its completion
# record, one step-output line `shard_<k>=<k>/<total>`; everything nextest
# prints goes to standard error, so nothing else reaches the output file.
#   scripts/ci-ubuntu-shard.sh <job-index> <job-total> <archive-path>
set -eu

usage() {
    echo "usage: scripts/ci-ubuntu-shard.sh <job-index> <job-total> <archive-path>" >&2
    echo "$1" >&2
    exit 2
}

# A decimal count without a leading zero, so arithmetic never reads octal.
is_count() {
    case $1 in
    0) return 0 ;;
    '' | 0* | *[!0-9]*) return 1 ;;
    esac
    return 0
}

[ "$#" -eq 3 ] || usage "three arguments are required"
index=$1 total=$2 archive=$3
is_count "$index" || usage "the job index '$index' is not a number"
is_count "$total" || usage "the job total '$total' is not a number"
[ "$total" -ge 1 ] || usage "the job total must be at least 1"
[ "$index" -lt "$total" ] || usage "the job index $index is outside 0 to $((total - 1))"
[ -s "$archive" ] || usage "the test archive '$archive' is missing or empty"

partition=$((index + 1))
# Without INSTA_WORKSPACE_ROOT, insta finds the workspace by spawning
# `cargo metadata`. The shard job never provisions the project's toolchain and
# the image ships a newer Rust, so that call makes rustup install the version
# pinned in rust-toolchain.toml, which fails or races between test processes;
# insta then looks for snapshots under the crate directory instead. The archive
# was extracted into this checkout, so the current directory is the workspace
# root.
INSTA_WORKSPACE_ROOT="$PWD" cargo-nextest nextest run --archive-file "$archive" --extract-to . --extract-overwrite \
    --profile ci --partition "count:$partition/$total" >&2
printf 'shard_%s=%s/%s\n' "$partition" "$partition" "$total"
