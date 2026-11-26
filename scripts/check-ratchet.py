#!/usr/bin/env python3
"""Holds lint suppressions to the committed baseline and reports size signals.

project/instrukt/policy/size-and-suppression-ratchet owns the rules: which
suppression directives count, what the baseline, ratchet-baseline.json at the
repository root, allows, and what a size signal is. This script measures the
tracked files as they are in the working tree, compares their suppressions
with the baseline and reports each size signal against a base revision.

    check-ratchet.py [--base REV]           compare the tree with the baseline
    check-ratchet.py --update               lower the baseline to the tree
    check-ratchet.py --init --threshold N   create the baseline from the tree

A size signal is a source file above the baseline's threshold that has more
lines now than at the base revision, HEAD unless --base names another. A file
the base holds under neither its path nor the path it was renamed from has
none there. The comparison prints each signal with the file's lines at the
base and now, and never fails on one.

Test code inside a Rust source file is not counted. It is an item whose outer
attribute is `#[cfg(test)]`, or `#[cfg(all(...))]` with `test` as a direct
operand, standing where an item stands: after `;`, `{`, `}`, other attributes
or the start of the file, and before an item keyword (`mod`, `fn`, `use`,
`impl`, `struct` and the like), a visibility allowed. Its lines run from that
attribute to the first `;`, or the `}` that closes its first brace, at bracket
depth zero. A test attribute anywhere else (a field, a variant, a match arm, a
parameter, a `let` or expression statement) is not recognized and its lines
count. Python and shell files hold no test code this script recognizes.

The comparison exits 1 when a lint's suppressions rose above the baseline, or
when the baseline allows more than the tree measures. --update lowers and
removes entries and never raises or adds one; while a rise stands it writes
nothing and exits 1. --init refuses an existing baseline. Exit 2 means the
check cannot run: no baseline, a malformed one, a base that names no commit,
or a source file that is not UTF-8, in the tree or at the base.
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
# What may follow a test attribute for it to gate an item: other attributes,
# a visibility, then one of the keywords an item begins with.
RUST_VISIBILITY = re.compile(r"pub\b(?:\s*\([^()]*\))?\s*")
RUST_WORD = re.compile(r"[A-Za-z_]\w*")
RUST_ITEM_KEYWORDS = frozenset(
    {
        "async",
        "const",
        "enum",
        "extern",
        "fn",
        "impl",
        "macro_rules",
        "mod",
        "static",
        "struct",
        "trait",
        "type",
        "union",
        "unsafe",
        "use",
    }
)
# The characters that end an item or change its bracket depth.
RUST_ITEM_EDGE = re.compile(r"[;()\[\]{}]")

# Ruff's rule codes after `noqa:`, separated by commas or spaces.
NOQA_CODES = r"(?:\s*:\s*([A-Z]+[0-9]+(?:[\s,]+[A-Z]+[0-9]+)*))?"
PYTHON_FILE_NOQA = re.compile(r"#\s*ruff\s*:\s*noqa" + NOQA_CODES, re.IGNORECASE)
PYTHON_NOQA = re.compile(r"#\s*noqa" + NOQA_CODES, re.IGNORECASE)
PYTHON_TYPE_IGNORE = re.compile(r"#\s*type\s*:\s*ignore(?:\[([^\]]*)\])?")
# Shellcheck codes (`SC2086`) or `all`, separated by commas.
SHELLCHECK_DISABLE = re.compile(r"#\s*shellcheck\s+disable=([A-Za-z0-9,]+)")
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
class Signal:
    """A source file above the threshold that grew since the base; review answers it."""

    path: str
    base_lines: int | None
    lines: int


@dataclass(frozen=True)
class Baseline:
    """What the tree may hold: ratchet-baseline.json."""

    threshold: int
    suppressions: dict[str, int]

    def render(self) -> str:
        document = {"suppressions": self.suppressions, "threshold": self.threshold}
        return json.dumps(document, indent=2, sort_keys=True) + "\n"


@dataclass(frozen=True)
class Findings:
    """Rises above the baseline, and entries the baseline holds above the tree."""

    rises: list[str]
    stale: list[str]


def git(root: Path | None, *args: str, stdin: bytes | None = None) -> bytes:
    """The output of one git command; the standard library has no reader of git's index or objects."""
    try:
        return subprocess.run(["git", *args], cwd=root, input=stdin, capture_output=True, check=True).stdout
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
    """The file's code: comments dropped, every string and character literal emptied.

    What is dropped leaves its newlines behind, so each line of the code is
    the same line of the file.
    """
    pieces: list[str] = []
    index = 0
    while (found := RUST_NOT_CODE.search(text, index)) is not None:
        pieces.append(text[index : found.start()])
        if found.group("block") is not None:
            index = block_comment_end(text, found.end())
            placeholder = " "
        else:
            index = found.end()
            placeholder = " " if found.group().startswith("//") else '""'
        pieces.append(placeholder + "\n" * text.count("\n", found.start(), index))
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


def rust_suppressions(code: str) -> list[str]:
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


