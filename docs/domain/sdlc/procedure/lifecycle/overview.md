---
eval: true
activation_trigger: Use when you need to prime as an orchestrator or need to know how orchestration works.
description: 'Software development lifecycle. Three-stage model: prepare, work, integrate.'
visibility: public
delivered_by: [instrukt-first-build]
---

# Lifecycle Overview — Procedure

## Goal

The software development lifecycle follows a three-stage model. Preparation
and work derive obligations from flavor definitions and current record
evidence; the standing integrator owns the integration gate. The evaluator
stores no phase and executes no command.

### 1. Prepare (`instrukt todo prepare`)

Sequential artifact production with review gates:

- **Human refinement / handoff** — the versioned `input` records intent for
  every kind. Its first non-author recipient checks it against the originating
  conversation and records the version-bound intent check. A changed input
  requires a new check; later recipients carry checked intent unchanged.
- **Intake** — for a feature, derive `prd` and the pending behavioral contract
  from checked `input`.
- **PRD review** — independently challenge the drafted product horizon,
  enrichment, scope reasoning, assumptions, omissions, and observable outcomes
  before the configured approval moment. Human approval is the default; a peer
  gate retains peer attribution and escalates human-owned exposure.
- **Architecture** — derive and independently review the code-grounded design
  in domain/design snippets, with `architecture` as its thin index/log, or skip
  with a recorded reason when it does not earn its place.
- **Plan drafting** — produce `implementation-plan` (or split the todo); plan
  tasks reference the checked intent, applicable PRD criteria, architecture
  constraints, and feature-spec scenarios. A change enters here; a bug enters
  after independent diagnosis, examination and current-digest settlement.
- **Plan review** — validate plan against policies, DoD gates, review lanes.
- **Readiness gate** — require the kind's current artifact, report, review and
  approval bindings, plus approving commit reviews for canonical-branch commits
  authored under the preparation claim.
- **Grounding check** — analyze pending drift. Substantive drift re-enters its
  owning derivation layer; cosmetic settlement binds the evidence assessed.
- **Task preparation** — draft the external action's `task-plan` and obtain its
  configured authorization. Tasks have no PRD, architecture, implementation plan
  or separate reviewer verdict. The runner owns authorized, eligible, due runs.

Outputs: the selected kind's reviewed derivations and applicable approval,
complete digest-bound review reports, and confirmed preparation-claim release.
Prepare authors pending documentation on canonical main; work executes it.

### 2. Work (`instrukt todo work`)

Build-review-fix-finalize cycle for prepared repository work with delivered
dependencies:

- **Entry** — obtain current record/repository evidence and execute the
  configured runtime-worktree, upstream-merge, branch-publication and build-start
  obligations. The runtime creates worktrees; the worktree request uses the
  derived name `<slug>/work`, and preparation remains on canonical main.
- **Build** — bind the feature-spec scenarios in scope under the project's
  testing contract and implement the code that satisfies them; execute the
  implementation plan task-by-task, commit per task and stamp build completion.
  Suite execution belongs to the integration gate; project hooks retain their
  own checks.
- **Review** — parallel review lanes (scope, code, principles, security, tests,
  errors, types, comments, docs), persisted as a complete report whose digest
  and reviewed worktree HEAD are bound to the verdict. Both approval and
  changes-requested verdicts require a current binding. Undetermined evidence
  preserves the verdict while exposing the gap.
- **Fix** — address review findings minimally, re-review while the orchestrator
  can name what the next pass resolves. A changed approved plan requires build
  reconciliation; a plan defect returns through a recorded kickback.
- **Deferrals** — process deferred items into dispositions and, where admissible,
  new todos before finalizer dispatch.
- **Finalize** — integrate latest main, push the branch and stamp readiness.
  Require a clean worktree and matching worktree, local branch, remote branch and
  readiness-stamp commits before the configured integration handoff.
- **Bug fix** — build from the independently settled diagnosis and prepared
  plan under the same review and finalization contract.

The handoff emits `workflow.finalized` for the exact candidate and confirms
claim/lane release. Work writes no doc contract and folds no pending docs;
the integrator reconciles delivery documentation before candidate verification.

