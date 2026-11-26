---
description: No source file above the line threshold grows, none enters above it, and no lint's suppression count rises; a committed baseline that a command only lowers and a reviewed hand edit alone raises.
delivered_by: [ratchet-file-size-and-suppressions]
---

# Size and Suppression Ratchet — Policy

## Rules

### What the ratchet holds

- **Size.** A source file above the threshold never grows beyond its baseline
  entry, and a source file without an entry never exceeds the threshold. The
  threshold is the baseline's `threshold`; changing it is a hand edit, and
  raising it is a rise. Source files are
  the tracked Rust, Python and shell files outside every `tests/` directory;
  shell files are `*.sh` files and the extensionless files directly under
  `scripts/` and `.githooks/`. A file's lines are its newline characters, plus
  one when it is non-empty and does not end in one. In-file test modules
  count.
- **Suppressions.** The repository's count of suppressions of any lint never
  rises above its baseline entry, and a lint without an entry has none. The
  count covers every tracked Rust, Python and shell file, tests included:
  - Rust: each lint named in an `allow(...)` or `expect(...)` list inside an
    attribute, `#[...]` or `#![...]`, wherever it stands in code, including
    one inside `cfg_attr(...)`, one spanning several lines and one inside a
    line, is one suppression of that lint under its name as written
    (`clippy::unwrap_used`, `dead_code`). Comments and string and character
    literals are not code. A `reason = "..."` entry is not a lint.
  - Python: each code in `# noqa: <codes>` is one suppression of
    `ruff::<code>`, and each code in `# type: ignore[<codes>]` one of
    `mypy::<code>`; a bare `# noqa` or `# ruff: noqa` is one of `ruff::noqa`,
    a bare `# type: ignore` one of `mypy::ignore`.
  - Shell: each code in `# shellcheck disable=<codes>` is one suppression of
    `shellcheck::<code>`.
- Lint levels in `Cargo.toml` and `clippy.toml` are configuration, not
  suppressions; relaxing them is governed by `sdlc/policy/linting-requirements`.

### The baseline

- The baseline is `ratchet-baseline.json` at the repository root: the
  `threshold`, a `files` object mapping each source file above the threshold
  to its allowed lines, and a `suppressions` object mapping each suppressed
  lint to its allowed count. Keys are sorted, one entry per line.
- The baseline matches the tree exactly. An entry above what the tree
  measures, an entry for a file now at or below the threshold or gone, and an
  entry for a lint now unsuppressed fail the check as a regression does.
- `make ratchet` lowers the baseline to the tree: it lowers each entry above
  its measurement and removes each entry the tree no longer needs. It never
  raises or adds an entry, and while any regression stands it writes nothing.
  The delivery that shrinks a file or removes a suppression lowers the
  baseline in the same delivery.
- A rise is a hand edit of `ratchet-baseline.json` in a commit whose message
  states why. It relaxes quality-defining configuration
  (`sdlc/policy/linting-requirements`). A file renamed above the threshold
  moves its entry to the new path by the same kind of commit.
- Adding to a file above the threshold takes as many lines out of it in the
  same delivery; moving in-file tests to a sibling file is one way.

### Where it runs

- `scripts/check-ratchet.py` measures the tree and compares it with the
  baseline. It runs in the commit checks whenever a commit adds, changes,
  renames or deletes a source file, a suppression-counted file or the
  baseline, in `make lint`, and in CI's every-file run of the commit checks.
- A failure names each file or lint with its baseline and measured numbers.

## Rationale

The Definition of Done's structural ratchet forbids worsening a principle
violation but leaves size and suppressions to judgment, and judgment defers
the split one delivery at a time: each addition to an oversized file, and each
new suppression, looks small beside what is already there. A count a script
checks is a fact a reviewer need not weigh. Integration test files sit outside the size rule
because `project/instrukt/policy/testing` keeps one case file per feature
spec. A baseline that matches the tree exactly makes every shrink permanent
at once, so no later delivery regrows into slack; a command that only lowers
keeps every rise a deliberate, reviewable edit.

## Scope

Every change to this repository. It is this project's check, not an instrukt
feature, and adopting repositories are unaffected.

## Enforcement

- The commit checks and CI run `scripts/check-ratchet.py`.
- Review rejects a baseline rise whose commit does not state why, and treats
  it as a relaxation under `sdlc/policy/linting-requirements`.

## Exceptions

- None.

## See Also

- [project/instrukt/policy/testing](testing.md)
