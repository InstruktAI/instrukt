---
description: Publication — a project publishes its general, domain and project positions from its committed main into the home, claims each by owner and patches the global index by owner; a repository's `AGENTS.md` files are rendered from its committed content at each commit; and the owner of `docs/general/` renders a global agent file, with its settings key, for each agent runtime the selected runtime reports as configured.
---
# Publication — Spec

## What it is

The publication context turns a project's committed documentation into what
agents on this computer read. `instrukt sync` publishes the project the caller
stands in, and only that project: from the committed state of its canonical
main it publishes the project's general, domain and project positions
into the home, claims each of them for the project, patches the
global index so that exactly the project's own entries change, and records
the commit it published beside the project's checkout path. The project that
owns `docs/general/` also renders a global agent file for each agent runtime
the selected runtime reports as configured, from what is published on the
computer, and sets the one settings key each needs to load project
instruction files. A repository's `AGENTS.md` files are committed: they are
rendered from the repository's committed content alone at each commit, by
the `pre-commit` hook section and by `instrukt docs render`, never by a sync. Every sync that publishes recomputes the eval fingerprint
and regenerates the editor schemas and modelines in the project's primary
worktree, which `instrukt init` also writes through the same operation.
Publication never reads another project's repository, keeps no registry of
projects, and after configuration migration does nothing when main equals the
commit it last published. It
does not own the snippet model, `index.yaml` and `schema.yaml`, validation or
retrieval, the configuration it reads, or the git hooks that trigger it.

## Ubiquitous language

**Home**
:  `~/.instrukt` by default, with isolated homes per worktree under
   `project/instrukt/spec/feature/configuration/owned-files`; holds publication and
   state. Not a git repository; publication outputs are re-creatable from
   committed sources. The home's recovery exception is defined by
   `project/instrukt/design/architecture/overview`.

**Canonical main**
:  The default branch of a project as it stands in the project's primary
   worktree — never a linked worktree's checkout, never the working tree's
   uncommitted state.

**Position**
:  One of the three places documentation lives in a repository:
   `docs/general/` (always on), `docs/domain/<x>/` (on demand), and
   `docs/project/` (the project). The docs context owns the snippet model of
   each (`project/instrukt/spec/domain/docs`).

**Published position**
:  A project's general, domain or project position, published from its
   committed main to `~/.instrukt/docs/general/` or `~/.instrukt/docs/<x>/`.
   The project position is likewise published to `~/.instrukt/docs/project/<slug>/…`.
   All three are read from that committed publication.

**Domain**
:  A published `docs/domain/<x>/` position: a body of knowledge one project is
   the authority on, published as one unit, named after what it is about.
   `general` and `project` are reserved and never a domain's name, nor is
   `third-party`, which keeps its existing special case.

**Owner**
:  The project that published an index entry or a position. Every index entry
   carries its owner as `source_project`; the claim on every published
   position is recorded in the ownership record.

**Ownership record**
:  Internal state under `~/.instrukt/state/publication/` naming, per
   published position, the project that owns it on this computer. Not a file
   any project authors.

**Published part**
:  What one project owns in the home: its published positions, its index
   entries, its claims in the ownership record, and its published commit
   beside its checkout path.

**Global index**
:  The one generated index of the home, `~/.instrukt/state/index.yaml`: every
   entry of every project, from every position, each with its owner.

**Patch**
:  The index write of one sync: drop the entries the invoking project owns,
   add its new ones, keep every other entry unchanged.

**Published commit**
:  The commit of canonical main a project's published part was made from,
   recorded per project beside its checkout path under
   `~/.instrukt/state/publication/`; the runner consumes that existing state
   (`project/instrukt/spec/domain/runner`).

**Retraction**
:  Removal of what a project published before and no longer owns.

