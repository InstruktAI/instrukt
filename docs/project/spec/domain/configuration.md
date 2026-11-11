---
description: The configuration bounded context — `instrukt.yaml` in a global and a project layer, per-key layer rules, schema versions and migration, `instrukt config get|patch|init|validate`, interpolation and runtime selection.
---

# Configuration — Spec

## What it is

The configuration context owns the layered `instrukt.yaml`, role-keyed
`agents.yaml` and protected machine-local secrets. `instrukt.yaml` uses one
schema, read in two layers on git's model of a user file and a repository file. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and the settings only a
machine may decide; the project layer (`instrukt.yaml` at a repository's root)
holds what a project declares and its overrides. The context decides which key
may live in which layer, validates documents, migrates older schema versions,
edits documents safely under concurrent change, and answers values with
interpolation — including the caller identity recognised by the explicitly
selected runtime adapter. It owns no behavior the keys configure: each key's
meaning belongs to the context or module that reads it.

## Ubiquitous language

**Layer**
: One `instrukt.yaml` document: the **global layer** of the machine or the
**project layer** of a repository.

**Effective configuration**
: The global layer with the project layer applied over it, for a caller
standing in a project; the global layer alone elsewhere.

**Layer rule**
: Which layer a key may live in: global-only, project-only, or both (the
project overriding).

**Schema version**
: The `schema_version` every layer document carries, naming the schema it was
written against.

**Migration**
: Rewriting a document of an older schema version into the current one,
preserving its settings.

**Revision**
: An opaque digest of a layer document's content, used to refuse a patch made
against a document that has changed since it was read.

**Merge patch**
: A partial document applied to a layer: mappings merge key by key, a null
removes a key, any other value replaces.

**Interpolation**
: Expanding a value's references when it is read: `{other.key}`, `${VAR}`, and
the derived `runtime.session_id` and selected `runtime.<operation>`.

**Selected runtime**
: The compiled adapter explicitly chosen by global `runner.runtime`, initially
`t3`. Absence means no runtime, not a request for detection or a default.

**Caller identity**
: The selected adapter's recognition of the current native session. It may
be absent outside that runtime and does not change runtime selection.

**Project slug**
: The `project` key of a project layer; the identity of the project's
`docs/project/` documentation (ids `project/<slug>/…`) and of its index entries.

## Ownership boundary

- **Owns** — the `instrukt.yaml` schema, `agents.yaml` role/class/scoped-grant,
  bundle and model-selection shapes and validation; GitHub App public profile
  and role-reference configuration; machine-local protected secrets and owned-file migration;
  layer rules; validation of a layer
  document; schema versions and migration; the effective configuration;
  revisions and safe patching; interpolation; validation of the explicit runtime
  choice, agent-action instruction entries and derived caller identity.
  Code owns the action catalog and operation mappings; configuration owns the
  composed grants, with admission consuming owner-supplied assignment facts.
- **Does not own** — what any key does: docs and publication read
  `project`; repository decides when publication runs and its commit
  composition reads adapter caller identity and `agents.<name>.git_author` and decides the
  commit author and trailers, including the selected App bot author for bound
  bot roles; the GitHub boundary owns installation-token issuance and actor
  verification; blueprint, work, workflow and runner read the
  keys and module switches governing their capabilities; the flavor's
  procedures run the checks the project documents. Configuration migration
  precedes refusal or skip on every path; configuration owns its ordering and conversion.
- **Published contracts** — `instrukt config get [<path>] [--global]`,
  `instrukt config patch --revision <rev> [--global]`,
  `instrukt config init [--global]`, `instrukt config validate [--global]`;
  the migration operation every entry path invokes before refusal or skip; the
  configuration schema type the editor schema of `instrukt.yaml` is generated
  from (`project/instrukt/spec/feature/publication/editor-schemas`); the explicit runtime selection and adapter caller-identity lookup used by
  commit composition.
- **Allowed dependencies** — none. Configuration depends on no other context.
  It declares the caller-identity lookup interface; CLI wiring supplies the
  selected adapter's environment-only recognition without importing runner.
- **Implementation roots** — the configuration module of the `instrukt-core`
  crate; its module layout is set by the architecture design.

## Canonical fields

Role-keyed model selection lives in `~/.instrukt/agents.yaml`, with `default`,
`roles` and typed provider-instance `profiles`, under
`project/instrukt/spec/feature/configuration/agent-selection`. Procedure content
supplies a role, never a model preference. Global runtime entries carry native agent-operation
instructions, resolved by `@config`; instrukt executes no instruction prose. `get --revision` prints the layer revision; `get --json` renders
JSON.

### Aggregates

#### Layer document (aggregate)

- **Root:** Layer document (global or project).
- **Contains:** `schema_version`; settings; explicit runtime choice (global only).
- **Invariants:**
  - Every key it holds is permitted in its layer; a key of the other layer is
    a finding, and a write that would introduce one is refused.
  - It carries a `schema_version`.
  - A written document is valid as a whole; no write leaves an invalid or
    partly written document behind.
  - A patch applies only against the revision it was made from.
- **Identifier:** its layer — the machine for the global layer, the
  repository root for a project layer.
- **Lifecycle:** created by `instrukt config init` (or `instrukt init`, which
  writes the project layer with the same defaults); changed by `patch`;
  migrated before refusal or skip on every entry path when an older schema is found; never
  deleted by instrukt.

### Domain events emitted

None. instrukt runs no daemon and emits no events from this context.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events.

## Use cases

- **layered-configuration** — `project/instrukt/spec/feature/configuration/layered-configuration` — a machine's settings and a project's settings combine, a cloned project cannot decide what runs on the machine, and older documents keep working.
- **configuration-editing** — `project/instrukt/spec/feature/configuration/configuration-editing` — settings are created, read and changed without losing a concurrent change or writing an invalid document.
- **roles-and-capabilities** — [project/instrukt/spec/feature/configuration/roles-and-capabilities](../feature/configuration/roles-and-capabilities.md) — class, scoped action grants, bundles, assignment-based object resolution, focused enforcement and awareness help.
- **agent-selection** — [project/instrukt/spec/feature/configuration/agent-selection](../feature/configuration/agent-selection.md) — role mappings, provider-instance profiles, tier chains, readiness and selection diagnostics.
- **owned-files** — [project/instrukt/spec/feature/configuration/owned-files](../feature/configuration/owned-files.md) — one protected machine-local secret source, owned-shape migration and isolated homes for developing instrukt itself.
- **interpolation** — `project/instrukt/spec/feature/configuration/interpolation` — values resolve across keys, the environment and the caller's own runtime session when read.
- **logging** — `project/instrukt/spec/feature/configuration/logging` — every command's diagnostics are recorded where the InstruktAI logging contract puts them, readable afterwards with `instrukt-ai-logs instrukt`.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — the module boundaries and shared publication, retrieval and integration flows.

## Context map position

- **Docs** — customer / supplier: configuration supplies read-time
  interpolation through the CLI.
- **Publication** — customer / supplier: configuration supplies the project
  slug, runtime choice and the configuration schema type the editor schema of
  `instrukt.yaml` is generated from. Publication also invokes configuration’s
  migration operation; it is a callable contract, not a configuration key. What a
  project publishes follows from its `docs/general/` and `docs/domain/<x>/`
  positions, never from a configuration key.
- **Repository** — customer / supplier: configuration supplies the project
  slug, runtime choice and agent author settings.
- **Specs** — customer / supplier: configuration supplies the project slug
  identifying the inventory and bindings.
- **Blueprint** — customer / supplier: configuration supplies its module
  switch and engine selection.
- **Work, workflow, runner** — customer / supplier: configuration supplies
  board, approval gates, runtime choice and runner settings.

These are context relationships; module switches determine enablement, not
semantic ownership. Configuration is the supplier in each relationship.

## Known caveats

None beyond those in the feature specs; key names, layers and defaults are
`project/instrukt/spec/feature/configuration/layered-configuration`.
