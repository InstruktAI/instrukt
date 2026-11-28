---
description: Recognition semantics for a project's standing coordinator — the case schema, the attention events that open each case, and the act, stop and settlement of every known coordination situation.
visibility: internal
---

# Coordination Cases — Concept

## What

The operational case catalog for
`sdlc/procedure/lifecycle/coordinate`. The procedure supplies
the posture and mechanism; this companion supplies recognition semantics and
concrete runtime branches — how an outsider reads each role's session states,
which party owns each judgment, what the coordinator's smallest act is, where
it stops, and what evidence settles it. The coordinator reads a case when a
fleet-attention event or a message wakes it; **Attention events and their
cases** below names the cases each event can open.

### Case schema

- **Evidence available** names live source addresses and observed records. A person
  appears only when their utterance is itself evidence. Lineage is never authorship,
  and `human_involved` is not proof of intent.
- **Judgment owner** is an exact cited role, a runtime role resolved by a named
  contract or record, Human, `not applicable — mechanically determined by <named
mechanism>`, or `none — unresolved`.
- **Execution owner** is an exact cited role, a runtime role resolved by a named
  contract or record, the Coordinator where expressly authorised, or none.
- **State authority** is the named mechanism or role authorised to mutate governed
  state, or none.
- A runtime-resolved owner is precise only when the address that resolves it is
  named. Human supplies intent, approval, and judgment; agents execute and governed
  mechanisms mutate state unless a loaded contract explicitly says otherwise.

A merge is valid only when trigger, evidence, judgment owner, execution owner, state
authority, coordinator act, intervention stop, and settlement evidence match.

### Attention events and their cases

An event opens a reading, not a verdict: the current record decides which case
applies, and a condition that has cleared opens none.

| What woke the coordinator | Cases it can open |
| --- | --- |
| `runner.attention.execution_stalled` | `NC-37`, `NC-38`, `NC-05` |
| `runner.attention.input_waiting` | `NC-39`, `NC-29`, `NC-34` |
| `runner.attention.hold_placed`, `runner.attention.hold_released` | `NC-41`, `NC-15-a`, `NC-15-b`, `NC-48`, `NC-51` |
| `runner.attention.fleet_quiet` | `NC-50` |
| `runner.attention.attempts_failing` | `NC-40`, `NC-08` |
| `runner.attention.raised` | `NC-44-a` to `NC-44-e`, `NC-45`, `NC-46`, `NC-47` |
| An origin's message about a return | `NC-48` |
| A technical coordinator's question | `NC-49` |
| A person's message in the coordinator's thread | `NC-30`, `NC-32`, `NC-33`, `NC-35` |
| A message that a held todo's stated condition is met | `NC-51` |

Every wake, whatever opened it, also reads the project's holds that wait on a
stated condition outside its todos under `NC-51` before the coordinator rests.

---

### Operational cases

#### Responsibility 1 — Alignment with authorised scope, policy, and mandate

##### NC-01 — Material scope or intent departure

- **Evidence available:** the approved intent artifacts, current task plan or output,
  declared dependencies, changed artifact or commit, and the producing session's role
  contract (`sdlc/procedure/lifecycle/coordinate`; lifecycle prepare/work contracts).
- **Ownership:** the judgment owner is the role named by the affected contract
  for scope or drift; the producing session executes any correction; the todo
  work record is state authority only when correction changes governed state.
- **Coordinator:** ask how the changed-output address reconciles with the approved
  intent/role address for the named obligation. Never supply the drift verdict.
- **Stop and settlement:** the intervention stops after one pointer. The observation
  settles on a corrected artifact/output, a grounded defence accepted by its judgment
  owner, or a durable route to the owner of the unresolved decision.

##### NC-02-a — Todo for work that belongs in context

- **Evidence available:** todo creation record, current input, orchestrator transcript and
  derived artifacts, and the in-context boundary in
  `sdlc/policy/todo-creation-discipline`.
- **Ownership:** The live claimant — the owning `todo-orchestrator` session named by the todo/session
  record — judges the creation audit and executes repair or retirement. The todo lifecycle mechanism is state authority.
- **Coordinator:** intervene only on positive evidence that the claimant derived
  without holding the required check, or when its stated basis materially contradicts
  the creation and policy addresses. Then flag the live claimant with the complete first message the reconciliation
  clause of `sdlc/policy/todo-creation-discipline` defines; never
  perform the audit, decide validity, or select the claimant's exit.
- **Stop and settlement:** the flag exchange and its settlement follow the
  reconciliation clause of `sdlc/policy/todo-creation-discipline`; a deadlock
  this role observes is taken up under `sdlc/procedure/lifecycle/coordinate`.

##### NC-02-b — Documentation-only work routed through the lifecycle

- **Evidence available:** creation record, input, declared deliverables, orchestrator
  transcript and derived artifacts, and `general/policy/documentation-discipline`.
  File paths alone do not prove the work is doc-only; directive agent text can be
  behavioural implementation.
- **Ownership:** The live claimant, resolved from the todo/session record, judges whether
  functionality exists and executes the audit result. The todo lifecycle mechanism is state authority for retirement.
- **Coordinator:** intervene only on positive evidence that the claimant derived
  without holding the check, or when its stated basis is contradicted by the
  deliverable and policy addresses. Never classify the todo from file paths. Then
  flag the live claimant with the complete first message the reconciliation
  clause of `sdlc/policy/todo-creation-discipline` defines; never
  perform the audit, decide validity, or select the claimant's exit.
- **Stop and settlement:** the flag exchange and its settlement follow the
  reconciliation clause of `sdlc/policy/todo-creation-discipline`; a deadlock
  this role observes is taken up under `sdlc/procedure/lifecycle/coordinate`.

