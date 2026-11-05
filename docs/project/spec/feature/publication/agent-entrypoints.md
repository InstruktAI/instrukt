---
description: Rendered agent instructions — a project's root `AGENTS.md` is its master's purpose followed by the rendering of `docs/project/index.yaml`, nested masters render beside themselves, and the owner of `docs/general/` renders every supported agent CLI's global file from what is published on the computer, with per-CLI `agents` snippets, never over a file instrukt did not write, merging exactly the entrypoint-loading settings keys.
delivered_by: [instrukt-first-build]
---
# Agent Entrypoints — Spec

## What it is

Agents receive instrukt's instructions through files their CLIs already load.
A project's sync renders its root `AGENTS.md` from the project's purpose and
the selection in `docs/project/index.yaml`, so an agent opened in the project
starts with what the project chose for it. The project that owns
`docs/general/` also renders each supported agent CLI's user-scope global
file from what is published on the computer: the always-on `general`
content, one line per on-demand domain with the command that loads it, and
that CLI's own `agents` snippets. It sets the one settings key that makes a
CLI load `AGENTS.md` files, and never writes over an instruction file
instrukt did not write. Every rendered file is self-contained: nothing is
left for an agent CLI to resolve. There is no global master and there are no
addenda.

## Canonical fields

When events is enabled, the root `AGENTS.md` also lists standing agents under
“Configured Responsibilities”. No branding line is emitted. For a supported
runtime that reads a separate project override instruction file, that file is
retained alongside the ordinary entrypoint.

### Rendering a selection

The global agent files, the root `AGENTS.md` and `instrukt docs get --domain
<x>` render a position's `index.yaml` the same way. The index model is
`project/instrukt/spec/feature/docs/position-index`; the listing-line
forms and closures are
`project/instrukt/spec/feature/docs/progressive-retrieval`, which also
owns the domain read. This spec owns the grouping and headings. A rendered
section, in order:

1. **Heading.** `# <Name>`: the position's name with its first letter
   capitalised (`# General`, `# Sdlc`, `# Project`).
2. **Expanded.** Each id in `expanded`, in the order listed, in full with its
   closure: its transitive required reads, each before the snippet that
   requires it. The traversal order is kept: each snippet sits under the
   `# <Domain>` heading of the domain it belongs to, so an `sdlc` snippet
   that `general`'s `expanded` selects sits under `# Sdlc`, not `# General`,
   and when the traversal moves to a snippet of another domain a heading for
   that domain opens there — a domain's heading appears again when the
   traversal returns to it (`general/A`, then `sdlc/B`, then `general/C`
   renders `# General`, `# Sdlc`, `# General`). Order is never changed to
   group a domain's snippets together. Every heading of a rendered
   snippet is demoted one level, except inside fenced code; an H6 stays H6.
3. **Listed.** One listing line per id in `listed`, naming
   `instrukt docs get <id>`.
4. **Triggered procedures.** One listing line per procedure of the position
   carrying an `activation_trigger`, naming its trigger and
   `instrukt docs get <id> --closure`.

Within one rendered file, a snippet appears in full at most once. A
triggered procedure keeps its trigger and its `instrukt docs get <id>
--closure` command wherever it appears: rendered in full, its trigger line
stands directly under its heading; listed, its listing line is the triggered
form (trigger and `--closure`) rather than the plain `get <id>` form; it is
not listed a second time.
Rendering resolves `@config` directives against the publishing project's
effective configuration, never inlines a pending snippet and drops planned
material (`project/instrukt/spec/feature/publication/project-publication`).
Only references outside code are resolved; a reference inside a code span or
fenced code block is left exactly as written and is never reported as
unresolved.

### Project entrypoints

- **The root master.** `AGENTS.master.md` at the repository root is the
  project's authored entry. It holds only `## Purpose`. A root master carrying
  `## Required reads` is a finding that refuses the sync, naming the master
  and `docs/project/index.yaml` as the place that selects what is included.
