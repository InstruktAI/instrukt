---
eval: true
description: Decide whether a todo should exist, audit that decision at preparation entry, and reconcile a live violation without wasting further preparation effort.
---

# Todo Creation Discipline — Policy

## Required reads

@~/.instrukt/docs/general/policy/documentation-discipline.md
@~/.instrukt/docs/general/policy/human-contract.md

## Rules

### Progress, hold, or retire

- **Keep active work actionable.** Each active todo has an authorized outcome,
  a concrete next contribution, and an owner able to make it. Judge progress
  by changed evidence, artifacts, or outcomes, not by repeated status checks
  or renewed attempts with the same premise.
- **Hold for a specific unblock condition.** Record the condition, who owns
  satisfying it, and the evidence that releases the hold. A named dependency
  delivery, a scheduled gate, or a specific decision from its responsible
  person can qualify. Naming an owner for hypothetical future evidence does
  not make that evidence an actionable dependency.
- **Bound investigation by an obtainable next observation.** Missing evidence
  can justify a concrete, bounded acquisition step. When the available inquiry
  is exhausted and no such step or admissible unblock condition remains,
  record the findings and limits and retire the investigation. Hoping that a
  failure recurs or that useful diagnostics arrive someday is not a hold
  condition. A later occurrence is assessed afresh against its actual evidence.
- **Preserve evidence without preserving speculative work.** Retain findings
  and necessary work products through the supported retirement path. An
  exhausted investigation does not become an icebox item or new
  diagnostic project merely to remain available. Additional machinery must
  independently earn its scope against a current authorized need.
- **Keep retirement distinct from success and abandonment.** An unresolved
  item may retire as unresolved; retirement supplies no diagnosis, approval,
  or delivery verdict. Concrete review, remediation, or delivery work already
  owed keeps its owner and gates. Withdrawing a human-owned outcome follows
  `general/policy/human-contract`; lack of a technical path does not confer
  that authority on the agent.
- **Reassess a hold when its basis changes.** Resume when its condition is
  satisfied, complete work whose outcome is delivered, and retire work with
  no actionable continuation under these rules. The existing state label is
  evidence to reconcile, never a reason to preserve the state.

### Decide before creating

- **Documentation evidence precedes lifecycle entry.** Pending contracts
  that preparation consumes are authored, committed under their intended todo
  slug, and independently reviewed with required changes resolved before
  entry. Associate the commits with `instrukt repo commit --todo <slug>` under
  `general/policy/commits`. Review evidence binds the committed revision;
  the author's declaration is not an independent review.
- **Documentation impact remains agent judgment.** The agent identifies the
  required pending contracts or records an explicit `doc_impact_none` basis.
  Neither workflow derivation nor a creation command grades prose or infers
  semantic documentation impact. Iceboxed forming work acquires its required
  evidence before entering the lifecycle.

- **A todo enters the lifecycle valid; the icebox parks ideas at any stage.** A
  todo is valid when its intended outcome and every known contract that Prepare
  will consume have converged, and no pending authoring, review, or commit is
  still expected to change them. Creating a todo without `--icebox` enters it
  into the lifecycle, so it is valid at creation. An idea still forming is
  created with `--icebox` and made valid before it is unfrozen; unfreezing
  enters it into the lifecycle. Hold is a brake on valid work awaiting an
  external gate.
- **Run the bounded creation check before every `instrukt todo create`.** Read
  `instrukt todo list`, which lists the live work — the todos a new one could
  overlap with. For each plausible match, read its `input`; if whether it has
  started build affects the choice, inspect its current state too. This is
  a cheap check for an obvious existing home, not the deeper judgment about
  whether one premise yields to, supersedes, or depends on another.
- **Use what the check finds.** Do not create a todo when the work is already
  plainly covered. When a small addition belongs in an existing `change`,
  `bug`, or `feature` that has not started build, route it to that todo's live
  claimant for amendment; when the todo is dormant and unclaimed, amend it
  directly in the session that found it. The check and any amendment concern
  live work; parked intent is re-grounded when it is picked up. Do not create a
  sibling merely to bypass artifact authority. A separate todo is warranted when
  the work is independently shippable, substantial in its own right, or the
  related todo is already building or finalizing.
