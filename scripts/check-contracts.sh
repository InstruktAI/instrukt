#!/bin/sh
# The repository's contract checks after its tests: use-case coverage of the
# feature specs, then the committed project layer
# (docs/project/policy/testing.md,
# docs/project/spec/feature/configuration/owned-files.md).
set -eu
scripts/check-coverage.sh
scripts/check-committed-config.sh
