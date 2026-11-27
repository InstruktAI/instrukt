---
description: Workflow flavor manifest (`workflow.yaml`) declares the driver role, runtime capabilities and role vocabulary.
---
# Workflow Definition — Spec

## What it is

The `workflow.domain` setting selects a published workflow flavor
(default `sdlc`). Its manifest declares the configured driver role, runtime
capability requirements and role vocabulary. The ledger reports recorded
work evidence and the integrity checks.

## Canonical fields

### Flavor manifest (`workflow/workflow.yaml`)

| Key | Contract |
| --- | --- |
| `orchestrator` | Configured driver role, listed in `roles` and declared by exactly one procedure in the domain. |
| `requires` | Typed list of runtime capability names (`worktree_create`, `worktree_attach`, `shared_delegation`, `link`, `notify`, `agent_selection`); may be empty `[]`. |
| `roles` | Non-empty list of unique role names declared by the domain's non-subscribed procedures. |

Unexpected YAML files in `workflow/` (such as retired state-machine files) and
unknown keys in `workflow.yaml` are validation findings.

### Runtime capability requirements

Enablement checks `requires` in `workflow.yaml` against the selected runtime's
declared capabilities (`worktree_create`, `worktree_attach`, `shared_delegation`,
`link`, `notify`, `agent_selection`). A missing required capability refuses
enablement without changing configuration and names the workflow file,
capability, and runtime. `config validate` reports the same mismatch for an
already enabled workflow.

The shipped `sdlc` flavor declares `requires: [worktree_create, shared_delegation]`:
worktree creation places the unified `todo-orchestrator` on `<slug>/work`, and
shared delegation keeps all workers in that single recorded workspace.

### Facts-only ledger

`instrukt todo show <slug> [--json]` and `instrukt todo work <slug> [--json]`
derive current facts directly from the work record and repository without
persisting a phase or dictating a next step:

- Identity, kind, status, claim, hold, and delivery commit.
- Workspace/git state (`branch`, `worktree_path`, cleanliness, worktree/branch/remote heads, push sync).
- Active attempts (interference lock status).
- Recorded artifacts, digests, and drift status.
- Preparation and build review verdicts, open findings, exact-SHA binding, and reviewer independence.
- Verification dispositions with their exact-commit and report binding, and whether one is current for the worktree head.
- Configured `approvals.prd` / `approvals.task` status and `integrate` handoff gate readiness: independent build-review approval and a current admitting verification disposition, both at the exact worktree head.
- Prerequisite standings.

## Use cases

### UC-WD1: Select a published workflow flavor

```gherkin
Given a project selects a published workflow domain
When instrukt loads its flavor manifest
Then it reads the declared driver role, capabilities and roles
```

### UC-WD2: Enable a workflow whose runtime can carry its declared work

```gherkin
Given a developer selected a runtime with worktree creation, sharing and existing-worktree attachment but no native notification operation
When they enable a workflow declaring those workspace capabilities
Then that requirement permits enablement
But when they enable a custom workflow requiring native notification
Then instrukt refuses naming the runtime, flow and missing capability
And the configuration remains unchanged
```

## Known caveats

- `workflow.yaml` declares runtime capability requirements explicitly; validation
  checks the declaration rather than scanning procedure prose.

## See Also

- [sdlc/principle/design-fundamentals](../../../../domain/sdlc/principle/design-fundamentals.md)
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md)
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
