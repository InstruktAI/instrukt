---
eval: true
description: 'Prepare-stage orchestration. Audit intent, enter the kind-specific preparation lane, and drive its flavor-defined derivation and review cycle to readiness or a recorded blocker.'
activation_trigger: 'Use when asked to prepare a todo, become its prepare orchestrator, or drive instrukt todo prepare for a slug.'
visibility: 'public'
role: prepare-orchestrator
parameters:
  - name: slug
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: slow
---

# Prepare Overview — Procedure

## Required reads

- @~/.instrukt/docs/general/principle/session-lifecycle.md
- @~/.instrukt/docs/general/procedure/orchestration.md
- @~/.instrukt/docs/general/concept/agent-characteristics.md
- @~/.instrukt/docs/general/policy/commits.md
- @~/.instrukt/docs/sdlc/policy/process-change-authority.md
- @~/.instrukt/docs/general/policy/human-involvement-recording.md
- @~/.instrukt/docs/sdlc/policy/todo-creation-discipline.md
- @~/.instrukt/docs/sdlc/policy/peer-invocation.md

## Goal

Route a todo into the preparation lane defined by its kind and drive that lane
from initial intent capture to its ready state. Feature, change, and bug work
use the prepare flow; task work routes to its non-SDLC task-plan procedure
within that flow. One session retains ownership while each routed phase loads
its focused procedure and records the evidence for the next derivation.

### Defaults — read these once and carry them through every phase

1. **One session drives the whole prepare lifecycle.** The session you are
   in loads each phase's procedure when the selected action routes there,
   does the work itself, and calls the flow again.
2. **Each phase has its own focus, sharpened by a hat.** The flavor's action
   and rendered template name the phase's procedure and hat — BA for intake, Architect for architecture,
   Planner for the plan. The hat sharpens focus for that phase on top of your
   lifecycle awareness and comes off when the next procedure loads.
3. **Reviewers are independently dispatched.** A review step runs in a separate
   session through `instrukt sessions run`, or continues through
   `instrukt sessions send` after the recorded binding is observed with
   `instrukt sessions alive`; you follow it under the orchestration
   loop (`general/procedure/orchestration`), and you own whether another review
   pass is due (`sdlc/procedure/lifecycle/review`). Escalation follows
   `general/policy/human-contract`, never the review count.
4. **PRD approval belongs to the feature lane only.** A drafted feature PRD
   first passes `prd_review`; the configured human or peer approval moment
   then judges product intent. Architecture and implementation plans advance
   on reviewer verdict alone. Task authorization is its own configured gate.
5. **You alone invoke a peer** for a design judgment still uncertain after
   grounding (`sdlc/policy/peer-invocation`). Every session you dispatch reports
   to you through its own output and artifacts.

The bug lane is the one exception to the first default: a dispatched peer
authors the `diagnosis` while this session triages before it and examines the
written mechanism after it, or settles a reporter's confirmed causal account at
triage without a peer (`sdlc/procedure/lifecycle/prepare/diagnose`).

## Preconditions

- Slug exists in the todo list, or is created during the session at the moment
  intent is concrete enough.
- Session was invoked via `sdlc/procedure/lifecycle/prepare/overview [slug]`, or
  auto-spawned to work a todo already on the list.
- Either a human is in the conversation or none is. The session reads that from
  the conversation it is having; nothing declares it and nothing carries it in.

## Steps

Use `instrukt <command> --help` for individual commands.

### 0. Resolve the entry mode

The slug's presence and the human's presence determine the posture:

- **No slug + a human in the conversation.** The conversation itself is the
  source for `@input`. Shape a concrete todo with the human, and once the intent
  is concrete, face the creation admission questions and create it iceboxed:
  `instrukt todo create <slug> <content> --kind <kind> --approved --icebox`.
  The icebox keeps the runner from claiming the new todo before you continue;
  unfreeze it with `instrukt todo unfreeze <slug>` once the todo is ready to
  enter its normal lane. Once the filing's outcome reports the todo created,
  continue as its prepare orchestrator from Step 0a.
