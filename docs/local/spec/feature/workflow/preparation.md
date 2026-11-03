---
description: Preparation derives the required artifacts, independent reviews and configured approvals before releasing a todo for execution.
delivered_by: [instrukt-first-build]
---
# Preparation — Spec

## Required reads

@docs/local/spec/feature/workflow/artifact-currency.md

## What it is

A developer prepares an intended change for execution through the flavor's
kind-specific preparation flow. The result is a prepared todo supported by
current derivations, reviews and applicable approval. Preparation is derived
on each call; it does not persist a phase.

## Canonical fields

### Kind routing

| Kind | Preparation contract |
| --- | --- |
| Feature | Intake derives product requirements; independent review precedes the configured approval moment; architecture derivation and review follow, then implementation-plan derivation and review. |
| Change | Implementation-plan derivation and independent review. |
| Bug | Diagnosis, examination and settlement of the diagnosis, then implementation-plan derivation and independent review. |
| Task | Preparation of the task instruction and its configured authorization moment. Execution is a runner dispatch when authorized and due, rather than a build/review workflow. |

Architecture may be skipped with a recorded reason. Otherwise its current
review is required before planning. A seeded plan is input to planning rather
than a reviewed implementation plan. Diagnosis settlement is bound to the
current diagnosis; an unresolved diagnosis remains an explicit hold.

Tasks have no separate reviewer verdict and authorize no repository
implementation work. Authorizing a task checks, against its `input`: the action
boundary, result, recipient and timing come from the input; no limit the input
does not carry is presented as decided; fulfilling it touches no repository
file; the duplicate check was done; the plan carries its intent line,
operations, result handoff and log. Task authorization evidence is recorded
per `local/projects/instrukt/spec/feature/work/approvals`.

### Review and revision obligations

The flavor's actions and templates distinguish drafting, review, revision and
substantive re-derivation at each layer. A current needs-work verdict returns
the findings to that layer's author. It is a remediation obligation rather
than a generic blocked state on the first review result.

Planning distinguishes fresh derivation, seeded derivation and substantive
re-derivation. A revision answering review findings uses the same owning
planning procedure and its findings; it introduces no additional dispatch
reason or synthetic digest-diff header. The orchestrator loads its own
derivation procedures inline and dispatches independent reviews through the
configured runtime capability.

Drift routes and binding rules are owned by
`local/projects/instrukt/spec/feature/workflow/artifact-currency`. A doctrine
conflict discovered during preparation re-enters the affected derivation
layer. Returning an executing todo to preparation is the work orchestrator's
kickback operation.

### Approval and carry

Project key `approvals.prd` controls PRD approval; `approvals.task` controls
task authorization. Each admits human approval or peer approval on the
person's behalf. The shipped flavor defaults to human approval. Independent
review remains a prerequisite where that flow has a review; task authorization
is its own check.

Work records approvals, attribution and task authorization evidence under
`local/projects/instrukt/spec/feature/work/approvals`; workflow determines
which configured approval moment gates readiness and how kinds route.

For every recorded approval, a current independent reviewer judges whether
an earlier approval carries to the current artifact. The claimant may record that
carry only on the reviewer's current conclusion, retaining the earlier
approval's source, identity and evidence and the current target binding. A
carried peer approval stays a peer approval. Recording carry does not
represent the claimant as a new approver. In the absence
of that conclusion the configured approval moment remains owed.

A gate set to `peer` is decided by an agent session in the human's place. A
peer escalates to the human instead of approving whenever the item touches
what the human owns: money, public statements in their name, granting access,
or legal commitments. Approval recording follows
`local/projects/instrukt/spec/feature/work/approvals`.

### Readiness and release

Prepared means every requirement of the selected kind's flow is satisfied,
required review and approval bindings are current, drift obligations are
settled, and commits authored on the canonical branch under the preparation
claim have their required approving commit-review stamps.

The orchestrator releases the claim with `instrukt todo update <slug> --unclaim` and
ends its lane and child sessions with `instrukt sessions end <id>` only
after the release is confirmed. The runner owns subsequent execution dispatch.
Manual preparation does not itself require dependency delivery; work admission
does. Entry claims and holds are work-record and runner concerns, distinct
from the derived preparation state.

### Use cases

#### UC-PR1: Prepare an intended feature for implementation

```gherkin
Given a developer has recorded the feature they want in their project
And the project's workflow requires review and approval of that intent
When the developer has the feature prepared for implementation
Then the todo is released for implementation only after its required derivations, reviews and approval are current
And review findings return to the responsible author for revision
```

## See Also

- `general/policy/human-contract` — new intent and human-owned decisions stay with the person.
- `general/policy/leadership` — the responsible session presents human-owned decisions in context.
- `sdlc/principle/documentation-first-development` — preparation authors the durable behavioral contract before implementation.
