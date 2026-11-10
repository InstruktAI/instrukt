---
description: Configured workflow evaluation derives the current obligation from record facts without storing a phase or executing commands.
---
# Workflow Definition — Spec

## What it is

A developer adopts a flavor's workflow and changes its order, actions or
instructions independently of the evaluation engine. The engine takes a
definition and facts, and returns a derived state, the selected enabled
transition, an action identifier and configured effects. Evaluation is pure;
execution belongs to the orchestrator and the effect executor.

## Canonical fields

### Definition and evaluation result

| Concept | Contract |
| --- | --- |
| Definition | A named flow with its orchestrator role, runtime-capability requirements, an initial state, declared states and ordered transitions. |
| Transition | A source state, destination state, condition, action identifier and optional effects. |
| Facts | Named, typed observations supplied from the work record and current evidence by the fact provider. |
| Result | The state where derivation stops, the last selected enabled transition, its action identifier, and the effects still owed from the record along the selected walk. If no transition is selected, there is no selected action or effect. |
| Action | A flavor-owned command and Markdown template addressed by an identifier; its responsible role follows from its named procedure or explicit setup role. Worker commands reserve assignment facts; the orchestrator delegates natively. |
| Effect | A flavor-configured command executed outside evaluation. |

Every call starts at the definition's initial state. At each visited state the
engine selects the first transition whose condition holds, in definition
order, and continues from its destination using the same supplied facts.
Derivation ends when no transition matches. The last selected transition
identifies the action at that stopping state. Intermediate actions are not
dispatched. Effects are derived across the selected walk: an effect is owed when it has
no `done_when` condition or its `done_when` does not hold on the supplied
facts; traversing an intermediate state does not discard an owed effect.
Evaluation returns owed effects in their listed order without executing them.
An effect without `done_when` is returned on every evaluation and must be
idempotent.

The same definition and facts yield the same result. Evaluation neither
changes the record nor reads the clock, repository, network or runtime.
Revisiting a state within a derivation is an invalid evaluation, reported to
the caller; it is not an executable obligation. Definition references resolve
to declared states, known condition operators and configured actions before
execution is admitted.

### Flavor source

The engine loads the domain selected by project key `workflow.domain`
(default `sdlc`) from `~/.instrukt/docs/<domain>/workflow/`. Its authoring source
is the owning project’s `docs/domain/<domain>/workflow/`, published with the
domain.
`actions.yaml` declares a unique role-name list under `roles:`, alongside the
action entries and `effects:`. Docs validation checks these files against the
workflow schema, separately from snippet taxonomy validation.

Each flow file declares its lane orchestrator with the required top-level key
`orchestrator: <role>`, a role declared under `roles:`. Exactly one procedure
declares that role; it is the procedure the runner dispatches for the lane. An
action's responsible role follows from its command:

- a command naming a procedure (`--procedure <id>` or `docs get <id>`) belongs
  to that procedure's `role`;
- setup reservation with `--role setup` names its record role without a procedure;
- a command naming no procedure or explicit setup role is a step of the orchestrator of the flow that
  selected it, executed by the session holding the todo's claim — the session
  the runner started into the orchestrator's procedure, or the one
  `instrukt todo prepare|work` made the orchestrator.

Every procedure the workflow dispatches declares one of the flavor's `roles:`.
Subscribed procedures are outside workflow role-list membership; the event
system uses their roles independently of todos. Definition findings
include a flow without `orchestrator`, an `orchestrator` role not declared
under `roles:`, no procedure declaring the `orchestrator` role, and an action
whose command names a procedure that declares no `role`. The setup-only
`todo attempt --role setup` action carries its explicit record role; other
`todo attempt --role` values match the procedure's declared role.

### Runtime capability requirements

Every flow definition declares `requires: [<capability>, …]` at top level,
including an explicit empty list when it adds no runtime requirements. Values
come from the selected runtime capability vocabulary: `worktree_create`,
`worktree_attach`, `shared_delegation`, `link`, `notify` and `agent_selection`. Unknown names,
non-list values and a missing declaration are definition findings. Baseline
module primitives remain required independently of this list.

Enablement unions `requires` across the selected workflow's flows and checks
it against the explicitly selected runtime's native capabilities for agent
operations and adapter capabilities for unattended operations. A nonempty
instruction is not proof of capability; supported native operations come from
the selected runtime's separately declared capability facts, not prose inspection. A missing required capability
refuses enablement without changing configuration and names the flow file,
capability and runtime. `config validate` reports the same mismatch for an
already enabled workflow. Action and template prose are not scanned to infer
requirements. The flavor author maintains the declaration with the commands
it delivers. Pure evaluation does no runtime admission or I/O.

The shipped sdlc prepare and work flows declare
`requires: [worktree_create, shared_delegation]`: creation places each phase's
orchestrator on its derived prepare or work branch, and shared delegation keeps all children
in that recorded workspace. A custom flow requiring unattended adapter attachment of a new top-level thread
to an existing worktree declares `worktree_attach`. Native MCP attachment remains a separate capability and does not supply that unattended CLI route. ASAP delivery
uses the runtime's supported realization and requires no steering declaration.

