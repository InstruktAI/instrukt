---
description: Unified delivery cycle enforced by three hard integrity invariants in a single todo worktree, with prose-routed build, review, fix, deferral, and finalize primitives.
---
# Delivery Cycle — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

A repository todo runs end-to-end under the unified `todo-orchestrator` in one
`<slug>/work` worktree. Code enforces three hard integrity invariants and
exposes a facts-only ledger; prose procedures own when to diagnose, plan,
build, review, verify, fix, process deferrals, and finalize.

## Canonical fields

### Three hard integrity invariants

<!-- planned-change:fix-autostart-prepare-work-boundary -->
1. **No interference (attempt lock):** at most one active `Workspace` or `Child`
   attempt may be open on `<slug>/work` at a time. Attempt reservation
   (`instrukt todo attempt`) requires a clean worktree on the recorded branch;
   completion stamps (`--role-complete`, `--build-complete`, review verdicts,
   verification dispositions, `--deferrals-processed`) release the attempt
   atomically.
2. **Independent exact-SHA handoff gate:** `--verdict approve --phase build_review`
   requires an independent reviewer session distinct from the orchestrator and
   all writer roles (`builder`, `setup`, `finalizer`, `intaker`,
   `architect`, `planner`). `--finalize-ready` (callable by a retained completed
   `finalizer` or directly by the claim-holding `todo-orchestrator`) requires
   no open workspace attempt, a clean worktree matching local and published
   `origin/<slug>/work` HEAD, a current independent `build_review` `approve`
   stamp bound to that **exact commit SHA** and current `review-findings`
   digest, and a current verification disposition for that exact commit that
   admits handoff (Live verification disposition below). `--finalize-ready`
   appends `workflow.finalized` atomically.
3. **Human-owned approval gates:** configured `approvals.prd` is enforced on
   `builder` and `finalizer` reservations and on `--finalize-ready`
   whenever the todo is a `feature` or carries a `prd` artifact.
<!-- change:fix-autostart-prepare-work-boundary -->
1. **No interference (attempt lock):** at most one active `Workspace` or `Child`
   attempt may be open on `<slug>/work` at a time. Attempt reservation
   (`instrukt todo attempt`) requires a clean worktree on the recorded branch;
   completion stamps (`--role-complete`, `--build-complete`, review verdicts,
   verification dispositions, `--deferrals-processed`) release the attempt
   atomically.
2. **Independent exact-SHA handoff gate:** `--verdict approve --phase build_review`
   requires an independent reviewer session distinct from the orchestrator and
   all writer roles (`builder`, `setup`, `finalizer`, `intaker`,
   `architect`, `planner`). `--finalize-ready` (callable by a retained completed
   `finalizer` or directly by the claim-holding `todo-orchestrator`) requires
   no open workspace attempt, a clean worktree matching local and published
   `origin/<slug>/work` HEAD, a current independent `build_review` `approve`
   stamp bound to that **exact commit SHA** and current `review-findings`
   digest, and a current verification disposition for that exact commit that
   admits handoff (Live verification disposition below). `--finalize-ready`
   appends `workflow.finalized` atomically.
3. **Human-owned gates:** configured `approvals.prd` is enforced on
   `builder` and `finalizer` reservations and on `--finalize-ready`
   whenever the todo is a `feature` or carries a `prd` artifact. With
   `runner.autostart` off for the todo's kind, workflow enforces a deliberate
   work start on `builder` reservations: for a todo that has had no `builder`
   round, the reservation is admitted only when the todo carries a
   `--work-ready` stamp (`project/instrukt/spec/feature/work/stamps`) and no
   standing dispatch hold, whether the orchestrator continues from preparation
   or is started again. Releasing that automatic hold — removing the `hold`
   label on GitHub (`project/instrukt/spec/feature/work/github-record`) or
   running `instrukt todo update <slug> --release` — starts the work. Turning
   `runner.autostart` on for a kind lets newly prepared todos continue into
   work; a todo already on hold waits until released.
<!-- /planned-change:fix-autostart-prepare-work-boundary -->

### Live verification disposition

The verification disposition is the second half of the handoff gate.
`--finalize-ready` requires that the current disposition
(`project/instrukt/spec/feature/workflow/artifact-currency`) names the exact
candidate commit and is a pass, a Tier 0 skip or a hotfix deferral. A
candidate whose current disposition is fail or blocked, or that carries none
for its commit, is refused readiness, and the refusal names the disposition
owed. What each disposition binds is work's
(`project/instrukt/spec/feature/work/stamps`).

Caller admission is closed for all five dispositions:

