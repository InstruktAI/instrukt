#!/bin/sh
# Runs a command holding an exclusive lock on the cargo target directory, so
# checkouts that share one CARGO_TARGET_DIR never build or test over each
# other: the built binary and test executables live at the same paths for
# every checkout, and a build in one would replace what another is testing.
set -eu
dir="${CARGO_TARGET_DIR:-target}"
mkdir -p "$dir"
exec python3 -c '
import fcntl, os, sys
lock = open(sys.argv[1], "w")
fcntl.flock(lock, fcntl.LOCK_EX)
os.set_inheritable(lock.fileno(), True)
os.execvp(sys.argv[2], sys.argv[2:])
' "$dir/.instrukt-build.lock" "$@"