##### NC-02-c — Process change past a human-owned consequence without its decision

- **Evidence available:** the actual human decision record, current input, creation
  record, orchestrator transcript and derived artifacts, and
  `sdlc/policy/process-change-authority`. Lineage and
  `human_involved` corroborate involvement but never prove the decision.
- **Ownership:** The live claimant, resolved from the todo/session record, applies the
  authority gate and executes refusal or continuation. Process evolution is
  agent-owned; the human owns only a consequence on the closed list of
  `general/policy/human-contract` (`sdlc/policy/process-change-authority`). The
  todo work record records the resulting state.
- **Coordinator:** intervene only on positive evidence that the claimant proceeded
  past such a human-owned consequence without the recorded decision, or that its
  stated basis contradicts those addresses. Never infer ratification or ask Human to re-decide a recorded decision.
  Then flag the live claimant with the complete first message the reconciliation
  clause of `sdlc/policy/todo-creation-discipline` defines; never
  perform the audit, decide validity, or select the claimant's exit.
- **Stop and settlement:** for a valid ordered beacon, deduplicated delivery stands
  outside the flag path; otherwise the flag exchange and its settlement follow the
  reconciliation clause of `sdlc/policy/todo-creation-discipline`; a deadlock
  this role observes is taken up under `sdlc/procedure/lifecycle/coordinate`.

##### NC-02-d — A role claims authority reserved elsewhere

- **Evidence available:** the claimed verdict/action and the exact contract address
  assigning that verdict elsewhere.
- **Ownership:** judgment belongs to the runtime owner named by that contract; the
  producing role withdraws or reroutes its claim; the contract-named lifecycle
  mechanism alone mutates state.
- **Coordinator:** ask how the claimed-action address reconciles with the ownership
  address. Do not decide the underlying product, drift, duplicate, kickback, or review
  verdict.
- **Stop and settlement:** stop after the pointer. Settle when the claim is withdrawn
  and the rightful owner has the evidence, or when that owner grounds why no conflict
  existed.

##### NC-03 — Runner dispatch before the creation audit is held

- **Evidence available:** creation record, input, current lifecycle phase, Prepare
  transcript and derived artifacts, and runner output or logs. A question-shaped
  origin is evidence to inspect, not proof of invalidity.
- **Ownership:** The live claimant, resolved from the todo/session record, judges and executes the audit; the todo work record
  records its result. Runner eligibility is mechanical, not creation approval.
- **Coordinator:** only on positive evidence that derivation began before the
  mandatory check, flag the live claimant with the complete first message the reconciliation
  clause of `sdlc/policy/todo-creation-discipline` defines; never
  perform the audit, decide validity, or select the claimant's exit.
  If no live claimant or recovery path exists, ground the broken mechanism and
  report it through `general/procedure/bug-reporting` — an unclaimed todo has no
  flag path.
- **Stop and settlement:** the flag exchange and its settlement follow the
  reconciliation clause of `sdlc/policy/todo-creation-discipline`; a deadlock
  this role observes is taken up under `sdlc/procedure/lifecycle/coordinate`.
  A reported defect settles separately.

##### NC-04 — Prepare's ordered process-authority beacon

- **Evidence available:** Prepare refusal, the process delta and rationale, the human
  decision record or its absence, and current delivery state
  (`sdlc/procedure/lifecycle/overview`).

- **Ownership:** Human owns only a consequence on the closed list of
  `general/policy/human-contract` that the process delta carries
  (`sdlc/policy/process-change-authority`); `todo-orchestrator` presents that
  question and later executes the decision; the todo work record records
  progression.
- **Coordinator:** leave a correctly delivered beacon alone. If the human-owned need
  is newly live and reaches no person, return it once to the todo's origin with
  its evidence and the smallest live question (`general/principle/origin-routing`).
- **Stop and settlement:** delivery ends the intervention, not the need. Settle when
  the decision is recorded and Prepare resumes, revises, or retires the work.

#### Responsibility 2 — Continuity of attention

##### NC-05 — Nonterminal slug with no accountable owner

- **Evidence available:** todo state, phase checkpoint, current owner/session
  association, runner output or logs where relevant, and the recovery section of the
  governing phase contract.
- **Ownership:** use the recovery owner named by that contract. If the contract
  and runner name none, judgment and execution remain unresolved; the todo work
  record remains the only state authority.
- **Coordinator:** route to a grounded recovery owner. If none or the path is broken,
  check existing defect coverage and, when uncovered, report one grounded defect
  through `general/procedure/bug-reporting`; never invent an owner or adopt the slug
  by claiming or driving its orchestration.
- **Stop and settlement:** the intervention stops after routing, or after the defect is
  reported. The
  slug observation settles only when a valid owner takes it or governed state becomes
  terminal; a reported defect alone does not settle the slug.

##### NC-06 — Unresolvable or orphaned `working_slug`

- **Evidence available:** session `working_slug`, project identity, project todo
  registry, and session association record (`sdlc/procedure/lifecycle/coordinate`).
- **Ownership:** project resolution is a factual lookup. Cross-project work belongs to
  that project's coordinator; an orphan has no execution owner, so its disposition is
  the Coordinator's. Only the session/todo registry mechanism
  may mutate the association.
- **Coordinator:** establish cross-project versus orphan. Leave cross-project work out
  of management scope; report an orphan association once as a defect through `general/procedure/bug-reporting`, from evidence and existing coverage. No origin
  is told of it.
- **Stop and settlement:** stop after classification and the report.
  Settle when the association resolves to its project, is repaired, or disappears
  with a valid terminal session.

##### NC-07 — Idle `todo-orchestrator` on unfinished work

- **Evidence available:** live `todo-orchestrator`, nonterminal slug, current
  factual ledger, relevant artifacts/child state, and terminal-handoff/recovery
  contracts in `sdlc/procedure/lifecycle/overview`.

