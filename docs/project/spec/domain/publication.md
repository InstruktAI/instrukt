---
description: Publication — a project publishes its general, domain and project positions from its committed main into the home, claims each by owner and patches the global index by owner; a repository's rendered root `AGENTS.md` is committed, rendered at each commit from the repository's committed content alone; and the owner of `docs/general/` renders a global agent file, with its settings key, for each agent runtime the selected runtime reports as configured.
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
instruction files. A repository carries one rendered instruction file, its root
`AGENTS.md`, which is committed: it is rendered from the repository's committed content alone at each commit, by
the `pre-commit` hook section and by `instrukt docs render`, never by a sync. A company's repository also carries the company's plugin,
which `instrukt docs plugin` renders on request from the same company index, with the plugin's one entry in the repository's plugin catalog.
Every sync that publishes recomputes the eval fingerprint
and regenerates the editor schemas and modelines in the project's primary
worktree, which `instrukt init` also writes through the same operation.
Publication never reads another project's repository, keeps no registry of
projects, and after configuration migration converges: it writes only what is
missing or different, whether or not main has moved. It
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

**Entry index**
:  The `index.yaml` a committed entrypoint is rendered from:
   `docs/project/index.yaml` in a project, `docs/index.yaml` in a company
   home, which selects from the company's own documents
   (`project/instrukt/spec/feature/publication/agent-entrypoints`).

**Rendering**
:  Producing one instruction file from documentation: selected snippets in
   full with their closure, listing lines, triggered-procedure lines, planned
   material dropped, no import left for an agent program to resolve. A
   global agent file inlines what is published on the computer and resolves
   `@config`; a committed entrypoint inlines only what its repository holds,
   names other repositories' doctrine by the command that reads it, and
   admits no configured value. Producing a company plugin from the company
   index is a rendering too: the same selection, held as the files of the
   plugin.

**Committed entrypoint**
:  The `AGENTS.md` at the root of a repository that holds its entry index, a
   project's or a company home's: committed with the repository, opening with
   the generated line that marks it as a rendering, and a function of the
   repository's committed content and the rendering instrukt release only. A
   repository has at most one.

**Company plugin**
:  The Claude plugin a company's repository carries in `plugins/<key>/`,
   `<key>` being the company's key: the selection the company index makes,
   held as one skill with a manifest. It is no instruction file
   (`project/instrukt/spec/feature/publication/company-plugin`).

**Plugin catalog**
:  `.claude-plugin/marketplace.json` at the root of a company's repository,
   from which Claude installs the plugins it lists. The repository's own
   file.

**Catalog entry**
:  The one member of the plugin catalog that lists the company plugin,
   named by the company's key.

**Visibility**
:  A company document's `visibility`: `internal` or `public`. Not the
   `visibility` of a snippet, which is docs'
   (`project/instrukt/spec/domain/docs`).

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
  published-commit and checkout-path record and convergence; the ownership
  record and its refusals (a second claimant, a reserved domain name);
  index patching by owner with its duplicate-id refusal; retraction;
  rendering of the committed root `AGENTS.md` — a project's and a company
  home's — and of the global agent files; rendering of a company's plugin
  from the company index, its version rule, and the merge of the plugin's
  one entry into the repository's plugin catalog; a company document's
  `visibility` and its findings; the choice of global
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
  `project/instrukt/spec/domain/work`); an agent runtime's installed
  skills, its MCP configuration and every other agent setting on a computer
  (outside instrukt); the plugin catalog's own name and owner and every
  entry but the company plugin's (the company's repository); committing and
  pushing what a rendering changed (its caller); whether and when a Claude
  client takes a new copy of a plugin (Claude).
- **Published contracts** — `instrukt sync`; the home's published tree
  (`~/.instrukt/docs/general/…`, `~/.instrukt/docs/<x>/…`, each with its
  `index.yaml` and `schema.yaml` when the position has them); the global
  index `~/.instrukt/state/index.yaml`; the ownership record, published
  commits and checkout paths under
  `~/.instrukt/state/publication/`; the eval fingerprint per published commit
  under `~/.instrukt/state/eval-fingerprint/`; the committed root `AGENTS.md`
  of a repository and `instrukt docs render [--check]`; a company's plugin
  under `plugins/<key>/` with its entry in `.claude-plugin/marketplace.json`,
  and `instrukt docs plugin --key <key> --name <name>`, which asks for a
  rendering, with `--check` for a check and with `--validate` for a
  validation, the two flags excluding each other
  (`project/instrukt/spec/feature/publication/company-plugin`); whether a
  repository's `docs/index.yaml` is its company index, which docs
  validation asks before reading the file; the global
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

- **Root:** the one `AGENTS.md` at the root of a project's repository or a
  company home.
- **Contains:** the rendered file, its generated line and the entry index it
  is rendered from.
