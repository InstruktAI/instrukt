---
description: Workflow derives the current work obligation from configured definitions and record evidence, and owns admission of verified integration candidates.
delivered_by: [instrukt-first-build]
---
# Workflow — Spec

## What it is

Workflow owns the derivation of preparation and delivery obligations and the
integration gate that protects a project's canonical branch. Its engine
computes from a flavor definition and facts; its orchestrators execute
configured actions and effects. Durable evidence belongs to the work record,
session capabilities to the configured runtime, and event delivery to the
runner and event capability. Workflow stores no phase.

## Ubiquitous language

**Flavor**
: The selected documentation and configuration defining how a project works.

**Definition**
: A flow's states, ordered transitions, conditions and action/effect references.

**Fact**
: A typed observation derived from the work record or current evidence.

**Derived state**
: The state computed from a definition and facts on the current call.

**Obligation**
: The action and effects selected by derivation for the responsible role: the
  `role` of the procedure the action's command names, or, for an action naming
  none, the orchestrator role its flow declares.

**Orchestrator**
: The agent driving a preparation or work lane and owning its command outcomes.

**Currency**
: Whether a recorded review, approval or drift judgment still binds current evidence.

**Kickback**
: A work-orchestrator decision returning execution to preparation with a recorded reason.

**Finalize-ready**
: The handoff of a finalized todo and its matching commit to the standing integrator.

**Integration candidate**
: The exact commit composed in the integrator's worktree and submitted for verification.

**Canonical branch**
: The shared branch advanced only through the integration gate.

## Ownership boundary

- **Owns:** definition evaluation, condition semantics, fact interpretation for
  workflow progression, review and drift currency, kind-specific preparation,
  the build/review/finalize cycle, kickback routing and integration admission.
- **Does not own:** work-record persistence, artifact versioning, claim storage
  or GitHub reflection; runner scheduling, task due-time calculation and
  runtime status normalization; event transport and notification settings;
  configuration layering; documentation publication or code-graph generation.
- **Published contracts:** definition plus facts to derived state, selected
  transition, action identifier and effects; `instrukt todo prepare <slug>` and `instrukt todo work <slug>`;
  finalize-ready handoff; verified candidate admission and recovery.
- **Allowed dependencies:** the work record supplies durable evidence;
  configuration supplies flavor and project action selection; publication
  supplies the selected domain’s published workflow files; the runner
  supplies session and event delivery capabilities; docs consumes workflow’s
  declared flavor roles through a core-declared interface wired by the CLI. The integration gate
  consumes GitHub results and branch protection, or the project's local test
  action when CI is absent.
- **Implementation roots:** the `instrukt-todos` crate in the Rust
  workspace, with workflow definitions, actions and templates supplied by the
  flavor. Internal source-directory allocation does not define this boundary.

### Flavor-facing contract

Project key `workflow.domain` selects the published domain, default `sdlc`.
The publishing project authors the flavor under `docs/domain/<domain>/workflow/`;
publication copies it to `~/.instrukt/docs/<domain>/workflow/`, where the engine
loads it. Paths below are relative to that published domain.

The selected flavor supplies definitions at `workflow/<flow>.yaml`, actions
at `workflow/actions.yaml`, and Markdown templates at
`workflow/templates/<action>.md`. A definition declares `flow`, its lane
`orchestrator` role, `initial`, `states` and ordered `transitions`; each
transition declares `from`, `to`,
`when`, `action` and optional ordered effects. Actions are keyed by action id
and declare `command` and `template`; template paths resolve within `workflow/`.
`workflow/actions.yaml` also declares `roles:`, a non-empty list of unique role
names used by procedure-role validation when todos is enabled. Action ids stay
at the file root alongside reserved `roles:` and `effects:` keys.
Effects are declared in the same file under `effects:`, keyed by effect name,
each with `command` and an optional `done_when` condition over facts.