- **Slug + a human in the conversation.** The human is reassessing or refining
  that todo. Assess the conversation and the human's intent for the slug before
  calling `instrukt todo prepare {slug}`, then proceed from Step 0a.
- **Slug + no human in the conversation.** Proceed directly from the artifacts
  through the prepare flow. A human who joins later governs only the
  acts from that point on and rewrites no involvement record already stamped.

### 0a. Audit the creation record and input

Perform the entry audit under `sdlc/policy/todo-creation-discipline` and carry
any input clarification into Step 2.

1. Read the creation record and current `input`, the intent record for every
   kind. Check whether an intent-checked stamp binds this version.
2. A current check travels unchanged. If this version has no current check
   and you are its first recipient other than its author, read its originating
   conversation through `instrukt sessions conversation <session-id>` and
   compare the outcome-level substance. Confirm the record, clarify its
   expression without changing its meaning, or return a mistaken interpretation
   to the human in this thread. Record the checked input version, conversation
   and checker with `instrukt todo stamp {slug} --intent-checked`.
   The check runs where the conversation is readable; a conversation is not
   copied into the work record. An author does not check their own input.
3. Derive the work from that basis by kind — the outcome and constraints for
   `feature` and `change`; the reported concern, symptom, and
   expected-versus-observed behavior for `bug`, whose causal and scope claims
   remain hypotheses diagnosis tests; and the action boundary, result,
   recipient, and timing for `task`. A technical choice needed for safe and
   correct delivery stays plan and build's to make.

A task performs the same audit before entering its task-specific lane.
Changing `input` invalidates the check; changing a derived artifact does not.

### 1. Pre-flight input viability

Read the input via `instrukt todo artifacts read {slug} input` and decide whether
derivation can proceed at all.

- **Hard-stop signals** (any one disqualifies the input as-is):
  - a single short phrase or noun phrase with no goal, no actor, no outcome —
    fewer than ~10 meaningful words;
  - a slug carrying a raw timestamp suffix (e.g., `…-1778958052`) with no
    companion context from a prior session;
  - empty input, placeholder text, or a known smoke-test pattern.
- **No human in the conversation** → stop the prepare flow before any
  derivation. Surface the gap by referencing `@input` (never pasting it) and
  ask for the missing intent in this session's thread, leaving it waiting
  for input. Removal (`instrukt todo remove {slug}`) still follows the
  human-owned withdrawal boundary.
- **A human in the conversation** → point the human at `@input`, name the gap,
  and ask one direct question to surface the goal, actor, and outcome. Assess
  the elaborated input for viability, then carry the input update into Step 2.

Substantive input — clear intent, concrete outcome, identifiable actor, even if
short — passes and proceeds to Step 1a.

### 1a. Process-change governance gate

Before artifact derivation, assess under `sdlc/policy/process-change-authority`
whether delivery changes a process-layer contract. When it does, identify the
pending process-contract changes here and author them on `main` in Step 2,
before planning. When an underlying consequence belongs to the human, surface
only that outcome (`general/policy/human-contract`) and absorb the decision into
the owning artifact.

### 1b. Route by todo kind

Read the todo's recorded kind and enter only the lane it owns:

| Kind      | Entry route                                                                                                                      | Preparation outputs                                                                                  |
| --------- | -------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `feature` | Intake → PRD review and configured approval → optional architecture and review → implementation plan and review                       | `prd`, behavioral feature-spec reflection, optional `architecture`, `implementation-plan`            |
| `change`  | Implementation-plan drafting and review                                                                                          | `implementation-plan`; no PRD or feature intake                                                      |
| `bug`     | Diagnosis, by a dispatched peer or settled at triage from a confirmed reporter account → implementation-plan drafting and review | `diagnosis`, `implementation-plan`; no PRD or feature intake                                         |
| `task`    | Load `sdlc/procedure/lifecycle/task/prepare-task` within the prepare flow                                                       | `task-plan` plus configured authorization; no PRD, architecture, implementation plan, or reviewer verdict |

When the recorded kind does not match the intent, reclassify it with
`instrukt todo update {slug} --kind <feature|change|bug|task>` before deriving
artifacts.

