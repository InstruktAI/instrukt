---
eval: true
description: 'Drift analysis phase. Classify each pending prepare-artifact drift across every applicable drift class as cosmetic or substantive and stamp the verdict so the lifecycle can continue or re-derive the affected layer.'
visibility: 'public'
role: drift-analyzer
parameters:
  - name: slug
    required: true
---

# Drift Analyze — Procedure

## Goal

Classify each pending prepare-artifact drift across every applicable drift
class. Stamp `cosmetic` when the artifact remains valid despite the
evidence change. Stamp `substantive` when the artifact must be re-derived
to match current reality. The same procedure handles drift dispatched from
a preparation hold AND drift dispatched from the work-entry return
to preparation — the procedure does not care which entry point routed the
worker; it reads the pending classes per artifact and stamps verdicts.

## Preconditions

- The current prepare or work obligation identifies artifacts pending drift
  analysis. The fact provider exposes their effective drift status and the
  evidence classes requiring judgment.
- The slug is active and not delivered.

## Steps

Use `instrukt <command> -h` for individual commands.

### 1. Identify pending artifacts and pending classes

Read the dispatching orchestrator's action and rendered template, obtained
with `instrukt todo prepare {slug}` or `instrukt todo work {slug}`, and read
`instrukt todo show {slug}` to
enumerate which artifacts are pending drift review and, for each, which
classes are pending. The collapsed per-artifact status is the
most-blocking class status; the analyzer must consider every pending
class, not only the one that drove the collapse.

The drift classes:

- **code** — `codebase_sha_at_write` vs current `origin/main` HEAD,
  scoped to `referenced_paths`.
- **upstream-artifact** — `upstream_digests` (persisted-state field
  name; aka `upstream_artifact_digests` in spec/documentation) vs
  current digest of each named upstream artifact.
- **owned-snippet** — `owned_snippet_digests` vs current digest of each
  snippet the Todo owns via its `delivered_by` membership at write time.
- **upstream-snippet** — `referenced_snippet_digests` vs current digest
  of each non-owned snippet the artifact references by validator-
  accepted inline references under
  `general/policy/referencing-doc-snippets`, mapped to
  canonical snippet IDs.
- **dependency-artifact** — `dependency_artifact_digests` vs the current
  digest of each recorded dependency's intent and prepared artifacts, for
  dependencies still in the `after` set that have not delivered.

### 2. For each pending (artifact, class)

- Read the artifact: `instrukt todo artifacts read {slug} <artifact>`.
- Read the recorded provenance for the class from `instrukt todo show {slug}`.
- Inspect the evidence change scoped to the artifact:
  - **code** — `git fetch origin main` then
    `git log <codebase_sha_at_write>..origin/main -- <referenced-paths>`
    and `git diff <codebase_sha_at_write>..origin/main -- <referenced-paths>`.
    Always compare against `origin/main`, never local HEAD, so the
    derivation matches the recorded write-time code anchor.
  - **upstream-artifact** — read the named upstream artifact at its
    current digest (`instrukt todo artifacts read {slug} <upstream>`) and
    compare to the recorded digest's content. The fact provider has
    already detected the digest mismatch; the analyzer's job is to read
    the upstream change and judge whether THIS artifact's claims still
    hold.
  - **owned-snippet** — read each owned snippet's current body through
    this todo's pending view (`instrukt docs get <snippet-id> --target {slug}`);
    the default view omits the body of a snippet still pending under a
    todo. Read the edit itself from the snippet source file's history over
    the same window as the code class:
    `git log --follow -p <codebase_sha_at_write>..origin/main -- <snippet-file>`.
    A sibling delivery may have edited a snippet this todo owns via
    `delivered_by`; judge whether the edit changes anything THIS artifact
    relies on.
  - **upstream-snippet** — read each referenced non-owned snippet's
    current body with `instrukt docs get <snippet-id>`, adding
    `--target <owner-slug>` when `instrukt docs index` lists the snippet as
    delivered by another todo, and read the edit from its source file's
    history as for owned-snippet. Judge whether the referenced snippet's
    edit changes the contract THIS artifact references.

  - **dependency-artifact** — read each moved dependency artifact at its
    current digest (`instrukt todo artifacts read <dependency> <artifact>`),
    never the plan's own account of it, and find every place the plan relies
    on that dependency: a task premise or an ordering or file-overlap claim.
    `substantive` when any of those no longer holds against the dependency's
    current contract; `cosmetic` when every such reliance still holds. A
    dependency artifact that is itself pending drift in its own Todo is not a
    contract to rely on — read that Todo's newest intent instead.
