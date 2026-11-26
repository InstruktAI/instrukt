---
description: The configuration bounded context — `instrukt.yaml` in a global and a project layer, company homes with their own document and secrets, per-key layer rules, schema versions and migration, `instrukt config get|patch|init|validate`, interpolation and runtime selection.
---

# Configuration — Spec

## What it is

The configuration context owns the layered `instrukt.yaml`, role-keyed
`agents.yaml` and protected machine-local secrets. `instrukt.yaml` uses one
schema, read in two layers on git's model of a user file and a repository file. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and the settings only a
machine may decide; the project layer (`instrukt.yaml` at a repository's root)
holds what a project declares and its overrides. A company home
(`~/.instrukt/companies/<name>/`) holds a company's own settings document and
secrets. The context decides which key
may live in which layer, validates documents, migrates older schema versions,
edits documents safely under concurrent change, and answers values with
interpolation — including the caller identity recognised by the explicitly
selected runtime adapter. It owns no behavior the keys configure: each key's
meaning belongs to the context or module that reads it.

## Ubiquitous language

**Layer**
: One `instrukt.yaml` document: the **global layer** of the machine or the
**project layer** of a repository.

**Company home**
: The folder `~/.instrukt/companies/<name>/` whose settings document and
secrets file make a company exist on the machine. It is the checkout of the
company's repository, which holds the company's knowledge; its settings
document, secrets file and local state are kept out of that repository. Its document is read on its
own and is no layer.

**Member**
: A person's entry in a company home: a key, a display name, their email
address and references to their own stored tool logins. An entry gives a
person logins; it does not decide whom the company's agent answers. A login a
person signs in for is kept by the runner, not in the entry.

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
: The compiled adapter chosen by global `runner.runtime`, initially `t3`.
Absent, it is the one runtime compiled into this instrukt; an unknown name is a
finding. No runtime is detected from installed tools or the caller's environment.

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
  company homes, their settings documents and their own protected secrets;
  a company home's `connectors:` map and its validation against the connector
  declarations the binary carries, which the CLI installs once when it starts,
  before any configuration is read (a document read without them finds no
  connector kind supported); layer rules; validation of a layer
  document; schema versions and migration; the effective configuration;
  revisions and safe patching; interpolation; validation of the runtime
  choice, agent-action instruction entries and derived caller identity.
  Code owns the action catalog and operation mappings; configuration owns the
  composed grants, with admission consuming owner-supplied assignment facts.
- **Does not own** — what any key does: docs and publication read
  `project`; repository decides when publication runs and its commit
  composition reads adapter caller identity and `agents.<name>.git_author` and decides the
  commit author and trailers, including the selected App bot author for bound
  bot roles; the GitHub boundary owns installation-token issuance and actor
  verification; the fields each connector kind's declaration takes are
  declared by its connector (`project/instrukt/spec/domain/company`);
  blueprint, work, workflow, runner and company read the
  keys and module switches governing their capabilities; the flavor's
  procedures run the checks the project documents. Configuration migration
  precedes refusal or skip on every path; configuration owns its ordering and conversion.
- **Published contracts** — `instrukt config get [<path>] [--global]`,
  `instrukt config patch --revision <rev> [--global]`,
  `instrukt config init [--global]`, `instrukt config validate [--global]`,
  each also addressing a company home with `--company <name>`;
  the migration operation every entry path invokes before refusal or skip; the
  configuration schema type the editor schema of `instrukt.yaml` is generated
  from (`project/instrukt/spec/feature/publication/editor-schemas`); the runtime selection and adapter caller-identity lookup used by
  commit composition.
- **Allowed dependencies** — the company context's connector declarations, as
  published data; configuration depends on no other context's behavior.
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

#### Company home (aggregate)

- **Root:** The company home folder.
- **Contains:** Its settings document (`schema_version`, `company`, the keys
  permitted in a company home, its members among them) and its secrets file.
- **Invariants:**
  - `company` equals the folder's name.
  - References in its settings resolve only from its own secrets file.
  - An email address is listed by at most one of its members.
  - It combines with no layer and names no project.
  - Its settings document, secrets file, `projects/` and local state are
    ignored by the home's repository; validation reports one that is tracked or not ignored.
- **Identifier:** the company name, unique among the machine's company homes.
- **Lifecycle:** created by `instrukt config init --company <name>`; changed by
  `patch` and the secrets commands; migrated like a layer document; never
  deleted by instrukt.

#### Layer document (aggregate)

- **Root:** Layer document (global or project).
- **Contains:** `schema_version`; settings; runtime choice (global only; absent, the one compiled runtime).
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

None. This context emits no events.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events.

## Use cases

- **layered-configuration** — `project/instrukt/spec/feature/configuration/layered-configuration` — a machine's settings and a project's settings combine, a cloned project cannot decide what runs on the machine, and older documents keep working.
- **company-homes** — [project/instrukt/spec/feature/configuration/company-homes](../feature/configuration/company-homes.md) — a company is added to a computer as its own home, each company's secrets stay in its own home, and its people's own tool logins are kept with their email addresses.
- **configuration-editing** — `project/instrukt/spec/feature/configuration/configuration-editing` — settings are created, read and changed without losing a concurrent change or writing an invalid document.
- **roles-and-capabilities** — [project/instrukt/spec/feature/configuration/roles-and-capabilities](../feature/configuration/roles-and-capabilities.md) — class, scoped action grants, bundles, assignment-based object resolution, focused enforcement and awareness help.
- **agent-selection** — [project/instrukt/spec/feature/configuration/agent-selection](../feature/configuration/agent-selection.md) — role mappings, provider-instance profiles, tier chains, readiness and selection diagnostics.
- **configuration-skill** — [project/instrukt/spec/feature/configuration/configuration-skill](../feature/configuration/configuration-skill.md) — one skill coordinates local configuration and infrastructure verification across participating computers through the selected runtime, or locally without a route.
- **owned-files** — [project/instrukt/spec/feature/configuration/owned-files](../feature/configuration/owned-files.md) — one protected machine-local secret source, owned-shape migration and isolated homes for developing instrukt itself.
- **interpolation** — `project/instrukt/spec/feature/configuration/interpolation` — values resolve across keys, the environment and the caller's own runtime session when read.
- **logging** — `project/instrukt/spec/feature/configuration/logging` — every command's diagnostics are recorded where the InstruktAI logging contract puts them, readable afterwards with `instrukt-ai-logs instrukt`.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — the module boundaries and shared publication, retrieval and integration flows.

## Context map position

<!-- planned-change:archive-finished-threads -->
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
  board, approval gates, runtime choice and runner settings, and to runner the
  company homes with their settings and resolved secrets.
- **Company** — customer / supplier: configuration supplies company homes with
  their settings, members, resolved secrets and declared connectors. Published
  language: configuration validates `connectors:` against the company
  context's connector declarations.

- **Runner (worktree reclamation)** — customer / supplier: configuration
  supplies a project's admitted declaration of regenerable output and decides
  that it stays inside the project's checkout
  (`project/instrukt/spec/feature/configuration/layered-configuration`).

<!-- change:archive-finished-threads -->
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
  board, approval gates, runtime choice and runner settings, and to runner the
  company homes with their settings and resolved secrets.
- **Company** — customer / supplier: configuration supplies company homes with
  their settings, members, resolved secrets and declared connectors. Published
  language: configuration validates `connectors:` against the company
  context's connector declarations.

- **Runner (worktree reclamation)** — customer / supplier: configuration
  supplies a project's admitted declaration of regenerable output and decides
  that it stays inside the project's checkout
  (`project/instrukt/spec/feature/configuration/layered-configuration`).

- **Runner (thread retention)** — customer / supplier: configuration supplies
  the computer's admitted retention window and refuses, where configuration
  enters, a window the selected runtime cannot carry
  (`project/instrukt/spec/feature/configuration/layered-configuration`).
<!-- /planned-change:archive-finished-threads -->

These are context relationships; module switches determine enablement, not
semantic ownership. Configuration is the supplier in each relationship except
the connector declarations, which it consumes.

## Known caveats

None beyond those in the feature specs; key names, layers and defaults are
`project/instrukt/spec/feature/configuration/layered-configuration`.
