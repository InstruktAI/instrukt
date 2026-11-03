---
description: Configured workflow evaluation derives the current obligation from record facts without storing a phase or executing commands.
delivered_by: [instrukt-first-build]
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
| Definition | A named flow with its orchestrator role, an initial state, declared states and ordered transitions. |
| Transition | A source state, destination state, condition, action identifier and optional effects. |
| Facts | Named, typed observations supplied from the work record and current evidence by the fact provider. |
| Result | The state where derivation stops, the last selected enabled transition, its action identifier, and the effects still owed from the record along the selected walk. If no transition is selected, there is no selected action or effect. |
| Action | A flavor-owned command and Markdown template addressed by an identifier; its responsible role follows from its command. |
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
is the publishing project’s `docs/global/<domain>/workflow/`.
`actions.yaml` declares a unique role-name list under `roles:`, alongside the
action entries and `effects:`. Docs validation checks these files against the
workflow schema, separately from snippet taxonomy validation.

Each flow file declares its lane orchestrator with the required top-level key
`orchestrator: <role>`, a role declared under `roles:`. Exactly one procedure
declares that role; it is the procedure the runner dispatches for the lane. An
action's responsible role follows from its command:

- a command naming a procedure (`--procedure <id>` or `docs get <id>`) belongs
  to that procedure's `role`;
- a command naming no procedure is a step of the orchestrator of the flow that
  selected it, executed by the session holding the todo's claim — the session
  the runner started into the orchestrator's procedure, or the one
  `instrukt todo prepare|work` made the orchestrator.

Every procedure's `role` is one of the flavor's `roles:`. Definition findings
include a flow without `orchestrator`, an `orchestrator` role not declared
under `roles:`, no procedure declaring the `orchestrator` role, and an action
whose command names a procedure that declares no `role`.

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
`local/projects/instrukt/spec/domain/workflow`.

| Change | Owning flavor surface |
| --- | --- |
| States, order and conditions | The flow definition. |
| Command associated with an action identifier | The action configuration. |
| Instructions presented for an action | Its Markdown template. |
| Automatic command associated with an effect | The effect configuration beside the action. |

The renderer fills configured variables from the supplied context. A command
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

### Use cases

#### UC-WD1: Adopt a project workflow

```gherkin
Given a developer is adopting instrukt for a project
And the developer has selected a flavor describing the project's workflow
When the developer configures the workflow's order, actions and instructions
Then the project's next obligations follow that configuration and its work record
And changing those choices requires no change to the evaluation engine
```

## See Also

- `sdlc/principle/design-fundamentals` — separation of evaluation, evidence and execution responsibilities.
- `general/principle/intelligence-boundaries` — agent judgment supplies semantic classifications.
