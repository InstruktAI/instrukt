---
description: Reviews, approvals and drift judgments bind the evidence they assessed and show whether it is current.
---
# Artifact Currency — Spec

## What it is

The currency contract determines which recorded decisions still apply to
current work. It supplies facts to the workflow; it neither classifies meaning
in code nor stores a workflow phase. It governs preparation artifacts, build
review evidence, verification dispositions and drift settlement.

## Canonical fields

### Review bindings

A preparation review is current only when its target digest and findings
digest match the current target and findings. A verdict stamp binds both in
the same write and is refused when either binding is missing. Rewriting the
target or findings requires a current review before progression.

A build verdict is current when it binds the exact current commit and the
current `review-findings` digest. The ledger distinguishes an exact binding,
a changed commit, changed findings and an unavailable target. Every change to
the candidate SHA requires a verdict at the new SHA for handoff.

A verification disposition binds the exact candidate commit and the findings
digest (`project/instrukt/spec/feature/work/stamps`). Only the latest
disposition recorded for the todo can be current, and it is current only while
that commit is the current worktree head and the `verification-findings`
report is unchanged. A later commit, a clean upstream merge included, or a
rewritten report leaves no disposition current until one is recorded for the
new head and report.

### Drift evidence

The provider compares recorded artifact provenance with current code,
upstream artifact digests, owned and referenced snippets, and dependency
artifacts.

An artifact declares its grounding in its own content:

- **Code:** a fenced `referenced-paths` block, one repository path or
  directory prefix per line; lines beginning with `#` are ignored.
- **Snippets:** its inline `@` snippet references, in the form validation
  accepts.

A write whose content, provenance and writer all match the artifact's current
version records no version.

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

Each tracked artifact has one effective drift classification: fresh, pending,
cosmetic or substantive. Where evidence classes disagree, substantive takes
precedence over pending, then cosmetic, then fresh. A classification stamp
records the artifact, verdict, reason and evidence it assessed.

### Substantive settlement

A substantive judgment marks dependent review evidence stale. Rewriting the
artifact captures current grounding and clears its re-derivation marker.

| Owning artifact | Reviews invalidated |
| --- | --- |
| Product requirements | Product requirements, architecture and implementation plan. |
| Architecture | Architecture and implementation plan. |
| Implementation plan | Implementation plan. |

Substantive PRD drift revokes PRD approval until a rewrite and current
approval. Approval carry follows
`project/instrukt/spec/feature/workflow/preparation`.

### Cosmetic settlement with incomplete provenance

A cosmetic judgment can settle the exact evidence presented for review even
when historical provenance or a referenced item cannot be resolved. The
classification records the reviewed artifact and current evidence, including
which provenance is unavailable. An unresolved reference is evidence of that
condition, not an empty digest treated as an equality match.

The ledger treats the cosmetic judgment as current while the artifact and
assessed evidence, including unresolved references, remain unchanged.

## Use cases

Currency is a supporting contract for preparation and delivery. Its
comparisons, invalidation and settlement steps introduce no separate user
goal.

## See Also

- [general/principle/grounding](../../../../general/principle/grounding.md) — decisions bind the evidence actually assessed.
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md) — agents judge substantive meaning.
- [sdlc/principle/documentation-first-development](../../../../domain/sdlc/principle/documentation-first-development.md) — preparation derives work against governing contracts.
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
