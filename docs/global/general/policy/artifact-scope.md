---
description: 'Determine whether an artifact belongs in general, a domain, or the project before creating it.'
visibility: 'public'
---

# Artifact Scope — Policy

## Rules

- Every authored doc snippet must have its scope determined before creation.
- A snippet belongs to exactly one of three positions:
  - **General** (`docs/general/`): every agent on every computer that adopts it needs it, in every project. Examples: collaboration principles, cross-project policies.
  - **Domain** (`docs/domain/<x>/`): it belongs to one body of knowledge a single publisher is the authority on, read on demand when a task enters that domain. Examples: software-development policies and procedures in `sdlc`.
  - **Project** (`docs/project/`): it belongs to this project and is never published; it is served from the project. Examples: project architecture, repo-specific workflows.
- The position decides where a snippet is served from and how its identity reads: a project snippet's identity begins with `project/<slug>/`, a general one's with `general/`, a domain one's with the domain name. A project snippet stays reachable from any location by that identity.
- The position never decides who may read a snippet, and neither does `visibility`: every reader reads every snippet.
- Scope must not be inferred or assumed. Apply these criteria:
  1. Does this snippet reference code, architecture, configuration, or knowledge specific to this project? → Project.
  2. Would every agent need it, in any project, with no changes? → General.
  3. Would it be useful in a different project with no changes, but only for work in one body of knowledge? → that knowledge's domain.
  4. If no criterion gives a clear answer, ask the requester before proceeding.

## Rationale

- Agents default to guessing scope when it is not explicit, leading to project-specific knowledge polluting the published namespace, global knowledge trapped in a single repo, or domain knowledge loaded by every agent when only work in that domain needs it.
- A single shared policy prevents each authoring procedure from duplicating scope logic.
- No reader is told apart from another, so the position answers only where a snippet is served from and how its identity reads.
- Two positions of one repository must never share an identity, or snippets at the same relative path could not coexist. The identity's first segment keeps them apart and lets any reader tell from the identity alone whether a snippet is published.

## Scope

- Applies to all authoring procedures: doc-snippet-authoring and any future authoring flows.

## Enforcement

- Authoring procedures must include this policy as a Required read.
- An artifact created in the wrong scope is a defect — move it and update references.

## Exceptions

- None.
