---
eval: true
description: 'Plan drafting phase for prepare. Produces implementation plans from the approved intent artifact and the optional reviewer-approved architecture (thin index artifact plus the doc snippets it points at).'
visibility: 'public'
role: planner
parameters:
  - name: slug
    required: true
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: slow
---

# Prepare Plan Draft — Procedure

## Required reads

- @~/.instrukt/docs/sdlc/bundle.md
- @~/.instrukt/docs/sdlc/policy/preparation-artifact-quality.md
- @~/.instrukt/docs/sdlc/policy/quality-predicate-registry.md
- @~/.instrukt/docs/sdlc/policy/substrate-anchoring.md
- @~/.instrukt/docs/sdlc/principle/module-depth.md

## Goal

Your focus this turn is the plan phase: produce an `implementation-plan`
artifact from the approved intent artifact and the reviewer-approved architecture
when present (the thin index artifact plus the doc snippets it points at); or
split the todo into dependent child work items when planning shows the work is
not atomic. The wider prepare lifecycle is still yours; this phase is one
focused part of it.

### Outcome

Produce `implementation-plan` from the approved intent artifact (Preconditions 1)
and the optional reviewer-approved `architecture` (the thin index artifact plus the
domain-spec / design-doc snippets it points at); or split the todo into
dependent child work items when step 2 finds the work is not one coherent delivery.

The plan must be detailed enough for a builder to execute without guessing, and
review-aware enough that the reviewer finds no surprises. Whether the work stays
one todo is the scope-and-size gate's decision
(`sdlc/policy/definition-of-ready`), applied in step 2.

## Preconditions

1. The approved intent artifact resolves — `prd` for a `feature`, `input` for a
   `change` or `bug` (see `sdlc/concept/approved-intent-artifact`). A
   `change` has no PRD by design; `input` is its intent. Every kind retains
   `input`, including a change with a seeded `implementation-plan`; the seed
   is a technical hypothesis rather than an alternate intent authority.
   A feature PRD has current independent review and the configured human or
   peer approval before this phase. The position-based intent check binds the
   current `input`, not a derived requirements or plan artifact.

2. `architecture` is either reviewer-approved or explicitly skipped.
3. Slug is active (not icebox, not delivered).

## Steps

Use `instrukt <command> --help` for individual commands.

First read the sdlc principles that apply to this work, as `instrukt docs get --domain sdlc` lists them, with `instrukt docs get`.

### 1. Read and ground

Apply the artifact-discovery rule from `preparation-artifact-quality`
(`instrukt todo artifacts list {slug}`) before assuming the list below is
everything present — read `log` when it exists, and any other artifact this
todo's history left behind that bears on grounding.

Read the full context before writing anything:

- The resolved approved intent artifact. For a feature, read the independently
  reviewed and approved `prd`; for a change or bug, read `input`. Read a
  seeded plan's _Intent & scope_ as a claim to check against that input.

- `instrukt todo artifacts read {slug} architecture` — when present, the thin
  index/log; otherwise read the skip reason. Read the domain-spec / design-doc
  snippets it points at for the actual design content.
- `instrukt todo artifacts read {slug} input` — the authorized basis. For
  feature/change it carries authorized product intent; for bug it carries the
  reported concern rather than an agent-added cause or remedy. A supplied plan or input may remain a technical
  hypothesis, but its unsupported business constraints are not approved scope.
- `instrukt todo artifacts read {slug} diagnosis` — when present (a `bug` carries
  this). It holds the verified, code-grounded root cause — the causal mechanism —
  and nothing more. For a `bug` the plan derives the fix that **removes** that
  cause and the verification that proves it gone, grounded in the root cause and
  the code — reference the diagnosis's root-cause section as the grounding rather
  than re-deriving the cause. Consume it exactly as `architecture` is consumed when
  present; no kind-specific branching beyond reading the artifact. When the root
  implicates a shared contract, its implementation census is the complete starting
  denominator for the plan's technical change surface: every listed implementation
  must be accounted for even when only some require edits.
