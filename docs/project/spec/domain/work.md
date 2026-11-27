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
: The item-level claim recorded in this computer's work record before work
  starts — by the runner on automatic dispatch, or by `instrukt todo work` on
  deliberate pickup — exposed as `claim.held`; distinct from per-todo, per-role session bindings.

**Session binding**
: The started session’s runtime and id, recorded per todo and role, distinct
  from the execution claim and reused by the runner.

**Task schedule**
: A task’s one-off `at` or recurring `cron` schedule and `catch_up` policy.

**Dispatch hold**
: An attributed pause on automatic pickup, carrying its cause.

**Reflection**
: The one-way showing of a todo on GitHub — its issue, labels, board card,
  artifacts, work branch and pull request — from the work record, which holds
  the only copy of the todo. Nothing reflected is read back as work state.

**People's control**
: An approval, a change request or a hold a person exercises on a todo's
  issue, entering the record as that person's attributed change, once.

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
  holds; per-role session bindings and task schedules; the work record's only
  copy, in the state database — its file, writer rules, migrations and work
  tables (`project/instrukt/spec/feature/work/state-database`); reflection of
  each todo to GitHub, including publishing a repository todo's work branch
  while it is built and reviewed and the pull request and reviews on it;
  people's controls read from a todo's issue; verified App actors and
  installation credentials at its GitHub boundary; reports derived from that
  record. The GitHub boundary's credential interface is shared through CLI
  wiring with runner and repository consumers, not a reverse core dependency.
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
  GitHub is the required surface reflection writes to and people's controls
  are read from, and holds no copy of the record; the project's git remote
  receives the published work branch; the runner supplies the event-log
  append the finalize-ready stamp uses, and the fleet-attention contract its
  status, hold and attempt changes call. Standing-agent event delivery
  is optional and is not the work record.
- **Implementation roots:** the `instrukt-todos` crate.

The GitHub boundary also answers for adding a company
(`project/instrukt/spec/feature/configuration/company-homes`, Generation).
It tells whether a login names an organisation, a personal account or
nothing, from what GitHub shows of it publicly. Under the project identity's
installation on the owning organisation it creates a private repository
there, answering whether GitHub refused because the repository exists, makes
a named account an administrator of it and sets its default branch. Its credential is stated in
`project/instrukt/spec/feature/work/github-apps` (Company repositories).
These are translations of GitHub's model and nothing more: which
repository, when and for whom are configuration's decisions, and the sequence
around them is repository's company home adoption. Configuration and
repository reach them through interfaces core declares and the CLI wires, so
core imports nothing of this crate.

For a company's knowledge proposals the GitHub boundary also answers whether
any pull request of the repository, open or closed, has a commit as its head,
and opens a pull request for a branch into the canonical branch under the
proposal pull-request credential, the identity the `company-agent` role
selects (`project/instrukt/spec/feature/work/github-apps`, Company
repositories).

- **Idempotency:** it opens one only after answering that no open pull
  request of that branch exists. A crash between the two leaves GitHub as the
  record, and a repeated ask opens no second one.
- **Content:** the title is fixed wording naming the branch. The body states
  in fixed words that the change was asked of the company's agent, followed
  by the commits' messages inside a fence longer than any run of backticks in
  them, so nothing in them mentions anyone or renders as markup.
- **Credential:** a readiness of its own for a company repository, apart
  from automatic review's, verified by requesting exactly its permissions
  (`project/instrukt/spec/feature/work/github-apps`, Company repositories).

When to open one, under which role's identity, and whether a closed pull
request rules a head out, are the runner's decisions.

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
workspace-writer checks. Admission reads this computer's record and waits on
nothing from GitHub. Configuration supplies App profiles and per-role
selection; work's shared GitHub boundary verifies actors and issues
installation credentials under
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

Every change work makes on this computer to a todo's status, hold or attempts
calls the runner's fleet-attention contract inside its own transaction, with
the facts of that change: the todo, its hold before and after, whether it left
active work, a failed attempt with that role's earlier failed attempts, and
whether the project still has an active todo that is unheld and not a
structural holder. Which of those facts are conditions, and their events, are
the runner's (`project/instrukt/spec/feature/runner/fleet-attention`).

### Domain events consumed