**Master**
:  An `AGENTS.master.md` file in a project or a company home. A project's root master holds only the
   project's `## Purpose`; a nested master is the source of the `AGENTS.md`
   beside it. A company home's master is the company's own knowledge, rendered
   into the home's `AGENTS.md`
   (`project/instrukt/spec/feature/publication/agent-entrypoints`).

**Rendering**
:  Producing one instruction file from documentation: selected snippets in
   full with their closure, listing lines, triggered-procedure lines, planned
   material dropped, no import left for an agent program to resolve. A
   global agent file inlines what is published on the computer and resolves
   `@config`; a committed entrypoint inlines only what its repository holds,
   names other repositories' doctrine by the command that reads it, and
   admits no configured value.

**Committed entrypoint**
:  An `AGENTS.md` beside an `AGENTS.master.md` in a repository or a company
   home: committed with the repository, opening with the generated line that
   marks it as a rendering, and a function of the repository's committed
   content and the rendering instrukt release only.

**Configured agent runtime**
:  An agent runtime the runtime `runner.runtime` selects reports as configured
   and enabled — for T3, an enabled provider instance — with the home that
   instance runs with.

**Global agent file**
:  The user-scope instruction file of a configured agent runtime, in the home
   it runs with, rendered from what is published on the computer: `general`'s
   section, one group per other domain, then that runtime's `agents`
   snippets.

**Entrypoint-loading key**
:  The one key in an agent runtime's user settings that makes it load
   `AGENTS.md` files.

**Eval fingerprint**
:  The digest of every published snippet that declares `eval: true`, recorded
   per published commit; the current digest is available to work
   (`project/instrukt/spec/feature/publication/eval-fingerprint`).

**Docs lock**
:  The home-wide lock under which every write to the home's publication
   happens, so syncs of different projects never interleave.

## Ownership boundary

- **Owns** — reading a project's committed main for publication; the
  published-commit and checkout-path record and the no-op; the ownership
  record and its refusals (a second claimant, a reserved domain name);
  index patching by owner with its duplicate-id refusal; retraction;
  rendering of the committed entrypoints — a project's `AGENTS.md` files and
  each company home's — and of the global agent files; the choice of global
  file targets from the configured agent runtimes; writing those files whole,
  with no record of what was written;
  merging the entrypoint-loading keys; the editor
  schemas in a repository's `.instrukt/schemas/` and their modelines, which
  sync and `instrukt init` write; the eval
  fingerprint; owner-scoped publication and retraction of domain workflow
  definitions, roles, actions and templates as non-snippet files; the
  `instrukt status` report, which reads this state together with the
  configuration layers and the repository's hook sections, and changes
  nothing.
- **Does not own** — snippet identity, the reference forms, the `@config`
  directive grammar, pending carriers, `index.yaml` and `schema.yaml` and
  their findings, structural validation, `instrukt docs get --domain` and
  every other read of the home (docs,
  `project/instrukt/spec/domain/docs`); the layered configuration, its
  schema migration and runtime choice (configuration,
  `project/instrukt/spec/domain/configuration`); the hook sections that
  start a sync, adoption and its claims report (repository,
  `project/instrukt/spec/domain/repository`);
  stamps that carry the eval fingerprint (work,
  `project/instrukt/spec/domain/work`); skills, MCP configuration and
  every other agent setting (outside instrukt).
- **Published contracts** — `instrukt sync`; the home's published tree
  (`~/.instrukt/docs/general/…`, `~/.instrukt/docs/<x>/…`, each with its
  `index.yaml` and `schema.yaml` when the position has them); the global
  index `~/.instrukt/state/index.yaml`; the ownership record, published
  commits and checkout paths under
  `~/.instrukt/state/publication/`; the eval fingerprint per published commit
  under `~/.instrukt/state/eval-fingerprint/`; the committed `AGENTS.md`
  files of a repository and `instrukt docs render [--check]`; the global
  agent files; `.instrukt/schemas/` and the modeline
  form (`project/instrukt/spec/feature/publication/editor-schemas`);
  `instrukt status [--json]` and its report
  (`project/instrukt/spec/feature/publication/instrukt-status`).
