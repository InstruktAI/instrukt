#!/bin/sh
# Points git at this repository's checked-in hooks (.githooks), once the
# pinned pre-commit framework they run is installed, and installs the tools
# .pre-commit-config.yaml pins for its checks, so the first commit after it
# does not install them. Installing is idempotent; a commit or every-file run
# that needs a tool not installed yet installs it itself. pre-commit's own
# `install` refuses while core.hooksPath is set, so the hooks run it
# themselves. The relative path lets each linked worktree run its own
# checkout's hooks.
set -eu
scripts/check-test-tool.sh pre-commit
pre-commit install-hooks
git config core.hooksPath .githooks