The kind decides the lane, not whether architecture review happens: a plan that
makes a structural decision no reviewed architecture carries — in a `change` or
`bug`, or in a `feature` whose architecture was skipped — carries that review
inside its plan phase (`sdlc/policy/preparation-artifact-quality`, Structural
decisions carry architecture review).

A `bug` keeps its lane when a current contract states the behavior its report
expected (`sdlc/policy/todo-creation-discipline`): open the contract the report
states and confirm it says that. A report that names none, about behavior a
current contract plainly states, gets that contract added to `input` and keeps
its lane. Whether the evidence shows the claimed behavior is diagnose triage's
judgment. A report whose premise is that nothing followed up is first checked
against the policy's platform-gap rule.

When no current contract states the expected behavior, the report is one of two
things:

- **A change request** — it names an outcome someone wants. Reclassify it to
  `change` before calling `instrukt todo prepare {slug}`.
- **An invalid bug report** — behavior its contract permits, asking for no
  outcome anyone wants. Record the reason in the todo's `log`. An
  agent-originated report is removed with `instrukt todo remove {slug}`, ending the
  session. A report carrying a human's request in their own words is that
  human's to withdraw: park it with
  `instrukt todo update {slug} --hold --reason <text>`. Record in `log` why it
  breaks no contract, and keep the human conversation open (`general/principle/session-lifecycle`). When they withdraw
  it, remove it and end. When they restate it as a change, record the
  restatement in `input`, release the hold with
  `instrukt todo update {slug} --release`, and reclassify it to `change` before
  calling `instrukt todo prepare {slug}`.

### 1c. Settled change

At audited entry, before drafting or review starts, a `change` that meets the
in-context criteria of `sdlc/policy/todo-creation-discipline` may be executed
here. It requires an explicit `doc_impact_none` basis, no pending documentation
or uncommitted delivery fold for the slug, and no prior build work or unresolved
review to bypass. A `bug` never takes this path.

Execute on canonical `main`, commit, and resolve independent review under
`general/policy/commits`. Record the eligibility basis, delivery commits, review
reference, and remaining publication obligation in the todo's `log`.

The integrator retains publication through the integration gate. Keep the
reviewed commits and delivery obligation in the record while publication is
pending; a local commit is not delivery. The integrator supplies the actual
delivered commit to `instrukt todo deliver <slug> --commit <sha>` only after candidate verification
and canonical-branch promotion. Release the preparation lane once preparation
is complete and the reviewed commits and delivery obligation are recorded;
do not wait for delivery. Cleanup must discard no slug-owned work or pending
material and commit nothing new.

If eligibility fails before execution, enter the ordinary change lane. If a
later finding requires preparation, retain the commits and unresolved evidence
and route the remaining work through normal recovery, leaving the todo
undelivered.

### 2. Call the prepare flow

For every kind, call `instrukt todo prepare {slug}` once the entry assessment
establishes settled, viable intent and before deriving preparation artifacts.
Entry claims, holds and lane ownership follow the work and runner contracts;
work returns to preparation through the work orchestrator's kickback or
substantive-drift return, preserving existing work.

Each call returns the derived state, selected transition, action, rendered
template and owed effects. The flavor owns the flow at
`workflow/prepare.yaml`, the command associated with each action in
`workflow/actions.yaml`, and the wording in `workflow/templates/<action>.md`.
Apply the input clarifications and process-contract changes identified during
preflight under the current obligation. Execute the selected action and its
rendered instructions; execute owed effects in listed order and inspect their
outcomes. Effects are idempotent and the next call derives what remains owed
from the record. Evaluation alone completes no work.

### 3. Do each routed phase's work

Load the procedure the selected action names and do its focused work. Dispatch
independent reviews through the sessions facade, reusing the recorded session
per todo and role after `instrukt sessions alive`. Read conversations through
`instrukt sessions conversation`; questions for the human stay in the asking
session's thread. Observe completion in the work record: the required artifact
and stamp must both be present and current. A successful ended turn alone is
not completion. Session recovery follows the runner's session-continuity
contract, including one send to the same session when evidence is missing and
an anomaly if it remains missing.

