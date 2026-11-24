---
activation_trigger: 'Use when adopting an existing (brownfield) repository into instrukt, migrating existing docs or `.instrukt/adopted/AGENTS.md` into `docs/project/`, or analyzing or re-analyzing a codebase to build or refresh its project documentation.'
description: 'AI-driven project analysis procedure. Guides the analysis session through codebase discovery, convention inference, and snippet generation.'
visibility: 'public'
role: project-analyst
---

# Project Analysis — Procedure

## Required reads

- [sdlc/procedure/feature-spec-authoring](feature-spec-authoring.md)
- [sdlc/principle/domain-driven-design](../principle/domain-driven-design.md)
- [general/procedure/doc-snippet-authoring](../../../general/procedure/doc-snippet-authoring.md)
- [sdlc/policy/linting-requirements](../policy/linting-requirements.md)
- [general/policy/agent-artifact-governance](../../../general/policy/agent-artifact-governance.md)
- [sdlc/spec/domain-spec](../spec/domain-spec.md)

## Goal

Analyze a codebase systematically and produce durable, project-specific doc snippets that
make the repository legible to AI from the first session. Every generated snippet must
reflect concrete findings from the analyzed repository, not generic templates.

## Preconditions

- The project has source files (not an empty repository).
- The project is adopted with `instrukt init`, so its `instrukt.yaml` (with the `project` slug), `docs/project` and hook sections exist.
- The required reads above provide the snippet-authoring discipline (`doc-snippet-authoring`, which transitively
  pulls in the value contract, schema, and referencing rules), and the
  agent-artifact governance (for the agent bootstrap decision in Step 15).
- The session runs inside the project root with full filesystem read access.

## Steps

On a re-run, read the project's existing `docs/project` and compare them with their git history. Use no markers or metadata files to track analysis.

### 1. Orientation

Determine the project's primary characteristics before deep analysis:

- Read `package.json`, `pyproject.toml`, `Cargo.toml`, `go.mod`, `Gemfile`,
  `pom.xml`, `build.gradle`, or equivalent to identify language and package manager.
- Read the project root directory listing to identify top-level structure.
- Check for existing `instrukt.yaml`, `docs/project/`, and `AGENTS.md` to
  understand current instrukt integration state.

### 2. Language and Framework Detection

Identify all languages and frameworks in use:

- **Python:** Check `pyproject.toml`, `setup.py`, `requirements.txt`. Identify
  web frameworks, CLI frameworks, test frameworks and asynchronous execution
  patterns.
- **TypeScript/JavaScript:** Check `package.json` dependencies and devDependencies.
  Identify UI and server frameworks, bundlers and test runners.
- **Go:** Check `go.mod`. Identify web and CLI frameworks.
- **Rust:** Check `Cargo.toml`. Identify application frameworks and asynchronous
  runtimes.
- **Other languages:** Identify from file extensions and build files.

### 3. Entry Points and Route/Handler Mapping

Locate the primary entry points:

- Main executables, CLI entry points, server bootstrap files.
- Route definitions, API handler registrations, middleware chains.
- Event handlers, message consumers, webhook receivers.
- Scheduled jobs or cron-like registrations.

### 4. Architecture Pattern Recognition

Identify the dominant architecture:

- Layer structure (MVC, hexagonal, clean architecture, monolith, microservice).
- Module organization (feature-based, layer-based, domain-driven).
- State management patterns (global state, dependency injection, actor model).
- Communication patterns (REST, GraphQL, gRPC, WebSocket, message queues).
- Database access patterns (ORM, raw SQL, repository pattern).

#### Brownfield domain and documentation classification

Before authoring any snippet, classify what the evidence is asking the doc
layer to represent. This classification is part of analysis, not a post-write
cleanup pass.