- **Ownership:** `todo-orchestrator` judges and executes its next work/recovery
  act; the todo work record owns governed state.
- **Coordinator:** idle is positive evidence of unfinished work, not by itself of
  failure. Point only when evidence shows a contractually due act was skipped and is
  not deliberately waiting.
- **Stop and settlement:** stop after one pointer. Settle on work-produced re-entry,
  valid deliberate waiting/rejection, or terminal handoff.

##### NC-08 — Dropped child recovery

- **Evidence available:** child error/finding, child and orchestrator transcripts,
  worktree and factual-ledger evidence, and orchestration recovery contract.
- **Ownership:** the existing child recovers from its own brief; the todo work
  record owns governed state; `todo-orchestrator` escalates blockers that are
  human-owned, architectural, or beyond the child's brief.
- **Coordinator:** point the orchestrator to the child evidence only after positive
  evidence that a contractually due escalation was skipped.
- **Stop and settlement:** stop after the pointer. Settle the coordination observation
  when the child resumes, the orchestrator grounds a deliberate wait, or the
  escalation lands; the orchestrator then owns watching the child.

##### NC-09 — Review threshold crossed

- **Evidence available:** configured round threshold, round counter, current review
  findings/verdict, `todo-orchestrator`'s recorded continuation ground or stop
  cause, and review convergence contract.
- **Ownership:** `todo-orchestrator` owns continuation and routing the next
  contribution under the shared convergence contract
  (`sdlc/procedure/lifecycle/review`); reviewer and producer retain verdict,
  finding-disposition, and remediation authority; the todo work record owns
  resulting state.
- **Coordinator:** when passes continue past the threshold with no grounded account
  of what the next pass resolves, point to the convergence contract's address and
  the current review-findings artifact address. Do not send findings content or the
  decision to Human merely because the rounds ended, and do not judge the loop.
- **Stop and settlement:** stop after the pointer. Settle when the orchestrator
  records its continuation ground or its stop cause and routing, or grounds that
  the remaining decision is independently human-owned.

##### NC-10 — Integrator drained the queue

- **Evidence available:** `INTEGRATION COMPLETE`, empty candidate queue, synced main,
  and Integrator lease (`sdlc/procedure/lifecycle/integrate`).
- **Ownership:** completion is established from the factual ledger and git refs;
  Integrator reconciles delivery and cleanup, then parks in its standing session;
  the todo work record owns delivery state.
- **Coordinator:** no intervention and no log line.
- **Stop and settlement:** settle when the Integrator records delivery and any owed cleanup,
  then parks under its standing binding. A later integration event continues
  that session under `sdlc/procedure/lifecycle/integrate`.

##### NC-11 — Integrator transient dirty-root block

- **Evidence available:** `REPO ROOT SYNC REQUIRED`, exact dirty paths and authorship
  evidence, the retained unresolved sync operation, standing-session binding, and delivery state.
- **Ownership:** Integrator retains the failed sync and routes clearance to the
  owner of the blocking work under `sdlc/procedure/lifecycle/integrate`. Once
  clearance is evidenced, the integrator rechecks ancestry and continues the
  owed fast-forward; the todo work record owns delivery state.
- **Coordinator:** leave the retained recovery with the standing integrator. Event-driven
  continuation rechecks the unresolved operation; human-owned questions stay
  in that session. Point only on positive evidence that the integrator
  omitted its recovery obligation or its delivery path failed.
- **Stop and settlement:** stop after leaving the valid retry alone or sending one
  pointer. Settle when main clears and integration advances, or when a human-owned question reaches the integrator's waiting thread.

##### NC-12 — Integrator unrecoverable human beacon

- **Evidence available:** exhausted recovery/window, exact conflict/block evidence,
  Integrator log/lease, and whether its human delivery already occurred.
- **Ownership:** Human owns the irreducible decision; Integrator executes the chosen
  recovery or disposition, with the authoring role acting where its own work must
  change; the integration worktree and work record own candidate state.
- **Coordinator:** do not duplicate a delivered beacon. One that reaches no person
  returns once, with its evidence and smallest live question, to the operator in the
  coordinator's thread; never instruct overwrite, stash, or forced retry.
- **Stop and settlement:** delivery ends the intervention. Settle when the human
  decision is durable and Integrator produces resulting work/state evidence.

##### NC-13-a — Peer-held knowledge missing from a derivation

- **Evidence available:** deriving session output, peer session/artifact address,
  artifact version, and the contract establishing their relationship.
- **Ownership:** the deriving role named by its session/phase contract owns its
  judgment and revision; its normal lifecycle mechanism remains state authority.
- **Coordinator:** send only the peer/artifact/contract addresses and named obligation.
- **Stop and settlement:** stop after one pointer. Settle when the deriving role
  incorporates the source, revises, or grounds its rejection from its own contract.

##### NC-13-b — A loaded contract obligation was dropped

- **Evidence available:** deriving output, exact required-read section, and positive
  evidence that its required act/constraint is absent.
- **Ownership:** the deriving role named by the current phase contract judges and
  corrects its derivation; its lifecycle mechanism owns any state mutation.
- **Coordinator:** point to the contract address and obligation, without interpretation
  of what the role must conclude.
- **Stop and settlement:** stop after one pointer. Settle on corrected work-produced
  evidence or a grounded contract-based rejection.

##### NC-14 — Competing obligations and capacity contention

- **Evidence available:** all due obligations, their owners, current owner capacity,
  and any ordering/prioritisation clauses in their governing contracts.
- **Ownership:** the contract-named prioritisation owner judges ordering; obligated
  roles execute in that order; their lifecycle mechanisms own state. If contracts name
  no prioritisation owner, judgment remains unresolved.