| Disposition | Admitted from |
| --- | --- |
| pass, fail, blocked | A bound `verifier` attempt whose session is independent: distinct from the orchestrator and from every writer role, as a build-review approval's reviewer is (`general/principle/verification-independence`). |
| Tier 0 skip | That independent bound `verifier`, or the claim-holding `todo-orchestrator`. |
| hotfix deferral | The claim-holding `todo-orchestrator` alone. |

From any other session — a writer role, a `verifier` that is not independent,
a session that does not hold the claim, or an unbound session — a disposition
is refused and nothing is recorded. The refusal names its door: a pass, fail
or blocked comes from a new independent `verifier` attempt, and a Tier 0 skip
or hotfix deferral from the claim-holding orchestrator.

A `verifier` attempt records one disposition, and recording it releases the
attempt. Its exact replay reports the retained outcome; a different
disposition from the released attempt is refused, naming a new `verifier`
attempt as the door. The orchestrator records a Tier 0 skip or a hotfix
deferral with no verifier attempt, as a record-only write at the clean head of
the shared worktree, and only while no workspace attempt is open on
`<slug>/work`. While one is open the write is refused, naming the door: that
attempt's release, or its recorded failure (`general/principle/trust-and-guardrails`,
interference).

What a `verifier` recorded for a commit limits what the orchestrator may
record for that commit. While the latest disposition a `verifier` recorded for
it is fail, the orchestrator's Tier 0 skip or hotfix deferral is refused, and
the refusal names what clears it: a new candidate commit, or a new `verifier`
attempt on that commit. While it is blocked, the orchestrator's Tier 0 skip is
refused, and the refusal names what clears it: a new candidate commit, a new
`verifier` attempt on that commit, or, under the hotfix exception of
`sdlc/policy/live-verification`, a hotfix deferral naming its tracked
follow-up. That deferral is admitted after a verifier's blocked and admits the
commit to readiness. Because only an independent `verifier` records fail or
blocked, a blocked that lifts a fail on the same commit comes only from a new
independent `verifier` attempt.

Whether a candidate warrants a skip, a deferral or a live run is the
orchestrator's judgment under `sdlc/policy/live-verification`; code admits the
record and judges no diff (`general/principle/intelligence-boundaries`).

`verifier` is a shared-worktree role under the attempt lock. Its attempt is
reserved, bound and released like every other role's, and it works in
`<slug>/work` at the candidate head, with no worktree or branch of its own.

### Fast-track and reversible routing

Code imposes no mandatory `implementation-plan`, `setup`, `--build-started`,
`quality-checklist`, or separate `finalizer` gate on bounded bugs or localized
changes. The orchestrator may dispatch `builder` directly in `<slug>/work`,
follow with independent `build-reviewer` and the verification disposition the
candidate owes, push the exact reviewed HEAD, stamp `--finalize-ready`, and
unclaim to hand off to `integrate`. If unexpected blast radius surfaces
mid-build, the orchestrator routes to planning or doc updates in prose without
skip-flag proliferation.

### Single workspace, cleanup & in-flight legacy migration

Repository todos share `<slug>/work` from freshly fetched `origin/main` (or
`origin/<slug>/work` when resuming on another computer). After `integrate`
promotes the exact verified commit and records `instrukt todo deliver <slug> --commit <sha>`,
the clean worktree and local/origin `<slug>/work` branch are removed while
retaining thread and attempt records. instrukt removes them itself on each
computer that holds them, and no session issues a removal command
(`project/instrukt/spec/feature/runner/worktree-reclamation`).

**In-flight migration from the retired two-phase model:**
- When creating `<slug>/work` for an existing todo that still has an
  un-integrated `origin/<slug>/prepare` or local `<slug>/prepare` branch (and no
  `origin/<slug>/work`), placement automatically branches `<slug>/work` from
  that `<slug>/prepare` head so reviewed or in-progress pending docs carry
  forward with zero commit loss.
- Retained `work-orchestrator` driver bindings on an existing `<slug>/work`
  worktree are adopted automatically by `todo-orchestrator` (preserving the
  existing session, worktree, and claim acceptance). Unreleased legacy
  `prepare-orchestrator` driver bindings and unbound legacy `work-orchestrator`
  driver reservations (`session: None`) do not block dispatch and are retired
  automatically when `todo-orchestrator` takes over.
- If an interrupted legacy start had already created a local `<slug>/work`
  branch without a retained bound session/worktree, dispatch reservation refuses
  to overwrite the existing branch (`todo branch '<slug>/work' already exists;
  no replacement worktree is created`); reconcile by attaching/resuming the
  existing worktree via `instrukt todo work <slug> --placement-evidence ...`,
  or remove the orphaned unstarted worktree/branch once confirmed empty so
  `todo-orchestrator` can create `<slug>/work` cleanly.