- **Do not infer a platform gap from missing automatic action.** When a report's
  premise is that no role automatically owned the next move, establish which
  agent observed the condition, which existing procedure governed its recovery,
  what that agent attempted, what it changed before retrying, and the exact
  sanctioned step that failed. If the deliberate agent-owned recovery path was
  never attempted, execute or route that path and create no todo. Keep three
  facts distinct: an observation that an action failed, a diagnosis that the
  recipient cannot progress, and evidence that the platform lacks a required
  capability.
- **Route cross-project work through the project that owns the changed files.**
  Reading another project to establish ownership or gather evidence does not
  transfer execution ownership to the current session. For doc-only work, create
  no todo and follow the cross-project handoff in Documentation Discipline. For
  code or mixed code-and-doc work, create the todo against the owning project
  (using `--project-root <project-path>` when invoking from elsewhere) and let
  that project's lifecycle own the change. Do not edit another project's files
  directly merely because the filesystem permits it. If ownership cannot be
  established, resolve it before dispatching or filing rather than letting the
  current working directory decide.


- **Documentation-only work creates no todo.** It executes in context under
  `general/policy/documentation-discipline`; when a snippet enters
  consideration, `general/procedure/doc-snippet-authoring` owns the authoring
  gate.


- **Execute settled code work in context.** When the approach is known,
  the edit is bounded, and the work needs neither a plan, a builder, nor a
  handoff, make the change on `main` only when an explicit execution request, a
  bounded brief, or a completed human conversation in which every active
  participant agreed the outcome and the human released the edit supplies
  authority. Human presence alone supplies none. Importance does not decide
  this boundary; remaining judgment does. A small change whose correctness is
  genuinely contested still belongs in the lifecycle. A `change` todo created
  despite meeting these criteria may still take the in-context path at prepare
  entry (`sdlc/procedure/lifecycle/prepare/overview`).

- **Use the todo's `log` artifact for a breadcrumb.** A minimal FYI that
  changes neither scope nor a decision does not justify a new todo or a rewrite
  of `input`, `prd`, or `implementation-plan`.
- **Process-layer work follows ordinary agent-owned intake.** A proposed change
  to lifecycle state, gate composition, review lanes, or todo semantics may be
  filed from authorized intent or from applicable principles applied to
  verified substrate. The process contract is authored doc-first during
  preparation and independently reviewed. Escalate only an underlying
  consequence that `general/policy/human-contract` assigns to the human; never
  send the process mechanism itself for human approval.
- **Declare genuine human involvement only on the filing act.** An agent filing
  while a human is present in the conversation uses the creation surface's
  involvement declaration. The recorded value is resolved from the human's
  observed presence, so a declaration made after they have gone records the same
  agent-initiated value that omitting it records, and the filing proceeds either
  way. A session binding, parent todo, or preceding act never supplies the
  filing's value. Use `--human-involved true|false` on `instrukt todo create`; the declaration
  is audit-only and supplies no authorization.

- **The creation check belongs to the filing agent.** Face the decisions
  above before `instrukt todo create <slug> <content> --kind <kind> --approved`.
  Without `--approved`, the command prints the admission questions and creates
  nothing. Pass it only after facing those questions.
  Creation writes the item and its first `input` version together in the
  local record. The runner dispatches eligible work under module enablement,
  pickup policy and the workflow's current obligation; creation supplies no
  approval or completed preparation.

- **A filing exists once its command confirms local creation.** Inspect the
  command outcome before preparing it, writing artifacts or releasing a hold.
  GitHub reflection follows the work-record contract: reflect saved record
  changes and reconcile remote revisions before retrying a refused write.

- **Failed integration checks stay with the integrator.** A red candidate
  remains unpromoted; the integrator reads the result and chooses remediation
  under the integration gate. A red canonical branch is reverted through the
  gate before its cause is repaired in separate work. A failure returned to
  a todo carries the check evidence as findings; it is not an automatically
  diagnosed bug or permission for a second filing mechanism.

