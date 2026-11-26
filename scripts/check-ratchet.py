#!/usr/bin/env python3
"""Holds source file sizes and lint suppressions to the committed baseline.

project/instrukt/policy/size-and-suppression-ratchet owns the rules: which
files and which suppression directives count, and what the baseline,
ratchet-baseline.json at the repository root, allows. This script measures the
tracked files as they are in the working tree and compares the measurement
with the baseline.

    check-ratchet.py                        compare the tree with the baseline
    check-ratchet.py --update               lower the baseline to the tree
    check-ratchet.py --init --threshold N   create the baseline from the tree

The comparison exits 1 when a file or a lint rose above the baseline, or when
the baseline allows more than the tree measures. --update lowers and removes
entries and never raises or adds one; while a rise stands it writes nothing
and exits 1. --init refuses an existing baseline. Exit 2 means the check
cannot run: no baseline, a malformed one, or a source file that is not UTF-8.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path, PurePosixPath

BASELINE = "ratchet-baseline.json"
POLICY = "project/instrukt/policy/size-and-suppression-ratchet"

# What a Rust file holds besides code: line and block comments, string
# literals (escaped and raw, byte and C strings among them) and character
# literals. A block comment's nesting is followed by `block_comment_end`.
RUST_NOT_CODE = re.compile(
    r"//[^\n]*"
    r"|(?P<block>/\*)"
    r'|(?<!\w)[bc]?r(?P<hashes>#*)".*?"(?P=hashes)'
    r'|"(?:\\.|[^"\\])*"'
    r"|'(?:\\(?:u\{[0-9A-Fa-f]*\}|x[0-9A-Fa-f]{2}|.)|[^'\\\n])'",
    re.DOTALL,
)
RUST_BLOCK_COMMENT_EDGE = re.compile(r"/\*|\*/")
RUST_BRACKET = re.compile(r"[\[\]]")
# An attribute, `#[...]` or `#![...]`, wherever it stands in code.
RUST_ATTRIBUTE = re.compile(r"#!?\[")
# A suppression list inside an attribute, at its start or after `(` or `,`.
RUST_LINT_LIST = re.compile(r"(?:^|[(,])\s*(?:allow|expect)\s*\(")
RUST_LINT = re.compile(r"[A-Za-z_]\w*(?:::[A-Za-z_]\w*)*")
RUST_REASON = re.compile(r"reason\s*=")

# Ruff's rule codes after `noqa:`, separated by commas or spaces.
NOQA_CODES = r"(?:\s*:\s*([A-Z]+[0-9]+(?:[\s,]+[A-Z]+[0-9]+)*))?"
PYTHON_FILE_NOQA = re.compile(r"#\s*ruff\s*:\s*noqa" + NOQA_CODES, re.IGNORECASE)
PYTHON_NOQA = re.compile(r"#\s*noqa" + NOQA_CODES, re.IGNORECASE)
PYTHON_TYPE_IGNORE = re.compile(r"#\s*type\s*:\s*ignore(?:\[([^\]]*)\])?")
SHELLCHECK_DISABLE = re.compile(r"#\s*shellcheck\s+disable=([^\s#]+)")
CODE_SEPARATOR = re.compile(r"[\s,]+")


class CannotCheck(Exception):
    """The check cannot run; the message names the door."""


@dataclass(frozen=True)
class Measurement:
    """What the tree holds: every source file's lines and every lint's suppressions."""

    sizes: dict[str, int]
    suppressions: dict[str, int]

    def above(self, threshold: int) -> dict[str, int]:
        return {path: lines for path, lines in self.sizes.items() if lines > threshold}


@dataclass(frozen=True)
class Baseline:
    """What the tree may hold: ratchet-baseline.json."""

    threshold: int
    files: dict[str, int]
    suppressions: dict[str, int]

    def render(self) -> str:
        document = {"files": self.files, "suppressions": self.suppressions, "threshold": self.threshold}
        return json.dumps(document, indent=2, sort_keys=True) + "\n"


@dataclass(frozen=True)
class Findings:
    """Rises above the baseline, and entries the baseline holds above the tree."""

    rises: list[str]
    stale: list[str]


def git(root: Path | None, *args: str) -> bytes:
    """The output of one git command; the standard library has no reader of git's index."""
    try:
        return subprocess.run(["git", *args], cwd=root, capture_output=True, check=True).stdout
    except subprocess.CalledProcessError as error:
        raise CannotCheck(f"git {' '.join(args)} failed: {os.fsdecode(error.stderr).strip()}") from error


def is_rust(path: PurePosixPath) -> bool:
    return path.suffix == ".rs"


def is_python(path: PurePosixPath) -> bool:
    return path.suffix == ".py"


def is_shell(path: PurePosixPath) -> bool:
    if path.suffix == ".sh":
        return True
    return str(path.parent) in ("scripts", ".githooks") and "." not in path.name


def in_tests(path: PurePosixPath) -> bool:
    return "tests" in path.parent.parts


def line_count(data: bytes) -> int:
    return data.count(b"\n") + (1 if data and not data.endswith(b"\n") else 0)