A transition's `when` is one condition; a map with several operator keys means
all of them hold (an implicit `all`). Commands and templates use `{name}`
placeholders, the same syntax as configuration interpolation: `{slug}`,
`{project}`, `{kind}`, `{brief}` (the rendered template) and any fact by its
name (`{prd.digest}`). Reserved bindings (`slug`, `project`, `kind`, `brief`)
resolve first, then declared fact names, then configuration paths. An absent
declared fact remains absent rather than falling through to a same-named
configuration key. Unknown names are findings; rendering never invents values.

The orchestrator calls `instrukt todo prepare <slug>` (flow
`workflow/prepare.yaml`) or `instrukt todo work <slug>` (flow
`workflow/work.yaml`) for its next obligation. When the todo is active and the calling session holds no claim on it, the
call first records and reflects one through work's command contract
(deliberate pickup; a claim held elsewhere refuses). For a delivered or
removed todo the call is read-only. Each call then obtains current facts, evaluates the
corresponding flow and returns the derived state,
selected transition, action and rendered instructions, with effects owed.

The provider exposes these fact names and types. In review-family rows,
`r` expands to `prd_review`, `architecture_review` and `plan_review`.
Absent or empty evidence is absent; false and zero remain typed values.

| Fact | Type |
| --- | --- |
| `kind` | enum feature\|change\|bug\|task |
| `deps.delivered` | bool |
| `prd.digest` | string |
| `architecture.digest` | string |
| `diagnosis.digest` | string |
| `implementation_plan.digest` | string |
| `implementation_plan.seeded` | bool |
| `prd_review_findings.digest` | string |
| `architecture_review_findings.digest` | string |
| `plan_review_findings.digest` | string |
| `<r>.verdict` for r in prd_review, architecture_review, plan_review | approve\|needs_work |
| `<r>.target_digest` | string |
| `<r>.report_digest` | string |
| `<r>.cleared_by_substantive_drift` | bool |
| `<r>.rounds` | int, informational |
| `prd_approval.digest` | string |
| `prd_approval.by_human` | bool |
| `architecture_skipped.reason` | string |
| `diagnosis_settled.digest` | string |
| `preparation.head` | SHA |
| `preparation.review_current` | bool |
| `preparation.review_verdict` | approve / needs_work, present only for a current head/report binding |
| `preparation.integrated` | bool |
| `drift.prd`, `drift.architecture`, `drift.implementation_plan` | fresh\|pending\|cosmetic\|substantive |
| `build.start_base_sha` | string |
| `build.plan_digest` | string |
| `build.complete` | bool |
| `build_review.verdict` | approve\|changes_requested |
| `build_review.target_digest` | SHA |
| `build_review.binding` | exact_target\|clean_upstream_merges\|todo_changed\|report_changed\|unbound\|undetermined |
| `build_review.round` | int, informational |
| `build_review.baseline_commit` | SHA, informational: the build-start base commit current when the build-review verdict was stamped |
| `deferrals.pending` | bool |
| `finalize.status` | pending\|ready |
| `finalize.sha` | SHA |
| `setup.complete` | bool, successful setup role completion at its clean shared head after all setup/admission obligations; failure alone never satisfies it |
| `finalizer.review_pending` | bool, latest successful finalizer completion is at the current todo head and review is `todo_changed`, `report_changed` or `unbound` |
| `finalizer.completion_current` | bool, latest successful finalizer completion is at the current todo head |
| `finalizer.session` | retained native id for the record-only readiness follow-up |
| `attempts.retained` | all role-attempt records, native ids and captured/completed heads, with the same todo workspace and branch for work children |
| `worktree.exists` | bool, present while the todo's one recorded worktree exists; absence satisfies delivered cleanup |
| `worktree.path` | string, the work orchestrator's runtime-recorded todo worktree, shared by its team and retained through delivery cleanup |
| `claim.held` | bool, execution claim currently held |
| `sessions.bound` | list of runtime/id session bindings for this assignment |
| `task_plan.digest` | string, current task-plan digest |
| `task_approved.target_digest` | string, approved task-plan digest |
| `task_approved.by` | human\|peer |
| `worktree.clean` | bool, todo's shared worktree |
| `worktree.head` | SHA, todo's shared worktree |
| `branch.name` | string, captured work-phase branch name for publication and cleanup |
| `branch.head` | SHA, the todo branch checked out in its shared worktree |
| `remote.head` | SHA, published todo branch |
| `quality_checklist.digest` | string, current quality-checklist artifact digest |
| `delivery.commit` | SHA, the confirmed delivered commit |

