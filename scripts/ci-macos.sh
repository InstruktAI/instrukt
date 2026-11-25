#!/bin/sh
# CI's macOS check (project/instrukt/policy/testing): workspace clippy always;
# on a pull request the macOS selection, the tests that assert
# platform-specific behaviour together with the hooks and write-fence paths;
# on every other event, a push to main, a tagged release or any event added
# later, the full workspace suite and the doctests. Only the literal
# `pull_request` narrows, as in scripts/ci-changes.sh, so an unforeseen event
# runs more, not less. Formatting, document, configuration, coverage, Python
# and shell checks are OS-independent and run on Linux alone.
#   scripts/ci-macos.sh <event-name>
set -eu
if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    echo "usage: scripts/ci-macos.sh <event-name>" >&2
    exit 2
fi
event=$1

# The macOS selection, one group per line:
#   Darwin process-group cleanup (crates/instrukt-core/src/process.rs);
#   the write fence's path resolution and admission
#   (crates/instrukt-core/src/fsx/_fence.rs);
#   launchd labels and scheduler units (instrukt-runner install and installation);
#   launchd services and socket custody (cases/module-installation.rs);
#   the launchd wake-up registration (cases/task-timing.rs);
#   the kqueue watch's folder descriptor (support/conversing.rs);
#   the write fence and /var to /private/var paths (harness);
#   git hooks and their paths (hook_sections).
# --no-tests=fail trips only when the whole expression selects nothing; a
# renamed test inside one group is not caught by it.
selection='package(instrukt-core) & test(darwin_group_tests)
| package(instrukt-core) & test(/^fsx::_fence::/)
| package(instrukt-runner) & test(install)
| package(instrukt-cli) & test(/^module_installation::/)
| package(instrukt-cli) & test(=task_timing::the_installed_wake_up_runs_due_work_under_its_missed_run_policy)
| package(instrukt-cli) & test(=conversations::a_members_question_is_answered_in_that_conversation_and_nowhere_else)
| package(instrukt-cli) & test(/^harness::/)
| package(instrukt-cli) & test(/^hook_sections::/)'

cargo clippy --workspace --all-targets --locked -- -D warnings
if [ "$event" = pull_request ]; then
    cargo nextest run --workspace --locked --profile ci --no-tests=fail -E "$selection"
else
    cargo nextest run --workspace --locked --profile ci
    cargo test --workspace --locked --doc
fi
