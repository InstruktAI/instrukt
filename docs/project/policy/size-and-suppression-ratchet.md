---
description: A source file above the line threshold that a change grows is a question review answers about the module's depth and cohesion, never a count to satisfy; no lint's suppression count rises, against a committed baseline that a command only lowers and a reviewed hand edit alone raises.
---

# Size Signal and Suppression Ratchet — Policy

## Rules

### File size is a review signal

- **Size signal.** A source file above the threshold whose lines a change
  grows raises a size signal for that change. Source files are the tracked
  Rust, Python and shell files outside every `tests/` directory; shell files
  are `*.sh` files and the extensionless files directly under `scripts/` and
  `.githooks/`. A file's lines are its newline characters, plus one when it is
  non-empty and does not end in one. Test code inside a source file is not
  counted; `scripts/check-ratchet.py` owns which items that covers. The
  threshold is the baseline's `threshold`; changing it is a hand edit, and
  raising it is a rise.
- **Review answers every signal.** Each size signal of a reviewed change is a
  subject of the paradigm-fit lane's structural-ratchet question
  (`sdlc/policy/definition-of-done-lanes`), judged by the test of
  `sdlc/principle/module-depth`. The author's ratchet disposition under
  `sdlc/policy/definition-of-done` answers it: a deepening commit, or a
  recorded deepening candidate with its reason for deferral. That the module
  remains one cohesive concern is such a reason, and review challenges it only
  by disproving it. Ad-hoc work records its candidate in the commit message. A
  signal the review leaves unanswered is a gap in that lane.
- **A move answers only as a deepening.** Moving lines into another file
  answers a signal only when the new module has its own reason to change and
  hides more than its interface shows.

### Suppressions never rise

- The repository's count of suppressions of any lint never rises above its
  baseline entry, and a lint without an entry has none. The count covers
  every tracked Rust, Python and shell file, tests included:
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
  `threshold` and a `suppressions` object mapping each suppressed lint to its
  allowed count. `make ratchet` and `--init` write it with sorted keys, one
  entry per line; the check reads its entries, not their order.
- The suppression entries match the tree exactly. An entry above what the
  tree measures and an entry for a lint now unsuppressed fail the check as a
  rise does.
- `make ratchet` lowers the baseline to the tree: it lowers each entry above
  its measurement and removes each entry the tree no longer needs. It never
  raises or adds an entry, and while any rise stands it writes nothing. The
  delivery that removes a suppression lowers the baseline in the same
  delivery.
- A rise is a hand edit of `ratchet-baseline.json` in a commit whose message
  states why. It relaxes quality-defining configuration
  (`sdlc/policy/linting-requirements`).

### Where it runs

- `scripts/check-ratchet.py` measures the tree. It fails on a suppression
  rise or a baseline that does not match the tree, naming each lint with its
  baseline and measured counts. It reports, against a given base revision,
  each size signal with the file's lines at that base and now, and does not
  fail on it.
- It runs in the commit checks whenever a commit adds, changes, renames or
  deletes a source file, a suppression-counted file or the baseline, in
  `make lint`, and in CI's every-file run of the commit checks. Review runs it
  against the base of the change it reviews.

## Rationale

The Definition of Done's structural ratchet already asks every delivery to
leave the modules on its change path no worse, and judgment defers that split
one delivery at a time: each addition to an oversized file looks small beside
what is already there. Growth of a file above the threshold is exactly where
that deferral happens, so the signal makes review answer it rather than merely
allowing review to notice it.

A line count is a syntactic measure of something only judgment can assess.
`sdlc/principle/module-depth` decides by depth because the caller pays for a
split, and `sdlc/principle/design-fundamentals` measures responsibility in
reasons to change, not lines. A hard gate on the count rewards the cheapest
change that moves the number — relocating tests, moving code verbatim — and
blocks a module that is legitimately deep. Not counting test code inside a
source file keeps the signal on production code and leaves tests where
`sdlc/policy/rust/testing` puts them.

A suppression is an explicit local waiver of a lint the project chose, and a
rise in the count is a fact no judgment needs to weigh, so that half stays a
gate. A baseline that matches the tree exactly makes every removal permanent
at once, and a command that only lowers keeps every rise a deliberate,
reviewable edit. Integration test files sit outside the size signal because
`project/instrukt/policy/testing` keeps one case file per feature spec.

This policy no longer catches growth of a file above the threshold that review
rules cohesive, which now passes with its recorded reason, nor growth in a
commit that owes no independent review under `general/policy/commits`, which
now passes unanswered: the commit checks report the signal without failing,
so outside review it goes unseen. The suppression count does not see a
module-level `#![allow(...)]` replacing several item-level ones, which lowers
the count while widening the waiver.

## Scope

Every change to this repository. It is this project's check, not an instrukt
feature, and adopting repositories are unaffected.

## Enforcement

- The commit checks and CI run `scripts/check-ratchet.py`; a suppression rise
  or a mismatched baseline fails them.
- The paradigm-fit lane of build and commit review is where size signals are
  answered.
- Review rejects a baseline rise, of a suppression entry or the threshold,
  whose commit does not state why, and treats it as a relaxation under
  `sdlc/policy/linting-requirements`.

## Exceptions

- None.

## See Also

- [project/instrukt/policy/testing](testing.md)
- [sdlc/principle/module-depth](../../domain/sdlc/principle/module-depth.md)