def call_operands(predicate: str, name: str) -> list[str] | None:
    """The operands of `predicate` when it is exactly `name(...)`, written without whitespace."""
    if not (predicate.startswith(f"{name}(") and predicate.endswith(")")):
        return None
    inner = predicate[len(name) + 1 : -1]
    operands: list[str] = []
    depth = 0
    start = 0
    for index, char in enumerate(inner):
        if char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth < 0:
                return None
        elif char == "," and depth == 0:
            operands.append(inner[start:index])
            start = index + 1
    if depth != 0:
        return None
    operands.append(inner[start:])
    return [operand for operand in operands if operand]


def is_test_cfg(attribute: str) -> bool:
    """Whether an attribute's content is `cfg(test)` or `cfg(all(...))` with `test` as a direct operand."""
    predicates = call_operands("".join(attribute.split()), "cfg")
    if predicates is None or len(predicates) != 1:
        return False
    predicate = predicates[0]
    return predicate == "test" or "test" in (call_operands(predicate, "all") or [])


def stands_after_item_boundary(code: str, start: int, attributes: dict[int, int]) -> bool:
    """Whether what precedes `start`, attributes crossed, is `;`, `{`, `}` or the start of the file."""
    index = start - 1
    while True:
        while index >= 0 and code[index].isspace():
            index -= 1
        if index < 0 or code[index] in ";{}":
            return True
        opening = attributes.get(index)
        if opening is None:
            return False
        index = opening - 1


def item_keyword_follows(code: str, index: int) -> bool:
    """Whether an item keyword follows `index`, past stacked attributes and a visibility."""
    while True:
        while index < len(code) and code[index].isspace():
            index += 1
        if not code.startswith("#[", index):
            break
        closing = closing_bracket(code, index + 1)
        if closing is None:
            return False
        index = closing + 1
    visibility = RUST_VISIBILITY.match(code, index)
    if visibility is not None:
        index = visibility.end()
    word = RUST_WORD.match(code, index)
    return word is not None and word.group() in RUST_ITEM_KEYWORDS


def item_end(code: str, index: int) -> int | None:
    """Where the item continuing at `index` ends: past its first `;` or the `}` closing its first brace."""
    depth = 0
    for edge in RUST_ITEM_EDGE.finditer(code, index):
        char = edge.group()
        if char == ";":
            if depth == 0:
                return edge.end()
        elif char in "([{":
            depth += 1
        else:
            depth -= 1
            if depth < 0:
                return None
            if depth == 0 and char == "}":
                return edge.end()
    return None


def rust_test_items(code: str) -> list[tuple[int, int]]:
    """The spans of the items test configuration gates, in order and not nested (see the module docstring)."""
    attributes: dict[int, int] = {}
    spans: list[tuple[int, int]] = []
    resume = 0
    for match in RUST_ATTRIBUTE.finditer(code):
        closing = closing_bracket(code, match.end() - 1)
        if closing is None:
            continue
        attributes[closing] = match.start()
        if match.start() < resume or match.group() != "#[" or not is_test_cfg(code[match.end() : closing]):
            continue
        if not (stands_after_item_boundary(code, match.start(), attributes) and item_keyword_follows(code, closing + 1)):
            continue
        end = item_end(code, closing + 1)
        if end is None:
            continue
        spans.append((match.start(), end))
        resume = end
    return spans


def rust_test_lines(code: str) -> int:
    """How many lines the file's test items span, each line once."""
    spanned = 0
    last = -1
    for start, end in rust_test_items(code):
        first = max(code.count("\n", 0, start), last + 1)
        final = code.count("\n", 0, end - 1)
        spanned += max(0, final - first + 1)
        last = max(last, final)
    return spanned


def counted_lines(data: bytes, code: str | None) -> int:
    """A source file's lines, its test code not counted; `code` is a Rust file's code, None for another file."""
    return line_count(data) - (rust_test_lines(code) if code is not None else 0)


def decode(data: bytes, name: str) -> str:
    try:
        return data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise CannotCheck(f"{name} is not UTF-8 ({error.reason} at byte {error.start})") from error


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
        text = decode(data, relative)
        code = rust_code(text) if rust else None
        if not in_tests(path):
            sizes[relative] = counted_lines(data, code)
        if code is not None:
            found = rust_suppressions(code)
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
    expected = {"suppressions", "threshold"}
    if keys != expected:
        named = ", ".join(sorted(keys ^ expected))
        raise CannotCheck(f"{BASELINE}: unknown or missing keys {named}; fix the named keys")
    threshold = document["threshold"]
    if not isinstance(threshold, int) or isinstance(threshold, bool) or threshold < 1:
        raise CannotCheck(f"{BASELINE}: `threshold` is not a positive integer; fix the named key")
    return Baseline(threshold, counts(document, "suppressions"))


