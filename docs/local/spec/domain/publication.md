---
description: Publication — a project publishes its own part of the documentation from its committed main into the home, patches the global index by owner, compiles its agent entrypoints, and, as owner of `general`, writes the global agent entrypoints and their settings keys.
delivered_by: [instrukt-first-build]
---
# Publication — Spec

## What it is

The publication context turns a project's committed documentation into what
agents on this computer read. `instrukt sync` publishes the project the caller
stands in, and only that project: from the committed state of its canonical
main it writes the project's published domains into the home, patches the
global index so that exactly the project's own entries change, compiles every
`AGENTS.master.md` in the project into the `AGENTS.md` beside it, and records
the commit it published beside the project's checkout path. The project that owns the `general` domain also
writes the global agent entrypoints of the supported agent CLIs and the one
settings key each needs to load them. Every sync that publishes recomputes the eval
fingerprint. Publication never reads another project's repository, keeps no
registry of projects, and does nothing when main equals the commit it last
published. It does not own the snippet model, validation or retrieval, the
configuration it reads, or the git hooks that trigger it.

## Canonical fields

### Ubiquitous language

**Home**
:  `~/.instrukt`, the fixed place on this computer that holds publication and
   state. Not a git repository; publication outputs are re-creatable from
   committed sources. The home's recovery exception is defined by
   `local/projects/instrukt/design/architecture/overview`.

**Canonical main**
:  The default branch of a project as it stands in the project's primary
   worktree — never a linked worktree's checkout, never the working tree's
   uncommitted state.

**Published part**
:  What one project owns in the home: its published domains (snippets,
   publisher record, bundle), its global index entries, its local index
   entries, and its published commit beside its checkout path.

**Published domain**
:  A domain a project declares in its `publishes` setting with its trigger
   line, authored under `docs/global/<domain>/` and published to
   `~/.instrukt/docs/<domain>/`.

**Owner**
:  The project that published an index entry or a domain. Every index entry
   carries its owner as `source_project`; every published domain carries a
   publisher record naming its owner and trigger.

**Global index**
:  The one index of the home: every published entry of every project, global
   and local, each with its scope and owner.

**Patch**
:  The index write of one sync: drop the entries the invoking project owns,
   add its new ones, keep every other entry unchanged.

**Published commit**
:  The commit of canonical main a project's published part was made from,
   recorded per project beside its checkout path under
   `~/.instrukt/state/publication/`; the runner consumes that existing state
   (`local/projects/instrukt/spec/domain/runner`).

**Retraction**
:  Removal of what a project published before and no longer owns.

**Master**
:  An `AGENTS.master.md` file in a project; the source an `AGENTS.md` is
   compiled from.

**Compilation**
:  Expanding a master, a bundle or a global entrypoint into one self-contained
   file: required reads inlined, include fragments expanded and `@config`
   directives resolved, planned material dropped, no `@` reference left.

**Bundle**
:  The generated `bundle.md` of a published domain: its baseline manifest
   compiled, followed by its progressive baseline index and activatable
   procedures.

**Global agent entrypoint**
:  The user-scope instruction file of a supported agent CLI, compiled from the
   `general` owner's global master, the `general` bundle, the agent's addendum
   and the roster of published domains.

**Entrypoint-loading key**
:  The one key in an agent CLI's user settings that makes it load `AGENTS.md`
   files.

**Eval fingerprint**
:  The digest of every published snippet that declares `eval: true`, recorded
   per published commit; the current digest is available to work
   (`local/projects/instrukt/spec/feature/publication/eval-fingerprint`).

**Docs lock**
:  The home-wide lock under which every write to the home's publication
   happens, so syncs of different projects never interleave.

### Ownership boundary

- **Owns** — reading a project's committed main for publication; the
  published-commit and checkout-path record and the no-op; the per-domain publisher record;
  index patching by owner with its duplicate-id and domain-ownership refusals;
  retraction; compilation of bundles, masters and global entrypoints; writing
  the global agent entrypoints and merging their entrypoint-loading keys; the
  eval fingerprint; owner-scoped publication and retraction of domain workflow
  definitions, roles, actions and templates as non-snippet files.
- **Does not own** — snippet identity, the reference forms, the `@config` and
  include directive grammar, pending carriers, structural validation, and
  every read of the home (docs,
  `local/projects/instrukt/spec/domain/docs`); the layered configuration, its
  schema migration and the runtime mapping (configuration,
  `local/projects/instrukt/spec/domain/configuration`); the hook sections that
  start a sync (repository,
  `local/projects/instrukt/spec/domain/repository`); stamps that carry the
  eval fingerprint (work, `local/projects/instrukt/spec/domain/work`); skills,
  MCP configuration and every other agent setting (outside instrukt).