Workspace observation reads the todo's one runtime-recorded worktree and todo
branch for setup, build, review, fixing, deferrals and finalization. Each role
reservation captures its current clean head and binds the child's shared placement.
Missing or mismatched placement is a visible gap, not another role's checkout.
Finalizer completion remains the source for record-only readiness after its
release and any subsequent independent review at the same shared head.

Setup performs synchronization and records admission evidence before successful
`--role-complete`; setup failure remains unsatisfied. Completion/release, review
verdict/release and deferral disposition/release are single record transactions.
There are no role-branch advancement or failed-branch recovery facts. A clean,
confirmed failed release permits continuation at the same tree's current head.

Delivered cleanup selects only that recorded todo path, executes removal from
the project folder and re-observes until absent. Dirty/in-use worktrees and failed
removal remain explicit obligations. All role-attempt evidence and the runtime
thread record remain after removal.

The provider also keeps these facts honest when the record changes: a new
build-relevant artifact version clears `build.complete`. Any work-branch commit
after a build-completion stamp also clears it when the branch head differs
from the stamped head, including fix commits after `changes_requested`; it
stays false until the next completion stamp. A kickback clears the build and
review facts it invalidates and sets `<r>.cleared_by_substantive_drift` for
each preparation review it invalidates; preparation head review binds the current clean phase head and report; an approval counts only
from an approver eligible under the gate's configuration; an intent check counts
only for the current `input` version.

The `kind` vocabulary covers feature, change and bug preparation and build
flows, and task preparation; a task's run is a runner dispatch, following the
task routing contract in
`project/instrukt/spec/feature/workflow/preparation`.
Review counts derive from verdict stamps, with no occurrence-admission write;
the counts and marked baseline commit are informational. Drift facts
come from recorded provenance and current evidence; semantic classifications
remain agent judgments.

## Canonical fields

### Aggregates

#### Workflow definition

- **Root entity:** the named flow selected from the flavor.
- **Contains:** declared states, initial state, ordered transitions,
  conditions, action references and configured effect references.
- **Invariants:** all references resolve; evaluation is deterministic and
  side-effect free; the evaluation contract is owned by
  `project/instrukt/spec/feature/workflow/workflow-definition`.
- **Identifier:** flavor and flow identity.
- **Lifecycle:** loaded from the selected configuration and evaluated against
  facts. A definition change changes subsequent derivation, rather than
  migrating a stored phase.

#### Integration candidate

- **Root entity:** the candidate commit submitted by the standing integrator.
- **Contains:** the finalized work represented by the commit and the
  verification evidence applying to it. Durable references live in the work
  record and repository, not a second workflow state store.
- **Invariants:** admission applies to the verified commit; composition,
  promotion and recovery follow
  `project/instrukt/spec/feature/workflow/integration-gate`.
- **Identifier:** project and candidate commit.
- **Lifecycle:** composed in the integration worktree, submitted for
  verification, then promoted or remediated. Remediation that changes the
  commit produces a candidate requiring its own verification.

The todo itself remains a work-record aggregate. An evaluation result is a
derived value, not another durable aggregate.

### Domain events emitted

#### Finalize-ready

- **Name:** `workflow.finalized`.

- **Emitted when:** the delivery cycle establishes current matching
  finalization evidence and hands off to integration.
- **Payload:** project identity, todo identity and finalized commit identity;
  the event makes that candidate addressable to its consumer.
- **Consumer:** the standing integrator, dispatched by the runner.
- **Idempotency key:** the logical handoff is identified by project, todo and
  finalized commit. Re-evaluating unchanged readiness is the same handoff.

#### Reviewed preparation

