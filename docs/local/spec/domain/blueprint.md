---
description: "The blueprint context — a structural node-link graph of a project per push- or pull-obtained revision of main, generated detached by a configured engine, and the questions answered from the latest graph: blast radius, path, and engine-offered query and explain."
delivered_by: [instrukt-first-build]
---
# Blueprint — Spec

## What it is

The blueprint context gives agents a structural map of a project's code. For
every remote-main revision obtained by push or pull it has a graph engine produce a node-link graph
of that revision, keeps the graph under the revision's SHA in the home, and
answers questions from the latest complete graph: which code depends on a set
of changed files, how one part reaches another, and, where the engine can, free
questions and explanations. Agents only ever call `instrukt blueprint …`; the
engine stays underneath. The context is a switchable module. It does not own
the git hook that starts generation, the module switch or engine setting, the
use-case bindings that trace joins with the graph, or the engine's extraction.

## Canonical fields

“Main” means the repository default branch. The machine generates graphs on pushes and pulls advancing that branch, never per local commit. The user installs the engine; a missing engine reports what to install and instrukt installs nothing. `instrukt blueprint status` shows the last generation outcome.

### Ubiquitous language

**Graph**
:  A node-link structure of one revision of a project: nodes and links that
   satisfy the graph contract.

**Graph contract**
:  The minimum every graph carries: nodes with `id` and `source_file`; links
   with `source`, `target` and `relation`. Every answer the context computes
   itself uses only these fields.

**Node**
:  A unit of code (a file, a module, a symbol) with an id and the
   repository-relative file it is defined in.

**Link**
:  A directed relation from one node to another, named by its relation.

**Dependency link**
:  A link whose relation is one of the dependency relations: `calls`,
   `indirect_call`, `references`, `imports`, `imports_from`, `re_exports`,
   `inherits`, `extends`, `implements`, `uses`, `mixes_in`, `embeds`. Its
   source depends on its target. Containment is not a dependency.

**Engine**
:  The tool that extracts a graph from a revision's tree: the shipped adapter,
   or a configured command.

**Adapter**
:  The boundary that runs an engine and translates its output into the graph
   contract, and declares which engine-offered questions it supports.

**Generation**
:  One run that produces the graph of one revision.

**Latest graph**
:  The complete graph of the most recently obtained remote-main revision that has one.

**Blast radius**
:  The dependents of a set of changed files: every node reachable backwards
   over dependency links from the nodes those files define, outside those
   files.

### Ownership boundary

- **Owns** — the graph contract and the dependency relations; adapter
  selection and contract validation of engine output; generation per remote-main revision obtained by push or pull, its serialization and supersession; storage per SHA and the choice
  of the latest graph; blast radius, path, and forwarding query and explain to
  an engine that offers them; use-case trace joins against specs inventory and bindings.
- **Does not own** — hook sections and detecting pushes or pulls advancing main
  (repository, `local/projects/instrukt/spec/domain/repository`); the module
  switch and the engine setting (configuration,
  `local/projects/instrukt/spec/domain/configuration`); use-case inventory and bindings (specs, `local/projects/instrukt/spec/domain/specs`); the
  engine's extraction itself; the blast-radius procedure agents follow (the
  flavor's documentation).
- **Published contracts** — `instrukt blueprint affected <files…>`,
  `instrukt blueprint path <A> <B>`, `instrukt blueprint query`,
  `instrukt blueprint explain`; `instrukt blueprint status`; `instrukt specs trace <spec_id> <UC-id>` and `instrukt specs trace --file <path>`; the generation entry repository hooks start for pushes and pulls advancing main; forward and backward traversal over the latest graph, used by
  blueprint trace.
- **Allowed dependencies** — configuration (switch, engine, project slug); specs (published inventory and bindings for trace);
  repository (hook invocation for push or pull advances of main and the obtained SHA); the configured engine through
  its adapter (anti-corruption layer).
- **Implementation roots** — `crates/instrukt-blueprint/`;
  `crates/instrukt-cli/` (command surface).

### Aggregates

#### Project graph store (aggregate)

- **Root:** the project's store in the home,
  `~/.instrukt/state/blueprint/<project>/`.
- **Contains:** complete graphs keyed by SHA; at most one running generation;
  at most one queued revision.
- **Invariants:**
  - At most one generation runs per project at a time.
  - A queued revision is replaced by a newer remote-main revision; the running
    generation is not interrupted for it.
  - Only a complete graph that satisfies the contract becomes visible; an
    interrupted or failed generation never replaces a complete graph.
  - The latest graph changes in one step: a reader sees the old one or the new
    one, never a mix.
- **Identifier:** the project slug from the project layer of `instrukt.yaml`.
- **Lifecycle:** created by the first generation; re-creatable at any time
  from the repository by the next push or pull.

#### Graph (aggregate)

- **Root:** the graph of one revision.
- **Contains:** nodes, links.
- **Invariants:**
  - `nodes` and `links` are lists.
  - Every `source_file` is a repository-relative path: not absolute, no `..`
    segment, no backslash.
- **Identifier:** the revision's SHA within its project's store.
- **Lifecycle:** written once by a generation, never modified.

### Domain events emitted

None. Generation runs detached; answers are command output.

### Domain events consumed

None. Generation is started by the repository context's pre-push or main-advancing post-merge/post-rewrite section
through this context's generation entry, not by an event.

### Use cases (index of feature specs)

- **graph-engine** — `docs/local/spec/feature/blueprint/graph-engine.md` — the graph contract, the shipped adapter and a project's own engine command.
- **graph-generation** — `docs/local/spec/feature/blueprint/graph-generation.md` — a graph per push- or pull-obtained revision of main, detached from the push, stored per SHA, one at a time, never replaced by an interrupted run.
- **blast-radius** — `docs/local/spec/feature/blueprint/blast-radius.md` — the dependents of changed files from the latest graph, naming its SHA.
- **graph-navigation** — `docs/local/spec/feature/blueprint/graph-navigation.md` — path between two parts, and query and explain where the engine offers them.

- **use-case-trace** — `docs/local/spec/feature/blueprint/use-case-trace.md` — joins the specs inventory and bindings to graph traversal; UC-ST1 and UC-ST2.

### Governing designs and decisions (index)

- **`local/projects/instrukt/design/architecture/overview`** — the blueprint module, the home's per-SHA store and the push-and-map flow.

### Context map position

- **configuration** — Customer / Supplier. Configuration supplies the module switch and the engine setting.
- **repository** — Customer / Supplier. Blueprint supplies generation to repository hooks for push and pull advances of main; the CLI wires the core-declared handoff interface.
- **specs** — Customer / Supplier. Specs supplies inventory and bindings; blueprint is their customer and owns the trace join.
- **Engine** — Anti-corruption layer at the adapter. The engine's output is translated to the graph contract and validated before it is stored.

Generation accepts both repository hook handoffs: a push to main and a pull
advancing main through post-merge/post-rewrite. Only operations that do not
advance main are excluded.

## Known caveats

- Answers reflect the latest obtained remote-main revision of main, not the caller's
  working tree or branch. Agents use them as they are and never check
  freshness.
- The store keeps the latest complete graph and the one before it; older graphs are removed.
