---
description: 'Governance for agent-artifact sources, renderings, and ownership: what the global agent files and a project AGENTS.md carry, master files as locked thin wrappers, generated entrypoints and runtime homes never edited, knowledge in snippets.'
visibility: 'public'
---

# Agent Artifact Governance — Policy

## Rules

- Authored sources and generated output never share a location. A directory that is written by the platform is not also read as input.

- Every `AGENTS.master.md` is inflated into an adjacent `AGENTS.md`, and that `AGENTS.md` is the only file generated beside a master. Publication inflates a project's masters.

- The project's root `AGENTS.md` is the root master's `## Purpose`, then the rendering of `docs/project/index.yaml`: its `expanded` snippets in full with their closure, then its `listed` snippets as one line each, then the project's procedures carrying an `activation_trigger`. What the project includes is decided only by `docs/project/index.yaml`.

- Each supported agent CLI's user-scope instruction file is computed from what is published on the computer: `general`'s section first — its `expanded` snippets in full, then its `listed` lines, then its triggered procedures — then one group per other published domain, carrying its `index.yaml` `description` with the command `instrukt docs get --domain <x>` and every procedure of that domain carrying an `activation_trigger`, then that CLI's `agents` snippets from `docs/general/index.yaml`. Expanded snippets sit under a heading naming their domain, with their own headings one level down. Whichever sync publishes `docs/general/` writes these files. No master file, addendum or other authored source exists for them: a correction for one CLI is a `policy` snippet that `docs/general/index.yaml` names under `agents`.

- `instrukt docs get --domain <x>` renders a domain's `index.yaml` in the same shape — `expanded` in full with closure, `listed`, then its triggered procedures — `general` included. For an on-demand domain this rendering is the full content its line in the global file reaches.

- A user-scope instruction file instrukt did not write is never overwritten. Sync stops and names the file; the door is to move its content into docs, or out of the way, and sync again. Before anything is published, `instrukt init` in a repository carrying `docs/general/` or `docs/domain/*` reports which domains it will claim and whether it will write the global agent files.

- Every generated entrypoint — each `AGENTS.md` beside a master and each agent runtime's user-scope entrypoint — is fully inlined: every required read is expanded into it and no import survives in the runtime's import syntax outside code spans and fenced blocks. An unsupported import fails the build; it is never left for the runtime to resolve, because an agent runtime reading `AGENTS.md` directly resolves an import outside its working directory only under an approval it may never have been given.

- Master files are thin wrappers. The repository-root `AGENTS.master.md` carries only `## Purpose`. A nested master carries `## Purpose` and `## Required reads`, and produces a directory-local entrypoint from them; it never inherits the project-wide rendering. They are never a knowledge home — all knowledge lives in doc snippets under `docs/`, and developer-experience prose lives in the README. A contract, procedure, architecture note, or operational note authored into a master file is a defect to be relocated to its rightful snippet.

- Every master is its project's own source, recorded by its repository's history. `instrukt init` seeds a project's root master only while it is absent, and nothing rewrites an existing one.

- The sync that writes the global agent files merges into each agent's user settings exactly the keys that make that agent load its entrypoints, and touches no other key.

- Master files are locked against unprompted edits: an agent never edits a master file on its own initiative or as a side effect of other work — only when the human raises the change in the session, and then the agent makes the edit. The lock guards the wrapper against accreting knowledge; it does not route typing to the human. It is declared by a `LOCKED_FILE` sentinel at the top and bottom of each master file.
- No runtime-specific instruction file is generated beside a master. A runtime with an instruction file of its own reads the generated `AGENTS.md` directly, alongside any such file in its directory or above, under the setting publication merges into that runtime's user settings. A runtime-specific instruction file in a repository is that repository's own authored file and publication never writes it; the one exception is removal: an untracked `CLAUDE.md` — tracked neither at the published commit nor in the repository's current `HEAD`, and absent from its index — beside a master, whose entire content is the pointer import of `AGENTS.md`, is a leftover generated file, and publication deletes it.

- Generated entrypoints are build outputs — never edit them directly. They are the `AGENTS.md` beside each master and each agent runtime's user-scope entrypoint in its deployed runtime directory, under the one name that runtime reads user-scope instructions from. Author the source artifacts; new instruction text belongs in a snippet and the `index.yaml` that selects it, never in a generated file.
- Deployed runtime directories (`~/.{agent}/`) are also build outputs — never create or edit files there directly. Always author in source. This applies to every agent runtime, current and future: its runtime directory is always a deployment target, never a source.
- The generated documentation index and the rendered sections of generated `AGENTS.md` files are written by publication; do not hand-edit them. Author the `index.yaml` they are rendered from.

## Rationale

- Inflating master files keeps agent runtimes in sync while keeping source files stable.
- One generated entrypoint per directory serves every agent runtime. A per-runtime companion is a second generated file that can drift from the first, and for a runtime with an instruction file of its own, such a file in a directory or above it would otherwise stop `AGENTS.md` from loading at all; the both-files setting, honoured only in user settings, removes that shadowing without asking any repository to give up its own file. Only a leftover generated pointer is removed, because any other such file may carry its owner's words.
- An agent's user settings belong to its user; changing only the keys that load the entrypoints leaves every other setting as the user made it.
- Thin, locked wrappers keep knowledge in its rightful home (snippets) and the master file from accreting a parallel, drifting manual. One authored selection per position — its `index.yaml` — means each rendering is decided in one place, reviewed like any other source.
- Computing the global files from what is published, with no master or addendum of their own, leaves nothing to drift from the snippets: a correction for one agent CLI is an ordinary snippet with ordinary review and pending markers.
- A user-scope file instrukt did not write may carry its owner's words; stopping, rather than overwriting, keeps them, and reporting claims at adoption lets the owner see what a repository will take over before it does.
- A master's purpose is its project's own words, so a seeded master is that project's source. Rewriting it from a template would erase the project's choices, and a master no history records would reach publication as a file nobody reviewed.

- Reading authored input from a directory the platform also writes makes a file's provenance unreadable from its location: an author cannot tell their own work from generated output, and a regenerating deploy is free to overwrite something a human wrote. Keeping the source root out of every delivery location is what makes that class of loss unrepresentable rather than merely avoided by convention.

## Scope

- Applies to every project that publishes with instrukt.

## Enforcement

- Generated entrypoints and the keys publication owns in deployed runtime directories are overwritten on each publication.
- A root master containing anything beyond `## Purpose`, or a nested master containing anything beyond `## Purpose` and `## Required reads`, is a defect; relocate the content to a snippet or the README.
- A nested generated `AGENTS.md` containing the project-wide rendering is a defect; only the repository-root generated entrypoint carries it.
- Sync refuses to overwrite a user-scope instruction file it did not write, naming the file.

## Exceptions

- None.
