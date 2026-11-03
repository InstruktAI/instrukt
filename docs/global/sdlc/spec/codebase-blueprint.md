---
description: "The codebase Blueprint — a generated structural map of the project's source that agents consult for an intended change's blast radius. The node-link contract and its adapter (one shipped), per-pushed-revision generation, the blast-radius command surface, use-case traceability through tests, languages, and blind spots."
visibility: 'public'
---

# Codebase Blueprint — Spec

## What it is

The **Blueprint** is a generated, structural map of the project's source —
symbols (functions, classes, modules) and the dependency edges between them
(calls, imports, inheritance, uses), parsed per-language. Agents consult it to
obtain the **blast radius** of an intended change — the dependents a change
can affect — instead of grepping and re-reading files. When and how to consult
it is codified in `sdlc/procedure/blast-radius-discovery`; this
spec is the canonical reference for what it is and where it lives.

Its object is the repository's own source. Its agent surface lives in the
`instrukt blueprint` namespace, and this spec is its contract. The engine sits
underneath an adapter, behind the node-link contract.

## Canonical fields

A graph is generated for every remote-main revision obtained by a machine: on push through pre-push or on a pull advancing main through post-merge/post-rewrite, never per local commit. The user installs the graph engine; a missing engine names what to install, and instrukt installs nothing.

### Ubiquitous language

**Blueprint**
: The generated structural map of one pushed revision of remote main, stored under that revision's SHA in the home, outside the repository, and shared across worktrees.

**Contract**
: The minimal node-link graph every adapter produces: nodes with `id` and `source_file`; links with `source`, `target` and `relation`.

**Adapter**
: What turns an engine's output into the contract. One adapter ships, for an engine whose output already has the contract's shape; either configuration layer (`blueprint.engine`) may instead name a command that writes the contract JSON.

**Blast-radius (affected set)**
: For an intended change to a symbol or file, the dependents it can affect — callers, importers, subclasses, tests — derived by reverse-traversal of the Blueprint's dependency edges. The structural input to a plan's change surface.

**Blind spot**
: A coupling the Blueprint cannot see because it is not a source-level edge — dynamic dispatch, registry/string-keyed wiring, config-driven behavior, reflection, and every cross-language boundary (Python↔Rust↔TypeScript via FFI/IPC/process). Named explicitly so the agent reads those areas manually rather than trusting an incomplete blast radius.

### Artifact

Each generation is stored under the SHA of the revision it was extracted from,
in `~/.instrukt/state/blueprint/<project>/`. Consumers read the latest complete
generation. A consumer pins one generation while reading it; an unsuccessful
or interrupted extraction never replaces a complete generation. These
invariants follow the explicit state and concurrency ownership in
`sdlc/policy/code-quality-practices`.

### Languages

A thin or empty answer is low confidence, never proof of no dependents. Read blind spots manually.

### Generation (automation, never agents)

Blueprint generation follows remote-main revisions obtained through push or pull, never individual
commits, and runs neither in CI nor after a CI result. When the `pre-push` hook
sees a push to `main`, it records the pushed SHA and starts generation
detached, so the push never waits. One generation runs at a time per project;
a newer SHA supersedes a queued older one. Generation extracts the committed
revision: uncommitted files and branch-only work are outside it, and a commit
or merge in a linked worktree changes no Blueprint. Generation has no public
command verb.

### Command surface

The agent-facing surface is the `instrukt blueprint` namespace; the adapter
stays underneath, and agents never call an engine directly. It requires the
`blueprint` module to be on.

- **`instrukt blueprint affected <changed-files>`** — changed files → their
  nodes → reverse traversal over dependency links → the union of dependents,
  from the latest generation. The answer names the SHA it reflects and names
  any failed generation newer than the graph it answers from.
- **`instrukt blueprint status`** — shows the last generation's outcome.
- **`instrukt blueprint path <A> <B>`** — reachability over the contract's
  links.
- **`instrukt blueprint query`** and **`instrukt blueprint explain`** — need
  what an engine knows beyond the contract, so they are offered only when the
  configured adapter's engine supports them (the shipped adapter's does); otherwise they say
  so.

### Use-case traceability

With the `blueprint` module on, `instrukt specs trace <spec_id> <UC-id>` lists
the tests binding that use case and the code they reach in the latest
generation; from a file, it lists the tests reaching it and the use cases they
bind. A link is evidence, not proof: a test also reaches helpers. Use-case ids
live in specs and test bindings only, never in production code.

### Fallback

Agents consume the latest generation as it is and never check its freshness;
every answer names the revision it reflects. When no generation exists yet, or
the latest one is unreadable, a query says so; the agent states the gap and
uses manual exploration. Unsupported languages and blind spots retain the
manual exploration contract above. Failure visibility follows
`sdlc/principle/design-fundamentals`.


## See Also

- ~/.instrukt/docs/sdlc/procedure/blast-radius-discovery.md — when and how to consult the Blueprint before editing.
