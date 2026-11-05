---
description: The docs bounded context — the snippet model across the three positions, position indexes and snippet schemas, structural validation, pending documentation, and retrieval through `instrukt docs index|get|schema`.
delivered_by: [instrukt-first-build]
---

# Docs — Spec

## What it is

The docs context owns documentation as the way agents are instructed: what a
snippet is, how its identity follows from the position it lives in, what a
domain is, what each position's `index.yaml` may say, which shape makes a
snippet valid (the root rules in the binary and a position's `schema.yaml`),
how pending material sits beside the current contract, and how an agent
discovers and reads snippets. It answers `instrukt docs validate`, `instrukt
docs index`, `instrukt docs get` and `instrukt docs schema`. Discovery reads the
generated global index in the home's state (`~/.instrukt/state/`); published
bodies are read from the home, while project bodies are read from the project
paths their index entries carry, including another project's file. Validation
and `docs schema` read the invoking repository's working tree. Docs never
writes the home, never renders an agent instruction file and never decides
what a project publishes. Publication writes published files, the global index
and every rendered agent file, and reads only its own repository; reading a
configured value is configuration's work.

## Canonical fields

### Ubiquitous language

**Position**
: One of the three places a repository's documentation lives:
`docs/general/` (always on: rendered into every agent CLI's user-scope
instruction file), `docs/domain/<x>/` (on demand: one line in that file, read
with `instrukt docs get --domain <x>`), and `docs/project/` (the project:
rendered into its root `AGENTS.md`). `general` and every domain are published
from committed main; the project position stays in the project, and only its
index entries enter the home.

**Snippet**
: A Markdown document under a position's taxonomy directories, with
frontmatter, an H1 ending in its type, and the H2 sections its effective schema
requires (`local/projects/instrukt/spec/feature/docs/snippet-schema`).

**Identity**
: A snippet's id, derived from its path and never authored:
`general/<taxonomy>/<rest>` under `docs/general/`, `<x>/<taxonomy>/<rest>`
under `docs/domain/<x>/`, and `project/<slug>/<taxonomy>/<rest>` under
`docs/project/`, where `<slug>` is the `project` the repository declares in its
`instrukt.yaml`. An id is a name, not a copy of its path: a domain id carries no
`domain/` prefix.

**Taxonomy**
: The snippet's type, from its taxonomy directory: principle, concept, policy,
procedure, design, spec, adr. Order of listing follows this sequence.

**Domain**
: A body of knowledge one publisher is the authority on, published as one
unit: one owner per computer, its own schema, its own index, optionally its own
workflow (`docs/domain/<x>/workflow/`). It is named after what it is about,
never after a category it belongs to. Domains carry no tags; they are found
through their description and their procedures' activation triggers.
`general` is a position with the same model; it is addressed like a domain
wherever a domain name is accepted.

**Reserved name**
: `general` and `project`. No domain may carry either; a folder
`docs/domain/general/` or `docs/domain/project/` is refused, naming the rule.
`third-party` keeps its existing special case: it is not a domain, its
published tree `~/.instrukt/docs/third-party/` is unchanged, and no domain may
be called `third-party` either.

**Position index**
: A position's `index.yaml`: which of its snippets are rendered in full
(`expanded`), which as one listing line (`listed`), a domain's one-line
`description`, and in `general`'s alone, which snippets go to one agent CLI
only (`agents`) (`local/projects/instrukt/spec/feature/docs/position-index`).
Not to be confused with the generated global index.

**Listing line**
: The one line that stands for a snippet wherever a rendering lists rather
than expands it: its id, its description, and the command that reads it
(`local/projects/instrukt/spec/feature/docs/progressive-retrieval`).

**Root rules**
: The snippet shape built into the binary: the seven types, their required and
optional sections, and the generic rules (H1, frontmatter, Required reads, See
also). Fixed; not configurable.

**Position schema**
: A position's `schema.yaml`, keyed by folder path relative to the position
root, adding required and optional H2 sections for the snippets under each
path.

**Effective schema**
: What a snippet at a folder is held to: the root rules for its type, then
every key of its own position's schema whose path contains that folder,
shortest first.

**Required read**
: An entry in a snippet's `## Required reads` section: a snippet id, or a bare
domain name standing for that domain's `expanded` set, that must have been read
before this snippet.

**Reference**
: A snippet named by its id in Required reads, See Also or a prose citation;
the CLI resolves it. Snippets are never referenced by path. Code is referenced
by its repository-relative path.

**Fragment**
: A shared text body inlined where an `@include` directive names it. Not a
snippet; never retrieved by id.

**Configured value**
: A `@config` directive in a snippet body naming a configuration path, resolved
in-process when the snippet is read, with the value placed where the directive
stood. The snippet format has no directive that runs a command.

**Third-party doc**
: Curated external documentation under `docs/third-party/`, indexed apart from
taxonomy snippets, never a required read.

**Pending snippet**
: A snippet carrying `delivered_by`: intended state not yet delivered.

**Companion**
: `<stem>.<slug>.md` beside a live snippet — the complete next version the
named todo delivers. An empty companion retires the snippet.

**Planned block**
: An in-body `planned:<slug>` (addition) or `planned-change:<slug>`
(replacement) block carrying a todo's pending change to a live snippet.

**Superseded by**
: The todo slugs with pending material for a live snippet, derived from its
companions and planned blocks; never authored.

**Lens**
: The view a read takes on pending material: the current contract by default,
or one todo's future with `--target <slug>`.

**Finding**
: A break of the snippet contract reported by validation: it fails the run and
refuses publication.

**Notice**
: A report that does not break the contract: printed with its file and code,
never failing the run.

**Global index**
: What the home's state records about every snippet published or indexed on the
computer, generated by publication. Each index entry holds identity, type,
domain, scope, description, pending markers, owning project, and for a project
snippet the absolute path of its file in the project.

**Closure**
: One snippet together with its transitive required reads, each once, read in
one pass; a bare-domain required read contributes that domain's `expanded`
set.

**Published commit**
: The commit of a project's canonical main that publication last published;
recorded by publication.

### Ownership boundary

- **Owns** — the snippet model (positions, identity derivation, the domain
  definition and reserved names, taxonomy, frontmatter fields and where each
  is allowed, sections, the reference form, fragments, third-party doc shape);
  the position index model, its keys per position, findings and notices; the
  root rules, the position schema chain and the effective schema; structural
  validation, its findings and notices, including the required-reads hygiene
  finding and the unreferenced-snippet notice; the pending-documentation model
  and the lens; listing, reading, closures, the domain read, the listing-line
  shape, the read pipeline and configured values; the publication-behind
  warning on read.
- **Does not own** — copying snippets into the home, writing the global index,
  rendering the global agent files and project `AGENTS.md` from position
  indexes, the editor JSON schemas for `index.yaml`, `schema.yaml` and
  `instrukt.yaml`, recording the published commit and the domain claim rule
  (publication); what `instrukt init` writes on adoption, including the copy of
  the published `sdlc` schema into `docs/project/schema.yaml` (repository);
  the `project` slug and every configured value (configuration); reading use
  cases from feature specs and their heading levels (specs); whether a todo
  slug names an existing todo (work, when the `todos` module is on); deciding
  when publication runs (repository).
- **Published contracts** — `instrukt docs validate [paths] [--json]`;
  `instrukt docs fold <slug>`; `instrukt docs index [--domain <x>…] [--areas]
  [--third-party] [--target <slug>] [--project <slug>]`; `instrukt docs get
  <id…> [--closure] [--target <slug>]`; `instrukt docs get --domain <x>`;
  `instrukt docs schema <folder>`; the snippet model as stated in
  `general/spec/snippet-authoring-schema`, the position index model and the
  listing-line shape, which publication relies on to derive index entries and
  render agent files.
- **Allowed dependencies** — publication (reads the home it writes and the
  published commit it records); configuration (the `project` slug, and the
  in-process resolution of `@config` paths); work (todo-slug existence through
  an interface core declares and the CLI wires only when `todos` is enabled);
  workflow (declared flavor roles through a core-declared lookup interface
  wired by the CLI when `todos` is enabled).
- **Implementation roots** — the docs module of the `instrukt-core` crate;
  its module layout is set by the architecture design.

### Aggregates

#### Snippet (aggregate)

- **Root:** Snippet.
- **Contains:** frontmatter, sections, required reads, references, directives
  (`@include`, `@config`), companions, planned blocks.
- **Invariants:**
  - Identity, type, domain and scope follow from the path and the project's
    declared slug; none is authored.
  - The H1 is the first content line and ends with `— <Type>` of its taxonomy.
  - `description` is present; a snippet without one is a validation finding,
    never silently dropped.
  - Every reference to a snippet is its id; a required read is an id or a
    bare domain name, listed under `## Required reads`, which is the first H2
    when present.
  - A companion shares its parent's identity and never appears in a listing.
  - `delivered_by` is the only pending frontmatter field; pending material for
    a live snippet lives only in companions and planned blocks.
- **Identifier:** the derived identity.
- **Lifecycle:** authored in a repository; current once delivered (its
  `delivered_by`, companion or planned block is folded away by the delivering
  work); retired by an empty companion that the delivery deletes together with
  the parent.

#### Position index (aggregate)

- **Root:** the `index.yaml` at a position's root.
- **Contains:** `expanded` and `listed` id lists; a domain's `description`;
  `general`'s `agents` map.
- **Invariants:**
  - Only the keys allowed at its position appear.
  - Every id it names is a delivered snippet; a domain's and the project's
    name only their own position's snippets, `general`'s may name any domain's.
  - No `listed` id is already in the closure of `expanded`.
  - One per position; never nested.
- **Identifier:** its position.
- **Lifecycle:** authored with the position; absent for a domain that chooses
  no rendering, which is a notice.

#### Position schema (aggregate)

- **Root:** the `schema.yaml` at a position's root.
- **Contains:** folder-path keys, each with added required and optional H2
  sections.
- **Invariants:**
  - A path never drops a section the root rules or a shorter path require.
  - It governs only its own position; inheritance never crosses positions or
    repositories.
- **Identifier:** its position.
- **Lifecycle:** authored with the position; once present in a project, the
  project's own and never rewritten by instrukt.

#### Third-party doc (aggregate)

- **Root:** Third-party doc.
- **Invariants:** authors only `description`; its `## Sources` are web URLs or
  identifiers of a documentation service; never a required read.
- **Identifier:** derived from its path below the third-party root.
- **Lifecycle:** authored and curated in a repository; published with it.

### Domain events emitted

None. instrukt runs no daemon and emits no events from this context.

### Domain events consumed

None.

### Use cases (index of feature specs)

- **progressive-retrieval** — `docs/local/spec/feature/docs/progressive-retrieval.md` — an agent finds what to read, reads it with its required reads or a domain on demand, and sees values computed in its own context.
- **position-index** — `docs/local/spec/feature/docs/position-index.md` — a publisher chooses per position what agents read in full and what is listed, and learns what the index cannot say.
- **snippet-schema** — `docs/local/spec/feature/docs/snippet-schema.md` — a position adds sections to a folder's snippets, and an author learns the shape a folder requires and who imposes it.
- **snippet-validation** — `docs/local/spec/feature/docs/snippet-validation.md` — a developer learns which snippets break the snippet contract before committing.
- **pending-documentation** — `docs/local/spec/feature/docs/pending-documentation.md` — intended changes are written ahead of the build without misleading readers of the current contract.

### Governing designs and decisions (index)

- **`general/spec/snippet-authoring-schema`** — the snippet schema this context enforces.
- **`general/procedure/doc-snippet-authoring`** — how authors use the pending carriers this context reads.
- **`general/policy/context-retrieval`** — how agents are directed to retrieve documentation.

### Context map position

- **Publication** — customer / supplier. Publication supplies the home's
  published files, global index and published commit. Docs supplies
  publication with snippet identity, validation, the position index model,
  closures and the listing-line shape it renders agent files from. Project
  bodies come from indexed project paths; validation reads the invoking
  working tree.
- **Configuration** — customer / supplier. Configuration supplies the
  project's slug and resolves `@config` paths in-process through its `get`.
- **Repository** — no direct relationship; repository triggers publication,
  and its `init` copies a published schema into a project.
- **Work** — customer / supplier: work supplies todo-slug existence when
  `todos` is enabled, through the core interface wired by the CLI.
- **Workflow** — customer / supplier: workflow supplies the flavor’s declared
  roles for validation through the core-declared lookup interface when todos is on.
- **Specs** — customer / supplier: docs supplies feature-spec use-case inventory
  and pending markers.
- **Runner** — customer / supplier: docs supplies procedures and their event
  subscriptions.

## Known caveats

- A snippet never decides what runs on a machine: `@config` resolves a
  configuration path in-process, and a leftover `@exec` is inert text and a
  validation finding.
- With no publication yet, `index` says nothing is published and directs the caller to `instrukt sync` in a project; `get` fails as for an unknown id.
- The pre-release layout (`docs/global/`, `docs/local/`, baseline manifests, bundles) has no compatibility path: a snippet left there yields no identity, which is a validation finding.
- A prose citation is not checked: validation does not judge prose, so a cited id that names nothing passes.