- **Coordinator:** when the contract names a role as prioritisation owner, send that
  role only the competing-obligation addresses and the ordering-clause address, as a
  reconciliation question. When Human owns the priority, carry the contention with
  evidence and interpretation. Never assign priority merely because a role holds the
  work; if no owner is grounded, preserve it unresolved in session context.
- **Stop and settlement:** stop after the contention is visible to its grounded owner,
  or after preserving it unresolved. Settle when a durable priority exists and work
  follows it.

##### NC-15-a — Governed standing gate

- **Evidence available:** held/parked phase, exact gate address, current gate verdict,
  and the owner named by that gate.
- **Ownership:** the gate-named role or Human judges; the runtime execution owner named
  by that gate or phase contract carries the decision; its lifecycle mechanism owns
  state.
- **Coordinator:** leave a valid standing decision alone. A newly live gate that
  reaches no person returns once to the todo's origin; age alone creates no finding.
- **Stop and settlement:** stop after leaving it alone or delivering a newly live
  human gate. Reassess only when the gate verdict or governed state changes.

##### NC-15-b — Untyped hold note

- **Evidence available:** held state, free-text note, hold audit/history, and absence
  of both a governed gate/owner discriminator and a stated condition outside the
  project's todos with evidence the Coordinator can read (`NC-51`).
- **Ownership:** judgment and execution are unresolved; only the todo mechanism may
  mutate the hold, and the Coordinator changes no such hold it did not place.
- **Coordinator:** leave the individual hold unchanged, assess the shared
  observability failure under `NC-24`, and never infer expiry from age or validity
  from the note. This case produces no separate per-hold report or log line.
- **Stop and settlement:** stop after leaving the hold unchanged and, if needed,
  reporting the uncovered defect under `NC-24`. Reassess only when a
  grounded owner or gate appears or
  the hold changes through its authorised path.

##### NC-16 — Ready work without movement

- **Evidence available:** exact todo and gate state, configured runner policy, current
  owner/session associations, and the runner output or logs explaining its treatment.
- **Ownership:** the configured runner and the lifecycle's existing owners govern
  whether and when work starts; the Coordinator acts only on a raced or failed
  launch, under `NC-37`.
- **Coordinator:** leave work alone when a standing gate or current owner explains the
  wait. When an existing owner owes an act, point that owner to the relevant
  addresses. When configured automation should have acted but no owner or governing
  explanation exists, ground the broken mechanism and report it through `general/procedure/bug-reporting`. Never recreate eligibility from prose, drive the lifecycle,
  or dispatch work.
- **Stop and settlement:** stop after leaving, one pointer, or the reported
  defect. Continue
  watching only an in-session handoff until work-produced evidence shows uptake or a
  grounded contract state explains the wait.

##### NC-17 — Prepared artifact with referenced-path drift

- **Evidence available:** grounding stamps, artifact path references, current path
  fingerprints, and factual-ledger drift facts
  (`sdlc/procedure/lifecycle/overview`).
- **Ownership:** the dispatched `drift-analyzer` exclusively judges and stamps impact;
  `todo-orchestrator` dispatches drift analysis or re-derivation from the factual
  ledger; the todo work record owns invalidation and drift stamps.
- **Coordinator:** do not classify drift or dispatch the analyzer independently. Point
  a live `todo-orchestrator` only after the ledger reports pending or substantive
  drift. If no owner or recovery path exists, preserve the missing path as a
  mechanism finding.
- **Stop and settlement:** stop after pointer or unresolved finding. Settle on the
  digest/path-bound `drift-reviewed` verdict and resulting re-entry or continuation.

##### NC-18 — Declared dependency ordering

- **Evidence available:** dependency graph, terminal state of dependencies, configured
  gates, current ownership, and runner output or logs where available.
- **Ownership:** the dependency and runner mechanisms govern eligibility and movement;
  their contract-named owners execute any required contribution. The Coordinator
  declares a dependency only between overlapping or contending todos, under
  `NC-44-b` and `NC-44-d`.
- **Coordinator:** no intervention while an unmet dependency or standing gate explains
  waiting. If dependencies are satisfied and a contract-named owner has missed a due
  act, point that owner to the addresses; if the mechanism itself is demonstrably
  broken, report the defect through `general/procedure/bug-reporting`.
  Never force movement or recreate eligibility from prose.
- **Stop and settlement:** stop after leaving, one pointer, or the reported
  defect. Reassess
  only on material dependency, gate, ownership, or work-produced change.

#### Responsibility 3 — Collision, duplication, and misplaced production

##### NC-19 — Potential candidate or finding collision

- **Evidence available:** both session/artifact/candidate addresses, lineage and
  purpose, commit/todo targets, and the relevant lane contract. Shared SHA or wording
  alone does not prove duplication.
- **Ownership:** the triage owner named by the relevant Prepare or Work contract judges
  whether the work collides; the affected producing roles execute its disposition;
  only the lane's lifecycle mechanism may close or mutate governed candidates.
- **Coordinator:** present both addresses as a potential collision, never a duplicate
  verdict.
- **Stop and settlement:** stop after one pointer. Settle when the triage owner records
  a disposition and affected roles act, or grounds why both are intentional.

##### NC-20-a — Deferral overlap before todo creation

- **Evidence available:** deferral entry, current open-todo landscape, suggested
  outcome and dependency rationale
  (`sdlc/procedure/lifecycle/work/deferrals` and todo creation
  discipline).
- **Ownership:** the dispatched deferral worker performs the mandatory minimal overlap
  judgment and executes creation/no-op; the todo creation mechanism is state authority.
- **Coordinator:** if the deferral worker is live and positive evidence shows the
  creation check was skipped, point it to the deferral and existing-todo addresses.