def block_comment_end(text: str, index: int) -> int:
    """Where the block comment whose `/*` ends at `index` closes, its nested comments included."""
    depth = 1
    for edge in RUST_BLOCK_COMMENT_EDGE.finditer(text, index):
        depth += 1 if edge.group() == "/*" else -1
        if depth == 0:
            return edge.end()
    return len(text)


def rust_code(text: str) -> str:
    """The file's code: comments dropped, every string and character literal emptied."""
    pieces: list[str] = []
    index = 0
    while (found := RUST_NOT_CODE.search(text, index)) is not None:
        pieces.append(text[index : found.start()])
        if found.group("block") is not None:
            index = block_comment_end(text, found.end())
            pieces.append(" ")
        else:
            index = found.end()
            pieces.append(" " if found.group().startswith("//") else '""')
    pieces.append(text[index:])
    return "".join(pieces)


def closing_bracket(code: str, start: int) -> int | None:
    """The index of the `]` that balances the `[` at `start`, or None when none does."""
    depth = 0
    for bracket in RUST_BRACKET.finditer(code, start):
        depth += 1 if bracket.group() == "[" else -1
        if depth == 0:
            return bracket.start()
    return None


def lint_list(attribute: str, start: int) -> list[str]:
    """The lints in the `allow(`/`expect(` list whose items begin at `start`."""
    depth = 0
    items: list[str] = []
    item: list[str] = []
    for char in attribute[start:]:
        if char == "(":
            depth += 1
        elif char == ")":
            if depth == 0:
                items.append("".join(item))
                break
            depth -= 1
        elif char == "," and depth == 0:
            items.append("".join(item))
            item = []
            continue
        item.append(char)
    lints = []
    for candidate in (entry.strip() for entry in items):
        if RUST_REASON.match(candidate):
            continue
        if RUST_LINT.fullmatch(candidate):
            lints.append(candidate)
    return lints


def rust_suppressions(text: str) -> list[str]:
    code = rust_code(text)
    lints: list[str] = []
    resume = 0
    for match in RUST_ATTRIBUTE.finditer(code):
        if match.start() < resume:
            continue
        end = closing_bracket(code, match.end() - 1)
        if end is None:
            continue
        resume = end
        inner = code[match.end() : end]
        for group in RUST_LINT_LIST.finditer(inner):
            lints.extend(lint_list(inner, group.end()))
    return lints


def codes(listed: str | None) -> list[str]:
    return [code for code in CODE_SEPARATOR.split(listed or "") if code]


def python_suppressions(text: str) -> list[str]:
    lints: list[str] = []
    for line in text.splitlines():
        for match in PYTHON_FILE_NOQA.finditer(line):
            lints.extend(f"ruff::{code}" for code in codes(match.group(1)) or ["noqa"])
        for match in PYTHON_NOQA.finditer(line):
            lints.extend(f"ruff::{code}" for code in codes(match.group(1)) or ["noqa"])
        for match in PYTHON_TYPE_IGNORE.finditer(line):
            lints.extend(f"mypy::{code}" for code in codes(match.group(1)) or ["ignore"])
    return lints


def shell_suppressions(text: str) -> list[str]:
    return [
        f"shellcheck::{code}"
        for match in SHELLCHECK_DISABLE.finditer(text)
        for code in match.group(1).split(",")
        if code
    ]


def measure(root: Path) -> Measurement:
    sizes: dict[str, int] = {}
    suppressions: dict[str, int] = {}
    for name in git(root, "ls-files", "-z").split(b"\0"):
        if not name:
            continue
        relative = os.fsdecode(name)
        path = PurePosixPath(relative)
        rust, python, shell = is_rust(path), is_python(path), is_shell(path)
        if not (rust or python or shell):
            continue
        file = root / relative
        # A tracked file the working tree no longer has is one this change
        # removes; a symbolic link's target is measured at its own path.
        if file.is_symlink() or not file.is_file():
            continue
        data = file.read_bytes()
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError as error:
            raise CannotCheck(f"{relative} is not UTF-8 ({error.reason} at byte {error.start})") from error
        if not in_tests(path):
            sizes[relative] = line_count(data)
        if rust:
            found = rust_suppressions(text)
        elif python:
            found = python_suppressions(text)
        else:
            found = shell_suppressions(text)
        for lint in found:
            suppressions[lint] = suppressions.get(lint, 0) + 1
    return Measurement(sizes, suppressions)


def counts(document: dict[str, object], key: str) -> dict[str, int]:
    value = document[key]
    if not isinstance(value, dict):
        raise CannotCheck(f"{BASELINE}: `{key}` is not an object; fix the named key")
    entries: dict[str, int] = {}
    for name, count in value.items():
        if not isinstance(count, int) or isinstance(count, bool) or count < 0:
            raise CannotCheck(f"{BASELINE}: `{key}.{name}` is not a non-negative integer; fix the named key")
        entries[str(name)] = count
    return entries


