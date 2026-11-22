---
description: Workflow flavor manifest (`workflow.yaml`) declares the unified orchestrator, runtime capabilities, and roles; code serves a facts-only ledger while prose procedures own routing.
---
# Workflow Definition — Spec

## What it is

A project selects a published workflow flavor via `workflow.domain` (default
`sdlc`). The flavor declares its unified orchestrator role, required runtime
capabilities, and role vocabulary in `workflow/workflow.yaml`. All activity
routing lives in the orchestrator's prose procedure; Rust code derives only a
facts-only ledger and enforces the three hard integrity invariants.

## Canonical fields

### Flavor manifest (`workflow/workflow.yaml`)

| Key | Contract |
| --- | --- |
| `orchestrator` | Single unified orchestrator role name (e.g. `todo-orchestrator`), listed in `roles` and declared by exactly one procedure in the domain. |
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
- Configured `approvals.prd` / `approvals.task` status and `integrate` review gate readiness.
- Prerequisite standings.

## Use cases

### UC-WD1: Adopt a project workflow

```gherkin
Given a developer is adopting instrukt for a project
And the developer has selected a flavor declaring its unified orchestrator, roles, and prose playbook
When the developer updates the flavor's orchestrator role or prose procedure
Then pickup and dispatch follow the published flavor declaration and current factual ledger
And changing routing choices in prose requires no change to the workflow engine
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