- `instrukt todo artifacts read {slug} investigation` — when present (a `bug` carries
  this). The evidence substrate behind the diagnosis: reproduction, avenues walked,
  observations. Read it for grounding context; the plan consumes the `diagnosis` root
  cause, not the investigation.
- The codebase: files that will be affected, adjacent patterns, existing implementations
  of similar functionality. Use a filtered `instrukt docs index --areas policy,spec` (add
  `--domains <domain>` to narrow) to surface relevant policies and specs.
- Definition of Done — every plan task must produce output that satisfies DoD gates.

Walk the governing-contracts entry points before drafting begins
(`sdlc/policy/preparation-artifact-quality`, Governing contracts). Where an
approved architecture carries the Governing contracts section, the plan
references it; otherwise the plan carries that section itself.


### 1b. Re-entry classification from the current obligation

Re-entry is the base case for plan drafting — fresh production is the special
case (first time only). Read the action and rendered template returned by
`instrukt todo prepare {slug}`, then ground that obligation in current
artifacts, findings, provenance and stamps. Wording belongs to the flavor's
actions and templates; no dispatch-reason header or token selects behavior.

- **Fresh derivation** — no prior implementation plan exists. Produce one
  from the approved intent artifact and any reviewer-approved architecture.
  Run the full atomicity decision (step 2) and draft (step 3).
- **Seeded derivation** — a supplied implementation plan is present at first
  entry with no prior review. Treat it as the draft basis, not an oracle:
  run grounding (step 1), atomicity (step 2) and blast radius (step 2c), verify
  every task against the code, input and domain specs, and record divergences
  under `preparation-artifact-quality`. Reflect doc snippets (step 4b), add
  their references and blast-radius-informed verification, and refine the
  seed in step 3. It skips neither grounding nor plan review.
- **Substantive re-derivation** — the current record invalidates the plan's
  grounding. Read the existing plan and examined evidence, re-derive affected
  tasks and preserve unaffected tasks. Distinguish changed code from changed
  upstream artifacts. Skip atomicity and continue at step 4c.
- **Kickback** — read the work orchestrator's recorded reason in `input`
  first. It names why the build could not execute the plan and how far back
  re-derivation reaches: a localized replan or the feature-spec scenarios it
  binds. Re-ground against that reason as well as code. A new input version
  requires a current position-based intent check.
- **Revision answering findings** — read `plan-review-findings` alongside
  the existing plan. Apply the validation, disposition and evidence rules of
  `sdlc/procedure/lifecycle/fix-review-findings`, preserving unaffected tasks
  and carrying changed premises through dependent statements. This procedure
  retains artifact writing and advancement. Skip atomicity; continue at 4c.
- **Changed upstream evidence** — compare retained versions with
  `instrukt todo artifacts diff {slug} <artifact> <old-digest> <new-digest>`
  using the plan's provenance and current artifacts. Update tasks grounded in
  changed upstream content and preserve the rest. Skip atomicity; continue
  at 4c.
- **Missing provenance** — read the existing plan and verify each task
  against current input, PRD and architecture as applicable. Update only
  grounding that has actually drifted. Skip atomicity; continue at 4c.

### 1c. Situate the premise in the existing todo landscape (fresh derivation only)

Applies only on fresh derivation, for `change` and `bug` — a
`feature` already does this at intake step 1.6 before it ever reaches this
phase. Before drafting, compare this premise against the existing todo
landscape: the same collecting discipline as intake step 1.6, pointed at the
todo list instead of the domain, and distinct from a `bug`'s diagnose Step 1
(which retires an exact-defect duplicate earlier in the lifecycle, before this
phase runs) — this step catches broader scope overlap with _unrelated_ todos,
not same-defect duplicates.

**Scan** the active and prepared-unclaimed todos (`instrukt todo list`). Keep it
bounded: that set only — delivered, removed, and iceboxed todos are out of
scope. Read cheap signals first (slug, then `input`), reaching for a
candidate's deeper artifacts only when it looks genuinely related.

**Judge the relation and its direction** — precedence is default, intent is
the override:

- **This premise yields** — redundant with an existing todo. Descope the
  overlapping portion and note the boundary in the plan's _Intent & scope_;
  for a true duplicate of intent, surface it to the human rather than
  silently proceeding or dropping.
- **This premise supersedes** — replaces, broadens, or obsoletes an existing
  todo. Act by that todo's state: a living orchestrator gets a best-effort
  informational message (`instrukt sessions send`) plus a surface to the human;
  a dormant todo gets surfaced and optionally frozen (`instrukt todo freeze`,
  reversible); a todo already claimed and building is halted only on
  certainty — below that bar, surface to the human and let it proceed.
- **Undeclared dependency** — this premise needs another todo's deliverable
  never declared as `after`. Record the edge:
  `instrukt todo update {slug} --after {dep}`.

**Fold the findings into the plan**: overlap into _Intent & scope_, declared
dependencies alongside the edges recorded. This step produces no artifact of
its own — its output is the plan's scope boundary and any recorded `after`
edges.

### 1d. Make no-PRD intent reviewable

For a `change` or `bug`, the plan is the first prepared artifact where intake-
quality scope reasoning can survive. Its _Intent & scope_ section therefore
states:

- the outcome horizon authorized by `input` and, for a bug, sharpened by the
  diagnosed root cause;
- what makes the present outcome coherent in this situation;
- consequential inclusions and exclusions with inspectable premises or named
  assumptions;
- future concerns retained only when they constrain today's design or carry a
  meaningful deferral trigger;
- the observable outcome the plan's tasks must deliver.

Apply the same four open shaping questions used for a PRD, but never use the
plan to acquire product authority the input does not carry. A discovered
product outcome beyond the authorized horizon returns to the feature intent
path. For a bug, betterment may make the root-cause removal coherent and
durable; it does not add adjacent hardening or capability.

The plan may be technically richer than the input where safe, correct
implementation requires it. It may not turn that technical governance into a
new product or business limit the input does not carry. If
another reasonable implementation could satisfy everything the human said
while omitting or varying a proposed business detail, the detail remains
unspecified and outside _Intent & scope_.

### 1e. Carry a structural decision no reviewed architecture carries

When the plan makes a structural decision that no reviewed architecture
carries — in a `change` or `bug`, or in a `feature` whose architecture was
skipped — it carries the architecture reasoning itself
(`sdlc/policy/preparation-artifact-quality`, Structural decisions carry
architecture review): the governing paradigms with every operation callers see
placed in them, the alternatives with the paradigm-native one among them, and
the durable design authored into its snippets through pending carriers naming
this slug, as `sdlc/procedure/lifecycle/prepare/architecture` routes them. A bug
whose `diagnosis` names a prescribing decision that has failed this way before
reopens that decision here, and every plan whose diagnosis names one records
the failure in that design's `Failure modes`.

### 2. Decide whether the todo is atomic

Apply the scope-and-size gate in `sdlc/policy/definition-of-ready` to the
grounded change surface and record the verdict before drafting. That policy owns
the splitting criteria.

**If the work fails the scope/size gate, stop here.** Do not draft a plan for
work that will fail gate validation. Group first:

- read the parent's full authorized scope from its resolved intent artifact and any approved architecture context
- create each child with `instrukt todo create <child> "<child-content>" --kind <k> --approved --group <slug>`, partitioning the parent's existing scope — adding no functional or business scope absent from the parent
- act on a child only once its filing outcome reports it created
- narrow each child to its scope slice, referencing that parent intent and any applicable approved architecture context
- add dependency ordering between the children with `instrukt todo update <child> --after <dep>`
- stop treating the parent as directly prepare/work-ready; it is now a structural holder

If the work passes the scope/size gate, continue to plan drafting.

### 2c. Derive the change's blast radius (before drafting)

Follow `sdlc/procedure/blast-radius-discovery` for every file
this plan intends to change. Ground the plan's change surface and test
adjacency in the verified candidate set it produces — if the procedure reports
the Blueprint unavailable, state the gap in the plan and fall back to manual
exploration; a stale answer is still used, with its named range checked as that
procedure describes; never silently skip the step.