- **Bounded context discovery:** identify candidate contexts from service or
  module boundaries, state ownership, domain vocabulary, entry points, emitted
  or consumed events, and persistence boundaries. A package, directory, or
  class is not automatically a bounded context; it earns that treatment only
  when it owns a coherent domain model.
- **Domain specs:** author or refresh `project/<slug>/spec/domain/<context>` only
  when a bounded context is evidenced. The spec names ubiquitous language,
  aggregates, domain events, use cases, and context-map position in domain
  terms. Do not replace this with a flat `project/<slug>/concept/domain-model`
  summary.
- **Feature specs:** author or refresh
  `project/<slug>/spec/feature/<context>/<concept>` for behavioral
  capabilities under `sdlc/principle/documentation-first-development`. Apply
  `sdlc/procedure/feature-spec-authoring` separately before writing Gherkin;
  implementation branches do not establish stakeholder goals. The concept is a
  durable product/domain noun, never the onboarding run slug or a tooling label.
- **Contract or tooling refinements:** route service contracts, CLI surfaces,
  generated-doc behavior, conventions, and scaffolding rules to the relevant
  domain spec, tool spec, policy, procedure, or design doc. Do not create a
  feature spec solely from an implementation inventory. The actor's label does
  not decide classification; use the same behavioral-contract and user-goal
  distinctions as feature intake (`sdlc/procedure/feature-spec-authoring`).
- **Current versus target truth:** onboarding analysis is code-grounded
  inventory. Generated snippets describe current behavior; they carry no
  `delivered_by` (absence = current). Design-delivery snippets that describe
  intended future state carry `delivered_by: [<owning-slug>]`; do not invent
  those during brownfield analysis.
- **Skip:** if the candidate only restates source files, repeats global
  policy, or fills a taxonomy slot without changing future-agent behavior, do
  not author it.

### 5. Test Pattern and Coverage Model

Characterize the testing approach:

- Test framework and runner configuration.
- Test directory structure and naming conventions.
- Test categories present (unit, integration, e2e, contract, snapshot).
- Fixture and mock patterns.
- Coverage configuration if present.

### 6. Dependency Inventory and Role Classification

Catalog dependencies with their roles:

- **Core:** Framework, runtime, and language dependencies.
- **Data:** Database drivers, ORMs, cache clients.
- **Infrastructure:** Logging, monitoring, configuration.
- **Development:** Test frameworks, linters, formatters, build tools.
- **Integration:** API clients, SDK wrappers, protocol libraries.

### 7. Build and Deploy Model

Document the build and deployment pipeline:

- Build commands and scripts (`Makefile`, `package.json` scripts, CI config).
- Container build definitions and multi-service orchestration.
- CI/CD workflow configuration.
- Environment configuration and secrets management.

### 8. Configuration Structure

Map the configuration landscape:

- Config file formats and locations (YAML, JSON, TOML, env files).
- Environment variable patterns.
- Feature flags or conditional configuration.
- Multi-environment setup (dev, staging, production).

### 9. Git History Patterns

Sample recent git history for conventions:

- Commit message style (conventional commits, free-form, ticket references).
- Branching model (trunk-based, gitflow, feature branches).
- Review patterns (PR-based, direct push, approval requirements).
- Release tagging conventions.

### 10. Existing Documentation Inventory

Catalog what documentation already exists:

- Preserved hand-authored agent instructions at `.instrukt/adopted/AGENTS.md` (created by `instrukt init` when adopting a repo with an existing root `AGENTS.md`).
- README files and their coverage.
- API documentation (OpenAPI specs, generated docs).
- Architecture decision records (ADRs).
- Inline documentation density and style.
- Existing `docs/` directory content or documentation in other locations and formats.

### 11. Sampling Strategy for Large Codebases

When the project exceeds comfortable analysis within a single context window:

- **File count > 500:** Focus on top-level structure, entry points, and
  configuration. Sample 2-3 representative modules deeply rather than
  scanning everything shallowly.
- **Monorepo:** Analyze the root package manifest and shared infrastructure
  first, then the largest or most active package.