- **Name:** `workflow.preparation_reviewed`.
- **Emitted when:** the preparer releases its claim after an independent review approved the current clean preparation head and complete findings digest, while its worktree still exists. The release and event append commit atomically.
- **Payload:** project, todo and reviewed preparation commit.
- **Consumer:** the standing integrator, for immediate documentation integration
  and publication before build, then preparation worktree and branch cleanup.
- **Idempotency key:** project, todo and reviewed preparation commit.

Other worker endings remain native parent attention. Event delivery does not
determine the engine's derived state.

### Domain events consumed

#### Candidate CI result

- **Name:** `runner.ci_completed`.

- **Producer:** the runner checks the integrate candidate’s GitHub CI status
  with a conditional request on each tick and emits the result event.
- **Reaction:** wake the standing integrator to assess the result for the
  exact candidate under
  `project/instrukt/spec/feature/workflow/integration-gate`.
- **Failure handling:** a missing, incomplete or mismatched result does not
  admit a candidate. Failed candidates remain with the integrator for
  remediation; transport recovery is the event capability's responsibility.

#### Failing canonical main

- **Name:** `runner.main_failing`.
- **Producer:** the runner, observing the CI result of remote canonical main's
  head on each tick.
- **Payload:** project identity and the failing head commit identity.
- **Idempotency key:** project and head commit; a commit's red result is
  reported once, and a new head is observed afresh.
- **Reaction:** wake the standing integrator to restore the canonical branch
  (revert first, repair as separate work). The event authorizes nothing by
  itself.
- **Not observed:** a red result on a commit that is no longer the head. The
  head's own CI covers the combined state.

## Event subjects and origins

`workflow.finalized` concerns the todo in its project origin; its payload
names the finalized commit (`project/instrukt/spec/feature/runner/event-envelope`).

## Use cases

- **Workflow definition** — `project/instrukt/spec/feature/workflow/workflow-definition` — adopt a flavor and configure project obligations independently of the engine.
- **Artifact currency** — `project/instrukt/spec/feature/workflow/artifact-currency` — shared evidence and re-derivation contract for preparation and delivery.
- **Preparation** — `project/instrukt/spec/feature/workflow/preparation` — prepare intended work through current reviews and applicable approval.
- **Delivery cycle** — `project/instrukt/spec/feature/workflow/delivery-cycle` — implement, review and finalize prepared work for integration.
- **Integration gate** — `project/instrukt/spec/feature/workflow/integration-gate` — admit verified candidates and restore a failed shared baseline.

## Governing designs and decisions

The context map indexes the cross-cutting architecture design.

- `project/instrukt/design/architecture/overview` — the module boundaries and shared publication, retrieval and integration flows.

## Context map position

- **Work → workflow — Customer / Supplier:** work supplies artifacts, stamps,
  provenance and recorded approvals; workflow owns gate requirements and kind routing.
- **Configuration → workflow — Customer / Supplier:** configuration supplies
  selected definitions, actions and approval settings.
- **Workflow → runner — Customer / Supplier; Published language:** workflow
  supplies computed obligations and `workflow.finalized` for dispatch.
- **Publication → workflow — Customer / Supplier:** publication supplies the
  selected domain’s published definitions, role declarations, actions and templates.
- **Workflow → docs — Customer / Supplier:** workflow supplies declared flavor
  roles for validation through a core-declared interface when todos is enabled.

- **Repository → workflow — Customer / Supplier:** repository supplies the
  candidate integration branch and exact-commit main advancement.
- **Runner → workflow — Published language:** `runner.ci_completed` and `workflow.preparation_reviewed` wake the
  integrator to assess candidates and reviewed preparation heads; `runner.main_failing`
  wakes it to restore a failing canonical branch.



Placeholder paths may also name configuration values (for example
`{runtime.session_id}`), resolved by the same rules as `config get`.

## Known caveats

- A flavor using this vocabulary requires matching provider and validator
  support. A consumer with an older fact catalogue refuses it before evaluation.

## See Also

- [project/instrukt/design/architecture/overview](../../design/architecture/overview.md)
- [project/instrukt/spec/feature/workflow/delivery-cycle](../feature/workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/runner/session-continuity](../feature/runner/session-continuity.md)