- **Stop and settlement:** stop after one pointer. Settle when the worker records its
  overlap decision and creates, links, or marks no-op accordingly.

##### NC-20-b — Deferral overlap discovered after creation

- **Evidence available:** created deferral todo and creation record, originating
  deferral, overlapping open todo, and orchestrator transcript or derived artifacts.
- **Ownership:** the live claimant, resolved from the todo/session record, judges and
  executes audit repair/retirement; the todo lifecycle mechanism owns state.
- **Coordinator:** only on positive evidence that the claimant derived without the
  required check, or when its stated basis is contradicted — with both todo addresses
  as the grounding sources — flag the live claimant with the complete first message the reconciliation
  clause of `sdlc/policy/todo-creation-discipline` defines; never
  perform the audit, decide validity, or select the claimant's exit.
- **Stop and settlement:** the flag exchange and its settlement follow the
  reconciliation clause of `sdlc/policy/todo-creation-discipline`; a deadlock
  this role observes is taken up under `sdlc/procedure/lifecycle/coordinate`.

##### NC-21 — Materially unjustified production or noise

- **Evidence available:** produced output, the obligation it claims to serve, its role
  contract, and downstream attention/effort cost. The minimal FYI rule in todo creation
  discipline is one concrete control, not the definition of all noise.
- **Ownership:** the producing role named by the session record owns its basis and any
  revision; its normal mechanism owns state only if the correction changes governed
  work.
- **Coordinator:** leave harmless output alone. For material waste, ask how the output
  address serves the obligation/mandate address; do not judge aesthetic merit. Only
  when there is positive evidence that a live-claimed todo violates a named section
  of `sdlc/policy/todo-creation-discipline` does the flag path apply
  instead of the ordinary address-only challenge — material waste alone is not a
  creation-policy conflict.
- **Stop and settlement:** stop after one challenge (or the flag exchange where it
  applies). Settle on revision, cessation, grounded defence, settlement read from the
  todo record, or route to a contract-named judgment owner.

#### Responsibility 4 — Broken mechanisms

##### NC-22 — Provably broken signal path

- **Evidence available:** contract-ordered signal, source state proving its trigger
  occurred, event/notification records showing absence or demotion, and bounded source
  inspection where needed (`project/<slug>/design/architecture/notification-delivery`).
- **Ownership:** Coordinator judges only whether the path is demonstrably broken.
  The defect is reported once or routed to existing coverage, through `general/procedure/bug-reporting`. Event and notification mechanisms are state authority.
- **Coordinator:** preserve the affected signal, report the broken path once, and
  return any still-live person's need to its origin. Audit no path the condition
  does not name.
- **Stop and settlement:** the report ends the defect
  intervention; the affected signal settles separately when its rightful recipient
  receives or owns it.

##### NC-24 — Hold-state observability defect

- **Evidence available:** free-text hold storage, inability to distinguish completed
  human-gated preparation from abandonment, affected todos, and existing defect
  coverage.
- **Ownership:** Coordinator judges the observability gap and its disposition. The
  todo mechanism remains the only authority over individual holds.
- **Coordinator:** check existing defect coverage and, when uncovered, report one
  grounded defect. Do not call the schema corrupt,
  mutate holds, or turn every hold into a separate defect.
- **Stop and settlement:** the report settles the mechanism
  observation for this pass. Each affected hold remains governed by `NC-15-b` until
  its owner becomes knowable.

#### Responsibility 5 — Human interface and irreducible decisions

##### NC-27 — Human PRD approval gate

- **Evidence available:** reviewed PRD, `approvals.prd` gate state in the factual
  ledger, approval absence, current delivery record, and `todo-orchestrator`
  session (`sdlc/procedure/lifecycle/overview`).
- **Ownership:** Human judges approval; `todo-orchestrator` executes the recorded
  decision; the todo work record owns approval state and enforces `approvals.prd`.
- **Coordinator:** none. An approval gate produces no attention event; it waits
  with its orchestrator. Never approve or refine it.
- **Stop and settlement:** nothing to settle for the Coordinator; the gate settles
  when the human decision is durably recorded and `todo-orchestrator` produces
  resulting state/artifact evidence.

##### NC-28 — Kickback

- **Evidence available:** builder's unbuildable-plan reason or red CI failure,
  `todo-orchestrator`'s read, reviewed plan, current factual ledger and delivery
  record (`sdlc/procedure/lifecycle/overview`).
- **Ownership:** `todo-orchestrator` (or `integrator` on red CI) owns the
  technical kickback decision and executes `--kickback --reason ...`; the todo
  work record mutates governed state.
- **Coordinator:** never stamp, recommend kickback from plausibility, or bypass the
  orchestrator.
- **Stop and settlement:** settle when the recorded kickback stamp and subsequent
  orchestration produce the resulting state.

##### NC-29 — Escalation answered by a governing contract

- **Evidence available:** escalation, loaded contract address that answers it, current
  role/session record, target transcript, and the coordinator's current session
  context showing whether the pointer was already sent. An owner's question to Human
  about a judgment `general/policy/human-contract` leaves with agents is such an
  escalation wherever it was asked, a live conversation with Human included.
- **Ownership:** the escalating role retains its judgment unless the cited contract
  names another owner; that runtime owner executes the next act; its lifecycle
  mechanism owns state.
- **Coordinator:** send one address-only contract pointer. Never suppress an ordered
  human beacon or treat agent uncertainty as human ownership.
- **Stop and settlement:** stop after one pointer. Settle on work-produced uptake,
  deliberate contract-based waiting/rejection, or a surviving irreducible question.

##### NC-30 — Human asks for fleet status

- **Evidence available:** current source-linked fleet surface, relevant governed state,
  and the coordinator's current session context. Never answer from a copied
  snapshot.