- **Directory prioritization:** `src/`, `lib/`, `app/`, `cmd/` before
  `vendor/`, `node_modules/`, `dist/`, `build/`.
- **Fallback:** When context is tight, produce fewer but higher-quality
  snippets rather than attempting comprehensive but shallow coverage.

### 12. Convention Inference Rules

Infer project conventions from evidence, not assumptions:

- **Naming:** Sample function, variable, and file names across modules. Report
  the dominant style (camelCase, snake_case, PascalCase, kebab-case).
- **Error handling:** Look for patterns — do functions return errors, throw
  exceptions, use Result types? Is there a custom error hierarchy?
- **Logging:** Identify the logging library and structured vs. unstructured
  patterns.
- **Import organization:** Note grouping conventions (stdlib first, third-party
  second, local third) and absolute vs. relative import preference.

### 13. Decision Boundaries

- **Infer:** When evidence is unambiguous (e.g., all files use snake_case,
  there is exactly one framework, commit messages consistently follow a format).
- **Preserve ambiguity:** When evidence is mixed or insufficient. Write
  "observed X in some files, Y in others" rather than picking one.
- **Leave placeholder for human follow-up:** When the analysis cannot determine
  intent (e.g., is a complex module intentionally structured that way or
  is it technical debt?). Mark with `<!-- human-review: reason -->`.

### 14. Snippet Output

Generated snippets must:

- Conform to the snippet schema (`general/spec/snippet-authoring-schema`).
- Carry the analyzed project's qualified IDs
  (`project/<slug>/…`, `<slug>` being the `project` slug in its `instrukt.yaml`). Flat IDs such as
  `project/<slug>/design/architecture` remain valid; nested spec IDs are reserved for
  earned domain specs and classified feature specs.
- Reference concrete findings: real package names, actual file paths, observed
  patterns — never generic placeholder text.
- Cover the categories implied by the analyzed codebase. Not every category
  applies to every project.

Apply the value test from `general/policy/snippet-value-contract` to every
candidate snippet. The table below names common project-snippet shapes worth
considering. Each row is conditional on the value test: if a candidate fails
— for example, if the project's "structure" or "conventions" are generic and
add no signal beyond what stdlib, framework, or global policy already imply —
do not produce that snippet. Coverage is not a virtue; signal is. When the
project implements a global policy, prefer a reverse pointer (`See also: <global policy ID>`)
over restating the contract.

| Snippet shape                                            | Taxonomy | Worth producing when…                                                                                                                                                                           |
| -------------------------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `project/<slug>/spec/domain/<context>`            | spec     | A bounded context is evidenced by coherent domain vocabulary, state ownership, events, entry points, and invariants.                                                                            |
| `project/<slug>/spec/feature/<context>/<concept>` | spec     | A behavioral capability earns a durable contract under `sdlc/principle/documentation-first-development`; Gherkin is added only for goals admitted by `sdlc/procedure/feature-spec-authoring`.   |
| `project/<slug>/design/architecture`              | design   | Repo structure encodes durable intent the code cannot express — non-obvious layering, a deliberate constraint, or topology agents must navigate.                                                |
| `project/<slug>/policy/conventions`               | policy   | Conventions diverge from the language's global policy in ways agents must respect (custom error hierarchy, project-specific naming). The global policy answers the rest.                        |
| `project/<slug>/spec/dependencies`                | spec     | Dependency choices encode constraints invisible in the manifest (why a version is pinned, why a library was chosen over alternatives). Otherwise the manifest is the source of truth.           |
| `project/<slug>/spec/entry-points`                | spec     | Entry points have non-obvious wiring (conditional registration, side-effect imports). Otherwise a 30-second source read answers it.                                                             |
| `project/<slug>/design/test-strategy`             | design   | The project tests in non-standard ways agents must follow (custom fixtures, coverage philosophy, specific frameworks). Generic test-runner patterns do not earn a snippet.                      |
| `project/<slug>/spec/build-deploy`                | spec     | The pipeline encodes constraints invisible in the CI YAML (why a step exists, what failure it defends). Otherwise the YAML is the source of truth.                                              |
| `project/<slug>/spec/configuration`               | spec     | Configuration semantics are project-specific. **If the project implements a global policy, write a reverse pointer (`See also: <global policy ID>`) and stop — do not duplicate the contract.** |
| `project/<slug>/spec/verification-substrate`      | spec     | Produced via `sdlc/procedure/verification-bootstrap`: declares the project's archetype(s), execution host, live surfaces, isolation primitive, seeders, and blind spots (`sdlc/policy/live-verification`). |
| `project/<slug>/policy/reliability-checklist`     | policy   | Paired with `verification-substrate` via `sdlc/procedure/verification-bootstrap`: holds diff-triggered live checks born from real escapes (`sdlc/policy/live-verification`). |