For a bug whose diagnosis identifies a shared-contract root, seed discovery also from
the contract owner and every producer, entry path, and consumer in the diagnosis's
implementation census — not only from files already expected to change. Reconcile
the intended change surface against every census entry: name how it uses the single
owner, why it must change, or why its evidence-backed independence leaves it untouched.
The outcome horizon remains the reported bug; widening the technical surface enough
to consolidate the rule into its established owner and adapt existing
consumers is causal completion, not adjacent refactoring. Moving an adjacent
owner's rule into the bug's current context is still scope and ownership creep.

### 2d. Reserve scenario test bindings on main when the project has tests

Before drafting the tasks in a project that opts into tests, reserve every
feature-spec scenario the plan will bind so a concurrent prepare cannot bind
the same scenario. Read the
current scenario inventory on `main` for each feature spec the plan touches — the
live scenarios via `instrukt docs get <id>`, and any pending claims via
`instrukt docs index --target <slug>` and `instrukt docs get <id> --target <slug>`.

Run `instrukt specs coverage` to inspect the inventory and existing bindings.
For every proposed binding, run
`instrukt specs reserve --slug {slug} --binding <spec_id>:<UC-id>` against the
on-main claims. This reports conflicts; the pending carriers below establish
the reservation.

For each scenario the plan will bind:

- **A scenario this todo authors** carries the claim as its own `planned:<slug>`
  block under the feature spec's `### Use cases`; the delivering todo is its binder.
- **An existing live-and-unbound scenario this todo will bind** is claimed by
  wrapping its `#### UC-…` block in a `planned-change:<slug>` that restates the
  scenario unchanged; the delivery fold clears the wrap and the test binding becomes
  the permanent record.
- **A scenario already reserved by another in-flight todo** — enclosed by that todo's
  pending marker on `main` — is not claimed again: reuse it by reference with no
  second binding, or author a distinct scenario. When both todos must author the same
  region, serialize with `instrukt todo update <this-slug> --after <owning-slug>` so this
  plan drafts against the owner's approved scenarios.

A scenario enclosed by no pending marker and bound by no test is unclaimed and free
to reserve. The plan's declared bindings are what the plan-review reservation check
validates against these on-`main` claims.

### 3. Draft the implementation plan

Apply the plan-specific quality rules from the preparation artifact quality
policy. Structure the plan as an ordered task list. Each task must include:

- **What**: the concrete change (file, function, component).
- **Owner**: the bounded context whose rule changes and the architectural layer
  inside that context. For a cross-context interaction, name the published
  command, query, event, or type at the boundary and keep each side's task
  owner-local.
- **Why**: the rationale for the approach. This prevents the builder from taking shortcuts.
  If the plan says "use adapter pattern" but doesn't say why, the builder may inline the
  logic instead. The "why" is the guard against drift.
- **Test binding, where the project opts into tests**: the scenario-bound test
  the task will add or update under the project's testing contract. Do not
  prescribe logger/config/native-shape/private-helper/wiring assertions as
  substitutes for behavior.

- **Referenced files**: exact file paths the grounded plan expects to create or
  modify. These are used by the fact provider for grounding and staleness detection.