### Ground every defect claim in evidence

The terms below are defined in `sdlc/concept/defect-evidence`.

- **A worker's observation is decided by the session that governs it.** A
  worker is a session executing one job for a governing session it reports back to: a
  builder, reviewer, diagnosis peer, fixer, or finalizer. What it notices
  outside that job reaches the governing session as an observation — what it
  saw, where, and how it came to see it — through the channel its own
  procedure names, or else in its final report. The governing session decides
  each observation under this standard. A build's entries are decided by the
  deferral worker the work orchestrator dispatches: deciding them, and
  creating or filing what this policy admits, is that worker's job
  (`sdlc/procedure/lifecycle/work/deferrals`).

- **A defect contradicts a current contract.** A defect is observed behavior
  that a current contract states otherwise: a snippet without pending
  ownership, or the declared outcome of the surface that misbehaved. Its fix
  restores what that contract says, correcting any lower text that contradicts
  it on the way. A bug report states that contract and how the behavior breaks
  it. Where
  no current contract states the expected behavior, it is not a defect,
  however real the failure: it is a `change`, created with
  `instrukt todo create --kind change --approved`, only when that behavior is wanted, and
  otherwise there is nothing to file. Evidence shows the behavior happened; the
  contract shows it is wrong.
- **A filed defect claim carries its evidence.** A todo whose premise is a
  defect, or a defense against one, cites an occurrence or a dangerous path,
  and is filed by its kind: a defect through
  `instrukt todo create <slug> <content> --kind bug --approved`, a defense as the work it is. A claim that can cite neither stays an observation with the
  session that holds it.

- **A bug report keeps what was observed apart from what it concludes.** It
  carries the observations and their addresses first, and any causal account
  apart from them: each transition with its evidence, the alternatives checked,
  and what remains unknown, or the statement that the cause is not established.
  Filing does not require a completed diagnosis. An account that is present lets
  preparation confirm it against its sources and spend no peer on what the
  filing already settles (`sdlc/procedure/lifecycle/prepare/diagnose`).

- **What a human asks for in their own words is authorized intent, not a defect
  claim.** It is filed by its content under the rest of this policy, and
  preparation recovers it from the creating session and never retires it for
  lacking defect evidence. A human's acknowledgement of an agent's proposal to
  file leaves the claim agent-originated; it still owes evidence.

- **Preparation decides.** The filer establishes that its evidence exists and
  does not classify the claim. Preparation judges it before spending further
  work — diagnose triage for a bug (`sdlc/procedure/lifecycle/prepare/diagnose`),
  the entry audit for any other kind — by opening a cited occurrence's address
  and confirming it shows the claimed defect in real use, or by confirming each
  step of a cited dangerous path against current sources: the actor holds that
  access, the surface permits that act, any real-use input is shown, and the
  consequence meets the bar. A claim whose evidence holds is processed; one
  without evidence, or whose evidence fails a check, is retired. Confirming
  cited evidence is preparation's work; supplying evidence the filing lacked is
  not.

### Audit at preparation entry

- **Audit before deriving from `input`.** At preparation entry, read the mechanically
  stamped creation record and the current `input`, establish the owning
  project, repeat the bounded creation check, apply the governing creation and
  process-authority contracts, and confirm that functionality work carries its
  required pending documentation, written by whoever holds the awareness
  (`general/policy/documentation-discipline`). The record establishes the
  filing actor, originating session, observed human involvement and input
  version; it does not prove which sources or queries the creator consulted.

- **Check intent by position.** `input` is the intent record for every kind,
  including work with a supplied plan. The first recipient other than its
  author reads the originating conversation through
  `instrukt sessions conversation <session-id>` and compares outcome-level
  substance. Confirm the record, clarify expression without changing meaning,
  or return a mistaken interpretation to the human in the responsible thread.
  Record `instrukt todo stamp <slug> --intent-checked` bound to that input
  version, checker and conversation. A current check travels unchanged to
  later recipients; role names and author membership are not substitutes for
  the version-bound stamp. The check runs where the conversation is readable,
  without copying it to GitHub. Unreadable evidence supplies no check, and an
  author never independently checks their own compression. If the originating
  conversation is permanently unavailable, return the intent to the human in the
  responsible thread; a new `input` version from that conversation becomes the checkable
  basis.