When `project/<slug>/spec/verification-substrate` or
`project/<slug>/policy/reliability-checklist` is absent or stale for the
analyzed repository's runtime surfaces, execute
`sdlc/procedure/verification-bootstrap` to establish or refresh both fills and
any earned verification stack.

### 15. Agent Bootstrap Content (project index decision)

The analysis encounters one of the repository states below. Apply the matching
case; do not improvise.

**Case 1 — `.instrukt/adopted/AGENTS.md` exists (preserved by `instrukt init`).**
Review `.instrukt/adopted/AGENTS.md` with the human: place the repository's
purpose in a project snippet named first in `expanded` of
`docs/project/index.yaml`, migrate every durable instruction
into its appropriate snippet under `docs/project/` with its entry in
`docs/project/index.yaml` (`expanded` for always-loaded baseline rules, `listed`
for progressive retrieval), and remove `.instrukt/adopted/AGENTS.md` once
nothing in it remains unplaced. On commit, instrukt overwrites the root
`AGENTS.md` with the generated rendering.

**Case 2 — `docs/project/index.yaml` already selects snippets, no `.instrukt/adopted/AGENTS.md`.**
Respect the selection and the project's purpose snippet. Place project
knowledge in snippets under `docs/project/` and select them through
`docs/project/index.yaml`.

**Case 3 — Substantive standalone runtime-specific instruction file, no `.instrukt/adopted/AGENTS.md`.**
Respect by default. Do not auto-migrate or delete a standalone runtime-specific
instruction file — moving its content into `docs/project/` snippets is an
explicit human-driven decision.

**Case 4 (greenfield) — an index that selects nothing and no prior instruction files.**
Write the project's purpose as a project snippet, name it first in `expanded`,
and select the other project snippets through `docs/project/index.yaml` so the
commit renders `AGENTS.md`.

**Case 5 — a former master exists: an `AGENTS.master.md` kept at the repository's root or in a subfolder as the source of the `AGENTS.md` beside it.**
No entrypoint is rendered from it, so what it holds reaches no agent through
the project's rendering. Move its `## Purpose` into the project's purpose
snippet and anything else it holds into snippets the index selects, then
delete the file. A document under `docs/` that the index selects stays where
it is, whatever its name.

**Anomaly — both `.instrukt/adopted/AGENTS.md` (or a hand-authored `AGENTS.md`) and a substantive runtime-specific instruction file exist.**
When the configured runtime loads both into the same session, two authored
instruction sources compete. Halt automatic migration and flag the conflict for
human inspection; which source is authoritative is the human's call before
analysis proceeds.

## Outputs

- A set of schema-valid doc snippets under `docs/project/`.
- A concise summary of what was created/updated.

## Recovery

- If analysis fails mid-run, any snippets already written are valid and
  can be kept. The session should report what was completed and what remains.
- Re-running analysis is safe — the merge rules prevent duplication and
  preserve human edits.
