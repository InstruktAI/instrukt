---
description: Coverage measured against use cases, not source files — the inventory of a project's feature specs against the bindings in its tests, reporting orphans, unbound, anonymous and multi-bound, with a ratchet that lets orphans and anonymous scenarios shrink but never grow.
delivered_by: [instrukt-first-build]
---
# Use-Case Coverage — Spec

## What it is

`instrukt specs coverage` tells a project how its tests stand against its use
cases. Every use case in the project's feature specs is a coverage unit; every
binding in its tests claims one. The report reconciles the two. With `--check`
it is a gate: new orphan bindings or new anonymous scenarios fail it. Coverage
is the scenario inventory bound, never source lines or a file mapping
(`sdlc/policy/testing`).

## Canonical fields

- Headings without Gherkin are retired and absent from the inventory. Companion files are excluded.

### The inventory

- **Which specs.** Every feature spec of the project the caller stands in, in
  each position: `docs/project/spec/feature/…`, `docs/general/spec/feature/…`
  and `docs/domain/<domain>/spec/feature/…`. Each file is one spec; its spec
  id is its snippet identity, `project/<slug>/…`, `general/…` or `<domain>/…`
  (`local/projects/instrukt/spec/domain/specs`).
- **Use cases.** Every use-case id that appears on a definition line — an ATX
  heading of level 2 to 6, or a Gherkin `Scenario:` line. The id is the first
  token of the form `UC-` followed by a letter or digit and then letters,
  digits or hyphens. Within one file an id counts once. sdlc's feature-spec
  shape puts use cases at level 3 under `## Use cases`; a spec written one
  level deeper reads the same.
- **Anonymous scenarios.** Gherkin with no use-case id: a heading section
  whose heading carries no use-case id and whose body holds a `When` line
  inside a fenced Gherkin block. A `Scenario:` line without an id inherits the
  id of the use-case heading it sits under when it is the only `Scenario:`
  under that heading, named or not; otherwise it is anonymous, so two
  scenarios never share an id. Prose beginning with "When" outside a
  fenced Gherkin block is not Gherkin and is not counted. Each anonymous
  scenario counts once. This is deliberately tighter than the predecessor's
  meter, which counted any line beginning with `When`.
- Pending use cases (inside `planned` or `planned-change` blocks) are in the
  inventory as written: a scenario authored before its build is bindable.

- A whole-file pending spec claims its use cases only when `delivered_by`
  names exactly one delivering slug. With several slugs, claims come from
  in-body pending markers. A whole file with several delivering slugs and no
  in-body markers is reported as a coverage finding.

### The bindings

| Language | Binding form | Attached to |
| --- | --- | --- |
| Rust | a line comment `// spec: <spec_id> <UC-id>` | the test function that follows it |
| Python | a decorator `@pytest.mark.spec(<spec_id>, <UC-id>)` | the test function it decorates |

- Python arguments are string literals or names of module-level string
  constants in the same file; any other argument form is not a binding.
- The spec id in a binding is the spec's snippet identity. A binding naming a
  spec any other way, the pre-release `local/projects/<slug>/…` form included,
  names no spec and is an orphan.

### Classification

| Class | Meaning | Gated by `--check` |
| --- | --- | --- |
| bound | a use case with at least one binding | no |
| unbound | a use case with no binding — the backlog | no |
| orphan | a binding naming no use case in the inventory | yes, ratcheted |
| anonymous | Gherkin with no use-case id | yes, ratcheted |
| multi-bound | a use case with more than one binding | no — a review lead |
| ambiguous claim | a whole spec file pending for several delivering slugs with no in-body markers | yes, always (not ratcheted) |

Unbound is not gated: under documentation first, use cases are written before
their tests and the backlog rises and falls with every delivery. Whether a
multi-bound use case's tests exercise distinct behavior is a reviewer's
judgment (`sdlc/policy/testing`), so it is reported, never failed.

### The report

- Counts: spec files with scenarios, total scenarios, bindable use cases,
  anonymous scenarios, bound, unbound, multi-bound, orphans.
- One line per orphan: the test, and the spec id and use-case id it cites.
- One line per unbound use case and per multi-bound use case with its number
  of tests.
- One line per ambiguous claim: the spec id and its delivering slugs.
- Without `--check` the command reports and exits zero.

### The ratchet

- `--check` fails (non-zero exit) on any ambiguous claim, naming each.
- `--check` compares the orphan and anonymous counts with the project's
  committed ratchet baseline. A count above its baseline fails the check
  (non-zero exit) and names each regression with its count and baseline; the
  remedy is to resolve the new debt.
- A missing baseline counts as zero for both.
- A count below its baseline passes and says the baseline can be tightened.
- The check never fails on unbound or multi-bound use cases.

### Use cases

#### UC-SC1: A developer sees which of the project's use cases are tested

```gherkin
Given a project whose feature specs hold use cases and whose tests bind some of them
When the developer asks for the project's use-case coverage
Then they see how many use cases are bound and which are not
And they see each test that cites a use case that does not exist
And they see each scenario that has no use-case id yet
```

#### UC-SC2: A developer's check refuses new spec-coverage debt

```gherkin
Given a project with a committed record of its orphan and anonymous counts
And a change adds a test citing a use case that does not exist
When the developer runs the coverage check
Then the check fails and names the count that grew beyond its record
And a change that only leaves use cases unbound passes the check
```

## Known caveats

- Bindings and use cases are matched by identity only; whether a bound test
  actually drives its use case is a reviewer's judgment.
- A retired heading without Gherkin leaves the inventory and is never counted unbound.

- The committed baseline is `instrukt-coverage-baseline.json` at the repository root, rewritten only by `instrukt specs coverage --update-baseline`.
- Bindings are read from every `.rs` file in the project and Python test files (`test_*.py`, `*_test.py`); this is not configurable.
- Companion files are pending next versions of their parents and are not counted in the inventory.

## See Also

- docs/local/spec/domain/specs.md — the specs context.
- ~/.instrukt/docs/sdlc/policy/testing.md — what a test is and how it binds a use case.
- ~/.instrukt/docs/sdlc/policy/rust/testing.md — the Rust binding comment.
