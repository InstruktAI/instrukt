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

## Canonical fields

### Ubiquitous language

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

### Ownership boundary

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
| `claim_commits.unreviewed` | list of SHA |
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
| `worktree.exists` | bool, present only while the assignment worktree exists; absence satisfies cleanup |
| `worktree.path` | string, runtime-returned path retained in the assignment’s build-role session binding, including after that session ends |
| `claim.held` | bool, execution claim currently held |
| `sessions.bound` | list of runtime/id session bindings for this assignment |
| `task_plan.digest` | string, current task-plan digest |
| `task_approved.target_digest` | string, approved task-plan digest |
| `task_approved.by` | human\|peer |
| `worktree.clean` | bool |
| `worktree.head` | SHA |
| `branch.head` | SHA |
| `remote.head` | SHA |
| `quality_checklist.digest` | string, current quality-checklist artifact digest |
| `delivery.commit` | SHA, the confirmed delivered commit |

The provider also keeps these facts honest when the record changes: a new
build-relevant artifact version clears `build.complete`. Any work-branch commit
after a build-completion stamp also clears it when the branch head differs
from the stamped head, including fix commits after `changes_requested`; it
stays false until the next completion stamp. A kickback clears the build and
review facts it invalidates and sets `<r>.cleared_by_substantive_drift` for
each preparation review it invalidates; `claim_commits.unreviewed` counts only
commits whose verdict is neither `approve` nor `exempt`; an approval counts only
from an approver eligible under the gate's configuration; an intent check counts
only for the current `input` version.

The `kind` vocabulary covers feature, change and bug preparation and build
flows, and task preparation; a task's run is a runner dispatch, following the
task routing contract in
`local/projects/instrukt/spec/feature/workflow/preparation`.
Review counts derive from verdict stamps, with no occurrence-admission write;
the counts and marked baseline commit are informational. Drift facts
come from recorded provenance and current evidence; semantic classifications
remain agent judgments.

### Aggregates

#### Workflow definition

- **Root entity:** the named flow selected from the flavor.
- **Contains:** declared states, initial state, ordered transitions,
  conditions, action references and configured effect references.
- **Invariants:** all references resolve; evaluation is deterministic and
  side-effect free; the evaluation contract is owned by
  `local/projects/instrukt/spec/feature/workflow/workflow-definition`.
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
  `local/projects/instrukt/spec/feature/workflow/integration-gate`.
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

This is the workflow's sole emitted event. Review approval is conveyed by the
configured action to its responsible role. Event delivery does not determine
the engine's derived state.

### Domain events consumed

#### Candidate CI result

- **Name:** `runner.ci_completed`.

- **Producer:** the runner checks the integrate candidate’s GitHub CI status
  with a conditional request on each tick and emits the result event.
- **Reaction:** wake the standing integrator to assess the result for the
  exact candidate under
  `local/projects/instrukt/spec/feature/workflow/integration-gate`.
- **Failure handling:** a missing, incomplete or mismatched result does not
  admit a candidate. Failed candidates remain with the integrator for
  remediation; transport recovery is the event capability's responsibility.

#### Settled direct commits

- **Name:** `runner.main_ahead`.
- **Producer:** the runner compares local and remote main on each tick when no
  integration candidate is pending.
- **Reaction:** wake the standing integrator to assess direct commits through
  the integration gate. The event does not authorize promotion.
- **Failure handling:** unsettled or unverified commits remain unpromoted.

#### Failing canonical branch

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

### Use cases

- **Workflow definition** — `local/projects/instrukt/spec/feature/workflow/workflow-definition` — adopt a flavor and configure project obligations independently of the engine.
- **Artifact currency** — `local/projects/instrukt/spec/feature/workflow/artifact-currency` — shared evidence and re-derivation contract for preparation and delivery.
- **Preparation** — `local/projects/instrukt/spec/feature/workflow/preparation` — prepare intended work through current reviews and applicable approval.
- **Delivery cycle** — `local/projects/instrukt/spec/feature/workflow/delivery-cycle` — implement, review and finalize prepared work for integration.
- **Integration gate** — `local/projects/instrukt/spec/feature/workflow/integration-gate` — admit verified candidates and restore a failed shared baseline.

### Governing designs and decisions

The context map indexes the cross-cutting architecture design.

### Context map position

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
- **Runner → workflow — Published language:** `runner.ci_completed` and `runner.main_ahead` wake the
  integrator to assess candidates and direct commits; `runner.main_failing`
  wakes it to restore a failing canonical branch.

`runner.main_ahead` is consumed by the subscribed integrator to assess direct
commits through the integration gate. The `--finalize-ready` work stamp owns
the append of `workflow.finalized` through the runner event-log interface;
derivation alone appends nothing.

Placeholder paths may also name configuration values (for example
`{runtime.session_id}`), resolved by the same rules as `config get`.

## Known caveats

None.
