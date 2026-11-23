---
description: "Governance for agent-artifact sources, renderings, and ownership: what the global agent files and a repository's one root AGENTS.md carry, an index.yaml as the only authored source of a rendering, generated entrypoints and runtime homes never edited, knowledge in snippets."
visibility: 'public'
---

# Agent Artifact Governance — Policy

## Rules

- Authored sources and generated output never share a location. A directory that is written by the platform is not also read as input.

- A repository has at most one generated instruction file: the `AGENTS.md` at its root, rendered from the repository's entry `index.yaml` — `docs/project/index.yaml` in a project, `docs/index.yaml` in a company home. It is committed with the repository, rendered at each commit from the repository's committed content alone, and opens with a line naming it as generated and where a change goes. That file is instrukt's own: each rendering writes it whole, whatever it held. No `AGENTS.md` below the root is generated. One there, or one at the root of a repository without its entry index, is removed only while its first line is exactly a line instrukt writes, or wrote when it rendered from a master.

- The project's root `AGENTS.md` is the rendering of `docs/project/index.yaml`: its `expanded` snippets in full with their closure, then its `listed` snippets as one line each, then the project's procedures carrying an `activation_trigger`. What the project includes is decided only by `docs/project/index.yaml`. The project's purpose is a snippet like any other, by convention the first `expanded` entry.

- Each configured agent runtime's user-scope instruction file — one for each agent runtime the selected runtime reports as configured, in the home it runs with — is computed from what is published on the computer: `general`'s section first — its `expanded` snippets in full, then its `listed` lines, then its triggered procedures — then one group per other published domain, carrying its `index.yaml` `description` with the command `instrukt docs get --domain <x>` and every procedure of that domain carrying an `activation_trigger`, then that CLI's `agents` snippets from `docs/general/index.yaml`. Expanded snippets sit under a heading naming their domain, with their own headings one level down. It opens with the same line naming it as generated. Whichever sync publishes `docs/general/` writes these files. No addendum or other authored source exists for them: a correction for one agent runtime is a `policy` snippet that `docs/general/index.yaml` names under `agents`.

- `instrukt docs get --domain <x>` renders a domain's `index.yaml` in the same shape — `expanded` in full with closure, `listed`, then its triggered procedures — `general` included. For an on-demand domain this rendering is the full content its line in the global file reaches.

- Each user-scope instruction file is instrukt's own. The sync that publishes `docs/general/` writes it whole every time, whatever it holds, keeps no record of what it wrote, and never removes one. Rules a person wants an agent runtime to read go into docs — a snippet `docs/general/index.yaml` selects, or an `agents` snippet for that runtime — never into the file, which the next sync overwrites. Before anything is published, `instrukt init` in a repository carrying `docs/general/` or `docs/domain/*` reports which domains it will claim and which global agent files the sync will write.

- No generated entrypoint is meant to leave an import in a runtime's import syntax outside code spans and fenced blocks; rendering inlines or names every reference instead, because an agent runtime reading `AGENTS.md` directly resolves an import outside its working directory only under an approval it may never have been given. A reference a rendering would leave is a finding, never left silently: a committed `AGENTS.md` with one keeps its previous content in the commit, which proceeds; a global file with one, reachable only through a snippet another project published that the sync does not validate again, is written as rendered and named, and the sync goes on. A user-scope entrypoint inlines every required read from what is published on the computer. A committed `AGENTS.md` inlines what its repository holds and names doctrine another repository publishes by its id and the `instrukt docs get` command that reads it, and carries no configured value.
- A company's knowledge stands alone: its index and its documents reference only the company's own documents, never `general`, a domain or any other body. The index selects which documents the rendering holds in full and which it names by one line. The passage that frames a company rendering is fixed by instrukt and is no part of the index. The company's agents receive `general` through their baseline and instrukt's own procedure through their role.

- No rendering of instrukt's has a master. An `index.yaml` is the only authored source of a rendering, and it carries selection only: all knowledge lives in documentation under `docs/`, and developer-experience prose lives in the README. An `AGENTS.master.md` kept at a repository's root or in a subfolder as the source of the `AGENTS.md` beside it is a former master: instrukt renders nothing from it and never removes it, and what it holds reaches agents through instrukt's rendering only as documentation the entry index selects. The file's name decides nothing by itself: a document under `docs/` that the entry index selects is documentation, whatever it is called, and is rendered as such.

- The sync that writes the global agent files merges into each agent runtime's user settings exactly the keys that make that agent load its entrypoints, and touches no other key.

