#!/usr/bin/env python3
"""Runs one check of .pre-commit-config.yaml for the files the commit touches.

The pre-commit framework passes a check the files it matched that remain in
the commit; it never passes a path the commit deletes or the old path of a
rename. This adds those paths, matched by the check's own `files` pattern with
the framework's rule (a regular-expression search), so the check also runs
when the commit removes or moves away a file it applies to. With neither, the
check has nothing to do and its command does not run.

    commit-check.py --pattern REGEX --run COMMAND [--takes-files]
                    [--with-removed] -- [FILE ...]

--takes-files appends the files to COMMAND; --with-removed appends the removed
paths too, for a check that judges what a path belonged to rather than its
content.
"""

import argparse
import os
import re
import shlex
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Check:
    """One check as its hook declares it, with the files the framework passed."""

    pattern: re.Pattern[str]
    command: list[str]
    takes_files: bool
    with_removed: bool
    files: list[str]


def parse(argv: list[str]) -> Check:
    parser = argparse.ArgumentParser(prog="commit-check.py")
    parser.add_argument("--pattern", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--takes-files", action="store_true")
    parser.add_argument("--with-removed", action="store_true")
    parser.add_argument("files", nargs="*")
    args = parser.parse_args(argv)
    pattern: str = args.pattern
    run: str = args.run
    takes_files: bool = args.takes_files
    with_removed: bool = args.with_removed
    files: list[str] = args.files
    return Check(re.compile(pattern), shlex.split(run), takes_files, with_removed, files)


def removed_paths() -> list[str]:
    """The paths the staged commit removes, the old side of every rename included."""
    # git is the only reader of the index here; the standard library has none.
    # --no-renames reports a rename as the removal of its old path.
    staged = subprocess.run(
        ["git", "diff", "--cached", "--no-renames", "--diff-filter=D", "--name-only", "-z"],
        capture_output=True,
        check=True,
    ).stdout
    return [os.fsdecode(path) for path in staged.split(b"\0") if path]


def command_line(check: Check, removed: list[str]) -> list[str] | None:
    """The command to run, or None when the commit touches nothing the check applies to."""
    matched = [path for path in removed if check.pattern.search(path)]
    if not check.files and not matched:
        return None
    if not check.takes_files:
        return check.command
    files = check.files + matched if check.with_removed else check.files
    # Every file the check reads was removed: nothing of it is left to judge.
    if not files:
        return None
    return check.command + files


def main(argv: list[str]) -> None:
    check = parse(argv)
    command = command_line(check, removed_paths())
    if command is None:
        return
    os.execvp(command[0], command)


if __name__ == "__main__":
    main(sys.argv[1:])