### 3. Integrate (standing integrator)

Merge candidates to canonical main:

- **Candidate composition** — merge finalized branches and settled direct
  local-main commits in the persistent integration worktree. The runtime owns
  worktree creation; the integration procedure owns its established location.
- **Conflict resolution** — resolve in the integration worktree, preserving
  both sides' meaningful intent. Run `instrukt docs fold <slug>` to fold the delivery's pending documentation
  before freezing the candidate.
- **Verification** — push the exact candidate to `origin/integrate`. Required
  green checks admit that commit; `runner.ci_completed` wakes the integrator
  with its result. Without CI, run the project's configured `test` action on
  that candidate in the integration worktree.
- **Push** — fast-forward canonical main to exactly the verified commit and
  confirm the remote result. Main movement or candidate edits require
  reconciliation and verification for the resulting commit.
- **Delivery bookkeeping** — record the confirmed delivered commit through
  `instrukt todo deliver <slug> --commit <sha>` and reconcile the work record and working
  resources through established commands.
- **Recovery** — if canonical main becomes red, put a revert through the same
  gate first; repair its cause in separate work.

The runner also emits `runner.main_ahead` when local main is ahead of remote
main with no pending candidate, so direct commits enter the same gate. The
standing integrator's subscription and activation routes carry that wake-up.

### Principles

- **Serial execution**: one obligation completes before its dependent work begins.
- **Deterministic handoffs**: each call derives from the current definition and
  evidence. Effects run in listed order and are idempotent; their optional
  `done_when` determines whether they remain owed on replay.
- **No micromanagement**: orchestrator dispatches and monitors; workers implement autonomously.
- **Quality gates**: project-owned hooks enforce their standards; independent
  review validates completeness and the integration gate verifies the candidate.
- **Tandem for decisions, single for execution**: material discovery and design decisions receive independent peer falsification; once a decision is settled, one agent owns execution, with no peer ceremony added to fleshed-out mechanical work.

## Preconditions

- `instrukt todo show {slug}` shows the item, checked input, claim and role
  session bindings; entry honors holds, workspace rules and dependencies.
- The selected flavor supplies `workflow/prepare.yaml`, `workflow/work.yaml`,
  `workflow/actions.yaml` and `workflow/templates/<action>.md`.
- The configured runtime supplies the required session and worktree capabilities.

## Steps

1. Run `instrukt todo prepare {slug}` to derive preparation obligations. Read
   the selected action, rendered template and owed effects; execute their
   established commands and re-derive after material outcomes.
2. After preparation release and runner admission, run
   `instrukt todo work {slug}` to build, review, fix and finalize. Reuse recorded
   sessions through `alive` and `send`; start only a required new session through
   `instrukt sessions run` and its runtime mapping.
3. Execute the configured finalize-ready handoff. The standing integrator
   composes, verifies and promotes the exact candidate; no work-role command
   performs integration.
4. Confirm record writes, handoff and release from their evidence. A reply,
   successful ended turn or workflow evaluation alone establishes none of them.

## Outputs

- Delivered work item merged to main through the integration gate, or an
  authorized task execution with a recorded occurrence outcome.
- Updated work artifacts, stamps and delivery records, with confirmed claim
  release and explicitly accounted-for cleanup.

## Recovery

- Re-calling prepare or work derives from current facts; no persisted phase is
  resumed. Execute effects still owed under their `done_when` conditions and
  inspect failures before dispatching further work.
- On an upstream merge conflict, abort the merge and route recovery to the
  recorded live builder's procedure. A missing builder is an explicit dispatch
  gap; a failed fetch or skipped merge is not synchronization.
- A remote record-revision conflict uses `instrukt todo refresh {slug}`, then
  reapplication of the intended mutation.
- If blocked, preserve the evidence and responsible owner. Human-owned questions
  remain in the responsible thread; the runner handles unattended anomalies.
- A missing command binding or runtime capability remains an open obligation.
  Inspecting a record records no approval, release, handoff or delivery. The
  flavor's templates expose unresolved command boundaries before execution.

## See also

- ~/.instrukt/docs/sdlc/procedure/lifecycle/prepare/overview.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/work/overview.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/integrate.md
