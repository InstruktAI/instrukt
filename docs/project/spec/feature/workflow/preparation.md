---
description: Preparation derives the required artifacts, independent reviews and configured approvals before releasing a todo for execution.
---
# Preparation — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

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
per `project/instrukt/spec/feature/work/approvals`.

### Review and revision obligations

The flavor's actions and templates distinguish drafting, review, revision and
substantive re-derivation at each layer. A current needs-work verdict, including an exact preparation-head verdict, returns the complete findings to that layer's author. A corrected head requires a new independent review. It is a remediation obligation rather
than a generic blocked state on the first review result.

Planning distinguishes fresh derivation, seeded derivation and substantive
re-derivation. A revision answering review findings uses the same owning
planning procedure and its findings; it introduces no additional dispatch
reason or synthetic digest-diff header. The orchestrator delegates doc writers and independent reviewers as shared
native children through the configured instruction rendered by `@config`,
with one writer at a time in `<slug>/prepare`. The workflow
supplies the brief, role, attempt and captured shared placement; work commands reserve
the attempt and bind the returned native child id. The runtime supplies parent
linkage and ending delivery. Agent follow-ups, conversation reads and inspection
likewise use configured native instructions; a verdict is record evidence, not
a wake event.

Drift routes and binding rules are owned by
`project/instrukt/spec/feature/workflow/artifact-currency`. A doctrine
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
`project/instrukt/spec/feature/work/approvals`; workflow determines
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
`project/instrukt/spec/feature/work/approvals`.

### Continue after an approval decision

Applying an approval or request-changes decision records its attribution and
current gate evidence, then directs as-soon-as-possible attention to the todo's
current orchestrator when it holds the claim and is alive. That orchestrator
re-derives preparation from the updated record. With no live orchestrator,
only the state changes; next pickup or a deliberate start continues from it.
This is direct attention under session-continuity, not an approval event or
an automatic worker dispatch. GitHub intake is specified by github-record.

### Preparation workspace and immediate integration

Preparation and work use separate T3-managed worktrees. instrukt derives `<slug>/prepare` and `<slug>/work`; agents never choose a bare slug or a role branch. Preparation starts from canonical main with `t3 thread start <project> - --worktree origin/main --branch <slug>/prepare --title <key>`. Its orchestrator and native shared children all use that preparation worktree. Creation refuses while the local phase branch still exists; retry or wake uses the retained native assignment, never another tree. Deliberate entry first returns placement and the configured native start instruction; after native startup, `todo prepare|work <slug> --placement-evidence <file|->` records supplied native session, key, recorded worktree, branch, head and source. The caller must be that native session in the matching registered tree. Git cwd alone never binds an orchestrator. Runner-created bindings instead use its verified adapter result.

Review binds the committed preparation-worktree head. An approving `instrukt todo stamp <slug> --verdict approve --phase preparation_head`, bound to `preparation-review-findings`, releases the independent reviewer. The preparer then releases its own claim through the normal update operation while its worktree still exists; that release atomically appends `workflow.preparation_reviewed` for the approved current head. The integrator merges those reviewed docs as soon as possible, before build, folds their pending markers in its integration worktree and publishes through the ordinary gate. Once integrated the docs are current on main; they do not wait for build delivery. Other todos that depend on them wait for that integration.

After confirmed preparation integration, the integrator removes the clean preparation worktree and deletes `<slug>/prepare` locally and on `origin`. Failure or dirty/in-use state retains the cleanup obligation and its cause; retained thread records and review evidence stay. Work starts later from the new main with `--worktree origin/main --branch <slug>/work`; on another computer its retained published work starts with `--worktree origin/<slug>/work --branch <slug>/work`. The work orchestrator and setup, builder, reviewer, fixer, deferrer and finalizer share that one work worktree through native shared delegation. Only the integrator writes main. Ad hoc authoring uses its own one-off worktree, reviewed before integration.

### Readiness and release

Prepared means every requirement of the selected kind's flow is satisfied,
required review and approval bindings are current, drift obligations are
settled, required independent review binds the current preparation head. The preparer releases and hands off then; work remains gated on integration and publication.

After an approving current head review, before integration or cleanup, the orchestrator releases the claim with `instrukt todo update <slug>
--unclaim`, confirms `claim.held` is false and records the handoff. The
dispatcher retains conversations needed for later review rounds or human
questions; session ending and cleanup belong to the runtime. The runner owns subsequent execution dispatch and does not reclaim the released preparer while integration or preparation cleanup remains owed. Manual preparation does not itself require
dependency delivery; work admission does. Entry claims and holds are work-record
and runner concerns, distinct from the derived preparation state.

### Fresh canonical base

Before creating either `<slug>/prepare` or `<slug>/work`, fetch origin and use
`t3 thread start <project> - --worktree origin/main --branch <slug>/<phase> --title <key>`.
Only the integrator writes and pushes gate-green exact commits to main; local
main can lag and supplies no new-phase base. A fetch failure leaves startup
unperformed, with its cause visible. A retained published `<slug>/work` on
another computer is continuation of the existing phase, using freshly fetched
`origin/<slug>/work` with the same local branch name; it is never reset to main.
For repositories whose canonical branch has another name, use that remote ref.

## Use cases

### UC-PR1: Prepare an intended feature for implementation

```gherkin
Given a developer has recorded and checked an intended feature
When preparation runs in its derived prepare worktree
Then its orchestrator delegates doc writers and independent reviewers as native shared children
And the feature receives its reviewed derivations and applicable approval
And independent review binds the preparation head for immediate integration before build
And remote main changes only through the integrator's gate
And findings return to the responsible author for revision
```

### UC-PR2: Resume preparation after giving an approval decision

```gherkin
Given a developer's todo is waiting at its configured approval gate
When an admissible approval or request-changes decision is applied
Then its live claim-holding orchestrator receives that decision as soon as possible
And re-derives the next preparation obligation from the record
But when no live orchestrator exists
Then the decision remains recorded for the next pickup or deliberate start
And no orchestrator is started solely to deliver the decision
```

## Known caveats

- Preparation integration does not deliver the build. Review and integration of the work candidate remain owed independently.

## See Also

- [general/policy/human-contract](../../../../general/policy/human-contract.md)
- [general/policy/leadership](../../../../general/policy/leadership.md)
- [sdlc/principle/documentation-first-development](../../../../domain/sdlc/principle/documentation-first-development.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
