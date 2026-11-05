#!/bin/sh
# Runs a command holding an exclusive lock on the cargo target directory, so
# checkouts that share one CARGO_TARGET_DIR never build or test over each
# other: the built binary and test executables live at the same paths for
# every checkout, and a build in one would replace what another is testing.
#
# Cargo also treats another checkout's workspace artifacts as fresh (their
# dep-info names that checkout's files, which have not changed), so test
# executables would keep the other checkout's compiled-in paths. Under the
# lock, when the last build came from a different checkout, this checkout's
# workspace packages are cleaned first so they are rebuilt from its sources.
set -eu
# Already inside a locked run (for example the pre-commit hook of a commit
# made under the lock): run the command as is rather than wait for ourselves.
if [ -n "${INSTRUKT_BUILD_LOCKED:-}" ]; then
    exec "$@"
fi
dir="${CARGO_TARGET_DIR:-target}"
mkdir -p "$dir"
exec python3 -c '
import fcntl, os, subprocess, sys
target, cmd = sys.argv[1], sys.argv[2:]
lock = open(os.path.join(target, ".instrukt-build.lock"), "w")
fcntl.flock(lock, fcntl.LOCK_EX)
marker = os.path.join(target, ".instrukt-last-checkout")
here = os.path.realpath(os.getcwd())
try:
    last = open(marker).read().strip()
except FileNotFoundError:
    last = ""
if last != here:
    packages = ["instrukt-cli", "instrukt-core", "instrukt-blueprint", "instrukt-runner", "instrukt-todos"]
    args = ["cargo", "clean", "--quiet"]
    for package in packages:
        args += ["-p", package]
    subprocess.run(args, check=True)
    with open(marker, "w") as out:
        out.write(here + "\n")
os.set_inheritable(lock.fileno(), True)
os.environ["INSTRUKT_BUILD_LOCKED"] = "1"
os.execvp(cmd[0], cmd)
' "$dir" "$@"
