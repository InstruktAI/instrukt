---
description: Current PRD and task-plan evidence, approval settings, and workspace placement.
---
# Preparation — Spec

## Required reads

- [project/instrukt/spec/feature/workflow/artifact-currency](artifact-currency.md)

## What it is

The work record keeps versioned preparation artifacts, review verdicts and
approval stamps. The ledger shows current digests, review currency and the
status of configured approval gates.

## Canonical fields

### Approval gates (Invariant 3)

- `approvals.prd` defaults to `human` and accepts `peer`. For a feature or any
  todo with a PRD, `--build-started` and `--finalize-ready` require eligible
  approval of the current PRD.
- `approvals.task` defaults to `human` and accepts `peer`. Task-run dispatch and
  `--task-firing-started` require eligible approval of the current task plan.
- A human gate requires a human approval stamp. A peer gate accepts human or
  peer attribution. Both bind the current target digest.

### Carry and attribution

`--carry-from <stamp-id>` requires a standing preparation approval verdict
with matching current artifact and findings digests. It retains the original
approver and approval source.

### Single workspace

Repository todos record a single `<slug>/work` branch and worktree. Placement
uses the published todo branch when present or the published default branch
for a new workspace. Task child assignments carry no Git placement.

## Use cases

### UC-PR2: Record a decision for the configured approval gate

```gherkin
Given a todo's current artifact requires human approval
When a human approval of the current digest is recorded
Then the ledger marks the corresponding gate approved
And the record retains that decision's attribution
```

### UC-PR3: Release an attempt without changing repository files

```gherkin
Given a todo has a bound workspace attempt
When its session records role completion from the unchanged clean head
Then the completion stamp and attempt release commit together
```

### UC-PR4: Gate feature implementation on configured PRD approval

```gherkin
Given a feature has a current PRD without an eligible approval
When a caller records build start against the current plan
Then the write is refused
When eligible PRD approval is recorded for the current digest
Then the PRD gate admits the build-start stamp
```

## See Also

- [general/policy/human-contract](../../../../general/policy/human-contract.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
