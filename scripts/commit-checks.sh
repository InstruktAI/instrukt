#!/bin/sh
# This repository's commit check (project/instrukt/policy/testing): the checks
# of .pre-commit-config.yaml, through the pre-commit framework at the version
# pinned in scripts/test-tools.env.
#   scripts/commit-checks.sh [pre-commit options]   the staged files, as the pre-commit hook runs it
#   scripts/commit-checks.sh --all-files            every file, as CI and `make commit-checks` run it
set -eu
# Linked worktrees build into the main checkout's target directory unless
# CARGO_TARGET_DIR says otherwise, so no worktree grows a target of its own;
# its cargo checks take that directory's lock (scripts/with-build-lock.sh).
if [ -z "${CARGO_TARGET_DIR:-}" ]; then
    common=$(git rev-parse --path-format=absolute --git-common-dir)
    CARGO_TARGET_DIR="$(dirname "$common")/target"
    export CARGO_TARGET_DIR
fi
scripts/check-test-tool.sh pre-commit
if [ "${1:-}" != "--all-files" ]; then
    exec pre-commit run --hook-stage pre-commit "$@"
fi
# Every file is validated with an instrukt built from this revision: built and
# copied out of the target directory within one hold of the build lock, so
# another checkout sharing the target can neither remove nor replace it before
# the copy, and no cargo command removes the copy.
checks=$(mktemp -d)
trap 'rm -rf "$checks"' EXIT
scripts/with-build-lock.sh scripts/build-check-instrukt.sh "$checks/instrukt"
INSTRUKT_CHECK_BIN="$checks/instrukt" pre-commit run --all-files --show-diff-on-failure