Work consumes no events in the first build. People's controls on a todo's
issue enter the record as attributed mutations, each applied once; nothing
else on GitHub changes the record. Workflow results enter as evidence-bound
mutations.

## Event subjects and origins

Work owns no event vocabulary. Its finalize-ready stamp appends workflow's
`workflow.finalized`, concerning the todo in its project origin
(`project/instrukt/spec/domain/workflow`); the fleet-attention events its
record changes lead to are the runner's (`project/instrukt/spec/domain/runner`).

## Use cases

- **known-issue-suppression** — [project/instrukt/spec/feature/work/known-issue-suppression](../feature/work/known-issue-suppression.md) — retained triage prevents repeated log-hunter filings.

- **Todos** — `project/instrukt/spec/feature/work/todos` — record work
  with its kind and recover withdrawn intent and evidence.
- **GitHub Apps** — [project/instrukt/spec/feature/work/github-apps](../feature/work/github-apps.md) — role-selected project/author/reviewer bots, installation credentials, bot trust and visible bot authorship.
- **GitHub record** — `project/instrukt/spec/feature/work/github-record`
  — retain local progress through outages, discuss and decide on a todo's
  issue, follow the live pull request and read its reviews, and follow
  reflected delivery evidence.
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
  `workflow.finalized` through the runner's event-log interface, and every
  change to a todo's status, hold or attempts calls the runner's
  fleet-attention contract, each in its record change's own transaction.
- **Work → runner (fleet attention)** — customer-supplier. The runner reads a
  project's active todos with their status and holds when it admits a raise and
  when one of its own observations opens an attention episode.
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


<!-- planned-change:commit-rereview-reuses-reviewer -->
- **Work → runner (requested review)** — customer-supplier. Work answers
  whether a commit's head verdict stands here, with the session that recorded
  its current version, or was handed off, with the export that handed it off,
  and which pinned base it admits the commit's verdict against; the runner
  consults this when it admits a requested review
  (`project/instrukt/spec/feature/runner/requested-review`). Work admits the
  requested reviewer's verdict under its own rules.
<!-- change:commit-rereview-reuses-reviewer -->
- **Work → runner (requested review)** — customer-supplier. Work answers
  whether a commit's head verdict stands here, with the session that recorded
  its current version, or was handed off, with the export that handed it off,
  which pinned base it admits the commit's verdict against, and, for that
  project, which commit's standing verdict has a version with a given report
  digest; the runner consults this when it admits a requested review
  (`project/instrukt/spec/feature/runner/requested-review`). Work admits the
  requested reviewer's verdict under its own rules.
- **Runner → work (requested review)** — customer-supplier. Once
  `instrukt review record` has recorded a version, work hands that version to
  the runner's verdict-notice queuing
  (`project/instrukt/spec/feature/runner/requested-review`).
<!-- /planned-change:commit-rereview-reuses-reviewer -->
- **Work → runner (knowledge proposals)** — customer-supplier. Through its
  GitHub boundary work answers the readiness of the proposal pull-request
  credential and whether any pull request has a commit as its head, and opens
  a proposal's pull request under that credential; the runner decides when.


- **Work → workflow (review coverage)** — customer-supplier. Work answers which
  evidence is current for a commit and whether a candidate's derived
  approval is one GitHub does not show yet;
  workflow reads approval of a head that is not a todo's, and exemptions, from
  that answer alone, and reads a candidate's result from it together with the
  review status.

- **Runner** — its runtime anti-corruption layer supplies session and conversation
  references; work stores these without translating native runtime mechanics.
- **GitHub** — anti-corruption-layer. Reflection translates work records one
  way into issues, labels, board items, native artifacts, the published work
  branch, its pull request and reviews; the boundary translates people's
  controls on an issue into attributed record mutations. Local operations keep
  their domain model.

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
- **Work → runner (thread retention)** — customer-supplier. Work answers which
  native sessions it associates with a project's todos that have not reached a
  terminal outcome, delivered or withdrawn: the sessions its session bindings
  hold, whatever the state of each role attempt, and the session holding each
  such todo's execution claim. It answers from its local record, with no
  provider call and no write, whether or not the project's code is on GitHub;
  a project with no local record has no such sessions. The runner keeps those
  threads (`project/instrukt/spec/feature/runner/thread-retention`). Work owns
  which outcomes are terminal; the runner owns what it keeps.
