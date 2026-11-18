#!/bin/sh
# Lints and unit-tests the crates the given files belong to and every crate
# that depends on them (docs/project/policy/testing.md): clippy over all their
# targets, and their colocated unit tests. A file outside every crate, such as
# the workspace manifest, the lock file or the toolchain and clippy
# configuration, selects every crate. Run it under the build lock
# (scripts/with-build-lock.sh), so what is already compiled is reused.
set -eu
packages=$(python3 - "$@" <<'PY'
import json, os, subprocess, sys
meta = json.loads(subprocess.run(
    ["cargo", "metadata", "--format-version", "1", "--no-deps", "--locked"],
    capture_output=True, check=True, text=True).stdout)
root = meta["workspace_root"]
crates = {p["name"]: p for p in meta["packages"]}
dirs = {os.path.relpath(os.path.dirname(p["manifest_path"]), root): n for n, p in crates.items()}
selected = set()
for path in sys.argv[1:]:
    owners = [n for d, n in dirs.items() if d != "." and path.startswith(d + "/")]
    if not owners:
        selected = set(crates)
        break
    selected.update(owners)
grown = True
while grown:
    grown = False
    for name, crate in crates.items():
        if name not in selected and any(d["name"] in selected for d in crate["dependencies"]):
            selected.add(name)
            grown = True
for name in sorted(selected):
    print("-p", name)
PY
)
[ -n "$packages" ] || exit 0
# shellcheck disable=SC2086 # one word per argument
cargo clippy $packages --all-targets --locked -- -D warnings
# shellcheck disable=SC2086
cargo test $packages --locked --lib --bins