- No runtime-specific instruction file is generated in a repository. A runtime with an instruction file of its own reads the generated `AGENTS.md` directly, alongside any such file in its directory or above, under the setting publication merges into that runtime's user settings. A runtime-specific instruction file in a repository is that repository's own authored file and publication never writes it; an `AGENTS.override.md` at the root of a repository whose root `AGENTS.md` is rendered is reported as a finding of the committed rendering, because Codex reads it in place of `AGENTS.md`, and is never deleted.

- Generated entrypoints are build outputs — never edit them directly. They are a repository's root `AGENTS.md` and each agent runtime's user-scope entrypoint in its deployed runtime directory, under the one name that runtime reads user-scope instructions from. Author the source artifacts; new instruction text belongs in a snippet and the `index.yaml` that selects it, never in a generated file.
- Deployed runtime directories (`~/.{agent}/`) are also build outputs — never create or edit files there directly. Always author in source. This applies to every agent runtime, current and future: its runtime directory is always a deployment target, never a source.
- The generated documentation index is written by publication and the generated `AGENTS.md` by instrukt at each commit; do not hand-edit them. Author the documentation and the `index.yaml` they are rendered from.

## Rationale

- One rendered file per repository, from one index, reaches every agent runtime the same way. Agent runtimes load an instruction file in a subfolder differently, and some not at all, so a rendering there reaches some agents and not others; the root file, with on-demand reads for what it lists, does not depend on how a runtime treats subfolders.
- A committed `AGENTS.md` reaches every agent that checks the repository out, including one on a computer or in a cloud where instrukt is not installed; an untracked one reaches only computers that render it. Rendering it from the repository's committed content alone makes it the same on every computer, so a commit never carries one computer's published doctrine or configuration, and no other computer's commit undoes it.
- A company's knowledge is consumed by the company's own agents, wherever its repository is checked out; a reference into doctrine published elsewhere would bind it to a computer that publishes that doctrine, and the agents already carry `general` in their baseline.
- A fixed company passage tells every company's agents the same thing about where the knowledge lives and how a change reaches them, whatever the company wrote. A passage a repository could replace would let one edit misdirect the agents that maintain that repository's knowledge.
- One generated entrypoint per repository serves every agent runtime. A per-runtime companion is a second generated file that can drift from the first, and for a runtime with an instruction file of its own, such a file in a directory or above it would otherwise stop `AGENTS.md` from loading at all; the both-files setting, honoured only in user settings, removes that shadowing without asking any repository to give up its own file.
- An agent's user settings belong to its user; changing only the keys that load the entrypoints leaves every other setting as the user made it.
- One authored selection per body of documentation — its `index.yaml` — means each rendering is decided in one place, reviewed like any other source. A second authored file beside it, however thin, is a second place a rendering is decided and a place where knowledge accretes outside documentation.
- Computing the global files from what is published, with no addendum of their own, leaves nothing to drift from the snippets: a correction for one agent CLI is an ordinary snippet with ordinary review and pending markers.
- The global agent files and a repository's root `AGENTS.md` are instrukt's. What a person writes into one by hand is replaced at the next sync or commit. A person's own instructions belong in documentation, from which the file is rebuilt every time. Reporting claims at adoption lets the owner see what a repository will take over before it does.
- instrukt renders nothing from a former master and never removes one: rendering from it would make a retired file a second source of a rendering, and removing it could destroy words a person wrote or a source another tool still reads.
- A file is judged by the part it plays, not by its name: the same name can be a former master at a repository's root and a selected document under `docs/`, and an instruction keyed on the name would move or delete knowledge that is already where it belongs.

- Reading authored input from a directory the platform also writes makes a file's provenance unreadable from its location: an author cannot tell their own work from generated output, and a regenerating deploy is free to overwrite something a human wrote. Keeping the source root out of every delivery location is what makes that class of loss unrepresentable rather than merely avoided by convention.

## Scope

- Applies to every project that publishes with instrukt.

## Enforcement

- A repository's generated `AGENTS.md` is re-rendered at each commit, and the repository's CI catches one a commit left stale; user-scope entrypoints and the keys publication owns in deployed runtime directories are overwritten on each publication.
- Knowledge agents need that sits only in a former master is a defect: move it into documentation the repository's entry index selects, or to the README. The former master is deleted once nothing reads it as a source. Where another tool still does, as a company repository's knowledge publisher does until it reads `docs/index.yaml`, the file stays until that tool reads the index, and goes in that same change. A document under `docs/` that the entry index selects is never moved, renamed or deleted on account of its name.
- A generated `AGENTS.md` below a repository's root is a defect; rendering removes it.
- Rules written by hand into a generated entrypoint — a committed `AGENTS.md` or a user-scope file — are a defect: the next rendering overwrites them; relocate them to a snippet.

## Exceptions

- None.
