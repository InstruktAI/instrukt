---
description: 'How software delivery applies documentation-first development: a todo carries its pending documentation in its work worktree, feature specs are the hub and domain specs the spokes, classification decides whether a feature spec is owed, and preparation authors directly to the docs layer.'
visibility: 'public'
---

# Documentation-First Delivery — Principle

## Required reads

- [general/principle/documentation-first-development](../../../general/principle/documentation-first-development.md)

## Principle

Software delivery applies documentation-first development through the todo. A todo authors pending documentation and implementation in its single `<slug>/work` worktree; target retrieval (`--target <slug>`) reads that worktree so workers see the pending contract before delivery. When the todo delivers, the integrator folds the pending markers into the current contract on `main` (clearing `binding_reservations` once the attached tests are present) without erasing unrelated current or pending contracts.

## Rationale

A todo's pending documentation sits beside the implementation that makes it current, so one worktree holds the design and the work that satisfies it, and its delivery folds both into `main` at once. The design is organized around the capability an observer experiences and the bounded contexts that realize it, so related deliveries read one shared design surface instead of one artifact per todo.

## Implications

### 1. Target retrieval answers what a todo owns

A worker operating on a slug runs `instrukt docs index --target <slug>` to see exactly which pending snippets it owns in `<slug>/work` (falling back to any legacy prepare branch during upgrade). Before delivery, target retrieval reads that worktree; after delivery, the documentation is folded and current on main. This is the canonical query that makes the doc-first discipline operational rather than aspirational: the worker asks the system what it has been put on the hook for, and the system answers.

### 2. A context rewrite is pending material on its domain spec

A context rewrite is expressed as pending changes to its domain contract (see `sdlc/spec/domain-spec`). The previous contract remains available on main until delivery folds the pending material; target retrieval (`--target <slug>`) surfaces the agreed new design in `<slug>/work` for planners, builders, and build reviewers. Publication records the agreed design; runtime evidence still establishes which implementation is running.

This is the structural reason a service rewrite is safe under this discipline: the domain spec is the shared design anchor; both the old implementation (in its declining role) and the new one (in its ascending role) must satisfy it; the cutover happens when the new implementation satisfies the same spec the old one satisfies.

### 3. Feature specs are the hub; domain specs are the spokes

Documentation organizes around two snippet kinds with one-directional flow:

- **Feature spec** (the hub) — describes owner-local behavior in business-value terms through its prose contract, adds Gherkin use cases when user goals warrant them, and points at the bounded contexts it touches. Lives under the context that owns the behavioral decision at `docs/project/spec/feature/<context>/<concept-name>.md`. Concept-named, NEVER slug-named — the snippet identity is the domain concept it describes, not the todo that authored or refined it. Other contexts may index the spec as consumers; that does not make them co-owners.
- **Domain spec** (the spokes) — describes a bounded context's ubiquitous language, aggregates, events, and design. Its Use cases section indexes the feature specs that touch this context.

Authoring flows from hub to spokes: an intake agent processes a human's input as a feature need (the hub), then identifies and updates the affected domain specs (the spokes). The reverse direction (spokes inferring hubs) is forbidden — domain concepts don't carry business value standalone; that's the feature's job.

#### 3a. Not every todo deserves a feature spec — classify first

Feature specs exist for **behavioral capabilities** — behavior some observer
experiences. They are NOT the default output of every prepared todo. The
classification is one question, asked at intent entry: does this change alter
what an observer can experience of the system?

| Classification     | What it is                                                                                                                                                                  | Durable doc home                                      | Feature spec? |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | ------------- |
| **Behavioral**     | Behavior an observer experiences. The observer may be a human (a delivered message, a rendered screen) or an agent (a CLI surface, an event, a structured decision record). | `docs/project/spec/feature/<context>/<concept-name>.md` | **YES**       |
| **Non-behavioral** | Refactor, migration, tooling, infrastructure with no contract delta — nothing any observer experiences differently.                                                         | None — workflow.artifacts only                        | **NO**        |

A feature spec states its contract in prose: rules, invariants, refusal
conditions and tables. Use cases are a separate, narrower question. A use case
exists only at user-goal level: a goal a stakeholder of the product achieves in
one sitting, stated as the stakeholder would state it, sized so the product
owner can accept or reject a release on it. The stakeholder is a person, or an
agent acting for a named person; the platform's own agents coordinating with
each other are machinery, not stakeholders. Subfunctions (the steps inside a
goal, such as setting a value, listing records or refusing invalid input) and
internal mechanics get no use case, and a refusal, an ordinary outage, or a failure with a recorded occurrence or
dangerous path (`sdlc/concept/defect-evidence`) that threatens a goal is an
extension of that goal's use case. Most behavioral changes therefore
refine a spec's prose contract and add no use case. The reason is that a use
case is what a product owner accepts: below goal level, scenarios restate
mechanics, multiply with every change to the steps, and turn the specs into
administration nobody reads. Silence on the classification question means
unclassified — it is asked, never defaulted.

A non-behavioral todo that produces a feature spec is a defect. The namespace
becomes polluted with slug-named non-features, delivery reconciliation
fires on docs nobody needed, and future readers can no longer tell
which feature specs describe real capabilities.

#### 3b. Concept-name derivation (behavioral work only)

