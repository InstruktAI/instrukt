---
description: Reviews, approvals and drift judgments bind the evidence they assessed and route changed work to its owning derivation layer.
delivered_by: [instrukt-first-build]
---
# Artifact Currency — Spec

## What it is

The currency contract determines which recorded decisions still apply to
current work. It supplies facts to the workflow; it neither classifies meaning
in code nor stores a workflow phase. It governs preparation artifacts, build
review evidence and drift settlement.

## Canonical fields

### Review bindings

A preparation review is current only when its target digest and findings
digest match the current target and findings. A verdict stamp binds both in
the same write and is refused when either binding is missing. Rewriting the
target or findings requires a current review before progression.

A build verdict binds the reviewed commit and findings digest. Its binding
distinguishes:

| Evidence | Interpretation |
| --- | --- |
| The reviewed commit is the current worktree head and findings are unchanged | Current. |
| Only clean upstream merges separate the reviewed head from the current head, with unchanged findings | Current. |
| The todo's changes or the findings changed | Stale; another review is owed. |
| Target or findings bindings are missing | Unbound; supplies no current verdict. |
| Repository evidence cannot determine whether the todo's changes are unchanged | Undetermined; preserve the verdict while reporting the evidence gap. |

Both approval and changes-requested verdicts must have current bindings to
route the work. An undetermined binding is neither an approval nor a
revocation. These are facts for the configured flow, not hidden phase changes.

### Drift evidence

The provider compares recorded artifact provenance with current code,
upstream artifact digests, owned and referenced snippets, and dependency
artifacts. Obtaining current repository evidence is an orchestrator command;
the pure evaluator receives its result. Artifact age is not a drift trigger
in the shipped flow.

Each tracked artifact has one effective drift classification: fresh, pending
analysis, cosmetic or substantive. Where evidence classes disagree,
substantive takes precedence over pending, then cosmetic, then fresh. A
pending classification requests agent analysis. A classification stamp records
the artifact, verdict, reason and evidence it assessed.

Cosmetic and fresh evidence permit continuation. Pending drift holds initial
build admission for analysis; after the build has begun, pending drift alone
does not return work to preparation. Substantive drift returns work to the
owning preparation layer.

### Substantive settlement

The substantive classification invalidates reviews according to the owning
artifact, and retains a re-derivation obligation until that artifact is
rewritten against current grounding.

| Owning artifact | Derivation owed | Reviews invalidated |
| --- | --- | --- |
| Product requirements | Intake re-derives the product requirements from current input and grounding. | Product requirements, architecture and implementation plan. |
| Architecture | Architecture re-derives its design and index artifact. | Architecture and implementation plan. |
| Diagnosis | Diagnosis is settled against its current content before planning continues. | Implementation plan. |
| Implementation plan | Planning re-derives the implementation plan. | Implementation plan. |

Substantive product-requirements drift enters intake before review of that
artifact. Re-reviewing an unchanged artifact does not discharge its
re-derivation obligation. A rewrite captures current provenance and clears
the artifact's drift classification and re-derivation marker. An unresolved
upstream obligation keeps dependent work from becoming prepared.

An approval of an earlier product-requirements digest does not approve the
rewritten artifact by digest coincidence or by the writer's identity. Approval
carry follows the preparation contract in
`local/projects/instrukt/spec/feature/workflow/preparation`.

### Cosmetic settlement with incomplete provenance

A cosmetic judgment can settle the exact evidence presented for review even
when historical provenance or a referenced item cannot be resolved. The
classification records the reviewed artifact and current evidence, including
which provenance is unavailable. An unresolved reference is evidence of that
condition, not an empty digest treated as an equality match.

When that evidence is unchanged, the provider recognizes the cosmetic stamp
as settled and continuation is possible. It does not repeatedly dispatch an
analyzer solely because the same provenance remains unavailable. Changes to
the artifact, evidence or unresolved-reference condition require a new
assessment; cosmetic settlement is not a timeless waiver.

### Use cases

Currency is a supporting contract for preparation and delivery. Its
comparisons, invalidation and settlement steps introduce no separate user
goal.

## See Also

- `general/principle/grounding` — decisions bind the evidence actually assessed.
- `general/principle/intelligence-boundaries` — agents judge substantive meaning.
- `sdlc/principle/documentation-first-development` — preparation derives work against governing contracts.