def load(root: Path) -> Baseline:
    file = root / BASELINE
    if not file.is_file():
        raise CannotCheck(f"{BASELINE} is missing; create it with `scripts/check-ratchet.py --init --threshold N`")
    try:
        document = json.loads(file.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise CannotCheck(f"{BASELINE} is not JSON ({error}); fix it") from error
    if not isinstance(document, dict):
        raise CannotCheck(f"{BASELINE} is not a JSON object; fix it")
    keys = set(document)
    expected = {"files", "suppressions", "threshold"}
    if keys != expected:
        named = ", ".join(sorted(keys ^ expected))
        raise CannotCheck(f"{BASELINE}: unknown or missing keys {named}; fix the named keys")
    threshold = document["threshold"]
    if not isinstance(threshold, int) or isinstance(threshold, bool) or threshold < 1:
        raise CannotCheck(f"{BASELINE}: `threshold` is not a positive integer; fix the named key")
    return Baseline(threshold, counts(document, "files"), counts(document, "suppressions"))


def compare(baseline: Baseline, measured: Measurement) -> Findings:
    threshold = baseline.threshold
    above = measured.above(threshold)
    rises: list[str] = []
    stale: list[str] = []
    for path, lines in sorted(above.items()):
        allowed = baseline.files.get(path)
        if allowed is None:
            rises.append(f"{path}: {lines} lines, not in the baseline (threshold {threshold})")
        elif lines > allowed:
            rises.append(f"{path} grew: baseline {allowed} lines, measured {lines}")
    for lint, count in sorted(measured.suppressions.items()):
        allowed = baseline.suppressions.get(lint, 0)
        if count > allowed:
            rises.append(f"{lint} rose: baseline {allowed} suppressions, measured {count}")
    for path, allowed in sorted(baseline.files.items()):
        if path in above:
            if above[path] < allowed:
                stale.append(f"{path} is below its entry: baseline {allowed} lines, measured {above[path]}")
        elif path in measured.sizes:
            lines = measured.sizes[path]
            stale.append(
                f"{path} needs no entry: baseline {allowed} lines, measured {lines} (threshold {threshold})"
            )
        else:
            stale.append(f"{path} needs no entry: baseline {allowed} lines, not a source file in the tree")
    for lint, allowed in sorted(baseline.suppressions.items()):
        count = measured.suppressions.get(lint, 0)
        if count == 0:
            stale.append(f"{lint} needs no entry: baseline {allowed} suppressions, measured 0")
        elif count < allowed:
            stale.append(f"{lint} is below its entry: baseline {allowed} suppressions, measured {count}")
    return Findings(rises, stale)


def write(root: Path, baseline: Baseline) -> None:
    """Replaces the baseline whole: written beside it, then renamed over it."""
    descriptor, temporary = tempfile.mkstemp(dir=root, prefix=f".{BASELINE}.", suffix=".tmp")
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            handle.write(baseline.render())
        os.replace(temporary, root / BASELINE)
    except BaseException:
        Path(temporary).unlink(missing_ok=True)
        raise


def report(lines: list[str]) -> None:
    for line in lines:
        print(f"ratchet: {line}", file=sys.stderr)


def check(root: Path) -> int:
    findings = compare(load(root), measure(root))
    report(findings.rises + findings.stale)
    if not (findings.rises or findings.stale):
        return 0
    print(
        "ratchet: take the lines or suppressions out in this change, run `make ratchet` to lower an entry "
        f"above the tree, or raise {BASELINE} by hand in a commit that states why ({POLICY})",
        file=sys.stderr,
    )
    return 1


def update(root: Path) -> int:
    baseline = load(root)
    measured = measure(root)
    findings = compare(baseline, measured)
    if findings.rises:
        report(findings.rises)
        print(f"ratchet: {BASELINE} is unchanged; --update never raises an entry ({POLICY})", file=sys.stderr)
        return 1
    if findings.stale:
        lowered = Baseline(baseline.threshold, measured.above(baseline.threshold), measured.suppressions)
        write(root, lowered)
        report([f"lowered: {line}" for line in findings.stale])
    return 0


def init(root: Path, threshold: int) -> int:
    if (root / BASELINE).exists():
        raise CannotCheck(f"{BASELINE} exists; --init only creates it, and `make ratchet` lowers it")
    if threshold < 1:
        raise CannotCheck("--threshold must be a positive integer")
    measured = measure(root)
    write(root, Baseline(threshold, measured.above(threshold), measured.suppressions))
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="check-ratchet.py")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--update", action="store_true")
    mode.add_argument("--init", action="store_true")
    parser.add_argument("--threshold", type=int)
    args = parser.parse_args(argv)
    is_init: bool = args.init
    is_update: bool = args.update
    threshold: int | None = args.threshold
    if is_init != (threshold is not None):
        parser.error("--init and --threshold go together")
    try:
        root = Path(os.fsdecode(git(None, "rev-parse", "--show-toplevel")).strip())
        if threshold is not None:
            return init(root, threshold)
        if is_update:
            return update(root)
        return check(root)
    except CannotCheck as error:
        print(f"ratchet: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
