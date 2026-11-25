---
description: Durable project work, evidence, approvals, dependencies and evaluation within the todos module.
visibility: public
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

**Verification disposition**
: A stamp recording what is established about a candidate commit's live
  behavior — pass, fail, blocked, Tier 0 skip or hotfix deferral — bound to
  that commit and its `verification-findings` report; workflow owns who may
  record it and what handoff requires of it.

**Prerequisite**
: Another project todo whose evidence or delivery gates a dependent's pickup.

**Structural holder**
: A group parent outside the todo lifecycle, retaining split evidence and
  grouping its children by durable identity.

**Execution claim**
: The item-level claim recorded and reflected before work starts — by the
  runner on automatic dispatch, or by `instrukt todo work` on
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

**Head verdict**
: An independent review's recorded outcome for one exact commit that no todo's
  record carries: a judgment of the commits from its pinned base up to that
  commit, delivered to GitHub as the reviewer bot's review status on it and as
  a review on each pull request it heads.

**Pinned base**
: The commit a review starts from, fixed by instrukt and never chosen by the
  reviewer. A head verdict covers the whole range from it, and covers the
  canonical branch only when its pinned base lies on that branch. A review
  request carries the pinned base the runner selected
  (`project/instrukt/spec/feature/runner/pull-request-review`).

**Exemption**
: The recorded decision that a commit owes no independent review under
  `general/policy/commits`, made by its author. It is a fact of its own beside
  a commit's verdict.

**Derived approval**
: The record that an integration candidate's content is covered, from
  workflow's finding and with no review of its own. Until GitHub shows it as
  the candidate's review status it stands only while that finding is found
  again.

**Revision**
: A later version of a commit's head verdict, recorded by the session that
  recorded the current one in a turn that no review request for that commit
  opened.

**Handed-over approval**
: A head verdict its reviewing session returned and did not record, entered by
  another session that states it is that session's standing result and adds
  no judgment of the reviewed change.

**Export**
: One snapshot of the review evidence that stands on a computer for a project,
  written to a file for another computer to import. The exported verdicts are
  handed off: they stand again where the file is imported.

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
  `--finalize-ready`, verdict stamps, verification disposition stamps
  (`--verification`) and `--prd-approved` / `--task-approved`
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

One-off work retains its reviewed-head report for the integrator; no direct-main notes mechanism exists.

Work also owns review evidence for exact commits that no todo's record
carries: head verdicts, exemptions and derived approvals, their admission,
which of them is current for a commit, their publication as the reviewer
bot's pull-request review and review status, and their transfer to another
computer. `instrukt review record`, `exempt`, `show`, `export` and `import`
are their published contract
(`project/instrukt/spec/feature/work/github-apps`). Which commits are owed a
review and which review requests exist is the runner's decision, whether a
candidate's content is covered is workflow's, and how a head is reviewed is its
review procedure's.

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
  heads) and the todo's single `<slug>/work` workspace path retained
  through integration and cleanup, task
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

#### Review evidence

- **Root entity:** the review evidence of one exact commit of a project's
  repository.
- **Contains:** its head verdict with its versions, each with verdict, pinned
  base, report, reviewing session, the request it answers and its moment, and
  with whether it stands on this computer, was handed off by an export or was
  set aside by an import; its exemption; for an integration candidate its
  derived approval with the coverage it rests on; and what GitHub shows of the
  current verdict at each destination.
- **Invariants:** evidence is recorded before anything of it is published; a
  verdict names one commit and no other commit inherits it; a verdict covers
  the whole range from a pinned base its reviewer cannot narrow; no reviewed
  commit names the verdict's reviewing session as its author; a commit's
  review is settled by the first verdict admitted for it, and only its
  settling session revises it; a verdict and an exemption of one commit never
  replace each other; each publication follows the current verdict; a
  derived approval is first published as the commit's status, by a tick that
  found its coverage again; a verdict an export carried is handed off where
  it was exported and stands where its file is imported; an import never
  changes which verdict of a commit is current where another one stands
  here, unless its operator states which stands; a current version carries
  its own report and moment and rests on no earlier one; only the reviewer
  identity publishes. Admission, publication,
  storage, transfer and readiness belong to
  `project/instrukt/spec/feature/work/github-apps`.