- **Repair before retiring.** Correct the project, input, missing
  documentation, dependencies, or other repairable omission in place. Retire
  only a confirmed duplicate, an owner that cannot be corrected in place,
  process intent defensible from neither authorized intent nor applicable
  principles, a defect claim carrying neither the evidence nor the human
  request that grounds it, an input for which no defensible intent remains, or
  work with no actionable continuation under this policy. A
  human-owned consequence is repaired by returning only that consequence to
  its owner; technical process intent remains agent-owned. Scheduling and
  `hold` state do not decide validity.
- **A new `input` version needs its own check.** The earlier intent check
  is stale and the next non-author checks the new version. Changes to derived
  requirements or plans leave a current input check intact.

- **Perform the kind's deeper landscape judgment at its actual phase.** These
  checks are not equivalent and do not all occur at creation:
  - `feature` — intake reconciles the broader todo landscape before scoping
    (`sdlc/procedure/lifecycle/prepare/intake`).
  - `bug` — diagnose triage retires an exact-defect duplicate before
    investigation (`sdlc/procedure/lifecycle/prepare/diagnose`); plan drafting
    later handles broader overlap (`sdlc/procedure/lifecycle/prepare/plan-draft`).
  - `change` — plan drafting handles broader overlap before planning
    (`sdlc/procedure/lifecycle/prepare/plan-draft`).
  - `task` — only the bounded entry audit exists; deep task-landscape
    reconciliation is not implemented.

### Reconcile a flagged live todo

- **Any observer may flag a policy conflict to a todo's live claimant.** A
  flag is a challenge, never a verdict. The first message is complete: it
  identifies the todo, the source address grounding the concern, the section
  of this policy under challenge, and this reconciliation section. It carries
  addresses, not rewritten instructions or a proposed outcome.

- **Communication precedes a dispatch brake.** When the todo already has a live
  claimant, deliver the complete flag to that claimant first — through the
  session conversation with `instrukt sessions send <session-id>` — and wait for its response. The response closes the loop:
  it confirms what was understood and reveals whether derived work is already
  contaminated. The claimant normally chooses and records its exit. A log is
  durable evidence after understanding; it is never a substitute for delivering
  the flag.

- **A flag binds only a live claimant.** An unclaimed todo has no message-based
  flag path and consumes no active preparation effort; its next claimant must
  apply the entry audit before deriving from it. The recorded creator is
  audit evidence, not a guarantee that the creator is still reachable.

- **The claimant pauses preparation immediately.** It advances no phase,
  dispatches no new child, and does not act on child output produced for the
  challenged premise until it has analyzed the concern and chosen one of the
  following exits. A child already mid-turn may finish; send it a stop-work
  instruction only when the challenge plainly makes its current work unusable.
  - **Defend.** Point to the grounded creation check and landscape evidence.
    A defense is not a request for the flagger's permission: after sending the
    grounded basis, the claimant resumes preparation. A permitted follow-up
    reopens the pause once.
  - **Hold.** Run `instrukt todo update <slug> --hold --reason <text>`
    to set the todo's automatic dispatch brake, then write the flag's source
    address and the reason the premise may become valid later to the todo's
    `log` artifact. The log retains the reason for parking and the reconciliation breadcrumb.
  - **Stand down.** Settle the claimant's preparation children through their
    recorded sessions. Before releasing
    the claim, make the todo tell the truth by either holding it with the log
    entry above or amending `input` to carry the reconciliation. Only then run `instrukt todo update <slug> --unclaim` and confirm
    its outcome, then end the settled child sessions with
    `instrukt sessions end --todo <slug>` and the lane last with
    `instrukt sessions end self`. A bare release is forbidden: it
    returns the suspect premise to the runner with less context and recreates
    the same preparation waste.
  - **Retract.** For work that is truly invalid — including doc-only work,
    process work defensible from neither authorized intent nor applicable
    principles, or a confirmed duplicate — settle the claimant's children, run
    `instrukt todo update <slug> --unclaim` and confirm release, then end the settled
    child sessions with `instrukt sessions end --todo <slug>`. Run
    `instrukt todo remove <slug>`. Inspect the command outcome and record;
    a refusal leaves the work unwithdrawn. Removal retains the item and its
    evidence during the work record's recovery window and releases working
    resources. The withdrawing agent removes that slug's pending markers in a doc commit;
    nothing removes them automatically. Record unresolved cleanup rather than
    claiming it complete. End the settled lane session with
    `instrukt sessions end self` as its last step; removal alone ends no
    runtime conversation.

