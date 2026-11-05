---
description: "Repository — instrukt's git integration in a project: adopting a repository with `instrukt init` and reporting its claims, instrukt's marked sections in git hooks, and attributed commits through `instrukt repo commit`."
delivered_by: [instrukt-first-build]
---
# Repository — Spec

## What it is

The repository context is where instrukt meets a project's git repository. It
adopts a repository (`instrukt init`): the project layer, `docs/project/`
with its index and starting schema, a purpose-only root master, the ignore
lines, the editor schemas with their modelines (generated through
publication's operation) and the hook sections, and it reports what the
repository's first sync will claim on the computer. It owns instrukt's
marked section in each git hook it uses, living beside whatever else the
project and its tools put there, and routes each hook to the context that
acts on it — publication after main changes, the blueprint on a push to main
or a pull advancing main, when blueprint is enabled.
It composes attributed commits (`instrukt repo commit`): the acting agent as
author, the session it ran in, the caller's declaration of human involvement,
and the work item when given. It does not decide what a sync publishes or a
graph contains, owns no lint or test hook, and never runs the project's own
commands.

## Canonical fields

### Ubiquitous language

**Hook file**
:  An executable file in the repository's hooks directory that git runs at
   a hook point.

**Hooks directory**
:  The directory git runs hooks from: the one `core.hooksPath` names when it
   is set, otherwise the repository's own hooks directory.

**Section**
:  instrukt's part of a hook file: the lines from `# >>> instrukt >>>` to
   `# <<< instrukt <<<`. Each tool sharing a hook file owns its own section.

**Hook run**
:  What a section calls: `instrukt hooks run <hook>`, which hands the hook
   point to the context that acts on it and returns without waiting.

**Hook manager**
:  A tool that generates the project's hook files itself and may drop
   sections it did not write.

**Adoption**
:  The state `instrukt init` establishes in a repository: project layer,
   `docs/project/` with its `index.yaml` and, when sdlc is published, its
   `schema.yaml`, the root master, the ignore lines, the editor schemas and
   modelines publication generates, and the hook sections.

**Claims report**
:  What `instrukt init` reports in a repository holding `docs/general/` or a
   `docs/domain/<x>/`: the positions its first sync will claim, their current
   owners, and whether that sync will render the global agent files.

**Attributed commit**
:  A commit made through `instrukt repo commit`: authored by the acting
   agent when one resolves, its message followed by the trailers instrukt
   owns.

**Trailer**
:  A `Key: value` line instrukt appends to a commit message: `Session:`,
   `Human-Involved:`, `Todo:` (recorded association), `Planned-Todo:` (planned
   association).

**Active runtime**
:  The first entry of the runtime mapping whose session variable is set in
   the caller's environment (`local/projects/instrukt/spec/domain/configuration`).

**Active agent**
:  The first agent CLI entry of the runtime mapping whose session variable is
   set in the caller's environment.

### Ownership boundary

- **Owns** — repository adoption and its claims report; the copy of the
  published `sdlc` `schema.yaml` into a new project; instrukt's section in
  each hook file, its placement,
  idempotence and removal; the hook status report, including each section's
  last run outcome and refused syncs; which hooks instrukt uses
  and what each hook run starts; composition of attributed commits: author,
  trailers, refusal of caller-written trailers, the paths committed;
  commit-review verdicts and report digests recorded as git notes.
- **Does not own** — what a sync publishes and the ownership it records
  (publication, `local/projects/instrukt/spec/domain/publication`); the
  editor schemas and modelines init asks publication to write (publication,
  `local/projects/instrukt/spec/feature/publication/editor-schemas`); graph
  generation for a
  remote-main revision (blueprint, `local/projects/instrukt/spec/domain/blueprint`);
  the project layer's schema, the runtime mapping and the agents' git authors
  (configuration, `local/projects/instrukt/spec/domain/configuration`); a work
  item's existence (work, `local/projects/instrukt/spec/domain/work`); the
  project's lint, test and other hooks and actions (the project).
- **Published contracts** — `instrukt init`;
  `instrukt hooks install|remove|status`; `instrukt hooks run <hook>`;
  `instrukt repo commit`;
  `instrukt repo stamp <sha…> --verdict approve|needs_work --report <file>`;
  `instrukt repo stamp <sha…> --verdict exempt --reason <text>`;
  `instrukt init --slug <slug>`; the claims report; the section markers;
  the trailer keys.
- **Allowed dependencies** — configuration (project slug, runtime mapping,
  agents' git authors, module switches, as customer); publication (the ownership record the claims report reads, the
  published `sdlc` `schema.yaml` init copies, and the editor-schema
  operation init invokes, as customer); publication and
  blueprint (started by hook runs, as customer); work (todo existence through
  the core-declared interface wired by the CLI only when todos is enabled).
- **Implementation roots** — `crates/instrukt-core/` (repository);
  `crates/instrukt-cli/` (command surface).

### Aggregates

#### Hook section (aggregate)

- **Root:** instrukt's section in one hook file.
- **Contains:** the section markers and the one hook-run call between them.
- **Invariants:**
  - A hook file holds at most one instrukt section.
  - No instrukt operation changes a byte of a hook file outside its section.
  - A section never `exec`s a command (never replaces the shell) and never
    ends the hook early, so every section after it runs; a redirection-only
    `exec` is allowed, and the pre-push section uses one to give the sections
    after it the full ref list on stdin.
  - A section in a post-hook never fails or delays the git operation.
- **Identifier:** the hook name within the repository's hooks directory.
- **Lifecycle:** written by `instrukt hooks install` or `instrukt init`;
  removed by `instrukt hooks remove`; lost when a hook manager regenerates the
  file, which `instrukt hooks status` reports.

#### Adoption (aggregate)

- **Root:** the adopted repository.
- **Contains:** the project layer `instrukt.yaml`, `docs/project/` with its
  `index.yaml` and `schema.yaml`, the root `AGENTS.master.md`, the ignore
  lines, `.instrukt/schemas/` and the modelines, the hook sections.
- **Invariants:**
  - Adoption never overwrites a file that exists, apart from the modeline
    publication keeps at the top of the files it describes.
  - Adoption publishes and claims nothing; its claims report only reads.
  - Running it again on an adopted repository changes nothing.
- **Identifier:** the repository.
- **Lifecycle:** created by `instrukt init`; completed by a later run when a
  part is missing.

#### Attributed commit (aggregate)

- **Root:** the commit.
- **Contains:** the caller's message, the author, the trailers, the paths.
- **Invariants:**
  - The caller's message holds no line with a trailer key instrukt owns.
  - A `Session:` trailer and an agent author appear only when the
    environment resolves them; never a guess.
  - `Human-Involved:` is always present and records the caller's declaration.
- **Identifier:** the commit SHA.
- **Lifecycle:** created once; git owns it from then on.

Commit-review notes bind a verdict and complete report digest to each named
commit. instrukt pushes and fetches the notes ref explicitly, so they reach
every machine without a separate work record. The integrator reads them; their command contract is owned by
`local/projects/instrukt/spec/feature/repository/attributed-commit`.

### Domain events emitted

None. Hook runs start other contexts' commands directly.

### Domain events consumed

None. Git invokes hook files; a developer or agent invokes the commands.

### Use cases (index of feature specs)

- **hook-sections** — `docs/local/spec/feature/repository/hook-sections.md` — instrukt's hook sections live beside the project's own hooks and keep publication current without blocking git.
- **attributed-commit** — `docs/local/spec/feature/repository/attributed-commit.md` — an agent's commit names the agent, its session and the human's involvement; outside a session it is the developer's own.
- **project-initialization** — `docs/local/spec/feature/repository/project-initialization.md` — a developer adopts instrukt in a repository in one idempotent step and sees what it will claim.

### Governing designs and decisions (index)

- **`local/projects/instrukt/design/architecture/overview`** — git hooks
  belong to everyone; instrukt touches only its marked section and never
  blocks git after the fact.

### Context map position

- **configuration** — Customer / Supplier. Configuration supplies the project
  slug, the runtime mapping, the agents' git authors and the blueprint switch.
- **publication** — Customer / Supplier. Post-hook runs start
  `instrukt sync`; publication decides what it does. Publication supplies the
  ownership record and published `sdlc` schema that `init` reads, and the
  editor-schema operation `init` invokes.
- **blueprint** — Customer / Supplier. With blueprint enabled, hook runs hand
  a push to main or a pull advancing main to blueprint
  (`local/projects/instrukt/spec/feature/blueprint/graph-generation`).
- **work** — Customer / Supplier. Recorded and planned associations are named
  in `Todo:` and `Planned-Todo:` trailers; work owns whether the item exists.

- **workflow** — Customer / Supplier. Repository supplies candidate integration
  branches and exact-commit main advancement.
- Core declares the optional todo-existence and graph-generation interfaces;
  the CLI wires work and blueprint implementations when their modules are on.

## See Also

- docs/local/spec/domain/context-map.md — the bounded contexts and their relationships.
