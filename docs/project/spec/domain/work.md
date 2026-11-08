---
description: Durable project work, evidence, approvals, dependencies and evaluation within the todos module.
visibility: public
delivered_by: [instrukt-first-build]
---
# Work — Spec

## What it is

Work owns the durable record of a project's todos: their intent, kind,
versioned artifacts, stamps, approval attribution, dependencies, group
structure and dispatch holds. It supplies evidence to the workflow engine and
evaluation reports within the optional todos module. It owns the record and
its GitHub reflection; workflow owns derivation and execution decisions, the
runner owns dispatch, and the configured runtime owns conversations.

## Ubiquitous language

**Todo**
: A uniquely identified piece of project work with explicit intent and kind.

**Kind**
: The work category selecting its preparation and completion path: feature,
  change, bug or task.

**Artifact**
: Named work evidence with retained content versions and writer provenance.

**Stamp**
: An attributed fact bound to evidence, carrying model identity, version and
  eval fingerprint.

**Approval record**
: An evidence-bound decision attributed to the observed approver; workflow
  owns what its approval gate requires.

**Intent check**
: The first non-author recipient's recorded check that an intent version
  faithfully reflects its originating conversation.

**Prerequisite**
: Another project todo whose evidence or delivery gates a dependent's pickup.

**Structural holder**
: A group parent outside the todo lifecycle, retaining split evidence and
  grouping its children by durable identity.

**Execution claim**
: The item-level claim recorded and reflected before work starts — by the
  runner on automatic dispatch, or by `instrukt todo prepare|work` on
  deliberate pickup — exposed as `claim.held`; distinct from per-todo, per-role session bindings.

**Session binding**
: The started session’s runtime and id, recorded per todo and role, distinct
  from the execution claim and reused by the runner.

**Task schedule**
: A task’s one-off `at` or recurring `cron` schedule and `catch_up` policy.

**Dispatch hold**
: An attributed pause on automatic pickup, carrying its cause.

**Remote revision**
: The committed version of a todo's reflected GitHub record, against which
  later reflection checks for conflicts.

**Eval fingerprint**
: The identity of the process-document set in force when evidence is stamped.

## Ownership boundary

- **Owns:** todo identity and record mutations; artifact versions; stamp
  evidence; approvals and intent-check records; prerequisite and group facts;
  holds; per-role session bindings and task schedules; local-leading
  persistence in the state database — its file, writer rules, migrations and
  work tables (`project/instrukt/spec/feature/work/state-database`);
  GitHub reflection and recovery, record-schema checks/conversion, verified App
  bot trust and installation credentials at its GitHub boundary; reports derived
  from that record. The GitHub boundary's credential interface is shared through
  CLI wiring with runner and repository consumers, not a reverse core dependency.
- **Does not own:** workflow definitions and next-action derivation, execution
  and integration decisions, runner scheduling and dispatch, runtime thread
  status and conversation storage, or documentation publication. Those
  capabilities consume or supply this context's evidence through their own
  boundaries.
- **Published contracts:** the `instrukt todo` record and artifact surfaces,
  `instrukt todo update <slug> --unclaim` for releasing the execution claim,
  `instrukt todo deliver <slug> --commit <sha>`, `instrukt todo stamp <slug> --build-started`,
  `--finalize-ready`, verdict stamps and `--prd-approved` / `--task-approved`
  with `--by human|peer [--session <runtime:id>]`; evidence read capabilities for workflow, and
  `instrukt eval report`. Workflow-driving subcommands belong to the workflow
  contract even though they share the todo namespace.
- **Allowed dependencies:** configuration supplies project settings; publication supplies
  published eval provenance; workflow and runner record evidence through
  work’s mutation contract; the configured runtime supplies session and conversation references;
  GitHub supplies the required reflected store; the runner supplies the event-log
  append the finalize-ready stamp uses. Standing-agent event delivery
  is optional and is not the work record.
- **Implementation roots:** the `instrukt-todos` crate.

Preparation head reviews belong to the todo record: exact clean head, complete findings digest, independent reviewer and immediate integration event. One-off work retains its reviewed-head report for the integrator; no direct-main notes mechanism exists.

## Canonical fields

### Assignment and GitHub admission

Work supplies its recorded todo-role attempt, native session and workspace
facts to command admission; the capability contract is
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. Role grants
supply focus and every caller still meets the object's claim, phase and admitted
workspace-writer checks. The record's GitHub schema is checked on every read
before dependent mutation under
`project/instrukt/spec/feature/work/github-record`. Configuration supplies App
profiles and per-role selection; work's shared GitHub boundary verifies actors
and issues installation credentials under
`project/instrukt/spec/feature/work/github-apps`.

### Aggregates

#### Todo

- **Root entity:** the todo identified by durable identity; the slug is its
  project-local address.
- **Contains:** recorded intent and version, kind, retained artifact versions,
  evidence-bound stamps, approval and intent-check attribution, hold audit,
  completion evidence, per-role attempt bindings (runtime, native id, semantic key, captured and completed
  heads) and separate preparation/work orchestrator workspace paths retained
  through their corresponding integration and cleanup, task
  schedule (`at`/`cron`, `catch_up`), optional agent/model preference consulted
  before the role mapping in `~/.instrukt/agents.yaml`, and reflected revision.
