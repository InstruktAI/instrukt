#!/bin/sh
# This repository's lints by hand (`make lint`): shellcheck over the shell
# scripts and hook files, rustfmt and clippy over the workspace, and the size
# and suppression ratchet (scripts/check-ratchet.py). It stops at the first
# that fails.
set -eu
shellcheck -x scripts/*.sh scripts/dev-instrukt scripts/instrukt-for-checks .githooks/*
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
scripts/check-ratchet.py
