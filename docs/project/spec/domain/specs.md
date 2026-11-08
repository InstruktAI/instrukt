---
description: Use-case identity and test bindings — the use-case inventory of a project's feature specs, the bindings its tests declare, coverage with a ratchet, reservation of use cases for a todo.
delivered_by: [instrukt-first-build]
---
# Specs — Spec

## What it is

The specs context owns the identity of use cases and the bindings that tie
tests to them. It reads a project's feature specs into a use-case inventory,
reads the project's tests for the bindings they declare, and answers two
questions over the two: which use cases are bound and which bindings point
nowhere (coverage), whether a use case a todo means to bind is already claimed
by another todo (reservation). It writes no spec, no test and no marker; it
reads them. It does not own the snippet model, the pending markers' grammar,
the structural graph, or the decision of who may author a use case.

## Ubiquitous language

**Use case**
:  A user-goal scenario in a feature spec: an identifying heading carrying a
   use-case id, followed by exactly one fenced Gherkin block. sdlc's
   feature-spec shape writes it as `### UC-<area><n>:` under `## Use cases`;
   the reader accepts the heading at any level from 2 to 6. Its identity is
   its spec's identity plus its id.

**Use-case id**
:  The `UC-` token on a use case's definition line, recognized as `UC-`, then a
   letter or digit, then letters, digits or hyphens. By authoring convention it
   is a per-spec area code plus a number. Assigned once, never renumbered, never
   reused.

**Definition line**
:  The line that defines a use case: an ATX heading (levels 2–6) or a Gherkin
   `Scenario:` line carrying a use-case id.

**Anonymous scenario**
:  Gherkin with no use-case id: a heading section whose heading carries no id
   and whose body has a `When` line inside a fenced Gherkin block, or a
   `Scenario:` line carrying no id that is not the only `Scenario:` under its
   use-case heading, named or not (the only one inherits that heading's id). Prose is never Gherkin. Not
   bindable; the id-assignment backlog.

**Retired use case**
:  A use-case heading left without a Gherkin block. Its id stays taken.

**Spec id**
:  A feature spec's snippet identity, derived from its position and its path
   within that position: `project/<slug>/<path>` for a spec under
   `docs/project/`, with the project's declared slug; `<domain>/<path>` under
   `docs/domain/<domain>/`; `general/<path>` under `docs/general/`. `<path>` is
   the file's path below its position without `.md`, such as
   `spec/feature/blueprint/use-case-trace`. The pre-release form
   `local/projects/<slug>/…` names no spec.

**Spec folder**
:  The repository-relative folder that holds a feature spec file. Trace groups
   use cases by it; instrukt attaches no further meaning to it. Under sdlc's
   convention it is the bounded context.

**Binding**
:  A test's declaration that it realizes one use case: the pair spec id and
   use-case id, attached to one test.

**Use-case inventory**
:  Every use case and anonymous scenario in the project's feature specs.

**Orphan**
:  A binding whose spec id and use-case id name no use case in the inventory.

**Unbound use case**
:  A use case no binding names. The binding backlog.

**Multi-bound use case**
:  A use case named by more than one binding. A review lead, never a fault by
   itself.

**Ratchet baseline**
:  The committed counts of orphans and anonymous scenarios that a coverage
   check may not exceed.

**Claim**
:  A pending marker (`planned:<slug>` or `planned-change:<slug>`) enclosing a
   use case's definition line; the slug it names holds the use case for its
   build. Whole-file `delivered_by` claims every use case only when it names exactly
   one delivering slug. With several slugs, claims come from in-body pending
   markers; a use case enclosed by no marker is unclaimed. A whole file with
   several delivering slugs and no in-body markers is a coverage/reservation
   finding.

**Trace**
:  The join of a use case's bindings with the structural graph: the tests
   binding it and the code they reach, or, from one or more files, the use
   cases bound by the tests reaching any of them, grouped by spec folder.

## Ownership boundary

- **Owns** — use-case identification in feature specs; the binding forms read
  from tests; the coverage classification (bound, unbound, orphan, anonymous,
  multi-bound); the ratchet over orphans and anonymous scenarios; the
  reservation check.
- **Does not own** — the snippet model, spec identity derivation and the
  pending-marker grammar (docs, `project/instrukt/spec/domain/docs`);
  the structural graph and its traversals (blueprint,
  `project/instrukt/spec/domain/blueprint`); who authors a use case and
  when (`sdlc/policy/testing`, `sdlc/procedure/feature-spec-authoring`); the
  project's own documented test and lint commands; git hooks
  (repository, `project/instrukt/spec/domain/repository`).
- **Published contracts** — `instrukt specs coverage [--check] [--update-baseline]`,
  `instrukt specs reserve --slug <slug> --binding <spec_id>:<UC-id>…`;
  the inventory (with each use case's spec file) and bindings consumed by
  blueprint trace.
- **Allowed dependencies** — docs (spec identity and pending markers) and
  configuration (project slug), both Customer / Supplier
  relationships with specs as customer.
- **Implementation roots** — `crates/instrukt-core/` (inventory, bindings,
  coverage, reservation); `crates/instrukt-cli/` (command surface).

## Canonical fields

### Aggregates

#### Use-case inventory (aggregate)

- **Root:** the project's feature-spec corpus as read from its working tree.
- **Contains:** use cases (spec id, use-case id, spec file), anonymous
  scenarios (spec id, spec file), claims (use case → slug).
- **Invariants:**
  - Within one spec file a use-case id is one use case, whichever definition
    convention carries it and however often it appears.
  - Every anonymous scenario is counted, never silently dropped.
  - A use case has at most one claiming slug. A single whole-file delivering
    slug claims the file; with several delivering slugs, non-nesting in-body
    markers identify the individual claims.
  - A whole file with several delivering slugs and no in-body markers is
    reported as a finding by coverage and reservation checks.
- **Identifier:** spec id plus use-case id.
- **Lifecycle:** read fresh on every command; nothing is stored.

#### Binding set (aggregate)

- **Root:** the project's tests as read from its working tree.
- **Contains:** bindings (spec id, use-case id, test location).
- **Invariants:**
  - A binding names exactly one spec id and one use-case id.
  - A binding's spec id is a snippet identity; any other form is an orphan.
- **Identifier:** test location plus spec id plus use-case id.
- **Lifecycle:** read fresh on every command; nothing is stored.

### Domain events emitted

None. Every answer is a command's output.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events.

## Use cases

- **use-case-coverage** — `project/instrukt/spec/feature/specs/use-case-coverage` — a project sees which use cases its tests bind, and its check refuses new orphan or anonymous debt.
- **binding-reservation** — `project/instrukt/spec/feature/specs/binding-reservation` — a todo's planned bindings are checked against the use cases other todos already hold.

## Governing designs and decisions

- **`project/instrukt/design/architecture/overview`** — `specs` is part of the always-on core; trace is available only with the blueprint module on.
- **`project/instrukt/policy/testing`** — this repository binds its own tests to use cases and checks them with `instrukt specs coverage --check`.

## Context map position

- **docs** — Customer / Supplier. Docs supplies spec identity and the pending-marker grammar; specs reads feature specs through them.
- **blueprint** — Customer / Supplier. Specs supplies inventory and bindings; blueprint owns their trace join.
- **configuration** — Customer / Supplier. Configuration supplies the project slug that spec ids carry.

## Known caveats

- Use cases are read from the working tree, not from committed main: a check
  answers for the tree the caller has.
- Trace belongs to blueprint, whose crate depends on core and consumes the specs context's published inventory and bindings.