Call `instrukt todo prepare {slug}` again after the required evidence is
recorded. The returned action, rather than an output token or dispatch header,
determines the next obligation.

### 4. Honor the feature PRD approval gate

In the feature lane, internal PRD review precedes its configured approval
moment. When intake writes or rewrites the PRD, the selected action routes to
an independent PRD reviewer. `needs_work` returns the
findings to intake; `approve` binds the verdict to the current PRD digest and
opens the configured approval moment.

Present only the reviewed PRD, its stated horizon, and genuine human-owned
decisions. At a human gate, the responsible session asks in its thread. At a
peer gate, a separate peer judges the reviewed outcome against checked intent
and records its reasoning; money, public statements in the human's name,
granting access and legal commitments return to the human in that thread.
Record approval with `instrukt todo stamp {slug} --prd-approved --by human|peer
[--session <runtime:id>]` under the work approval contract; GitHub approvals arrive by pull:
GitHub decisions retain the observed GitHub identity, session decisions retain
the runtime and session id, and peer decisions are explicitly peer approvals.
On refinement, capture the delta, rewrite the PRD, and rerun `prd_review`;
the rewritten PRD returns to the applicable approval moment only when approval
does not carry.

Every recorded approval, human or peer, approves intent, not a digest. Once
a PRD has approval, a later revision returns to the configured approval gate
only when it carries intent not already decided. Every other revision carries
that approval. The independent
PRD reviewer judges whether it does, in its report on the revised digest
(`sdlc/procedure/lifecycle/prepare/review-prd`); the claimant never classifies
its own revision. When the current `prd_review` approves and its report
concludes that the approval carries, the prepare claimant records `--prd-approved`
without asking. A revision carries the approval when each of its changes is
one of these:

- a decision the human gave in the live conversation, applied as given;
- removal or narrowing of scope that the `input` does not carry and the human
  never asked for;
- a correction of reasoning, evidence, wording, or mechanism that leaves every
  outcome and success criterion as previously approved.

For the first ground, the claimant records the human's direction and the
session that carries it in the todo `log` before the review is dispatched, so
the reviewer judges it against the source.

A revision that adds or widens an outcome or changes a success criterion
beyond recorded direction returns to the configured approval gate, as does
one whose classification remains uncertain. A new decision in the human's
closed lane (`general/policy/human-contract`) returns to the human. A carried approval never skips
`prd_review`. Before stamping, the claimant records in the todo `log` the
approved digest it carries forward and the review report that carried it. The
human is not asked to confirm a carried approval and receives no separate
notice of it.


A first approval is obtained at the configured gate, human by default or
peer when configured. Session startup supplies no approval. Carry retains
the earlier approval's attribution and evidence and the current reviewer's
target binding. A carried peer approval stays a peer approval; the recorder
is not a new approver.

### 5. Re-read against changed doctrine

The fact provider detects code drift and the selected action dispatches its
analysis. Doctrine drift requires agent assessment: on resuming a todo whose
artifacts were reviewed before a change to a principle, policy, or procedure
they are judged against,
re-read each artifact against the doctrine changed since its last review. Only
a material conflict returns the affected derivation layer for re-review;
doctrine drift alone invalidates no approval.

### 6. Handle refinement

When the human refines an artifact at any layer, classify the refinement scope
and re-enter the matching phase:

- **Intent-level** (new thinking, scope changes) → repeat kind routing. A
  feature re-enters intake; a change re-enters plan; a bug re-enters diagnosis
  or plan according to whether the causal conclusion changed.
- **Architecture-level** (design-snippet change in a feature) → re-enter
  architecture.
- **Plan-level** (task adjustment) → re-enter the plan phase.

Record the input or artifact revision and re-run the affected phases.
Workflow derives which evidence bindings are stale; retain earlier stamps as
history. A new input version requires the next non-author's intent check.
A substantial human refinement of an autonomously produced artifact is
authoritative for that refinement, and downstream re-derives from it even
when downstream was already produced.

### 7. Terminal state

