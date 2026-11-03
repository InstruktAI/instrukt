---
description: 'Determine whether an artifact belongs in global or project scope before creating it.'
visibility: 'public'
---

# Artifact Scope — Policy

## Rules

- Every authored doc snippet must have its scope determined before creation.
- Every repository has two halves, and a snippet belongs to exactly one:
  - **Global** (`docs/global/`): the knowledge is published beyond its project — the domains it publishes. Examples: general procedures, cross-project policies.
  - **Local** (`docs/local/`): the knowledge belongs to this project and is never published; only its index entry enters the home. Examples: project architecture, repo-specific workflows.
- The half decides where a snippet is served from and how its identity reads: a local snippet's identity begins with `local/`, a published snippet's never does. A local snippet stays reachable from any location by that identity.
- The half never decides who may read a snippet, and neither does `visibility`: every reader reads every snippet.
- Scope must not be inferred or assumed. Apply these criteria:
  1. Does this snippet reference code, architecture, configuration, or knowledge specific to this project? → Local.
  2. Would this snippet be useful in a different project with no changes? → Global.
  3. If neither criterion gives a clear answer, ask the requester before proceeding.

## Rationale

- Agents default to guessing scope when it is not explicit, leading to project-specific knowledge polluting the global namespace or global knowledge trapped in a single repo.
- A single shared policy prevents each authoring procedure from duplicating scope logic.
- No reader is told apart from another, so the half answers only where a snippet is served from and how its identity reads.
- Two halves of one repository must never share an identity, or a published snippet and a local one at the same relative path could not coexist. The `local/` prefix keeps them apart and lets any reader tell from the identity alone whether a snippet is published.

## Scope

- Applies to all authoring procedures: doc-snippet-authoring and any future authoring flows.

## Enforcement

- Authoring procedures must include this policy as a Required read.
- An artifact created in the wrong scope is a defect — move it and update references.

## Exceptions

- None.