- **Ownership:** Coordinator judges status interpretation and executes the response;
  no lifecycle state mutation occurs.
- **Coordinator:** answer only the question Human asked, from current source-linked
  evidence and at the scope the question requires. Do not expand it into a prepared
  fleet taxonomy or general status report.
- **Stop and settlement:** response delivery settles the query; it does not settle the
  underlying fleet observations.

##### NC-31 — Human-owned product or process design

- **Evidence available:** the live product/process question, current intent artifacts,
  affected downstream work, and whether Human is already collaborating with its owner.
- **Ownership:** Human supplies and judges intent; the runtime role owning the affected
  artifact executes authoring after the decision; its governed artifact/lifecycle
  mechanism owns state.
- **Coordinator:** do not interpose in live collaboration. If no owner currently holds
  the question, return the smallest live question and its evidence to the todo's
  origin (`general/principle/origin-routing`).
- **Stop and settlement:** delivery ends intervention. Settle when Human's decision is
  durable and the owning role produces the resulting artifact/state evidence.

##### NC-32 — Human asks Coordinator for a merit judgment

- **Evidence available:** human query, exact code/plan/artifact address, current phase,
  and the phase contract assigning depth review.
- **Ownership:** the depth role named by that phase contract judges and executes the
  assessment; no state mutation follows merely from answering. If the axis is actually
  product/process intent, use `NC-31` instead.
- **Coordinator:** route the question and evidence address to an existing depth owner;
  do not answer merit from fleet-level context, create an assessor, or dispatch one.
  If no depth owner/path exists, preserve that absence as unresolved.
- **Stop and settlement:** routing ends intervention, not the query. Settle when the
  depth role returns its independently grounded answer to Human.

##### NC-33 — Human asks to start currently ineligible work

- **Evidence available:** human mandate, todo/gate state, gate owner, configured runner
  policy, and any current runner output or log explaining the wait.
- **Ownership:** Human owns the mandate or gate decision; the runtime owner named by
  the governing gate records/executes it; the configured runner and lifecycle
  mechanisms govern any subsequent movement.
- **Coordinator:** state the boundary and route the mandate to the gate owner for
  durable recording. Never start or dispatch the work, drive its orchestration, or
  override a runner skip. If no gate owner/path exists, preserve it unresolved.
- **Stop and settlement:** routing ends intervention. Settle when governed state
  reflects the decision; any later lack of movement is assessed separately under
  `NC-16`.

#### Responsibility 6 — Fleet attention operations

##### NC-37 — Raced or failed launch

- **Evidence available:** the `runner.attention.execution_stalled` event naming a
  start that never produced a session, or a session that ended before its first
  record write; the todo's claim, attempts and bindings; runner output.
- **Ownership:** the runner's pickup and the todo's orchestrator own starting and
  recovering the work; the todo work record owns claims and attempts.
- **Coordinator:** when the cause was transient — availability, a race with
  another start, a runtime refusal that no longer holds — wake the todo's
  recorded orchestrator thread with the event's addresses, or, where none
  exists, leave the todo to the runner's next pickup once nothing it owns blocks
  it. A cause that recurs is `NC-40` or a defect.
- **Stop and settlement:** stop after the wake or after confirming the next pickup
  is unblocked. Settle when a session holds the work and records progress.

##### NC-38 — Stopped execution its recovery did not move

- **Evidence available:** the event, the session's state and closing messages,
  the todo's factual ledger and the recovery the runner already sent.
- **Ownership:** the todo's orchestrator owns recovery of its own work; a worker's
  failure is its orchestrator's evidence; the work record owns state.
- **Coordinator:** read why it stopped. Point a live orchestrator to the failure
  address; where the claim-holding session is gone and no recovery path exists,
  apply `NC-05`. A platform cause is reported through `general/procedure/bug-reporting`.
- **Stop and settlement:** stop after one pointer or the report. Settle on recorded
  progress, a grounded deliberate wait, or terminal state.

##### NC-39 — A session waits for input nobody is asked for

- **Evidence available:** the `runner.attention.input_waiting` event, the session's
  pending request and recent conversation, and the todo's person wait as work
  decides it.
- **Ownership:** a runtime permission or approval prompt is the operator's to
  answer; a question an agent asked belongs to the owner its contract names.
- **Coordinator:** diagnose what the session waits for. A stalled permission or
  tool-approval prompt is shown to the operator in the coordinator's thread with
  the session's address and what it asks; it places no hold, and the todo stays
  runnable, so the coordinator does not rest while it waits. A question an
  agent can answer goes to
  that owner under `NC-29`; a person-owned question goes to the work's origin
  under `NC-46`.
- **Stop and settlement:** stop after routing. Settle when the session resumes or
  the question reaches its owner.

##### NC-40 — A role's attempts keep failing

- **Evidence available:** the `runner.attention.attempts_failing` event, both failed
  attempts' reasons, heads and workspaces, and the orchestrator's account.
- **Ownership:** the todo's orchestrator owns recovery and whether to change the
  approach; the work record owns attempts.
- **Coordinator:** distinguish a repeated environmental or platform cause from a
  work cause. Point the orchestrator to both failure addresses when its next
  attempt would repeat the same premise; report a platform cause through `general/procedure/bug-reporting`.
- **Stop and settlement:** stop after one pointer or the report. Settle on a
  changed approach, a succeeding attempt, or a grounded stop.

##### NC-41 — A hold placed or released

- **Evidence available:** the hold event, the hold's source and reason, the
  todo's record and the coordinator's own open returns.
- **Ownership:** the hold's source owns it; the Coordinator owns the holds it
  placed for a settlement, and releases a manual hold whose stated condition
  outside the project's todos is met (`sdlc/policy/todo-creation-discipline`).