- **Referenced paths contract**: author exactly one machine-readable
  ` ```referenced-paths ` fenced block in the plan body, listing every file
  the grounded plan expects the work to create or modify, one path per line. The
  block anticipates the work and is not an allowlist of builder edits
  (`sdlc/policy/preparation-artifact-quality`). This block is the only code
  drift surface the fact provider reads. File paths mentioned in prose may help
  the builder, but they are not a drift contract unless they appear in this
  block.

Apply the structural ratchet (`definition-of-done` §2) to the change path
derived in step 2c. Where a preparatory deepening meets its five conditions —
behavior-preserving, owner-local, contract-preserving, one coherent reviewable
commit, within the todo's cost cap — add it as its own task before the
behavioral task it prepares, anchored to the module it deepens per
`substrate-anchoring`. Where it does not, record the deepening candidate and
the reason for deferral in the plan, so deferral processing decides it rather
than losing it. A plan with neither on a change path that carries structural debt
is incomplete.

Verify plan tasks against the domain specs loaded in step 1. Plans that reference
non-existent APIs, wrong schema fields, or incorrect directory structures are
defective — the builder will discover the error at build time, wasting a session.
Plans that place a rule in its caller, adapter, runner, scheduler, shared
utility, or current file location without establishing that context as owner
are also defective, even when the proposed layer is otherwise correct.

### 3b. Intelligence-boundary check

Per `principle/intelligence-boundaries`, scan every task you've drafted with
this question: does the task prescribe code that classifies, scores, judges,
evaluates, or auto-edits content semantics? If yes, that task is intelligence
in the wrong layer — the rule belongs in the agent's briefing, not in code
the agent wraps. Strike the task, or restate it as a briefing change.

Detection patterns to refuse in plan tasks:

- "Build a classifier / scorer / validator / checker for [free-text content]."
- "Add a regex / vocabulary table / word-count threshold that decides whether
  agent output is meaningful / actionable / complete / on-topic."
- "Auto-edit / auto-fix / auto-rewrite agent artifacts to match a structural
  rubric."
- Predicates on natural language: `is_meaningful`, `should_emit`,
  `is_actionable`, `is_ack`, `classify_*` on free text.

The defining test: would this task be unnecessary if the agent's briefing
said the right thing? If yes, the task is policy in the wrong layer — fix
the briefing instead.

### 3c. Bug-scope clamp (kind == bug)

A bug plan removes the diagnosed mechanism (`diagnosis#mechanism`) where the
diagnosis places it — the reproducing site and, for a shared-contract root,
every entry in its implementation census — with the smallest change that does
so, plus a scenario-bound regression test where the project opts into tests.
The family census (`diagnosis#family-census`) stays a record in the diagnosis;
its sites outside that boundary are outside the plan, and a family site enters
the lifecycle only on evidence of its own (`sdlc/concept/defect-evidence`,
family site). Except for a structural-ratchet deepening permitted below, every
task traces to removing that mechanism. Guards against causes no diagnosis has established — extra
validation layers, defensive defaults, fallbacks — and adjacent refactors, new
capability, or any scenario beyond the single regression scenario below, are
scope violations to strike before the plan is written, regardless of how
prudent they look. Tracing a task to a scenario or
snippet this same prepare pass authored does not create scope; the `diagnosis`
and the `input` symptom are the only scope authorities for a bug
(`sdlc/procedure/debugging`, step 5).

State the operation's observable outcome and relevant effects after the fix
under the diagnosed triggering conditions. If the settled diagnosis cannot
support this outcome, return the gap to diagnosis rather than substituting a
narrower outcome. Record any unresolved part of the
reported concern as undelivered; naming follow-up work does not discharge it.

A structural-ratchet deepening on the bug's direct change path is authorized
HOW when all five ratchet conditions hold (`definition-of-done` §2). Other
structural work the investigation surfaced remains out of scope: record it as a
deferral with its reason; it never rides the fix.

**Regression scenario — the only scenario authoring a bug plan owns.** When the
reported symptom has no matching scenario in the owning feature spec, author
exactly one pending scenario encoding the reported symptom as
`Given / When / Then` — nothing adjacent, nothing speculative. When a bindable
scenario already exists, the plan binds it and authors nothing. When the fix is
a contract-defining declaration with no bindable scenario
(`sdlc/policy/testing`), there is correctly no test and no
scenario.

**Doc reflection for a bug defaults to none — the regression scenario excepted.**
The single regression scenario above is the one snippet addition a bug plan may
carry without further justification. Beyond it, a fix that restores code to the
documented contract has no behavioral delta; record
`doc_impact_none: fix restores documented contract` in step 4b instead of
authoring new contract text. Only when the diagnosed cause is that the
documented contract itself is wrong does any other snippet change enter the
plan — bounded to correcting that contract.

An ambiguous or stale symptom is resolved by the `diagnosis`; only an ambiguity
in the intended behavior itself is held as one question for the human
(`general/policy/human-contract`) — never resolved by interpretation (Recovery 1
does not apply to a `bug`).

