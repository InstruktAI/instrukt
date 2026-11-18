#!/bin/sh
# Points git at this repository's checked-in hooks (.githooks), once the
# pinned pre-commit framework they run is installed, and installs the tools
# .pre-commit-config.yaml pins for its checks: a commit never installs them and
# refuses without them. Run it again when a pinned tool changes. pre-commit's
# own `install` refuses while core.hooksPath is set, so the hooks run it
# themselves. The relative path lets each linked worktree run its own
# checkout's hooks.
set -eu
scripts/check-test-tool.sh pre-commit
pre-commit install-hooks
# A newly installed tool's first start is much slower than every start after
# it. Each hook the framework installs a tool for (the hooks of the repos
# other than `local` in .pre-commit-config.yaml) starts here once, over the
# files it applies to, so that first start belongs to this step and not to a
# commit. Its verdict is shown; acting on it is the commit's and CI's part.
pre-commit run ruff-check --all-files || true
pre-commit run mypy --all-files || true
git config core.hooksPath .githooks
