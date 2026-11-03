---
description: Compiled agent instructions — every AGENTS.master.md compiles into a self-contained AGENTS.md beside it, and the project that owns `general` writes the global entrypoints of the supported agent CLIs and merges exactly their entrypoint-loading settings keys.
delivered_by: [instrukt-first-build]
---
# Agent Entrypoints — Spec

## What it is

Agents receive instrukt's instructions through files their CLIs already load.
A project's sync compiles each of its masters into the `AGENTS.md` beside it,
so an agent opened in the project starts with the project's instructions and
its baseline. The project that owns the `general` domain also writes each
supported agent CLI's user-scope entrypoint — the global baseline every agent
on the computer starts with — and sets the one settings key that makes a CLI
load `AGENTS.md` files. Every compiled file is self-contained: nothing is left
for an agent CLI to resolve.

## Canonical fields

When events is enabled, the root `AGENTS.md` also lists standing agents under “Configured Responsibilities”. No branding line is emitted. For a supported runtime that reads a separate project override instruction file, that file is retained alongside the ordinary entrypoint.

### Project entrypoints

- Every `AGENTS.master.md` in the committed tree of the project's canonical
  main compiles into an `AGENTS.md` in the same directory of the project's
  primary worktree.
- Compilation inlines every required read the master names, expands include
  fragments and `@exec` directives, drops planned material and never inlines a
  pending snippet (`local/projects/instrukt/spec/feature/publication/project-publication`).
- The repository-root `AGENTS.md` is followed by two generated sections: the
  project's baseline index (the ids and descriptions its progressive baseline
  lists, to load on demand) and its Activatable Procedures catalog (each
  procedure with an activation trigger, with its description and trigger). A
  nested master compiles its own content only and receives neither section.
- An `AGENTS.md` whose master no longer exists is removed by the sync that
  finds it gone.
- An `AGENTS.md` is a generated file; an edit to it is overwritten by the next
  sync that publishes.

### Global agent entrypoints

Written only by the sync of the project whose `publishes` setting holds
`general`, from that project's committed main.

- **Composition**, per agent CLI, in order: the project's global master with
  its required reads inlined (the `general` bundle among them); the agent's
  addendum; and the roster of published domains.
- **Roster.** Under the heading "Published Domains — load the relevant
  domain's bundle when needed", one line per domain in the global index that
  the global master does not already inline: the domain, its number of global
  snippets, its bundle path `~/.instrukt/docs/<domain>/bundle.md`, and the
  trigger line its publisher recorded.
- **Destinations.** An agent CLI whose user directory does not exist on the
  computer is skipped.

Supported deployment targets are Claude Code, Codex, Gemini and DeepSeek.
Publication writes each target's user-scope instruction file at the location
that target loads. For a target requiring an entrypoint-loading setting,
publication merges the setting that enables project instruction files alongside
its own instruction files; a target that loads them already needs no settings
change. Destinations and loading keys are builder-extracted constants in the
publication implementation for each supported target, extracted from its
existing entrypoint deployment behavior. They are not configuration keys.
Each constant identifies the user instruction destination, settings file and
loading key (or that no key is required).

- **Settings merge.** Only the entrypoint-loading key is set; every other key
  in the settings file, and its other values, stay as they were. A missing
  settings file is created holding only that key. A settings file that cannot
  be parsed is refused and left untouched, and the sync fails naming it.
- **No other agent configuration.** Skills, MCP servers, hooks and every other
  agent setting are outside instrukt and never written.

### Use cases

#### UC-AE1: An agent opened in a developer's project starts with the project's instructions

```gherkin
Given a developer's project with a master instruction file that requires one of its snippets
And the project's documentation lists a baseline and an activatable procedure
When the developer commits to main and publication runs
Then an agent opened in the project reads one instruction file holding the required snippet's content
And it lists the project's baseline snippets and its activatable procedure to load on demand
And it holds no reference left for the agent to resolve
```

#### UC-AE2: Every supported agent on a developer's computer starts with the global baseline

```gherkin
Given a developer's computer with several supported agent CLIs installed
And the project that owns the general domain has its global master and per-agent addenda committed
And the CLIs' settings hold the developer's own preferences
When publication runs for that project
Then each installed agent CLI starts with the global baseline, its own addendum and the list of published domains
And each agent CLI that needs it is set to load project instruction files
And every other setting the developer had is unchanged
```

## Known caveats

- The general-domain owner authors `agents/AGENTS.global.md` and `agents/<AGENT>.addendum.md`.
- When the project stops publishing `general`, its next sync retracts its owned global entrypoints, like any other owned output.

## See Also

- docs/local/spec/domain/publication.md — the publication context.
- docs/local/spec/feature/publication/project-publication.md — the sync these outputs are part of.