The `<concept-name>` in the feature-spec path is the durable domain noun
for the capability — short, noun-shaped, descriptive of what users get,
stable across multiple deliveries that may shape it. Derived from the
capability, NEVER from the todo slug.

- RIGHT: `docs/project/spec/feature/agent_runtime/session-revival.md`
- RIGHT: `docs/project/spec/feature/workflow/multi-pillar-split.md`
- WRONG: `docs/project/spec/feature/<context>/<v2-cutover-slug>.md` (todo slug verbatim)

**Smell test.** If the proposed concept name equals, contains, or is an
obvious rewording of the todo slug, you have not named a concept — you
have copied a slug. Stop. Either re-derive the concept name from the
durable capability, or recognize the work is non-behavioral and produce no
feature spec at all.

**Tie-breaker — classification is asked, not guessed.** A spurious feature
spec is worse than a missing one: it lies to future readers about what's a
real capability. But silence does not classify — when the behavioral question
has no clear answer, ask it of the intent owner rather than defaulting either
way. A missing spec for true behavioral work surfaces as a gap that the next
delivery in that capability fills naturally.

**Pending carriers are delivery association.** Pending carriers associate intended changes with their deliveries. One delivery may affect several snippets, and several deliveries may affect one snippet under the authoring procedure's ownership and ordering rules.

**The architecture design lives in doc snippets; the `architecture` workflow.artifact carries delivery reasoning.** Design decisions that survive delivery land in docs across two locations:

- **Per-context architectural changes** (new aggregates, new events, refined invariants, internal model adjustments) → the affected domain spec snippet at `docs/project/spec/domain/<context>.md`
- **Cross-cutting architectural changes** (new bounded context, communication patterns, transport rules, context-map shifts, deployment shape) → the architecture design docs at `docs/project/design/architecture/*.md`

The doc-snippet updates ARE the design record — they are authoritative, and the todo's own documentation changes establish which snippets changed. The `architecture` workflow.artifact carries delivery reasoning and ephemeral tactical decisions (library selection, internal build sequencing); it carries no design body and is never the source of truth. There is no `architecture` snippet kind. The architecture phase routes per change: internal-to-context → domain spec; cross-cutting → design doc; or both when a change has both faces.

**PRDs do not survive as snippets.** The PRD captures work-guidance for one delivery (MoSCoW, Why, risks). It is scaffolding. It stays a workflow.artifact and retires after delivery. The behavioral contract (feature-spec prose and any admitted Gherkin use cases) and the design (updates in domain specs) become docs.

### 4. Prepare workers author directly to the docs layer

The intake and architecture phases of the prepare lifecycle write DIRECTLY to docs — feature spec snippets (intake), domain spec snippets and architecture design docs (architecture). No design-bearing workflow.artifact that later "migrates." The doc IS the source; the architecture phase additionally writes a delivery-reasoning artifact.

**Admitted use cases are written in Gherkin in the feature spec.** Intake authors the business value and prose contract, adding Gherkin only for user goals admitted by `sdlc/procedure/feature-spec-authoring`. A behavioral change that adds no user goal can refine that contract without adding a use case. There is no separate `test-specs` phase.

Ephemeral workflow.artifacts (PRD, implementation plan, review findings, drift analysis, deferrals) remain in `workflow.artifacts` because they're process scaffolding, not behavioral contract.

`todo-orchestrator` routes preparation primitives in prose inside `<slug>/work`; each primitive's output lands in pending docs (or `workflow.artifacts` for scaffolding). A primitive with nothing to author (e.g., architecture on a Gherkin-only feature enrichment) is skipped or produces no doc change cleanly.

### 5. Multi-pillar features group at intake

A feature whose acceptance scenarios cross multiple bounded contexts (pillars) gets handled by the intake phase via proactive grouping:

1. Intake identifies which contexts the feature touches (the pillars).
2. If the per-pillar slices deliver independently (the typical case), intake creates a container todo and per-pillar children via `instrukt todo create <child> "<content>" --kind <k> --group <container>`. Each child owns its own feature spec(s) 1:1 — there is no shared feature spec across children. Each child carries its own PRD workflow.artifact scoped to its slice.
3. Children prepare and deliver independently in their own `<slug>/work` worktrees. Each child's pending material on its own spec folds to `main` when that child delivers through `integrate`.
4. If the slices MUST land together (e.g., a new event taxonomy where every consumer needs the coordinated update on the same day), intake keeps a single todo with multiple owner-local feature specs and per-context commits. Atomic delivery coordinates the owners; it never collapses them into one shared contract. Independence is the discriminator.

Grouping is an agent intelligence decision derived from the work's shape — there is no human approval gate. The agent partitions the parent's EXISTING scope into children; a child that needs scope absent from the parent is a NEW todo, not a grouped child.

## Tensions

- **Design churn during prepare.** Authoring structural design directly in pending snippets lets architecture review judge the authoritative design at the cost of revising those snippets if review rejects a premise; phase ownership between structural snippets and post-convergence reflection updates is governed by `sdlc/procedure/lifecycle/prepare/architecture`.

- **Cross-context pending work.** A delivery spanning multiple bounded contexts updates multiple owner-local snippets, each through its own pending carrier. No behavioral or domain snippet spans ownership merely because the deliveries coordinate; cross-cutting design docs describe only the relationship among owners.
