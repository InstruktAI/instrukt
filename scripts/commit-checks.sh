#!/bin/sh
# This repository's commit check (project/instrukt/policy/testing): the checks
# of .pre-commit-config.yaml, through the pre-commit framework at the version
# pinned in scripts/test-tools.env.
#   scripts/commit-checks.sh [pre-commit options]   the staged files, as the pre-commit hook runs it
#   scripts/commit-checks.sh --all-files            every file, as CI and `make commit-checks` run it
# A commit never installs a check's tool: `make hooks` does
# (scripts/enable-hooks.sh), and the every-file run installs what is missing.
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
    # The framework fetches a hook's repository with git and installs its
    # environment with virtualenv and pip. Here git may only reach local
    # paths, pip may not reach a package index and virtualenv does not update
    # its seed packages, so nothing can be downloaded. `pre-commit
    # install-hooks` then only confirms that every hook's environment is in
    # place and healthy; anything missing fails it before any check runs, and
    # whatever it started is removed by the framework.
    GIT_ALLOW_PROTOCOL='file'
    PIP_NO_INDEX=1
    VIRTUALENV_NO_PERIODIC_UPDATE=1
    export GIT_ALLOW_PROTOCOL PIP_NO_INDEX VIRTUALENV_NO_PERIODIC_UPDATE
    if ! installed=$(pre-commit install-hooks 2>&1); then
        printf '%s\n' "$installed" >&2
        cat >&2 <<'EOF'

Commit refused: the tools .pre-commit-config.yaml pins for this commit's
checks are not installed, or not at the pinned revision. A commit does not
install them. Run `make hooks` in this checkout (it runs `pre-commit
install-hooks`), then commit again.
EOF
        exit 1
    fi
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