- **The root `AGENTS.md`**, written in the project's primary worktree when the
  root master exists in the committed tree: the master's `## Purpose`,
  followed by the rendering of `docs/project/index.yaml` (`expanded`, then
  `listed`, then the project's triggered procedures), as above.
- **Without `docs/project/index.yaml`**, the root `AGENTS.md` holds the
  purpose and the project's triggered procedures, and the sync reports a
  notice naming the missing file.
- **Nested masters.** Every other `AGENTS.master.md` renders into the
  `AGENTS.md` beside it, from its own content with its required reads
  inlined; it receives no project selection and no procedure lines.
- An `AGENTS.md` whose master no longer exists is removed by the sync that
  finds it gone.
- An `AGENTS.md` is a generated file; an edit to it is overwritten by the next
  sync that publishes.

### Global agent files

Rendered from what is published on the computer, and written only by the sync
of the project that owns `docs/general/`, at the end of a sync that
publishes.

- **Composition**, per agent CLI, in order:
  1. `general`'s section, rendered from `docs/general/index.yaml`. Its
     `expanded` and `listed` may name ids of any published domain.
  2. One group per other published domain that has an `index.yaml`, ordered
     by domain name: under `# <Name>`, the domain's `description` with the
     command `instrukt docs get --domain <x>`, then one line per procedure of
     that domain carrying an `activation_trigger`, as in step 4 above. A
     domain's `expanded` and `listed` are not rendered here; they are what
     `instrukt docs get --domain <x>` renders.
  3. That CLI's `agents` snippets: the ids `docs/general/index.yaml` maps to
     the CLI's name, in full with their closure, appended to that CLI's file
     only.
- **A domain without `index.yaml`** is absent from every global file; the
  sync reports a notice naming it, and it is found through
  `instrukt docs index --domain <x>`.
- **Targets and destinations.** Supported deployment targets are Claude Code,
  Codex, Gemini and DeepSeek, named in `agents` as `claude`, `codex`,
  `gemini` and `deepseek`. Publication writes each target's user-scope
  instruction file at the location that target loads. For a target requiring
  an entrypoint-loading setting, publication merges the setting that enables
  project instruction files alongside its own instruction files; a target
  that loads them already needs no settings change. Destinations and loading
  keys are builder-extracted constants in the publication implementation for
  each supported target. They are not configuration keys. Each constant
  identifies the user instruction destination, settings file and loading key
  (or that no key is required). An agent CLI whose user directory does not
  exist on the computer is skipped; its `agents` entries are still validated.
- **Never over a file instrukt did not write.** Before writing anything, the
  sync checks every destination: a file that exists and whose content is not
  the content the written record holds for it — a file instrukt never wrote,
  or one changed since instrukt wrote it — stops the sync before anything is
  published. The refusal names each such file and the door: move its content
  into documentation (a snippet selected by `docs/general/index.yaml`, or an
  `agents` snippet for that CLI) or move the file out of the way, then sync
  again. A missing destination is written.
- **Settings merge.** Only the entrypoint-loading key is set; every other key
  in the settings file, and its other values, stay as they were. A missing
  settings file is created holding only that key. A settings file that cannot
  be parsed is refused and left untouched, and the sync fails naming it.
- **No other agent configuration.** Skills, MCP servers, hooks and every other
  agent setting are outside instrukt and never written.

### Use cases

#### UC-AE1: An agent opened in a developer's project starts with what the project selected

```gherkin
Given a developer's project whose master states its purpose
And the project's index selects one snippet in full, lists another, and the project has a triggered procedure
When the developer commits to main and publication runs
Then an agent opened in the project reads one instruction file starting with the project's purpose
And it holds the selected snippet in full with what that snippet requires
And it lists the other snippet and the triggered procedure, each with the command that loads it
And it holds no reference left for the agent to resolve
```

#### UC-AE2: Every supported agent on a developer's computer starts with the always-on doctrine and a map of the rest

```gherkin
Given a developer's computer with several supported agent CLIs installed
And the computer publishes always-on doctrine and an on-demand domain with a description and a triggered procedure
And the CLIs' settings hold the developer's own preferences
When publication runs for the project that owns the always-on doctrine
Then each installed agent CLI starts with the always-on doctrine in full
And always-on doctrine taken from another domain sits under that domain's heading, in the order the doctrine requires it, even where the domains interleave
And it reads one line naming the on-demand domain, its description and the command that loads it
And it reads the domain's triggered procedure with the command that loads it with its closure
And each agent CLI that needs it is set to load project instruction files
And every other setting the developer had is unchanged
```

#### UC-AE3: A developer gives one agent CLI rules of its own

```gherkin
Given a developer's computer with two supported agent CLIs installed
And the always-on doctrine maps a policy to one of those CLIs
When publication runs for the project that owns the always-on doctrine
Then that CLI's global instructions end with the policy
And the other CLI's global instructions do not contain it
When the developer maps a policy to an agent CLI instrukt does not support
Then publication is refused and names the CLI and the supported ones
```

#### UC-AE4: Publication never overwrites global agent instructions the developer wrote

```gherkin
Given a developer's computer where an agent CLI's global instruction file holds the developer's own text
When publication runs for the project that owns the always-on doctrine
Then publication stops and names that file
And it says to move the file's content into documentation or the file out of the way
And the file and the previously published documentation are unchanged
When the developer moves the file out of the way and publication runs again
Then that agent CLI starts with the rendered global instructions
```

#### UC-AE5: A developer learns that a domain without an index is not on the agents' map

```gherkin
Given a developer's project with a domain that has no index
When publication runs for the project
Then the domain is published and the developer is told it has no index
And the agents' global instructions do not name it
And the domain's snippets are still found by listing that domain
```

## Known caveats

- When the project stops publishing `docs/general/`, its next sync retracts
  the global files it wrote, like any other owned output; a file whose
  content is no longer what instrukt wrote is left in place and named.
- A global file the developer edits after instrukt wrote it stops the next
  rendering sync until the edit is moved into documentation or the file is
  moved away.

## See Also

- docs/project/spec/domain/publication.md — the publication context.
- docs/project/spec/feature/publication/project-publication.md — the sync these outputs are part of.
- docs/project/spec/domain/docs.md — the `index.yaml` model these renderings follow.
