---
description: 'Governance for agent-artifact sources, renderings, and ownership: what the global agent files and a project AGENTS.md carry, master files as locked thin wrappers, generated entrypoints and runtime homes never edited, knowledge in snippets.'
visibility: 'public'
---

# Agent Artifact Governance — Policy

## Rules

- Authored sources and generated output never share a location. A directory that is written by the platform is not also read as input.

- Every `AGENTS.master.md` is inflated into an adjacent `AGENTS.md`, and that `AGENTS.md` is the only file generated beside a master. It is committed with the repository, rendered at each commit from the repository's committed content alone, and opens with a line naming it as generated and where its sources are. The file beside a master is instrukt's own: each rendering writes it whole, whatever it held. An `AGENTS.md` with no master beside it is removed only while it opens with that line.

- The project's root `AGENTS.md` is the root master's `## Purpose`, then the rendering of `docs/project/index.yaml`: its `expanded` snippets in full with their closure, then its `listed` snippets as one line each, then the project's procedures carrying an `activation_trigger`. What the project includes is decided only by `docs/project/index.yaml`.

- Each configured agent runtime's user-scope instruction file — one for each agent runtime the selected runtime reports as configured, in the home it runs with — is computed from what is published on the computer: `general`'s section first — its `expanded` snippets in full, then its `listed` lines, then its triggered procedures and configured responsibilities — then one group per other published domain, carrying only its `index.yaml` `description` with the command `instrukt docs get --domain <x>`, then that CLI's `agents` snippets from `docs/general/index.yaml`. Expanded snippets sit under a heading naming their domain, with their own headings one level down. Whichever sync publishes `docs/general/` writes these files. No master file, addendum or other authored source exists for them: a correction for one agent runtime is a `policy` snippet that `docs/general/index.yaml` names under `agents`.

- `instrukt docs get --domain <x>` renders a domain's `index.yaml` in the same shape — `expanded` in full with closure, `listed`, then its triggered procedures and configured responsibilities — `general` included. For an on-demand domain this rendering is the full content its line in the global file reaches.

- Each user-scope instruction file is instrukt's own. The sync that publishes `docs/general/` writes it whole every time, whatever it holds, keeps no record of what it wrote, and never removes one. Rules a person wants an agent runtime to read go into docs — a snippet `docs/general/index.yaml` selects, or an `agents` snippet for that runtime — never into the file, which the next sync overwrites. Before anything is published, `instrukt init` in a repository carrying `docs/general/` or `docs/domain/*` reports which domains it will claim and which global agent files the sync will write.

- No generated entrypoint is meant to leave an import in a runtime's import syntax outside code spans and fenced blocks; rendering inlines or names every reference instead, because an agent runtime reading `AGENTS.md` directly resolves an import outside its working directory only under an approval it may never have been given. A reference a rendering would leave is a finding, never left silently: a committed `AGENTS.md` with one keeps its previous content in the commit, which proceeds; a global file with one, reachable only through a snippet another project published that the sync does not validate again, is written as rendered and named, and the sync goes on. A user-scope entrypoint inlines every required read from what is published on the computer. A committed `AGENTS.md` inlines what its repository holds and names doctrine another repository publishes by its id and the `instrukt docs get` command that reads it, and carries no configured value.
- A company's knowledge stands alone: its master references only the company's own documents, never `general`, a domain or any other body. The company's agents receive `general` through their baseline and instrukt's own procedure through their role.

- Master files are thin wrappers. The repository-root `AGENTS.master.md` carries only `## Purpose`. A nested master carries `## Purpose` and `## Required reads`, and produces a directory-local entrypoint from them; it never inherits the project-wide rendering. They are never a knowledge home — all knowledge lives in doc snippets under `docs/`, and developer-experience prose lives in the README. A contract, procedure, architecture note, or operational note authored into a master file is a defect to be relocated to its rightful snippet.

- Every master is its project's own source, recorded by its repository's history. `instrukt init` seeds a project's root master only while it is absent, and nothing rewrites an existing one.

- The sync that writes the global agent files merges into each agent runtime's user settings exactly the keys that make that agent load its entrypoints, and touches no other key.