def compare(baseline: Baseline, measured: Measurement) -> Findings:
    rises: list[str] = []
    stale: list[str] = []
    for lint, count in sorted(measured.suppressions.items()):
        allowed = baseline.suppressions.get(lint, 0)
        if count > allowed:
            rises.append(f"{lint} rose: baseline {allowed} suppressions, measured {count}")
    for lint, allowed in sorted(baseline.suppressions.items()):
        count = measured.suppressions.get(lint, 0)
        if count == 0:
            stale.append(f"{lint} needs no entry: baseline {allowed} suppressions, measured 0")
        elif count < allowed:
            stale.append(f"{lint} is below its entry: baseline {allowed} suppressions, measured {count}")
    return Findings(rises, stale)


def resolve(root: Path, revision: str) -> str:
    """The commit `revision` names, as its full id."""
    try:
        resolved = git(root, "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}")
    except CannotCheck as error:
        raise CannotCheck(f"--base {revision} names no commit here; pass a revision that exists") from error
    return os.fsdecode(resolved).strip()


def blobs(root: Path, base: str, paths: list[str]) -> dict[str, bytes]:
    """Each path's content at the base commit; a path the base has no file at is left out."""
    if not paths:
        return {}
    request = b"".join(os.fsencode(f"{base}:{path}\n") for path in paths)
    output = git(root, "cat-file", "--batch", stdin=request)
    contents: dict[str, bytes] = {}
    offset = 0
    for path in paths:
        header_end = output.index(b"\n", offset)
        header = output[offset:header_end].split(b" ")
        offset = header_end + 1
        # `<name> missing`, or `<id> <type> <size>` followed by the object and a newline.
        if header[-1] == b"missing":
            continue
        size = int(header[-1])
        if header[-2] == b"blob":
            contents[path] = output[offset : offset + size]
        offset += size + 1
    return contents


def renamed_from(root: Path, base: str) -> dict[str, str]:
    """Each path of the tree that a rename since the base produced, mapped to its path at the base."""
    fields = git(root, "diff", "--find-renames", "--diff-filter=R", "--name-status", "-z", base, "--").split(b"\0")
    # Each rename is `R<score>`, the old path and the new path.
    return {os.fsdecode(fields[index + 2]): os.fsdecode(fields[index + 1]) for index in range(0, len(fields) - 2, 3)}


def signals(root: Path, base: str, threshold: int, measured: Measurement) -> list[Signal]:
    above = measured.above(threshold)
    origins = {path: path for path in above}
    contents = blobs(root, base, sorted(origins.values()))
    absent = [path for path in above if path not in contents]
    if absent:
        renames = renamed_from(root, base)
        moved = {path: renames[path] for path in absent if path in renames}
        origins.update(moved)
        contents.update(blobs(root, base, sorted(moved.values())))
    grown: list[Signal] = []
    for path, lines in sorted(above.items()):
        origin = origins[path]
        data = contents.get(origin)
        base_lines = None
        if data is not None:
            code = rust_code(decode(data, f"{origin} at {base}")) if is_rust(PurePosixPath(origin)) else None
            base_lines = counted_lines(data, code)
        if lines > (base_lines or 0):
            grown.append(Signal(path, base_lines, lines))
    return grown


def describe(signal: Signal, base: str, threshold: int) -> str:
    then = "absent" if signal.base_lines is None else f"{signal.base_lines} lines"
    return (
        f"size signal: {signal.path}: {then} at {base[:12]}, {signal.lines} lines now (threshold {threshold}); "
        f"review answers it ({POLICY})"
    )


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


def check(root: Path, revision: str) -> int:
    """Reports every size signal, which never fails; fails on a suppression rise or a stale entry."""
    baseline = load(root)
    base = resolve(root, revision)
    measured = measure(root)
    report([describe(signal, base, baseline.threshold) for signal in signals(root, base, baseline.threshold, measured)])
    findings = compare(baseline, measured)
    report(findings.rises + findings.stale)
    if not (findings.rises or findings.stale):
        return 0
    print(
        "ratchet: take the suppressions out in this change, run `make ratchet` to lower an entry "
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
        lowered = Baseline(baseline.threshold, measured.suppressions)
        write(root, lowered)
        report([f"lowered: {line}" for line in findings.stale])
    return 0


def init(root: Path, threshold: int) -> int:
    if (root / BASELINE).exists():
        raise CannotCheck(f"{BASELINE} exists; --init only creates it, and `make ratchet` lowers it")
    if threshold < 1:
        raise CannotCheck("--threshold must be a positive integer")
    measured = measure(root)
    write(root, Baseline(threshold, measured.suppressions))
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(prog="check-ratchet.py")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--update", action="store_true")
    mode.add_argument("--init", action="store_true")
    mode.add_argument("--base", metavar="REV", help="the revision size signals are measured against (HEAD)")
    parser.add_argument("--threshold", type=int)
    args = parser.parse_args(argv)
    is_init: bool = args.init
    is_update: bool = args.update
    base: str | None = args.base
    threshold: int | None = args.threshold
    if is_init != (threshold is not None):
        parser.error("--init and --threshold go together")
    try:
        root = Path(os.fsdecode(git(None, "rev-parse", "--show-toplevel")).strip())
        if threshold is not None:
            return init(root, threshold)
        if is_update:
            return update(root)
        return check(root, base or "HEAD")
    except CannotCheck as error:
        print(f"ratchet: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