- **Coordinator:** a release of a hold you placed is read under `NC-48`. A hold
  with a governed gate is `NC-15-a`, a manual hold whose reason names a
  condition outside the project's todos and the evidence that releases it
  `NC-51`, and one with no discernible owner `NC-15-b`. A hold that leaves no
  unheld todo in the project is the coordinator procedure's rest condition.
- **Stop and settlement:** settles on reading; nothing is owed for a valid hold.

##### NC-42 — Stale thread of delivered or withdrawn work

- **Evidence available:** a todo in a terminal state, its recorded orchestrator
  thread still active, and its open obligations.
- **Ownership:** finishing a terminal todo's thread is owed by the
  terminal-finishing rule of `sdlc/policy/todo-creation-discipline`; the
  Coordinator executes it when its owner left it undone.
- **Coordinator:** after confirming the todo is terminal and the thread holds no
  unfinished human or worker obligation, finish it through the configured
  `runtime.finish_thread` action. Never finish another standing agent's thread.
- **Stop and settlement:** settle when the thread is finished.

##### NC-43 — Agents confused about who does what

- **Evidence available:** both sessions' records and messages, and the contract
  addresses that assign the act.
- **Ownership:** the role the contract names.
- **Coordinator:** ask each session how its claimed act reconciles with the
  ownership address, as in `NC-02-d`.
- **Stop and settlement:** stop after the pointers. Settle when one owner acts and
  the other stands down.

##### NC-51 — A hold waits on a stated condition outside the todos

- **Evidence available:** the manual hold's reason, time and recording session
  (`instrukt todo show <slug>`), and the record at the address the reason
  names: a branch's merge into the canonical branch, a published release, an
  external gate's recorded state, or a person's recorded decision.
- **Ownership:** whoever the reason names satisfies the condition; whether it
  is met is `not applicable — mechanically determined by the record at the
  address the reason names`. The Coordinator releases the hold, expressly
  authorised by `sdlc/policy/todo-creation-discipline`; the todo mechanism is
  the state authority.
- **Coordinator:** read that record on the event that placed the hold, on a
  message that the condition is met, and on every later wake before resting.
  When it shows the condition met, release the hold with
  `instrukt todo update <slug> --release` and inspect the outcome; the todo
  returns to the workflow's other gates. While it shows the condition unmet,
  leave the hold. A message alone, the hold's age, or a reading that the
  condition stopped mattering releases nothing. A reason that names another
  todo's delivery is a misplaced prerequisite: point the hold's recording
  session at the placement rule of `sdlc/policy/todo-creation-discipline`. A
  reason that names no readable evidence is `NC-15-b`; a hold you placed for a
  settlement is `NC-48`.
- **Stop and settlement:** settle on the release, or on reading the condition
  unmet; the next wake reads it again.

#### Responsibility 7 — Converging work and origin routing

Which of these cases applies is the coordinator's reading of a raised
condition; the raise does not say. Each return carries the shared surface and
a recommended resolution, never what a person said in another conversation
(`general/principle/origin-routing`).

##### NC-44-a — One capability framed differently by different people

- **Evidence available:** both todos' intent, the shared capability, and each
  todo's creation record.
- **Ownership:** the people who commissioned them decide, through their origins.
- **Coordinator:** pause only the overlapping build work, naming the release, and
  return the shared surface with a recommended resolution to each origin.
- **Stop and settlement:** wait for a settlement under `NC-48`.

##### NC-44-b — Compatible extensions of one subsystem

- **Evidence available:** both todos' plans or work, the shared subsystem and its
  contracts.
- **Ownership:** the Coordinator aligns the boundary; each orchestrator keeps its
  own work.
- **Coordinator:** send each claimant the boundary by address and declare the
  dependency that orders them (`instrukt todo update <slug> --after <other>`).
  Nobody else is told.
- **Stop and settlement:** settle when the dependency is recorded and both
  owners work to the boundary.

##### NC-44-c — Incompatible behaviour on one surface

- **Evidence available:** both todos' specified behaviour for the surface, and each
  creation record.
- **Ownership:** the people concerned decide, through their origins.
- **Coordinator:** pause the conflicting work and return the contradiction with a
  recommended resolution to each origin.
- **Stop and settlement:** wait for a settlement under `NC-48`.

##### NC-44-d — Unrelated work contending for the same code

- **Evidence available:** the todos' changed paths, branches and candidates.
- **Ownership:** the integrator's gate settles what reaches `main`; the
  Coordinator orders work only where contention would waste it.
- **Coordinator:** declare a dependency where one todo must land first; leave the
  rest to the integrator. Nobody else is told.
- **Stop and settlement:** settle on the recorded order or on finding none needed.

##### NC-44-e — Agents filed the same thing more than once

- **Evidence available:** both filings, their creation records and inputs; neither
  carries a person's request.
- **Ownership:** the Coordinator retires the redundant agent filing under
  `sdlc/policy/todo-creation-discipline`.
- **Coordinator:** choose the survivor by progress and completeness, write what
  the redundant filing carried that the survivor lacks into the survivor's
  `log`, withdraw the redundant filing and finish its orchestrator thread.
  Nobody is told.
- **Stop and settlement:** settle when one todo remains and its log carries the
  folded evidence.

##### NC-45 — An existing solution found while work is under way

- **Evidence available:** the raise, the solution's documentation, and what the
  todo builds.
- **Ownership:** the person who commissioned the work decides whether to adopt
  it, through the origin.
- **Coordinator:** pause only the affected build work and return the finding with
  a recommendation to the origin.
- **Stop and settlement:** the work resumes, changes or retires on the settlement
  (`NC-48`).

##### NC-46 — A person's decision no conversation of the work can ask

- **Evidence available:** the raise or the session's question, and the contract
  that makes the decision a person's.