- Master files are locked against unprompted edits: an agent never edits a master file on its own initiative or as a side effect of other work — only when the human raises the change in the session, and then the agent makes the edit. The lock guards the wrapper against accreting knowledge; it does not route typing to the human. It is declared by a `LOCKED_FILE` sentinel at the top and bottom of each master file.
- No runtime-specific instruction file is generated beside a master. A runtime with an instruction file of its own reads the generated `AGENTS.md` directly, alongside any such file in its directory or above, under the setting publication merges into that runtime's user settings. A runtime-specific instruction file in a repository is that repository's own authored file and publication never writes it; an `AGENTS.override.md` beside a root master is reported as a finding of the committed rendering, because Codex reads it in place of `AGENTS.md`, and is never deleted; the one exception is removal: an untracked `CLAUDE.md` — tracked neither at the published commit nor in the repository's current `HEAD`, and absent from its index — beside a master, whose entire content is the pointer import of `AGENTS.md`, is a leftover generated file, and publication deletes it.

- Generated entrypoints are build outputs — never edit them directly. They are the `AGENTS.md` beside each master and each agent runtime's user-scope entrypoint in its deployed runtime directory, under the one name that runtime reads user-scope instructions from. Author the source artifacts; new instruction text belongs in a snippet and the `index.yaml` that selects it, never in a generated file.
- Deployed runtime directories (`~/.{agent}/`) are also build outputs — never create or edit files there directly. Always author in source. This applies to every agent runtime, current and future: its runtime directory is always a deployment target, never a source.
- The generated documentation index is written by publication and the generated `AGENTS.md` files by instrukt at each commit; do not hand-edit them. Author the masters and the `index.yaml` they are rendered from.

## Rationale

- Inflating master files keeps agent runtimes in sync while keeping source files stable.
- A committed `AGENTS.md` reaches every agent that checks the repository out, including one on a computer or in a cloud where instrukt is not installed; an untracked one reaches only computers that render it. Rendering it from the repository's committed content alone makes it the same on every computer, so a commit never carries one computer's published doctrine or configuration, and no other computer's commit undoes it.
- A company's knowledge is consumed by the company's own agents, wherever its repository is checked out; a reference into doctrine published elsewhere would bind it to a computer that publishes that doctrine, and the agents already carry `general` in their baseline.
- One generated entrypoint per directory serves every agent runtime. A per-runtime companion is a second generated file that can drift from the first, and for a runtime with an instruction file of its own, such a file in a directory or above it would otherwise stop `AGENTS.md` from loading at all; the both-files setting, honoured only in user settings, removes that shadowing without asking any repository to give up its own file. Only a leftover generated pointer is removed, because any other such file may carry its owner's words.
- An agent's user settings belong to its user; changing only the keys that load the entrypoints leaves every other setting as the user made it.
- Thin, locked wrappers keep knowledge in its rightful home (snippets) and the master file from accreting a parallel, drifting manual. One authored selection per position — its `index.yaml` — means each rendering is decided in one place, reviewed like any other source.
- Computing the global files from what is published, with no master or addendum of their own, leaves nothing to drift from the snippets: a correction for one agent CLI is an ordinary snippet with ordinary review and pending markers.
- The global agent files and the `AGENTS.md` beside each master are instrukt's. What a person writes into one by hand is replaced at the next sync or commit. A person's own instructions belong in documentation, from which the file is rebuilt every time. Reporting claims at adoption lets the owner see what a repository will take over before it does.
- A master's purpose is its project's own words, so a seeded master is that project's source. Rewriting it from a template would erase the project's choices, and a master no history records would reach publication as a file nobody reviewed.

- Reading authored input from a directory the platform also writes makes a file's provenance unreadable from its location: an author cannot tell their own work from generated output, and a regenerating deploy is free to overwrite something a human wrote. Keeping the source root out of every delivery location is what makes that class of loss unrepresentable rather than merely avoided by convention.

## Scope

- Applies to every project that publishes with instrukt.

## Enforcement

- Generated `AGENTS.md` files are re-rendered at each commit, and a repository's CI catches one a commit left stale; user-scope entrypoints and the keys publication owns in deployed runtime directories are overwritten on each publication.
- A root master containing anything beyond `## Purpose`, or a nested master containing anything beyond `## Purpose` and `## Required reads`, is a defect; relocate the content to a snippet or the README.
- A nested generated `AGENTS.md` containing the project-wide rendering is a defect; only the repository-root generated entrypoint carries it.
- Rules written by hand into a generated entrypoint — a committed `AGENTS.md` or a user-scope file — are a defect: the next rendering overwrites them; relocate them to a snippet.

## Exceptions

- None.