- **External hold is the unreachable-claimant fallback.** Only a proven delivery
  failure or other authoritative evidence that the claimant cannot answer —
  never silence or impatience — permits the observer to set `hold` with the
  grounded reason. That hold is only a brake against future dispatch while
  session recovery resolves the claimant; it neither stops the current agent nor
  proves that agent stopped. Route claimant recovery as a separate act.
- **The flagger does not select or negotiate the exit.** Except for the
  unreachable-claimant dispatch brake above, after the complete
  flag, it may send at most one source-grounded, address-only follow-up when a
  defense exposes a specific unresolved contradiction. When that follow-up
  arrives, the claimant pauses again and answers by choosing another exit or
  maintaining its defense. A grounded defense with no follow-up is the only
  conversational settlement; flagger silence leaves nothing pending. For
  every state-changing exit, the flagger reads settlement from the todo
  record: the hold and its log entry, the amended `input`, or removal.
- **Deadlock is held, never routed.** Deadlock exists only when the claimant
  maintains its defense after the permitted follow-up and the flagger still
  cannot reconcile the conflict. The flagger preserves the conflict unresolved,
  states both sides' grounding in its final turn, and goes idle
  (`general/principle/autonomy`); it routes the conflict to no other party and
  never manufactures human ownership for it. Only a surviving decision that
  `general/policy/human-contract` independently assigns to the human is left
  in that idle turn, as one live question. A resolved defense, hold,
  stand-down, or retraction does not become human clerking.

### Withdraw your own filing

- **A filer withdraws its own filing once its premise fails.** When the
  session that filed a todo establishes that its premise does not hold, it
  owns the retraction rather than leaving the live claimant to rediscover it.
  Communicate the evidence through `instrukt sessions send` to the recorded
  claimant and workers; observe their status through `instrukt sessions alive`
  and settle their current work before release and removal. A hold brakes
  future dispatch and does not establish that current work stopped. Once the
  claim is released through `instrukt todo update <slug> --unclaim` and release is
  confirmed, end the settled claimant and worker sessions with
  `instrukt sessions end <id>`, then run `instrukt todo remove <slug>` and
  inspect its outcome. The withdrawing agent removes the slug's pending
  markers in a doc commit; nothing does so automatically.

- **Only an agent-originated filing is the filer's to withdraw.** A filing that
  carries a human's request in their own words is withdrawn only by that human:
  abandoning an intended outcome is theirs (`general/policy/human-contract`). A
  human's acknowledgement of the agent's proposal to file does not make the
  filing theirs.

### Read authorship from the creation record

- **Never infer authorship from account identity.** Read the recorded filing
  actor and originating session together with the input's writer provenance.
  GitHub account identity alone does not identify who authored the work.

## Rationale

Creation is locally atomic; GitHub reflection may finish later without
undoing accepted local work. Inspecting each command's actual outcome keeps
dependent acts from assuming a filing or remote reflection that did not occur.

Keeping an item alive spends attention and invites further work. An unresolved
question alone justifies neither expense. Concrete next contributions and
unblock conditions keep the workflow tied to an achievable outcome; retained
records preserve what was learned without turning uncertainty into permanent
work or speculative infrastructure.

Creation stays intentionally cheap. Its bounded check prevents obvious waste
without forcing every creator to perform the preparation worker's deeper
landscape analysis. The entry audit then reads the actual todo and current landscape before
derivation. The position-based intent check compares the recorded input with
its originating conversation once per version, so a lossy summary does not
become the authority for every later receiver. Parked intent re-grounds
against current code and docs when picked up, without invalidating a current
intent check merely because a derived plan changed.