Preparation is complete once the selected kind's required derivations,
artifact-and-stamp evidence, current reviews, applicable approvals and drift
settlement are present. Every commit authored on canonical `main` under the
preparation claim also needs an `approve` note with its independent report
binding, or an author's `exempt` note with reason under
`general/policy/commits`. A `needs_work` or missing note remains unsettled;
`claim_commits.unreviewed` includes every commit with neither settled verdict. The returned
obligation names any remaining work.

Run `instrukt todo update <slug> --unclaim` and confirm release before leaving
the lane. End completed lane and child sessions with `instrukt sessions end <id>`
after confirmed release. The runner owns later execution dispatch; a derived ready state
alone releases no claim and ends no session. Report readiness to the human.
An unfinished human conversation stays in this thread after the lane ends.

When review continuation stops or drift cannot be resolved, preserve the
findings and current verdict and record the grounded cause, next contribution
and owner. Follow `sdlc/procedure/lifecycle/review` for review halts and
`general/policy/human-contract` for ownership. A blocked obligation does not
itself make recovery the human's decision.

Confirm `claim.held` is false after `instrukt todo update <slug> --unclaim`,
then end each assignment child from `sessions.bound` with
`instrukt sessions end <id>` before ending the completed lane.

## Outputs

- Every lane begins from the `input` as its authorized basis.
- Feature: digest-bound `prd_review`, approved `prd`, behavioral feature
  spec reflection, optional reviewer-approved `architecture`, and a
  reviewer-approved `implementation-plan`.
- Change: reviewer-approved `implementation-plan`, with no PRD or feature
  intake artifacts.
- Bug: a `diagnosis` authored by a dispatched peer, or transcribed from a
  confirmed reporter account, followed by a reviewer-approved
  `implementation-plan`, with no PRD or feature intake artifacts.
- Task: a `task-plan` and configured authorization from the task procedure;
  no PRD, architecture, implementation plan or separate reviewer verdict.
- Lanes carry current evidence and a derived ready or blocked obligation;
  completion and claim release are confirmed from the record and effect outcomes.

## Recovery

- **Creation record is absent or incomplete.** Derive from the input and prior
  artifacts; never solicit an attestation from the creator. Reading a named
  session's transcript under Step 0a recovers meaning and is not solicitation.
- **The originating conversation cannot be read for an owed intent check.**
  Preserve the gap; no intent-checked stamp is invented. The runner dispatches
  the check where that conversation is readable. If the originating conversation is
  permanently unavailable, return the intent to the human in the responsible thread; a
  new `input` version from that conversation becomes the checkable basis. Corroborative
  transcript gaps
  in an otherwise reviewable target follow the shared review contract.
- **The input is too thin to derive from after the audit.** Stop before
  kind-specific derivation and surface the gap; do not manufacture product
  detail.
- **A bug concern is faithfully recovered but not yet proven.** Proceed to
  diagnosis. Filing fidelity establishes what was reported, not whether the
  report is correct or what caused it.
- **The returned action is unfamiliar or unresolved.** Read its governing
  contract and route any unresolved technical discrepancy to its agent owner
  under `general/policy/human-contract`; do not improvise a state transition.
- **Procedure loading fails.** Report the failure; do not proceed without the
  procedure loaded.
- **A preparation write was refused before entry.** Complete Step 2 and retry
  the in-scope write against the current state. If it remains refused, inspect
  the current record, claim and failed command outcome while retaining the
  session and claim, and ground recovery in that fresh evidence
  (`general/principle/grounding`).
- **Reviewer dispatch fails repeatedly.** Preserve the failure evidence and
  apply the orchestration recovery (`general/procedure/orchestration`). The
  attempts do not substitute for reviewer verification or establish human
  ownership.
- **Human disengages mid-flow.** The record retains artifacts, stamps and
  session bindings; the next call derives the remaining obligation.

## See Also

- ~/.instrukt/docs/sdlc/procedure/lifecycle/prepare/intake.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/prepare/architecture.md
- ~/.instrukt/docs/sdlc/procedure/lifecycle/prepare/plan-draft.md