- **Allowed dependencies** — docs (snippet model, `index.yaml` and
  `schema.yaml` model, validation, as customer); configuration (project slug,
  runtime choice, its migration operation, and the configuration schema type
  the editor schema of `instrukt.yaml` is generated from, as customer);
  for `instrukt status` only, repository (hook-section state, as customer);
  runner (the configured agent runtimes and their homes, read through the
  selected runtime's adapter behind an interface core declares and the CLI
  wires, as customer).
- **Implementation roots** — `crates/instrukt-core/` (publication);
  `crates/instrukt-cli/` (command surface).

## Canonical fields

### Aggregates

#### Global index (aggregate)

- **Root:** the home's global index.
- **Contains:** index entries (id, description, position, owner, published-file
  location, source checkout provenance and pending ownership).
- **Invariants:**
  - An id appears at most once.
  - Every entry names exactly one owner.
  - A published position's entries all have the position's owner.
  - A patch by one project changes only entries that project owns.
- **Identifier:** one per home.
- **Lifecycle:** created by the first sync on a computer; patched by every
  sync that publishes; re-created from the projects if the home is deleted.

#### Project publication (aggregate)

- **Root:** the published part of one project, identified by its slug.
- **Contains:** its published positions with their claims; the project's
  entries in the global index; its published commit and checkout path.
- **Invariants:**
  - Its publication outputs were produced from one commit: the published
    commit.
  - Its state records the project's checkout path beside that commit.
  - Each position it publishes is claimed by it alone on this computer.
  - A sync that is refused or fails leaves the previous publication readable
    and records no commit.
  - Nothing the project published before and no longer owns remains.
- **Identifier:** the project slug from the project's `instrukt.yaml`.
- **Lifecycle:** created by the project's first sync; replaced by each sync
  whose main differs from the published commit.

#### Global agent files (aggregate)

- **Root:** the set of user-scope instruction files of the configured agent
  runtimes.
- **Contains:** one rendered file per configured agent runtime that has a
  user-scope file, per home its instances run with; one entrypoint-loading key
  per agent runtime that needs one. Nothing records which files were written.
- **Invariants:**
  - Written only by the sync of the project that owns `docs/general/`.
  - Written whole by every sync that publishes them, whatever the destination
    held; never removed.
  - A settings merge changes the entrypoint-loading key and no other key.
  - Targets only the agent runtimes the selected runtime reports as
    configured; when it cannot be asked, nothing in the set changes.
- **Identifier:** one per computer.
- **Lifecycle:** written by the `docs/general/` owner's sync; replaced on its
  next sync that publishes; left where it is when its runtime is no longer
  configured or that project stops publishing `docs/general/`.

#### Committed entrypoint (aggregate)

- **Root:** one `AGENTS.md` beside a master, in a project's repository or a
  company home.
- **Contains:** the rendered file, its generated line and the master it is
  rendered from.
- **Invariants:**
  - A function of the repository's committed content and the rendering
    instrukt release: nothing published on the computer, no configuration
    layer and no environment enters it.
  - Changes only in a commit, or in the working tree through
    `instrukt docs render`; no sync, checkout, start or tick writes it.
  - Written over whatever the file holds; removed, when its master no longer
    exists, only while it opens with the generated line.
  - A company home's rendering is its master's own content and references
    nothing outside the company's own documents.
  - It has no published commit, no claim and no index entry, and belongs to
    no project's published part.
- **Identifier:** the file's path in its repository or home.
- **Lifecycle:** rendered and staged by the `pre-commit` section at each
  commit, or written by `instrukt docs render`, including a repository's CI
  workflow; removed with its master
  (`project/instrukt/spec/feature/publication/agent-entrypoints`).

### Domain events emitted

None. A sync's outcome is its exit status, its output and the state it
records.

### Domain events consumed

None. A sync starts when a developer, an agent or a repository hook section
runs it.

