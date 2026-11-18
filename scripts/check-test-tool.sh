#!/bin/sh
# Keep local checks on the same tool releases as CI.
set -eu
# shellcheck source=scripts/test-tools.env
. "$(dirname "$0")/test-tools.env"
case "$1" in
    nextest)
        package=cargo-nextest; version=$NEXTEST_VERSION
        actual=$(cargo nextest --version 2>/dev/null | head -n 1) || actual=
        install="cargo install $package --locked --version =$version" ;;
    deny)
        package=cargo-deny; version=$DENY_VERSION
        actual=$(cargo deny --version 2>/dev/null | head -n 1) || actual=
        install="cargo install $package --locked --version =$version" ;;
    pre-commit)
        package=pre-commit; version=$PRE_COMMIT_VERSION
        actual=$(pre-commit --version 2>/dev/null | head -n 1) || actual=
        install="uv tool install $package==$version" ;;
    *) echo "usage: $0 nextest|deny|pre-commit" >&2; exit 2 ;;
esac
case "$actual" in
    "$package $version"|"$package $version "*) ;;
    *) echo "Install the pinned tool: $install" >&2; exit 1 ;;
esac