- Apply the verdict rubric:
  - **`cosmetic`** — the artifact's intent and claims still hold. The
    evidence change is real but does not invalidate this artifact.
    Examples: rename/refactor that preserves behavior; constants
    extraction; formatting; an upstream snippet's `delivered_by`
    frontmatter churn that did not change the body; a dep delivering
    without affecting this artifact's plan tasks.
  - **`substantive`** — the artifact's claims are now wrong, its
    referenced surfaces no longer exist as described, or its grounding
    no longer matches reality. The artifact's owning layer must
    re-derive.

### 3. Stamp the verdict per artifact

Stamping is per-artifact, not per-(artifact, class):

```bash
instrukt todo stamp {slug} --drift-reviewed <artifact> cosmetic <reason>
# or
instrukt todo stamp {slug} --drift-reviewed <artifact> substantive <reason>
```

The reason text MUST name each class evaluated and what was found:
"code: cosmetic — rename of `foo()` to `bar()` preserves the call
contract the plan references. upstream-snippet: cosmetic — the
referenced domain spec gained a clarifying paragraph that does not
change the contract the architecture depends on." The reason is the
audit trail per class for the per-artifact stamp.

When ANY class for an artifact is `substantive`, the overall verdict
for that artifact is `substantive` — substantive on any class clears the
review verdict for the artifact and every downstream artifact's review
verdict per the spec's re-derivation contract. Only when EVERY pending
class is cosmetic does the artifact get a `cosmetic` stamp.

The record captures the **evidence anchor** at stamp time: the reviewed
artifact and current code, upstream artifacts, snippets and dependency
evidence, including any unavailable provenance or unresolved reference.
The analyzer records the judgment and reason; provenance is captured by the
record-writing boundary rather than supplied as invented flags.
A cosmetic verdict at this anchor expires lazily on a future derivation
if evidence has moved past the anchor on any anchored dimension; the
analyzer is re-dispatched at that point. A substantive verdict does not
need lazy expiry — re-derivation produces a fresh artifact whose write
clears the drift stamps.

### 4. Go idle

After every pending artifact has a stamp, end the worker turn. The turn
ends at the stamp — do not prescribe the next obligation. The
dispatching session follows its current workflow obligation, not this worker's
narration.

## Outputs

- One `--drift-reviewed <artifact> cosmetic|substantive <reason>` stamp
  per pending artifact, with the reason naming each class evaluated and
  its per-class finding.

## Discipline

Classify drift only. Do not rewrite prepare artifacts, do not approve
gates, do not start build work. The record must carry the assessed artifact
and its drift stamp; an ended worker turn alone completes nothing. The
dispatching orchestrator calls prepare or work for the next obligation.
Substantive drift is re-derived by the owning layer (intake, architecture,
diagnosis, plan), never on this worker's prompting.

A `cosmetic` verdict must be grounded in the actual evidence change for
EVERY pending class, not assumed. Missing historical provenance or an
unresolved reference does not by itself force substantive re-derivation.
Assess the exact current evidence, name what cannot be resolved, and stamp
cosmetic only when the artifact's claims still hold. The stamp settles that
unchanged evidence, including the unresolved condition; a later evidence
change requires assessment again. Evidence that invalidates the artifact
requires substantive re-derivation. Age alone is not a drift trigger.

The same per-class evaluation discipline applies whether the analyzer
was dispatched from a preparation hold or from a work-entry
bounce. The work-entry path produces no separate verdict semantics — it
is the same stamp on the same artifact, evaluated against the same
evidence.

## Recovery

- **Referenced path deleted or renamed.** Read the changed code and judge
  whether the artifact's claims still hold. A removed surface it relies on is
  substantive; a behavior-preserving rename may be cosmetic.
- **Baseline SHA, upstream digest or snippet unavailable.** Record the
  unresolved provenance and the exact evidence assessed. Apply the current
  evidence rubric above; unresolvable history is neither an empty matching
  digest nor an automatic substantive verdict.
- **Evidence insufficient for a judgment.** If any pending class cannot be
  cleanly classified, record the evidence gap and stamp `substantive` so the
  owning layer re-derives the artifact.

## See Also

- [sdlc/procedure/lifecycle/prepare/overview](overview.md)