- **Invariants:** every change, creation included, commits in one
  transaction of the state database; identity survives rename and
  recovery; workflow facts bind the evidence they concern. The detailed
  record lifecycle belongs to
  `project/instrukt/spec/feature/work/todos`, evidence validity to
  `project/instrukt/spec/feature/work/stamps`, and reflection to
  `project/instrukt/spec/feature/work/github-record`.
- **Identifier:** durable identity, scoped to a project, with an unambiguous
  live slug.
- **Lifecycle:** created into active work or the icebox; retained through
  preparation and execution; delivered read-only, withdrawn with recovery, or
  converted into a structural holder before preparation completes.

#### Project work graph

- **Root entity:** a project's set of dependency and group relationships.
- **Contains:** prerequisite edges, structural holders and child memberships.
- **Invariants:** relationships remain project-local and acyclic; grouping and
  dependency checks agree on holder ancestry. Readiness and holder semantics
  belong to `project/instrukt/spec/feature/work/dependencies-and-holds`.
- **Identifier:** project identity; relationships address durable todo identities.
- **Lifecycle:** grows and changes as work is filed, grouped, reordered and
  retired; it preserves the context needed to read retained work.

### Domain events emitted

Stamp history is durable work evidence, not an event bus. Workflow derives
state from the record and owns lifecycle signals such as finalize readiness.
The work finalize-ready stamp appends workflow’s `workflow.finalized`, and
independent preparation-head approval appends `workflow.preparation_reviewed`,
through the runner’s event-log interface. These atomic work mutations retain
the corresponding reviewed head; workflow owns the lifecycle vocabulary.

### Domain events consumed

Work consumes no events in the first build. GitHub changes reconcile into
local records and workflow results enter as evidence-bound mutations.

## Event subjects and origins

Work owns no event vocabulary. Its readiness and preparation-head review stamps
append workflow's `workflow.finalized` and `workflow.preparation_reviewed`,
concerning the todo in its project origin
(`project/instrukt/spec/domain/workflow`).

## Use cases

- **known-issue-suppression** — [project/instrukt/spec/feature/work/known-issue-suppression](../feature/work/known-issue-suppression.md) — retained triage prevents repeated log-hunter filings.

- **Todos** — `project/instrukt/spec/feature/work/todos` — record work
  with its kind and recover withdrawn intent and evidence.
- **GitHub Apps** — [project/instrukt/spec/feature/work/github-apps](../feature/work/github-apps.md) — role-selected project/author/reviewer bots, installation credentials, bot trust and visible bot authorship.
- **GitHub record** — `project/instrukt/spec/feature/work/github-record`
  — retain local progress through outages and follow reflected delivery evidence.
- **Stamps** — `project/instrukt/spec/feature/work/stamps` — retain
  attributed evidence for approval, workflow and evaluation goals.
- **Approvals** — `project/instrukt/spec/feature/work/approvals` —
  authorize reviewed outcomes and hand off checked intent.
- **State database** — `project/instrukt/spec/feature/work/state-database`
  — read the local work record while instrukt writes and keep the last
  completed state through an interrupted write.
- **Dependencies and holds** —
  `project/instrukt/spec/feature/work/dependencies-and-holds` — sequence
  dependent work and pause pickup with an inspectable reason.
- **Evaluation report** — `project/instrukt/spec/feature/work/eval-report`
  — compare delivery outcomes across process and model changes.

## Governing designs and decisions

The feature contracts above carry this context's settled design. No separate
owning design snippet or decision record is declared.

- `project/instrukt/design/architecture/overview` — the module boundaries and shared publication, retrieval and integration flows.

## Context map position

- **Configuration → work** — customer-supplier. Work consumes project settings.
- **Publication → work** — customer-supplier. Work consumes the fingerprint
  recorded per published commit.
- **Work → workflow** — customer-supplier. Work supplies evidence and relationships;
  workflow interprets them into preparation, execution and integration decisions.
- **Work → runner** — customer-supplier. The runner consumes work readiness and
  records claims and their reflection, flushes pending reflection, writes last-fired
  stamps and automatic holds, purges expired retained records, and updates
  per-role session bindings through work’s command contract.
- **Runner → work** — customer-supplier. The finalize-ready stamp appends
  `workflow.finalized` through the runner's event-log interface, in the
  stamp's own transaction.
- **Work → runner (state database)** — customer-supplier. The runner's tables
  live in work's state database, under its writer lock and migrations; the
  runner owns their meaning and is their only writer.
- **Work → docs** — customer-supplier. Work supplies pending todo-slug existence
  through the core-declared interface wired by the CLI.
- **Work → repository** — customer-supplier. Work supplies todo existence
  through the core-declared interface wired by the CLI when todos is enabled.

- **Runner** — its runtime anti-corruption layer supplies session and conversation
  references; work stores these without translating native runtime mechanics.
- **GitHub** — anti-corruption-layer. Reflection translates work records into
  provider artifacts and issues while local operations keep their domain model.
