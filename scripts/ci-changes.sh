#!/bin/sh
# Prints, as GitHub step outputs, which of CI's checks the change under test
# reaches (project/instrukt/policy/testing). A pull request runs the checks its
# changed files reach; every other event (a push to main, a release calling the
# pipeline) runs every check. A pull request whose changed files cannot be
# read, or that changes a file no rule below names, runs every check too. A
# moved file counts as both the path it left and the path it reached.
#   scripts/ci-changes.sh               the change of GITHUB_EVENT_NAME, between BASE_SHA and HEAD_SHA
#   scripts/ci-changes.sh --classify    the outputs for the paths read from standard input, one per line
# The outputs, each true or false:
#   shell     shellcheck
#   rust      on Linux fmt, clippy, the doctests and the test suite sharded
#             across runners; on macOS clippy and the macOS tests
#   docs      document, entrypoint, use-case coverage and configuration checks (Linux)
#   deps      the dependency audit
#   graphify  the pinned Graphify contract
#   t3        the pinned T3 contract
#   python    the Python lint and type check among the commit checks
#   validate  the Linux build job builds instrukt and validates (rust or docs):
#             with validate true and rust false it validates and no test runs;
#             with both false neither the build job nor a test job starts
# and precommit_skip, the commit checks (.pre-commit-config.yaml hook ids) the
# Linux job leaves out: the Rust ones unless rust, shellcheck unless shell,
# the Python ones unless python; none when every check runs.
set -eu

shell=false rust=false docs=false deps=false graphify=false t3=false python=false

everything() {
    shell=true rust=true docs=true deps=true graphify=true t3=true python=true
}

# Each rule names the checks that read the paths it matches; a path takes the
# union of every rule it matches.
classify() {
    case $1 in
    # The pipeline itself, this script, the pinned tool versions (nextest,
    # cargo-deny, uv and pre-commit), and the scripts the workflow's jobs
    # delegate to, which are the pipeline's body: each selects every check.
    # A pipeline script added later selects every check only once it is
    # named here.
    .github/workflows/ci.yml | scripts/ci-changes.sh | scripts/test-tools.env | \
        scripts/ci-ubuntu-build.sh | scripts/ci-ubuntu-shard.sh | \
        scripts/ci-verify-jobs.sh | scripts/ci-macos.sh)
        everything; return 0 ;;
    esac
    known=false
    case $1 in
    # The commit checks' definition and the scripts they run; the CLI's
    # integration tests also read scripts (dev-instrukt, with-build-lock.sh,
    # install.sh, publish-release.sh).
    .pre-commit-config.yaml | scripts/*)
        known=true shell=true rust=true docs=true ;;
    esac
    case $1 in
    # What the Python lint and type check read. This rule alone names no
    # path as known: a Python file outside scripts/ and crates/, or a Python
    # project file, still runs every check.
    .pre-commit-config.yaml | *.py | pyproject.toml | uv.lock) python=true ;;
    esac
    case $1 in
    .githooks/* | *.sh) known=true shell=true ;;
    esac
    case $1 in
    Cargo.toml | Cargo.lock) known=true rust=true deps=true graphify=true t3=true ;;
    # A crate's manifest names its dependencies and its licence.
    crates/*/Cargo.toml) known=true rust=true deps=true ;;
    # The toolchain, cargo and nextest configuration, and the shared test
    # support compiled into the CLI's integration tests.
    rust-toolchain* | .cargo/* | .config/* | tests/*)
        known=true rust=true graphify=true t3=true ;;
    clippy.toml | schemas/*) known=true rust=true ;;
    deny.toml) known=true deps=true ;;
    esac
    case $1 in
    crates/*) known=true rust=true ;;
    esac
    case $1 in
    # What the Graphify contract test runs: the CLI and its harness, the
    # engine and its pin (instrukt-blueprint), module installation and the
    # generation the runner drives, and the crates those build on.
    crates/instrukt-cli/* | crates/instrukt-blueprint/* | crates/instrukt-runner/* | crates/instrukt-core/* | crates/instrukt-connector/*)
        graphify=true ;;
    esac
    case $1 in
    # What the T3 contract test runs: the CLI and its harness, the T3 adapter
    # and its pin (instrukt-runner), the crates it builds on (instrukt-blueprint
    # among them), and dispatch.
    crates/instrukt-cli/* | crates/instrukt-runner/* | crates/instrukt-blueprint/* | crates/instrukt-core/* | crates/instrukt-connector/* | crates/instrukt-todos/*)
        t3=true ;;
    esac
    case $1 in
    # The instrukt-todos unit tests and the CLI's integration tests read the
    # workflow flavor.
    docs/domain/sdlc/workflow/*) known=true rust=true docs=true ;;
    # Documents, the rendered instruction files and skills, the committed
    # project layer and the use-case coverage baseline. Root instrukt.yaml and
    # AGENTS.master.md belong here, not to rust: only the configuration and
    # instruction-file checks read them; no crate compiles or tests them.
    docs/* | skills/* | *.md | instrukt.yaml | instrukt-coverage-baseline.json)
        known=true docs=true ;;
    esac
    [ "$known" = true ] || everything
}

emit() {
    validate=false
    if [ "$rust" = true ] || [ "$docs" = true ]; then
        validate=true
    fi
    skip=
    [ "$rust" = true ] || skip="$skip,cargo-fmt,cargo-clippy-and-unit-tests"
    [ "$shell" = true ] || skip="$skip,shellcheck"
    [ "$python" = true ] || skip="$skip,ruff-check,mypy"
    printf '%s\n' "shell=$shell" "rust=$rust" "docs=$docs" "deps=$deps" \
        "graphify=$graphify" "t3=$t3" "python=$python" \
        "validate=$validate" "precommit_skip=${skip#,}"
}

classify_lines() {
    seen=false
    while IFS= read -r path || [ -n "$path" ]; do
        [ -n "$path" ] || continue
        seen=true
        classify "$path"
    done
    # A change without a single path is one this script cannot read.
    [ "$seen" = true ] || everything
    emit
}

# Every check, when the pull request's changed files cannot be read.
unreadable() {
    echo "::warning::$1; every check runs" >&2
    everything
    emit
    exit 0
}

if [ "${1:-}" = "--classify" ]; then
    classify_lines
    exit 0
fi
if [ "${GITHUB_EVENT_NAME:-}" != "pull_request" ]; then
    everything
    emit
    exit 0
fi
if [ -z "${BASE_SHA:-}" ] || [ -z "${HEAD_SHA:-}" ]; then
    unreadable "the pull request's base or head commit is not given"
fi
# Without rename detection a move lists both its old and its new path.
if ! paths=$(git diff --no-renames --name-only "$BASE_SHA...$HEAD_SHA"); then
    unreadable "the pull request's changed files could not be read"
fi
printf '%s\n' "$paths" | classify_lines
