#!/bin/sh
# Keep local checks on the same tool releases as CI.
set -eu
# shellcheck source=scripts/test-tools.env
. "$(dirname "$0")/test-tools.env"
case "$1" in
    nextest) package=cargo-nextest; version=$NEXTEST_VERSION ;;
    deny) package=cargo-deny; version=$DENY_VERSION ;;
    *) echo "usage: $0 nextest|deny" >&2; exit 2 ;;
esac
actual=$(cargo "$1" --version 2>/dev/null | head -n 1) || actual=
case "$actual" in
    "$package $version"|"$package $version "*) ;;
    *) echo "Install the pinned runner: cargo install $package --locked --version =$version" >&2; exit 1 ;;
esac
