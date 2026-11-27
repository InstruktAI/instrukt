---
description: Unified delivery cycle enforced by three hard integrity invariants in a single todo worktree, with prose-routed build, review, fix, deferral, and finalize primitives.
---
# Delivery Cycle — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

Repository todos record their workspace, assignments and evidence in one
ledger. The ledger and write commands expose three integrity checks.

## Canonical fields

### Three hard integrity invariants

1. **No interference.** At most one workspace or child attempt is open per
   todo. The claim holder reserves and binds attempts. Workspace binding and
   completion require the recorded branch and a clean worktree. A completion
   stamp releases the caller's bound attempt in the same transaction.
2. **Independent exact-SHA handoff.** An approving build verdict and a
   verification pass require a session distinct from the claimant, driver
   and identified writers. Writer evidence comes from commit trailers and
   recorded workspace head changes. `--finalize-ready` requires a clean
   workspace with no open attempt and matching local and published heads.
   It requires a current build-review approval of that exact SHA and the
   current `review-findings` digest, plus a disposition admitting handoff
   of that SHA with the current `verification-findings` digest. It records
   the commit and appends `workflow.finalized` atomically.
3. **Human-owned approval gates.** A feature or todo with a PRD checks
   `approvals.prd` at build start and finalize readiness. Task firing checks
   `approvals.task`. Both checks bind the current artifact digest.

### Live verification disposition

The latest disposition covers the exact commit and findings digest it names.
`pass`, `tier_0_skip` and `hotfix_deferred` admit handoff.
`fail` and `blocked` leave the handoff gate unsatisfied. A hotfix deferral
records a follow-up todo. Stamp forms and evidence requirements are defined
in `project/instrukt/spec/feature/work/stamps`.

### Single workspace, cleanup & in-flight legacy migration

Repository todos use `<slug>/work`. Placement uses the published todo branch,
otherwise it can carry forward an unintegrated legacy `<slug>/prepare` head
or start from the published default branch. A retained legacy work driver
binding on `<slug>/work` remains resolvable. A recorded delivery makes its
clean, merged workspace and branch eligible for reclamation. Attempt records
remain readable.

## Use cases

### UC-DC1: Record the integration candidate

```gherkin
Given a todo has a clean shared workspace with current approvals and verification
When finalize readiness is recorded
Then the stamp and workflow.finalized event identify the same exact commit
```

### UC-DC2: Record a subsequent build completion

```gherkin
Given a released attempt left commits on a todo branch
When its successor records build completion at the clean current head
Then the newer completion and release commit together
And that stamp identifies the current head
```

### UC-DC3: Role-branch handoff (retired)

Repository attempts share the recorded todo branch.

### UC-DC4: Serialize workspace attempts

```gherkin
Given a todo has an open workspace attempt
When the claim holder requests another workspace attempt
Then the second reservation is refused
When the bound session stamps completion on the clean recorded branch
Then that stamp releases its attempt in the same write
```

### UC-DC5: Release an attempt while recording deferral evidence

```gherkin
Given a session holds an open workspace attempt and the todo has a deferrals artifact
When that session stamps deferrals processed in the clean workspace
Then the stamp binds the current deferrals digest and releases the attempt
```

### UC-DC6: Require approval at the exact published SHA

```gherkin
Given a todo's earlier commit has an independent approving build verdict
When a different commit becomes the current candidate
Then finalize readiness is refused until approval covers that exact commit and its current findings
```

### UC-DC7: Preserve committed work on attempt failure

```gherkin
Given a workspace attempt has committed work on the todo branch
When the claim holder records matching stop and workspace evidence
Then the attempt releases with the observed head and failure reason
And the branch retains the commits
But a dirty workspace leaves the attempt open
```

### UC-DC8: Retain attempt evidence after workspace reclamation

```gherkin
Given a delivered todo has a clean, merged workspace
When reclamation removes the workspace and branch
Then the todo's native session and attempt records remain readable
```

### UC-DC9: Readiness while an attempt is open

```gherkin
Given a todo has a current exact-SHA build approval and verification disposition
When finalize readiness is requested with a workspace attempt still open
Then the readiness stamp and event are refused
```

### UC-DC10: Require current independent verification

```gherkin
Given a candidate is independently approved at its exact commit
When no matching verification disposition is current
Then finalize readiness is refused
When an independent session records a pass for the exact commit and current findings
Then the verification gate admits handoff
But a pass by the claimant, driver or a writer is refused
```

## See Also

- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