Documentation-only work stays in its live authoring context because a todo
would dispatch a fresh worker that never saw the conversation which made the
content correct. Re-deriving that intent cold destroys the information the
handoff was supposed to preserve; routing it in context avoids the loss rather
than merely saving lifecycle ceremony.


The absence of an automatic actor proves only how the system is arranged. When
the contract assigns recovery to an observing agent, filing automation from
that absence erases the agent's unattempted responsibility and turns a missed
procedure into product scope. Requiring the failed sanctioned step preserves
the difference between an observed symptom and a missing platform capability.

A worker sees one job's slice of the system and cannot judge how a defect is
reached or what it costs; the session governing it holds the wider frame, and
preparation judges a claim independently of the filer's stake in it. The
evidence requirement attaches to the premise rather than the todo kind because
a guard filed as a `change` adds the same conjectural code a bug fix would.

Kind routes the work, and evidence cannot choose it: a real failure proves that
something happened, never that a contract forbids it. The bug lane spends an
independent diagnosis on finding why behavior departs from its contract; when
no contract states the wanted behavior, that diagnosis rediscovers the gap and
the work still needs the contract authored first, as a change does. Naming the
violated contract at filing makes that difference visible before the lane is
entered.

Communication comes before a state brake because only the claimant can report
what it understood and whether its current derivation remains usable. A hold is
still necessary when that feedback path is provably unavailable, but it carries
one narrow fact: future dispatch is paused. It says nothing about the current
agent's state, so recovery remains a separate owned act.

An observer who can name a violation needs a consequence it can trigger, or
naming it changes nothing. A live-claimant flag supplies one at the moment cost
is being incurred: preparation pauses, while the claimant retains judgment over whether
to defend, hold, stand down, or retract. Restricting the flagger to addresses
and one follow-up prevents a challenge from becoming a parallel
orchestration. Holding an unresolved defense where it arose, with only what
the human contract assigns to the human left as a question, keeps every
technical disagreement out of the human's attention without inventing a
channel that may not exist.

The flag serves an observer of someone else's filing. The filer of a disproven
filing holds the reasoning the filing came from; the claimant only derived from
it and cannot judge the withdrawal better. Flagging instead would leave a
session working a premise its author already knows is false.

The message-based flag is deliberately not durable todo state. It protects
actively prepared work, where a claimant exists and effort is being spent.
Unclaimed work is protected by the mandatory entry audit when preparation
next begins.

A failed candidate stays with the standing integrator, which owns the
verification and recovery decision. The work record preserves its evidence;
neither a check result nor an automated filing supplies a diagnosis.

## Scope

All todo creation and preparation, every kind, every human or agent creator,
every observer that flags a live claimant, and every filer that withdraws its
own filing under this policy.

## Enforcement

- A todo created without the bounded check, in the wrong owning project, for
  plainly duplicate or amendable work, for in-context work, or for process work
  with no defensible intent or principled basis violates this policy.
- A filing by a session whose job carries no filing, or a defect claim filed
  without cited evidence, violates this policy; preparation and independent review catch it. Preparation
  judges a filed claim only by the evidence it carries, never by who filed it.
- A prepare claimant that derives from `input` before the entry audit, or
  continues preparation after receiving a flag and before reconciliation,
  violates this policy.
- A flagger that supplies remediation content, chooses the claimant's exit,
  continues beyond one grounded follow-up, or carries a resolved case or an
  unresolved technical deadlock to the human violates this policy. A filer
  withdrawing its own agent-originated filing is not a flagger.
- A filer that leaves its own disproven filing in preparation, or withdraws a
  filing that carries a human's request, violates this policy.
- Parking a flagged todo with `instrukt todo update <slug> --hold --reason <text>` without
  the associated todo-log explanation is incomplete reconciliation.

## Exceptions

- `task` receives the bounded creation and entry audits with no carve-out. It
  has no kind-specific deep landscape reconciliation today; do not imply that
  one ran. See `sdlc/spec/todo-kind-task`.
