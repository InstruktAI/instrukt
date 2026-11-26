---
description: 'Documentation leads, implementation catches up. Pending material names the delivery that makes it current, so the gap stays honest and the design stays shareable.'
visibility: 'public'
---

# Documentation-First Development — Principle

## Principle

Documentation is the source of truth for what the system is AND what it is becoming. Design happens in the document layer first; code catches up. A snippet that describes the intended shape of the system is not a wish or a forecast — it is the design, written down, shareable, reviewable, and binding on what gets built.

“First” orders the documentation before implementation; it does not put a tracked document before the design has formed. Conversation and scratchpad work may shape an outcome, but they are not the contract. The document becomes that contract only after the active participants converge and the authoring context releases the agreed outcome for materialization.

For this to work without becoming a fiction, pending documentation distinguishes intended behavior from the current contract and identifies the delivery that makes it current. Its representation and authoring mechanics are owned by `general/spec/snippet-authoring-schema` and `general/procedure/doc-snippet-authoring`. When the delivery lands, its pending material is folded into the current contract without erasing unrelated current or pending contracts. The reconciliation is enforced by the system, not left to memory.

## Rationale

The alternative — designing inside todos, inside PRDs, inside scattered artifacts — fragments the design surface. Each todo's artifacts are local to that todo. Other agents working in parallel can't easily reference them. The orchestrator that needs to coordinate three related deliveries has to read three different PRDs to see the shared design. Conversations about future state degrade into "go read the X artifact in todo Y."

The alternative on the other side — refusing to update docs until code lands — leaves the design unspoken between conception and delivery. The product owner thinks about the future constantly; the agent reading docs sees only the past. There's no shared surface where forward design lives. So forward design ends up in chat, in team-message threads, in mental models that don't propagate.

Doc-first resolves both by making the design space first-class:

- **Shareable.** Pending material is a URL all agents can reach. The design lives in one place, not five.
- **Honest.** Pending material declares intended state and names its delivery; unmarked text describes current behavior. A reader knows what they are looking at.
- **Reviewable.** Pending snippets get the same review surface as code — pull requests, lint, validation. Bad design gets caught at doc time, before implementation costs are spent.
- **Aligned.** The orchestrator dispatching three related builds points all three at the same pending snippet. The builders read the same design. Their outputs converge.
- **Reconciled.** When delivery lands, its pending material becomes the current contract. Stale "future" doesn't accumulate.

This is the same shape as TDD, schema-first API design, and ADRs. The discipline isn't new — what's new is naming it for documentation, encoding it in frontmatter, and making the reconciliation automatic.

## Implications

The **enforceable rules** of this principle — functionality work authors its
snippets first, every behavioral delta is reflected, a stale current snippet is
a defect — live in `general/policy/documentation-discipline`. The **authoring mechanics** —
planned sections, companions, frontmatter, the publication pipeline — live in
`general/procedure/doc-snippet-authoring`, and the **field shapes** in
`general/spec/snippet-authoring-schema`. This section states the conceptual
consequences that shape how documentation is organized.

### Discoverability surfaces ownership

`instrukt docs index` renders the owning slugs inline for every pending snippet — current snippets render with no suffix, a brand-new pending snippet renders with a `(delivered by: <slugs>)` suffix, and a live snippet with an incoming change renders with a `(superseded by: <slugs>)` suffix. The same listing answers "what is the state of this snippet?" and "who is on the hook to close the gap?" without a second lookup.

## Tensions

- **Pending snippets that never deliver.** The slug they point to may be deferred, frozen, or abandoned. The author must either revive the delivery path or remove the pending material so the snippet reflects current state — design that nobody is actually committing to deliver doesn't belong in the doc layer.

- **Product owner anchor vs. agent ground.** Product owners reading pending material see forward state; agents making code decisions need current state. The ` (delivered by: …)` and ` (superseded by: …)` suffixes in `instrukt docs index` keep the distinction visible. When a human writes "in our system, X works like Y" as pending material, the human asserts design; an agent reading later must check whether it is pending and not assume X works that way today.

- **Doc-first vs. velocity.** Design that only becomes clear by touching the code: author it as pending material, attempt the work, revise as you learn, then deliver. The principle does not require omniscience at design time — it requires writing down what you learn AS you learn it, with honesty about what is current and what is intended. For trivial implementation-internal changes (a one-line bug fix with no contract change), the doc layer is not touched; doc-first scales with the size and reach of the change.