### 4. Anticipate review lanes

Apply the DoD-driven review-awareness rule from the preparation artifact quality
policy. The plan must pre-satisfy what the reviewer will check — the reviewer's
lanes are the review-lanes catalog
(`sdlc/policy/definition-of-done-lanes`). Produce a complete
lane-coverage report in the plan: one line per lane in the catalog, marking it
triggered (naming how the plan satisfies it) or `N/A` (with reason). No lane is
omitted — silence is not coverage.

- Deliverable-producing lanes (tests, security, docs, config, browser/UI, ADR)
  are ticked by naming the task and the artifact it produces (the doc lane via
  step 4b).
- The tests line names the governing project testing contract and, where tests
  are opted into, the tasks that bind scenario-derived tests.
- Lanes that produce no artifact — `code` Duplication, `paradigm` Component
  reuse, and `simplify` — are ticked against the plan's own `referenced-paths`:
  confirm no task re-derives, mirrors, or re-implements logic an existing cited
  symbol already provides; where shared logic is needed, one task extracts it
  once and the others call it. A task that duplicates an existing symbol instead
  of reusing it is a defect to fix before the plan is complete. When a diagnosis
  carries an implementation census, the verdict also accounts for every census
  entry; limiting the check to the paths the plan chose would make omission pass
  as reuse. `simplify` is also ticked against the smallest alternative that
  reaches the same outcome through the system's existing paths
  (`sdlc/policy/substrate-anchoring`): name that alternative, and give a reason
  for every task, file, or mechanism the plan carries beyond it.

A plan that does not tick every lane will produce review findings that cost more
to fix than they cost to prevent. Ticking a lane never means adding work: for a
scoped change most lanes resolve to `N/A` with a reason, and an `N/A` line is
full compliance. A task added only to mark a lane satisfied is gold-plating —
the defect, not the cure.

### 4a. Propose standing-agent event subscriptions

For new standing-agent documentation only, when the events module is on,
propose in the plan which events wake which standing agent. Ground each
proposed subscription in the event contract and the standing role's
responsibility, and name the procedure that carries it. Use the project's
event vocabulary and keep source acquisition with the runner. With events
off, propose no subscriptions. This is a plan proposal, never a question to
the human.

### 4b. Reflect the behavioral delta in doc snippets (mandatory)

Per `sdlc/principle/documentation-first-development`, the prepare
phase authors the pending snippets that describe the work's intended state. A
`change` has no PRD, so this step owns that identification for the no-PRD lane
(and confirms it for a `feature` whose PRD already lists them):

- Identify every doc snippet (spec / principle / policy / concept / ADR) whose
  contract the work's behavioral delta changes or adds.
- Author or update each as a **pending** snippet carrying `delivered_by: [{slug}]`
  — in-context `<!-- planned:{slug} -->` blocks for additions, or a
  `<stem>.{slug}.md` companion for a wholesale rework (see the doc-snippet
  authoring procedure).
- Author no plan task for the snippets. Doc reflection is complete once prepare
  has authored them, and the integrator folds them at delivery; the work phase
  never takes the doc layer's state as work. A plan that carries a documentation
  task — including one that only confirms or verifies pending markers — has
  misplaced prepare's own work, and the plan reviewer rejects it.
- If the change genuinely has no documented-behavior impact, record an explicit
  `doc_impact_none: <reason>` note in the plan rather than silently omitting docs.
- If the exact snippet region that must change is enclosed by a declared
  dependency's pending marker, record
  `doc_deferred: <holding-slug> — <inline-snippet-reference>#<section>` in the
  plan and draft the replacement text directly beneath it. The holding slug
  must already be in this todo's `after` set. The snippet reference must use the
  validator-accepted inline form from `general/policy/referencing-doc-snippets`
  (`@docs/...` for a project snippet); it becomes referenced-snippet provenance,
  so the holder's fold raises upstream-snippet drift and routes this todo back
  to plan drafting. An undeclared holder is a collision to resolve, not a
  deferral, and no nested marker is authored.
