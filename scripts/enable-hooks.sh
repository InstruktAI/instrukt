#!/bin/sh
# Points git at this repository's checked-in hooks (.githooks), once the
# pinned pre-commit framework they run is installed. pre-commit's own
# `install` refuses while core.hooksPath is set, so the hooks run it
# themselves. The relative path lets each linked worktree run its own
# checkout's hooks.
set -eu
scripts/check-test-tool.sh pre-commit
git config core.hooksPath .githooks
