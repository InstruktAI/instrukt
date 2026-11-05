---
description: "What every bounded context's canonical domain spec holds — ubiquitous language, ownership boundary, model, events, use-case index, governing designs, context map position. Its section shape comes from sdlc's schema."
visibility: 'public'
---

# Domain Spec — Spec

## What it is

The canonical domain spec of a bounded context. One spec per context. The spec is the source of truth for what the context does in domain terms — independent of implementation language or technology. Implementation (Python, Rust, anything else) must satisfy the spec; the spec does not describe implementation.

Domain specs are documentation snippets (`general/spec/snippet-authoring-schema`). A project's domain specs live under `docs/project/spec/domain/<context>.md`. A context owned by a published domain uses that domain's `spec/domain/<context>` path when a published snippet is warranted.

Each spec is authored once per bounded context and evolves with the context. New intended state is added as pending material naming its delivering slug (`general/procedure/doc-snippet-authoring`). Implementation changes that change behavior MUST update the spec.

Together with the project's policies, the domain specs and the project's context map are the fixed entry points from which preparation reaches every contract governing a work item (`sdlc/policy/preparation-artifact-quality`, Governing contracts). The context map is the project's `spec/domain/context-map`. It lists every bounded context with its domain spec and every cross-cutting design, the designs whose subject is the relationship among contexts rather than one context's model. Every design and decision record is listed by the context map or by one domain spec, from the change that adds it.

## Canonical fields

### Section shape

The H2 sections a domain spec carries, and which are required, are declared
for `spec/domain` in sdlc's `schema.yaml` and copied into a project's own
`schema.yaml` at adoption. `instrukt docs schema <folder>` prints them with a
skeleton. What follows is what each section holds. Everything is written in
the ubiquitous language of the context.

### What it is

One paragraph: what domain activity the context owns, what state it manages,
what it does not own.

### Ubiquitous language

Glossary of terms used inside this bounded context. Each term has one meaning
here: its definition in plain prose, using other glossary terms where they
apply, with synonyms and any difference from another context's use of the same
word. Terms include entities, value objects, statuses, actions and roles.
Implementation terms (table names, function names) appear only when they are
the domain term.

### Ownership boundary

The context's semantic boundary, stated independently of current deployment or
module topology:

- **Owns** — the decisions, behaviors, invariants, state models, and vocabulary
  for which this context is the single authority.
- **Does not own** — adjacent responsibilities a future author might plausibly
  place here, naming their actual owning contexts.
- **Published contracts** — commands, queries, events, or public types through
  which other contexts use this context's behavior.
- **Allowed dependencies** — the contexts this one may depend on and the
  relationship from the context map.
- **Implementation roots** — the source directories whose code realizes this
  context.

Implementation roots are recorded for grounding, but never substitute for this
boundary. A file's current location is evidence about code, not authority over
the rule it contains. A claim about who decides what between this context and
another lives here, or as a plain citation of the other context's spec by id;
no spec carries a separate list of cross-context references.

### Canonical fields

The context's model.

**Aggregates.** Each aggregate names:

- **Root entity** — the entry point. State changes go through this entity.
- **Contained entities and value objects** — what the aggregate cluster owns.
- **Invariant(s)** — the conditions the root enforces. State outside an invariant is impossible.
- **Identifier** — how aggregates are uniquely identified (UUID, slug, composite key).
- **Lifecycle** — how aggregates come into existence and how they end (creation, deletion, archival).

**Domain events emitted.** Each event names:

- **Name** — in the form `<context>.<past-tense-verb>` (e.g., `workflow.finalized`)
- **Emitted when** — the domain condition that triggers emission
- **Payload** — the data carried (field names and types in domain terms; serialization is implementation detail)
- **Consumers** — known downstream contexts that act on this event, citing their domain specs
- **Idempotency key** — what makes two emissions of "the same event" the same (so consumers can dedupe)

**Domain events consumed.** Each consumed event names:

- **Name** — the producing context's event (e.g., `integration.hook_received`)
- **Producer** — which context emits it, citing its domain spec
- **Reaction** — what this context does in response, in domain terms
- **Failure handling** — what happens if the reaction fails (retry, dead-letter, compensating event)

### Event subjects and origins

For every event this context emits, what it states as its subject — the one
thing its act concerned, the same across every emission about that thing — and
as its origin: the party the act was for, such as the project it belongs to, or
none. Consumers scope, correlate and deduplicate events by these, so two
emissions about one thing state one subject. A context that emits no events
says so in one line.

### Use cases

An index of the feature specs that touch this bounded context: one entry per
feature spec, naming it by id with a one-line summary of its use case in domain
language. Per `sdlc/principle/documentation-first-development`, feature specs
are the hub (Gherkin lives there); domain specs are the spokes. When a new
feature touches this context, its entry is added here and the feature spec is
authored under the context's `spec/feature/<context>/` folder.

Gherkin scenarios and business value live in the feature spec, never here. The
domain spec stays compact because its job is to describe the bounded context's
model and index its features.

### Governing designs and decisions

An index of the design snippets that shape this context's model and of the
decision records behind them: one entry per snippet, by id, with a one-line
statement of what it governs in this context. A cross-cutting design is listed
in the context map instead.

### Context map position

How this bounded context relates to others: for each related context, one entry
naming the relationship and a one-line description of its purpose or
constraint.

- **Partnership** — both contexts evolve together; tight coordination expected.
- **Customer / Supplier** — downstream depends on upstream; upstream commits to a contract for the downstream's needs.
- **Conformist** — downstream accepts whatever upstream provides without translation.
- **Anti-corruption layer (ACL)** — downstream translates upstream's model to keep its own clean. The ACL location is named.
- **Shared kernel** — both contexts share a small common model (use sparingly).
- **Open host service** — this context provides a stable open interface for any consumer.
- **Published language** — the events/contracts this context publishes for any consumer to read.

### Allowed values and Known caveats

`Allowed values` holds enumerations the context owns that don't fit naturally
inside an aggregate (e.g., role taxonomies, lifecycle hierarchies that cross
aggregates). `Known caveats` holds the limits of the context's current contract
(`general/spec/snippet-authoring-schema`).

## Allowed values

- Relationship labels in context map: `partnership`, `customer-supplier`, `conformist`, `anti-corruption-layer`, `shared-kernel`, `open-host-service`, `published-language`.

## Known caveats

- Aggregates are sometimes implicit in current code (no single class named `TodoAggregate`). The spec names them in domain terms regardless; identifying aggregates is part of authoring the spec, and may surface refactor opportunities in the implementation.
- Cross-context aggregates do not exist. If two contexts both want to "own" the same concept, the spec authoring process MUST split it — either re-draw the context boundary or model the concept in one context with the other consuming events about it.
- A cross-cutting design document may describe several contexts but owns none of
  their behavior. Every invariant it names resolves to one context's Ownership
  boundary and published contract.
- A bounded context may have multiple aggregates, but each aggregate belongs to exactly one bounded context. Aggregates don't span contexts.