- **Invariants:**
  - A function of the repository's committed content and the rendering
    instrukt release: nothing published on the computer, no configuration
    layer and no environment enters it.
  - Changes only in a commit, or in the working tree through
    `instrukt docs render`; no sync, checkout, start or tick writes it.
  - Written over whatever the file holds; removed, when its entry index no
    longer exists, only while it is instrukt's rendering.
  - At most one per repository, at its root; a rendering of instrukt's below
    the root is removed.
  - A company home's rendering is the selection its index makes from the
    company's own documents and references nothing outside them.
  - It has no published commit, no claim and no index entry, and belongs to
    no project's published part.
- **Identifier:** the file's path in its repository or home.
- **Lifecycle:** rendered and staged by the `pre-commit` section at each
  commit, or written by `instrukt docs render`, including a repository's CI
  workflow; removed with its entry index
  (`project/instrukt/spec/feature/publication/agent-entrypoints`).

#### Company plugin (aggregate)

- **Root:** the folder `plugins/<key>/` of a company's repository, `<key>`
  being the company's key.
- **Contains:** the manifest with the plugin's version; the skill's main
  file; one file for each other document the plugin carries; the plugin's
  entry in the repository's plugin catalog.
- **Invariants:**
  - A function of the repository's company index and documents, the key and
    the name its caller gives, the plugin the repository already holds and
    the rendering instrukt release: nothing published on the computer, no
    configuration layer and no environment enters it.
  - It holds the selection the company index makes, as the repository's
    committed `AGENTS.md` does, and carries every document it names.
  - The version is the one thing a rendering takes from the plugin the
    repository holds. Its last number is raised by one exactly when the
    rendering differs from that plugin; instrukt never changes the first
    two.
  - Changes only in the working tree, through `instrukt docs plugin`; no
    commit, sync, checkout, start or tick writes it, and rendering a
    repository's `AGENTS.md` writes none.
  - The folder is written whole, whatever it held, and never removed.
    Nothing is read, written or removed through a symbolic link.
  - Of the plugin catalog, a rendering changes the plugin's entry and
    nothing else; it creates no catalog.
  - With a finding, no file is written or removed.
  - It has no published commit, no claim and no index entry, and belongs to
    no project's published part.
- **Identifier:** the company's key, in its repository.
- **Lifecycle:** rendered when a caller asks for it; committed by that
  caller; left as it is where the repository holds no company index
  (`project/instrukt/spec/feature/publication/company-plugin`).

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
- **agent-entrypoints** — `project/instrukt/spec/feature/publication/agent-entrypoints` — a repository's committed `AGENTS.md` gives any agent that checks it out what the project selected, a company home's gives the company's own knowledge, and the `docs/general/` owner gives every configured agent runtime a rendered global file, written whole over whatever the destination held.
- **eval-fingerprint** — `project/instrukt/spec/feature/publication/eval-fingerprint` — every publishing sync leaves a digest of the evaluated doctrine for stamps to carry.
- **editor-schemas** — `project/instrukt/spec/feature/publication/editor-schemas` — a developer's editor checks instrukt's YAML files against the rules the installed instrukt validates with.
- **docs-repositories** — `project/instrukt/spec/feature/publication/docs-repositories` — a computer takes its doctrine from adopted docs repositories, which anyone may fork or replace; instrukt enforces their format and never depends on their wording.
- **instrukt-status** — `project/instrukt/spec/feature/publication/instrukt-status` — an agent or developer learns in one read where this computer and repository stand with instrukt before changing anything.
- **company-plugin** — `project/instrukt/spec/feature/publication/company-plugin` — people and agents who work for a company through Claude without a checkout receive the company's knowledge as a plugin whose version changes exactly when the knowledge does.

## Governing designs and decisions

- **`project/instrukt/design/architecture/overview`** — the publish
  flow, the home layout, and the invariant that each project publishes only
  its own part from committed state.

## Context map position

- **docs** — Customer / Supplier, both ways. Docs supplies the snippet model,
  the `index.yaml` and `schema.yaml` model and validation publication renders
  and refuses by; publication supplies the home's published tree and global
  index that retrieval reads, the published commit the behind-main
  warning compares with, and whether a repository's `docs/index.yaml` is
  its company index, which validation leaves to publication.
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
<!-- planned:fleet-eval-aggregation -->
- **work (evaluation report)** — Customer / Supplier. Publication supplies the
  projects a home publishes and their checkout paths from its existing state,
  read by the evaluation report (`project/instrukt/spec/feature/work/eval-report`).
<!-- /planned:fleet-eval-aggregation -->
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
- **Claude's plugin formats** — Conformist. The company plugin, its skill
  and its catalog entry follow Claude's plugin, skill and plugin catalog
  formats as their vendor states them
  (`project/instrukt/spec/feature/publication/company-plugin`). instrukt
  writes files in those formats and runs no Claude program.

## Known caveats

- The global agent files are rendered only by the `docs/general/` owner's
  sync. A domain another project publishes, or changes, reaches them at that
  owner's next sync; `instrukt sync` in that project renders them at once.
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
