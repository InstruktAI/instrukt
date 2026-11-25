---
description: Reviews, approvals and drift judgments bind the evidence they assessed and route changed work to its owning derivation layer.
---
# Artifact Currency — Spec

## What it is

<!-- planned-change:verification-stamp-and-evidence-store -->
The currency contract determines which recorded decisions still apply to
current work. It supplies facts to the workflow; it neither classifies meaning
in code nor stores a workflow phase. It governs preparation artifacts, build
review evidence and drift settlement.
<!-- change:verification-stamp-and-evidence-store -->
The currency contract determines which recorded decisions still apply to
current work. It supplies facts to the workflow; it neither classifies meaning
in code nor stores a workflow phase. It governs preparation artifacts, build
review evidence, verification dispositions and drift settlement.
<!-- /planned-change:verification-stamp-and-evidence-store -->

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

<!-- planned:verification-stamp-and-evidence-store -->
A verification disposition binds the exact candidate commit and the findings
digest (`project/instrukt/spec/feature/work/stamps`). It is current only while
that commit is the current worktree head and the `verification-findings`
report is unchanged. A later commit, a clean upstream merge included, or a
rewritten report leaves no disposition current until one is recorded for the
new head and report.
<!-- /planned:verification-stamp-and-evidence-store -->

### Drift evidence

The provider compares recorded artifact provenance with current code,
upstream artifact digests, owned and referenced snippets, and dependency
artifacts. Obtaining current repository evidence is an orchestrator command;
the pure evaluator receives its result. Artifact age is not a drift trigger
in the shipped flow.

An artifact declares its grounding in its own content:

- **Code:** a fenced `referenced-paths` block, one repository path or
  directory prefix per line; lines beginning with `#` are ignored.
- **Snippets:** its inline `@` snippet references, in the form validation
  accepts.

When a version is written, its provenance records the canonical branch's head
commit, the referenced paths, the digest of each non-owned referenced snippet
(read from the project's primary checkout or published store), and, derived
without declaration, the digests of the snippets the todo owns (read solely
from the todo's active worktree when present, else the primary checkout, never
from the published store, combining any todo companion file with its parent)
and of its prerequisites' artifacts. Drift observation compares each class
against the same source, ignoring any overlap where an earlier grounding also
listed an owned snippet as referenced. A drift judgment retains previously owned snippet IDs at their current state in
the selected checkout — absent if the snippet was deleted, or the surviving
parent digest if only its companion was removed — so restoring either requires
reassessment.

An artifact has drifted when a non-merge commit between its recorded head and
the current canonical head changes a referenced path (exactly, or under a
directory prefix), when a referenced or owned snippet's digest changed, or
when a prerequisite artifact's digest changed. When git cannot compare the two
commits, or there is no current canonical head, the classification is
pending. An artifact that references no paths has no code drift. A path absent
at both heads is a planned creation. Its absence alone leaves the artifact
fresh. A path missing from the current head is incomplete provenance if it
existed at the recorded head or its prior absence cannot be established.
An unresolved snippet is also incomplete provenance.

Each tracked artifact has one effective drift classification: fresh, pending
analysis, cosmetic or substantive. Where evidence classes disagree,
substantive takes precedence over pending, then cosmetic, then fresh. A
pending classification requests agent analysis. A classification stamp records
the artifact, verdict, reason and evidence it assessed.

Cosmetic and fresh evidence permit continuation. The factual ledger surfaces
pending and substantive drift classifications per artifact so `todo-orchestrator`
routes `drift-analyzer` (for pending drift) or re-derivation (for substantive
drift) in prose before building; code enforces only the three hard integrity
invariants (`approvals.prd` is revoked automatically when `prd` has substantive
drift).

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

Substantive product-requirements drift invalidates `prd` review and approval
in the factual ledger and revokes PRD approval in code until `prd` is rewritten
and re-approved. Re-reviewing an unchanged artifact does not discharge its
re-derivation obligation. A rewrite captures current provenance and clears
the artifact's drift classification and re-derivation marker; `todo-orchestrator`
reads these facts from the ledger before advancing dependent artifacts.

An approval of an earlier product-requirements digest does not approve the
rewritten artifact by digest coincidence or by the writer's identity. Approval
carry follows the preparation contract in
`project/instrukt/spec/feature/workflow/preparation`.

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

## Use cases

Currency is a supporting contract for preparation and delivery. Its
comparisons, invalidation and settlement steps introduce no separate user
goal.

## See Also

- [general/principle/grounding](../../../../general/principle/grounding.md) — decisions bind the evidence actually assessed.
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md) — agents judge substantive meaning.
- [sdlc/principle/documentation-first-development](../../../../domain/sdlc/principle/documentation-first-development.md) — preparation derives work against governing contracts.
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