### Conditions

The engine implements these structural operators. Domain decisions are
expressed by composing them over facts in the definition.

| Operator | Meaning |
| --- | --- |
| `equals` | A present fact equals a typed literal. |
| `same` | Both named facts are present, non-empty and equal. |
| `exists` | The named fact is present and non-empty. |
| `all` | Every nested condition holds. |
| `any` | At least one nested condition holds. |
| `not` | The nested condition does not hold. |
| `at_least` | A present numeric fact meets or exceeds the supplied numeric bound. |
| `at_most` | A present numeric fact does not exceed the supplied numeric bound. |

An absent value cannot satisfy equality or numeric comparison. Empty text and
empty collections are absent evidence; a supplied boolean false or numeric
zero remains a typed value. A comparison does not coerce text to a number or
boolean. Numerical bounds belong to the definition, not an engine policy.

### Record-derived facts

The provider translates artifacts, digests, stamps, provenance, recorded
session references and repository observations into facts. It reports
evidence; semantic classifications such as cosmetic versus substantive drift
come from the agent's recorded judgment.

Review-round counts are derived from verdict stamps, with no occurrence-admission write, for the applicable review
cycle. There are no mutable admission counters in the engine. A repeated
evaluation is not a review round. The shipped flow treats review counts as
information rather than an automatic failure threshold.

There is no persisted workflow phase. Orchestration, the board and other
readers derive it from the same definition and record evidence. A stored
display value does not become an independent authority for progression.

### Configuration ownership

The flavor paths, definition/action shapes, provider fact vocabulary and
prepare/work command surface are specified in
`project/instrukt/spec/domain/workflow`.

| Change | Owning flavor surface |
| --- | --- |
| States, order and conditions | The flow definition. |
| Command associated with an action identifier | The action configuration. |
| Instructions presented for an action | Its Markdown template. |
| Automatic command associated with an effect | The effect configuration beside the action. |

The renderer fills configured variables from the supplied context and resolves
`@config` in Markdown action templates using the same read-time lookup as docs.
A worker action's `todo attempt` command receives the rendered UTF-8 template on
stdin, loads the named procedure closure, binds its parameters and returns the
brief, role, attempt, title/key, workspace requirement and captured base as
structured facts. Its output includes the selected dispatch instruction
from `@config: runtime.dispatch`; the template
body remains addressed to the worker, and dispatch instructions are addressed
separately to the orchestrator. The workflow runs no runtime operation.
The orchestrator delegates and records `todo bind` next to that native call.
The opening worker brief identifies the reserved attempt and requires its
confirmed binding before workspace work or record writes; a worker whose binding
is not yet visible returns that prerequisite to its parent and preserves the
assignment. This initial report does not release the attempt. After binding,
the orchestrator continues that same child natively.
Follow-ups to an active child use `@config: runtime.message_child`, with the bound id and new brief kept as separate fields. A command
in an approval template is an instruction for the responsible session after
the decision is obtained; rendering it does not record approval.

Adding a fact or a condition operator changes the engine or its provider.
Changing a flow, an action's command or its wording uses the corresponding
flavor surface. Commands may target any CLI; the engine has no dependency on
the command vocabulary or on a particular runtime.

### Execution boundary

The orchestrator obtains current evidence through configured commands,
requests an evaluation, and acts on the resulting obligation. A small effect
executor runs effects in their listed order. Each effect is idempotent. On
the next call, evaluation re-derives from the record which effects are still
owed and the executor runs those, including after interruption or failure. Command outcomes remain
visible to the orchestrator; a failed or skipped effect is not recorded as
successful work. Evaluation alone establishes no completion, dispatch,
approval or event delivery.

The workflow is enabled with the work-record capability. A project uses one
workflow engine at a time. Event delivery is a separate capability; it does
not drive the engine's state calculation.

## Use cases

### UC-WD1: Adopt a project workflow

```gherkin
Given a developer is adopting instrukt for a project
And the developer has selected a flavor describing the project's workflow
When the developer configures the workflow's order, actions and instructions
Then the project's next obligations follow that configuration and its work record
And changing those choices requires no change to the evaluation engine
```

### UC-WD2: Enable a workflow whose runtime can carry its declared work

```gherkin
Given a developer selected a runtime with worktree creation and shared native delegation but no unattended existing-worktree attachment
When they enable the shipped sdlc workflow declaring worktree creation and shared delegation
Then that requirement permits enablement
But when they enable a custom workflow declaring existing-worktree attachment
Then instrukt refuses naming the runtime, flow and missing capability
And the configuration remains unchanged
```

## Known caveats

- A flavor's declared requirements describe its own commands; validation checks
  the declaration, not the meaning of arbitrary command or template prose.

## See Also

- [sdlc/principle/design-fundamentals](../../../../domain/sdlc/principle/design-fundamentals.md) — separation of evaluation, evidence and execution responsibilities.
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md) — agent judgment supplies semantic classifications.
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