## Event subjects and origins

This context emits no events.

## Use cases

- **project-publication** — `project/instrukt/spec/feature/publication/project-publication` — a project's committed positions reach the agents on this computer, side by side with other projects' docs, without either taking the other's.
- **agent-entrypoints** — `project/instrukt/spec/feature/publication/agent-entrypoints` — a repository's committed `AGENTS.md` gives any agent that checks it out what the project selected, a company home's gives the company's own knowledge, and the `docs/general/` owner gives every configured agent runtime a rendered global file without overwriting one it did not write.
- **eval-fingerprint** — `project/instrukt/spec/feature/publication/eval-fingerprint` — every publishing sync leaves a digest of the evaluated doctrine for stamps to carry.
- **editor-schemas** — `project/instrukt/spec/feature/publication/editor-schemas` — a developer's editor checks instrukt's YAML files against the rules the installed instrukt validates with.
- **docs-repositories** — `project/instrukt/spec/feature/publication/docs-repositories` — a computer takes its doctrine from adopted docs repositories, which anyone may fork or replace; instrukt enforces their format and never depends on their wording.
- **instrukt-status** — `project/instrukt/spec/feature/publication/instrukt-status` — an agent or developer learns in one read where this computer and repository stand with instrukt before changing anything.

## Governing designs and decisions

- **`project/instrukt/design/architecture/overview`** — the publish
  flow, the home layout, and the invariant that each project publishes only
  its own part from committed state.

## Context map position

- **docs** — Customer / Supplier, both ways. Docs supplies the snippet model,
  the `index.yaml` and `schema.yaml` model and validation publication renders
  and refuses by; publication supplies the home's published tree and global
  index that retrieval reads, and the published commit the behind-main
  warning compares with.
- **configuration** — Customer / Supplier. Configuration supplies the project
  slug, the runtime choice and the migration operation invoked before
  publication.
- **repository** — Customer / Supplier. Repository's hook sections run
  `instrukt sync`; publication decides what a run does. Publication supplies
  `instrukt init` the ownership record its claims report reads, the published
  `sdlc` schema it copies, and the editor-schema operation it invokes.
- **work** — Customer / Supplier. Publication supplies the current fingerprint
  and its per-published-commit association; stamp behavior belongs to
  `project/instrukt/spec/domain/work`.
- **runner** — Customer / Supplier, both ways. Publication supplies project
  checkout paths beside published commits in its existing state
  (`project/instrukt/spec/domain/runner`); the selected runtime's adapter
  supplies the configured agent runtimes and their homes the global agent
  files are written for, through an interface core declares and the CLI
  wires.
- **Workflow** — Customer / Supplier. Publication supplies definitions, roles,
  actions and templates from `docs/domain/<x>/workflow/` to
  `~/.instrukt/docs/<x>/workflow/`. The engine selects the domain with
  project key `workflow.domain` (default `sdlc`); these files have no snippet
  index entries.

- **distribution** — Published language. `instrukt status` is the state the
  instrukt cockpit skill reads on every visit
  (`project/instrukt/spec/domain/distribution`).

## Known caveats

- `instrukt sync --force` republishes even when main is unchanged.
- The global agent files are rendered only by the `docs/general/` owner's
  sync. A domain another project publishes, or changes, reaches them at that
  owner's next sync that publishes; `instrukt sync --force` in that project
  renders them at once.
- A sync contributes its result, including a refusal, to hook-run outcome
  reporting owned by
  `project/instrukt/spec/feature/repository/hook-sections`; the
  behind-main warning belongs to `project/instrukt/spec/domain/docs`.
- There is no backward compatibility for the pre-release layout:
  `docs/global/`, `docs/local/`, `publishes` and per-domain `publisher.yaml`
  are not read.

## See Also

- [project/instrukt/spec/domain/context-map](context-map.md)
- [project/instrukt/design/architecture/overview](../../design/architecture/overview.md)