- If a legacy `prepare-orchestrator` session still holds an active claim on
  upgrade, release it with `instrukt todo update <slug> --unclaim` (or from the
  claiming computer) so `todo-orchestrator` can pick it up on `<slug>/work`,
  then remove any leftover `<slug>/prepare` worktree (`git worktree remove <path>`)
  before deleting the local and remote `<slug>/prepare` branch once `<slug>/work`
  is created.
- `agents.yaml` v3 migration converts `prepare-orchestrator` / `work-orchestrator`
  entries into `todo-orchestrator` (preferring `work-orchestrator` settings when
  both exist, logging a warning if they differed).

## Use cases

### UC-DC1: Deliver a change for integration

```gherkin
Given a developer has an active repository todo whose dependencies are delivered
When the todo is implemented, independently reviewed at its exact HEAD, and finalized
Then an integration candidate is handed off only with exact-SHA independent approval and matching published candidate evidence
And findings requiring changes return to implementation before that handoff
```

### UC-DC2: A fix reaches the next review and the delivered todo branch

```gherkin
Given a developer's change has review findings requiring fixes
When the orchestrator dispatches a shared builder remediation round after reviewer release
Then it commits corrections directly on the todo branch in the same worktree
When it records build completion at the clean head
Then completion and attempt release commit together
And the next independent reviewer sees those corrections at the current shared head
And retained attempt evidence identifies the builder and completed commit
```

### UC-DC3: Role-branch handoff (retired)

Retired: every role commits directly in the todo's single shared worktree.

### UC-DC4: Serial workspace attempts preserve clean handoff between roles

```gherkin
Given a developer's todo-orchestrator is bound to the todo's single worktree
When a shared workspace role completes its work and records completion at the clean todo head
Then that stamp releases the attempt atomically
And the next role (such as builder) starts in the same worktree on the same todo branch only after that release
```

### UC-DC5: Deferral disposition permits finalization without changing the code

```gherkin
Given a developer's approved change has deferred items awaiting disposition
When the shared deferrer records dispositions covering the current artifact and stamps processing
Then that attempt is released without changing the todo branch
And finalization can proceed in that same clean worktree at its current head
But missing or stale processing evidence keeps the deferrer outstanding and blocks readiness
```

### UC-DC6: Integration receives a currently reviewed exact-SHA candidate

```gherkin
Given a developer's candidate is published in the shared todo worktree
When the candidate head changes, independent review examines the new exact HEAD in the same worktree
And readiness waits for that review's independent exact-SHA approval and release
When readiness is recorded for that matching candidate
Then readiness and the finalized event commit together
```

### UC-DC7: Recover a failed workspace attempt without losing committed work

```gherkin
Given a developer's workspace child has committed work on the shared todo branch but cannot finish
When native evidence confirms it has stopped and the shared tree is clean
And the orchestrator records the failed attempt with its reason and workspace evidence
Then its release preserves the committed work and creates no completion stamp
When a successor is admitted
Then it continues in that same worktree from the current todo head
And completed implementation receives independent review before finalization
But dirty or conflicted work keeps the original attempt outstanding for recovery in place
```

### UC-DC8: Finish delivery without leftover phase resources

```gherkin
Given a developer's reviewed work branch has integrated and the todo is delivered
When the runner next runs on the computer that holds the todo's worktree
Then the recorded worktree and merged work branch are removed locally and on origin
And no agent session issued a removal command
And retained native thread and role-attempt evidence remains readable
But a worktree holding uncommitted changes stays and the todo's record still shows it
```

### UC-DC9: Fast-track a bounded change while enforcing the three integrity invariants

```gherkin
Given a developer has a bounded change or bug in its single todo worktree
When the unified todo-orchestrator dispatches a builder directly without an implementation plan, setup, or quality checklist
Then concurrent workspace attempts are refused by the interference lock
And finalization is refused until an independent reviewer approves the exact published commit SHA
When the independent reviewer approves that exact commit SHA and its verification disposition admits handoff
Then the claim-holding orchestrator can record finalize readiness directly for integration
```

### UC-DC10: Hand off a candidate whose live behavior is accounted for

```gherkin
Given a developer's candidate is independently approved at its exact commit
When no verification disposition is current for that commit
Then finalize readiness is refused
When an independent verifier records a pass for that exact commit and its findings
Then readiness can be recorded for that candidate
But a pass recorded by the orchestrator or a writer of the change is refused
```

### UC-DC11: A failed candidate is not waved through

```gherkin
Given an independent verifier recorded a fail for a developer's candidate commit
When the orchestrator records a Tier 0 skip or a hotfix deferral for that commit
Then the record is refused and names a new candidate commit or a new verifier attempt as what clears it
And finalize readiness for that commit stays refused
```

## Known caveats

- Re-creating a deleted orchestrator thread while its todo branch remains checked
  out requires existing-worktree binding or safe worktree removal first.

## See Also

- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