- **Identifier:** project and commit.
- **Lifecycle:** recorded by the reviewing session, by the commit's author for
  an exemption, or by the runner through work's command contract for a derived
  approval; published on ticks by a computer the project is admitted on;
  moved to another computer by an export and an import; kept while current and
  retired as history under the same contract.

### Domain events emitted

Stamp history is durable work evidence, not an event bus. Workflow derives
facts from the record and owns lifecycle signals such as finalize readiness.
The work finalize-ready stamp appends workflow’s `workflow.finalized`
through the runner’s event-log interface. This atomic work mutation retains
the reviewed head; workflow owns the lifecycle vocabulary.

### Domain events consumed

Work consumes no events in the first build. GitHub changes reconcile into
local records and workflow results enter as evidence-bound mutations.

## Event subjects and origins

Work owns no event vocabulary. Its finalize-ready stamp appends workflow's
`workflow.finalized`, concerning the todo in its project origin
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
- **Work → runner (state database)** — customer-supplier. The runner's
  operational tables live in work's state database, under its writer lock and
  migrations; the runner owns their meaning and is their only writer. The
  runner's history store lies outside it, with its own file, lock and
  migrations (`project/instrukt/spec/feature/runner/history`). History reaches
  the state database only to move earlier history out of it, through work's
  open, its migrations and its space reclamation.
- **Work → docs** — customer-supplier. Work supplies pending todo-slug existence
  through the core-declared interface wired by the CLI.
- **Work → repository** — customer-supplier. Work supplies todo existence
  through the core-declared interface wired by the CLI when todos is enabled.

- **Work → runner (automatic review)** — customer-supplier. Work answers whether
  a pull request is a todo's, whether a commit carries a verdict and which
  review evidence is current for it, and whether both identities are ready.
  Through its command contract it records a derived approval, carrying the
  coverage finding the runner read from workflow, withdraws one that a later
  finding no longer covers, and publishes what is outstanding to the pull
  requests the runner observed.
- **Runner → work (automatic review)** — customer-supplier. The runner supplies
  a commit's review requests with their standing and pinned base, which input
  opened the turn a session has in progress when it offers a revision, and a
  named session's conversation; work consults them when it admits a head
  verdict.


- **Work → runner (requested review)** — customer-supplier. Work answers
  whether a commit's head verdict stands here, with the session that recorded
  its current version, or was handed off, with the export that handed it off,
  and which pinned base it admits the commit's verdict against; the runner
  consults this when it admits a requested review
  (`project/instrukt/spec/feature/runner/requested-review`). Work admits the
  requested reviewer's verdict under its own rules.


- **Work → workflow (review coverage)** — customer-supplier. Work answers which
  evidence is current for a commit and whether a candidate's derived
  approval is one GitHub does not show yet;
  workflow reads approval of a head that is not a todo's, and exemptions, from
  that answer alone, and reads a candidate's result from it together with the
  review status.

- **Runner** — its runtime anti-corruption layer supplies session and conversation
  references; work stores these without translating native runtime mechanics.
- **GitHub** — anti-corruption-layer. Reflection translates work records into
  provider artifacts and issues while local operations keep their domain model.

- **Work → runner (worktree reclamation)** — customer-supplier. Through its
  command contract work gives the runner two things for
  `project/instrukt/spec/feature/runner/worktree-reclamation`. The first is a
  section of the work record in which no role attempt is open on a todo's
  worktree and none opens: the runner moves regenerable output out of the
  worktree inside it and holds it for nothing else. The second is the release
  of a delivered todo's worktree and work branch
  (`project/instrukt/spec/feature/work/todos`), which work performs when the
  runner asks. Work owns the attempt lock and the release; the runner owns
  when it asks.