- For a `bug`, apply the doc-reflection default from step 3c: the single
  regression scenario is the one sanctioned snippet addition; beyond it, a fix
  that restores the documented contract records `doc_impact_none` and authors no
  new contract text.

A change that alters documented behavior without a corresponding snippet update
(and without a `doc_impact_none` justification or structurally complete
`doc_deferred` record) is not plan-complete.

### 4c. Validate a revision

Before writing a revised plan, apply the task-composition and changed-premise
rules in `sdlc/policy/preparation-artifact-quality`. Refresh the complete
lane-coverage report and doc reflection under steps 4 and 4b.

### 5. Write artifacts

If the todo stays one delivery, write (using the approved intent artifact and the optional reviewer-approved architecture — the thin index plus the snippets it points at — as inputs):

- `instrukt todo artifacts write {slug} implementation-plan <content>` — the full plan with tasks, rationale, and verification.
  Each task references the intent criteria it delivers and, for Category A work, the feature-spec scenarios it satisfies (`sdlc/policy/preparation-artifact-quality`).

Do not write grounding fields directly. The work record captures provenance; the fact provider derives grounding
and current worktree facts.

If the todo is grouped instead:

- Create each child with `instrukt todo create <child> "<child-content>" --kind <k> --approved --group <slug>`, one per independently deliverable slice.
- Treat the parent as a stateless holder once children group-reference it. Its retained input artifact is frozen source material; do not plan follow-up writes against the holder.
- Each child's content carries the relevant subset of the parent's intent artifact and reviewer-approved architecture-snippet context.
- Treat only the children as prepare/work targets. The parent is a holder and must not be prepared or worked as an implementation item.

### 6. Enforce boundaries

- Deliver exactly the intent of the approved artifact (`prd` for a feature;
  the verified input basis, sharpened by `diagnosis`, for a change/bug) and
  preserve the existing documented and working behavior the change touches —
  see "Scope is bounded" in `preparation-artifact-quality`. A forced technical
  decision that stays within scope follows established behavior; one that would
  shift product or business scope is surfaced to the human.
- Do not rewrite approved or reviewer-approved upstream artifacts unless the plan is blocked by a genuine
  contradiction in those artifacts.
- Do not change todo status beyond the child dependency links needed for a split.
- If blocked by uncertainty, record the blocker and continue with what is known.

### 7. Final step — advance

When the implementation-plan artifact is written (or child grouping has been
executed via `instrukt todo create ... --approved --group` and each child's filing outcome
reports it created), call:

```
instrukt todo prepare {slug}
```

The returned action and rendered template carry the next obligation. Dispatch
or reuse the independent plan-review session through the sessions facade.
The review is done when the record carries the current findings artifact and
verdict stamp. Approval permits the next derived obligation; needs-work
returns here with `plan-review-findings` for fix, evidence-backed rejection,
or re-derivation under step 1b. Record the revision and call prepare again.

## Outputs

1. Atomic case:
   `instrukt todo artifacts read {slug} implementation-plan`.
2. Grouped case:
   child todos, dependency links, and a stateless holder row with frozen input only.

## Recovery

1. If the intent artifact is ambiguous, note the ambiguity in the plan and derive the most
   likely interpretation where it stays within existing authority, marking it
   `[inferred]`. An ambiguous product or business choice the input does not
   settle is returned as a human-owned decision instead of inferred. For a
   `bug`, ambiguity resolves through the `diagnosis`, or is held for the human
   only when it concerns the intended behavior, never by inference (step 3c).
2. If planning reveals the work is not one coherent delivery under the
   scope-and-size gate (`sdlc/policy/definition-of-ready`), group it into
   children.
3. If a file referenced in an upstream artifact does not exist, verify whether it should be
   created or whether the upstream artifact is stale.

## Discipline

Reserve every scenario binding on `main` before drafting tasks — plan-draft step
2d. Skipping it lets a concurrent prepare bind the same scenario in its own
worktree; the plan-review reservation check exists to catch that late, not to
excuse skipping the reservation early.