- **Ownership:** the person, through the origin.
- **Coordinator:** return the question with a recommended answer to the origin;
  pause only what cannot proceed without it.
- **Stop and settlement:** settle on the origin's answer under `NC-48`.

##### NC-47 — A worker's question its orchestrator cannot answer

- **Evidence available:** the orchestrator's raise, the worker's question by
  address, and the contracts it touches.
- **Ownership:** the owner the governing contract names; a person only when
  `general/policy/human-contract` assigns it.
- **Coordinator:** answer it by address when a contract settles it (`NC-29`);
  route it to the owning role otherwise; return it to the origin under `NC-46`
  when it is a person's.
- **Stop and settlement:** settle when the orchestrator has the answer or the
  question has reached its owner.

##### NC-48 — A settlement returned

- **Evidence available:** the outcome recorded in the `log` of any todo the
  return concerns by its actual originating session (or confirmed directly in
  that originating conversation as verified through `runtime.read_thread` when
  the origin cannot mutate the todo directly), or — where the platform operator
  is the origin — the operator's answer in the coordinator's thread, together
  with the open return it answers, the holds waiting on it, and the todos
  concerned (`sdlc/policy/todo-creation-discipline`). A peer message alone only
  points where to look.
- **Ownership:** the people decided; the origin records or confirms the
  outcome, and any changed intent lands in `input` in clean, publishable form
  (written directly or recorded by the retained claimant after verifying the
  originating or coordinator thread under
  `sdlc/policy/todo-creation-discipline`); the Coordinator applies the rest
  within that policy.
- **Coordinator:** apply it to every todo it names, one or several: release the
  pause of a todo that resumes (recording the publishable confirmation in `log`
  when the operator settled it), resume a changed todo once its `input` carries
  the adopted outcome (and the publishable `log` note when the operator settled
  it), fold a superseded todo into its survivor and withdraw it, withdraw a
  todo retired outright, record a boundary or dependency the outcome sets,
  release the holds waiting on it once prerequisite record updates are in place
  and send the closure notices (`sdlc/procedure/lifecycle/coordinate`),
  including the withdrawal of another origin's todo, whose origin receives the
  closure notice. An outcome neither an actual originating session nor the
  operator-origin gave is no settlement.
- **Stop and settlement:** settle when no todo the outcome retires, hold
  waiting on it or open return remains and every other affected origin has its
  notice.

##### NC-49 — A technical coordinator's live-overlap consult

- **Evidence available:** the question, the todos it names, and their live sessions.
- **Ownership:** the technical coordinator keeps product and stakeholder decisions.
- **Coordinator:** answer the one question from live records and sessions; add no
  recommendation on intent unless asked.
- **Stop and settlement:** the answer settles the consult.

##### NC-50 — An attention episode ended

- **Evidence available:** the `runner.attention.fleet_quiet` event and the
  project's current todo list.
- **Ownership:** the runner established that no unheld work remains; the
  Coordinator owns only its own rest.
- **Coordinator:** confirm from `instrukt todo list` that no todo is active,
  unheld and not a structural holder, then rest
  (`sdlc/procedure/lifecycle/coordinate`). Work that has become runnable again
  since the event is read as any other condition.
- **Stop and settlement:** settles when the thread rests.

### Control and boundary cases

##### NC-34 — Live Human-owner collaboration

- **Evidence available:** active direct conversation, the session/phase record that
  identifies its current owner, and the question the owner is waiting on Human for,
  when there is one.
- **Ownership:** the live owner named by that record holds judgment and execution; its
  normal lifecycle mechanism owns state.
- **Coordinator:** while the collaboration holds what `general/policy/human-contract`
  assigns to Human, leave it whole — no duplicating, summarizing, filtering,
  challenging, or rerouting unless Human explicitly queries the coordinator
  interface. When the owner waits on Human for a judgment that contract leaves with
  agents, the collaboration is not holding Human's attention for him: apply `NC-29`
  to the owner, in that session.
- **Stop and settlement:** leave a collaboration on Human's own decisions alone and
  stop; reassess only on material state change or an explicit query, and read no
  meaning into conversation silence. An agent-owned wait settles under `NC-29`.

##### NC-35 — Human queries Coordinator during another live collaboration

- **Evidence available:** explicit human query, source-linked fleet records, and the
  boundary of the active owner conversation.
- **Ownership:** Coordinator judges fleet-status interpretation and executes the
  answer; no lifecycle state mutation occurs. The live owner retains its underlying
  work judgment.
- **Coordinator:** answer only the explicit question from source-linked evidence,
  without expanding it into a general fleet report, injecting the answer into, or
  rerouting the active owner session.
- **Stop and settlement:** answer delivery settles only the interface query.

##### NC-36 — Healthy execution: nothing owed

- **Evidence available:** session-linked, contract-specific work-produced evidence in
  the expected sequence—phase output, artifact/version, review finding, state-machine
  transition, or commit tied to that session. Timestamps and quiet alone are not
  progress.
- **Ownership:** the live owner named by the current phase contract retains judgment
  and execution; its lifecycle mechanism owns state.
- **Coordinator:** no intervention and no log line. Healthy work is read from the live
  sources when needed, not accumulated.
- **Stop and settlement:** the current observation settles as nothing owed. A later
  material change is a new observation.

---

## Why

The loaded lifecycle contracts describe how each role _executes_; they do not
say how an outsider reads its session states. That a `todo-orchestrator` alive
and idle means unfinished work, or that one parked at the PRD
gate is waiting correctly, is observer knowledge and is not derivable from
those contracts — so it is recorded here, once, instead of being re-derived
at each wake or hand-copied into the procedure. Cases are recognition semantics,
not a checklist to run: most sessions match no case and need nothing.