- **Published contracts** — `instrukt sync`; the home's published tree
  (`~/.instrukt/docs/<domain>/…`, per-domain `publisher.yaml` and `bundle.md`);
  the global index `~/.instrukt/docs/index.yaml`; the per-project published
  commit beside its checkout path under `~/.instrukt/state/publication/`; the
  eval fingerprint per published commit under `~/.instrukt/state/eval-fingerprint/`; the compiled `AGENTS.md` files in a project; the
  global agent entrypoints.
- **Allowed dependencies** — docs (snippet model, as customer); configuration
  (project slug, `publishes`, runtime mapping, and its migration operation, as
  customer).
- **Implementation roots** — `crates/instrukt-core/` (publication);
  `crates/instrukt-cli/` (command surface).

### Aggregates

#### Global index (aggregate)

- **Root:** the home's global index.
- **Contains:** index entries (id, description, scope, owner, the absolute
  path of a local entry's file in its project, pending ownership).
- **Invariants:**
  - An id appears at most once.
  - Every entry names exactly one owner.
  - A published domain's global entries all have the domain's owner.
  - A patch by one project changes only entries that project owns.
- **Identifier:** one per home.
- **Lifecycle:** created by the first sync on a computer; patched by every
  sync that publishes; re-created from the projects if the home is deleted.

#### Project publication (aggregate)

- **Root:** the published part of one project, identified by its slug.
- **Contains:** published domains with their publisher records and bundles;
  the project's entries in the global index; the compiled `AGENTS.md` files
  in its primary worktree; its published commit and checkout path.
- **Invariants:**
  - Its publication outputs were produced from one commit: the published commit.
  - Its state records the project's checkout path beside that commit.
  - A sync that is refused or fails leaves the previous publication readable
    and records no commit.
  - Nothing the project published before and no longer owns remains.
- **Identifier:** the project slug from the project's `instrukt.yaml`.
- **Lifecycle:** created by the project's first sync; replaced by each sync
  whose main differs from the published commit.

#### Global agent entrypoints (aggregate)

- **Root:** the set of user-scope entrypoints of the supported agent CLIs.
- **Contains:** one compiled entrypoint per agent CLI present on the
  computer; one entrypoint-loading key per agent that needs one.
- **Invariants:**
  - Written only by the sync of the project that owns `general`.
  - A settings merge changes the entrypoint-loading key and no other key.
- **Identifier:** one per computer.
- **Lifecycle:** written by the `general` owner's sync; replaced on its next
  sync that publishes.

### Domain events emitted

None. A sync's outcome is its exit status, its output and the state it
records.

### Domain events consumed

None. A sync starts when a developer, an agent or a repository hook section
runs it.

### Use cases (index of feature specs)

- **project-publication** — `docs/local/spec/feature/publication/project-publication.md` — a project's committed docs reach the agents on this computer, side by side with other projects' docs, without either taking the other's.
- **agent-entrypoints** — `docs/local/spec/feature/publication/agent-entrypoints.md` — project instructions compile into `AGENTS.md`, and the `general` owner gives every supported agent CLI its global baseline.
- **eval-fingerprint** — `docs/local/spec/feature/publication/eval-fingerprint.md` — every publishing sync leaves a digest of the evaluated doctrine for stamps to carry.

### Governing designs and decisions (index)

- **`local/projects/instrukt/design/architecture/overview`** — the publish
  flow, the home layout, and the invariant that each project publishes only
  its own part from committed state.

### Context map position

- **docs** — Customer / Supplier, both ways. Docs supplies the snippet model
  publication compiles from; publication supplies the home's published tree
  and global index that retrieval reads, and the published commit the
  behind-main warning compares with.
- **configuration** — Customer / Supplier. Configuration supplies the project
  slug, the published domains with their triggers, the runtime mapping and
  the migration operation invoked before publication.
- **repository** — Customer / Supplier. Repository's hook sections run
  `instrukt sync`; publication decides what a run does.
- **work** — Customer / Supplier. Publication supplies the current fingerprint
  and its per-published-commit association; stamp behavior belongs to
  `local/projects/instrukt/spec/domain/work`.
- **runner** — Customer / Supplier. Publication supplies project checkout paths
  beside published commits in its existing state
  (`local/projects/instrukt/spec/domain/runner`).

- **Workflow** — Customer / Supplier. Publication supplies definitions, roles,
  actions and templates from `docs/global/<domain>/workflow/` to
  `~/.instrukt/docs/<domain>/workflow/`. The engine selects the domain with
  project key `workflow.domain` (default `sdlc`); these files have no snippet
  index entries.

## Known caveats

- `instrukt sync --force` republishes even when main is unchanged.
- A sync contributes its result, including a refusal, to hook-run outcome
  reporting owned by
  `local/projects/instrukt/spec/feature/repository/hook-sections`; the
  behind-main warning belongs to `local/projects/instrukt/spec/domain/docs`.

## See Also

- docs/local/spec/domain/context-map.md — the bounded contexts and their relationships.
- docs/local/design/architecture/overview.md — the publish flow and the home layout.
