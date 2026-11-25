#!/bin/sh
# Decides CI's required Linux check, `fmt, clippy, test (ubuntu-latest)`
# (project/instrukt/policy/testing), from the classification and from the
# build and shard jobs. The archive job that builds the shards' test archive is
# not examined: it is reached through the partition records, which exist only
# when it succeeded and the shards ran from its archive. A part is deselected
# only when the `changes` job succeeded and wrote the literal `false` for it;
# validation is also selected whenever Rust is. Every selected job must both
# have left its completion record and be reported as `success`: a record shows
# the job finished its work, a result shows how the job ended, and neither
# stands in for the other.
# The k-th of N partition records must read exactly k/N. A deselected part is
# not examined. Prints each requirement and whether it was met, and exits
# non-zero when a selected requirement is unmet.
#   scripts/ci-verify-jobs.sh <changes-result> <validate> <rust> <build-result>
#       <build-record> <test-result> <partition-record>...
set -eu

# The record the build job writes as its last step (.github/workflows/ci.yml).
build_record_literal=built

if [ "$#" -lt 7 ]; then
    echo "usage: scripts/ci-verify-jobs.sh <changes-result> <validate> <rust> <build-result> <build-record> <test-result> <partition-record>..." >&2
    exit 2
fi
changes_result=$1 validate=$2 rust=$3 build_result=$4 build_record=$5 test_result=$6
shift 6

rust_selected=true
validate_selected=true
if [ "$changes_result" = success ]; then
    [ "$rust" != false ] || rust_selected=false
    [ "$validate" != false ] || validate_selected=false
fi
[ "$rust_selected" = false ] || validate_selected=true

unmet=0
require() {
    if [ "$2" = "$3" ]; then
        printf 'met:   %s (%s)\n' "$1" "$2"
    else
        printf 'UNMET: %s (expected %s, got %s)\n' "$1" "$3" "${2:-nothing}"
        unmet=$((unmet + 1))
    fi
}

printf 'classification: changes %s, validate %s, rust %s\n' \
    "$changes_result" "${validate:-unset}" "${rust:-unset}"
if [ "$validate_selected" = true ]; then
    require "build job result" "$build_result" success
    require "build job record" "$build_record" "$build_record_literal"
else
    echo "validation deselected: the build job is not examined"
fi
if [ "$rust_selected" = true ]; then
    require "shard job result" "$test_result" success
    total=$#
    k=0
    for record in "$@"; do
        k=$((k + 1))
        require "partition $k record" "$record" "$k/$total"
    done
else
    echo "Rust deselected: the shard job is not examined"
fi

if [ "$unmet" -gt 0 ]; then
    echo "::error::$unmet selected requirement(s) unmet" >&2
    exit 1
fi
