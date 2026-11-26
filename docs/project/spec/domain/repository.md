---
description: "Repository — instrukt's git integration in a project: adopting a repository with `instrukt init` and reporting its claims, instrukt's marked sections in git hooks, and attributed commits through `instrukt repo commit`."
---
# Repository — Spec

## What it is

The repository context is where instrukt meets a project's git repository. It
adopts a repository (`instrukt init`): the project layer, `docs/project/`
with its index and starting schema, the ignore
lines, the editor schemas with their modelines (generated through
publication's operation) and the hook sections, and it reports what the
repository's first sync will claim on the computer. It owns instrukt's
marked section in each git hook it uses, living beside whatever else the
project and its tools put there, and routes each hook to the context that
acts on it — publication after main changes, the blueprint on a push to main
or a pull advancing main, when blueprint is enabled.
It composes attributed commits (`instrukt repo commit`): the acting agent as
author, using the selected App's bot noreply identity for a recorded bot role;
the session it ran in; the caller's declaration of human involvement; and the
work item when given. It does not decide what a sync publishes or a
graph contains, owns no lint or test hook, and never runs the project's own
commands.

It also adopts a company's home (`instrukt init --company <key>`), carrying
out the generation decision configuration makes
(`project/instrukt/spec/feature/configuration/company-homes`, Generation).
For a company whose repository is to be created, it composes the company
skeleton's first commit in a staging checkout, has work's GitHub boundary
create the private repository, pushes that commit as the repository's default
branch and has the company's GitHub administrator made an administrator of it.
For every company it then clones the repository as the home, has
configuration write the home's settings document, installs the hook sections
there and has publication sync the home.

## Ubiquitous language

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
   point to the context that acts on it; the `pre-commit` run completes its
   staging before returning, and every other run returns without waiting.

**Hook manager**
:  A tool that generates the project's hook files itself and may drop
   sections it did not write.

**Adoption**
:  The state `instrukt init` establishes in a repository: project layer,
   `docs/project/` with its `index.yaml` and, when sdlc is published, its
   `schema.yaml`, the ignore line for `.instrukt/`, the
   editor schemas and modelines publication generates, and the hook sections.

**Claims report**
:  What `instrukt init` reports in a repository holding `docs/general/` or a
   `docs/domain/<x>/`: the positions its first sync will claim, their current
   owners, and whether that sync will render the global agent files, for
   which configured agent runtimes.

**Attributed commit**
:  A commit made through `instrukt repo commit`: authored by the acting
   agent when one resolves, its message followed by the trailers instrukt
   owns.

**Trailer**
:  A `Key: value` line instrukt appends to a commit message: `Session:`,
   `Human-Involved:`, `Todo:` (recorded association), `Planned-Todo:` (planned
   association).

**Active runtime**
: The explicitly selected adapter recognising the caller's native session
  (`project/instrukt/spec/domain/configuration`). No recognised caller means
  no session attribution, not a different runtime choice.

**Active agent**
: The agent identity the selected adapter can resolve from its caller
  metadata. An unresolved identity remains unknown.

**Company home adoption**
: What `instrukt init --company <key>` does: it gives a company its home on
  this computer as the checkout of the company's repository, creating the
  repository first when configuration's decision says so.

**Staging checkout**
: The working trees under `~/.instrukt/state/company-generation/<key>/` in
  which a company's new repository is composed before it exists on GitHub,
  holding the skeleton's root commit, and in which the home's clone is made
  ready before it moves into place.

**Creation intent**
: `intent.yaml` beside the staging checkout: the owning organisation, the
  repository, the company's name and its GitHub administrator of a creation
  under way, written once its prerequisites are checked and before anything
  on GitHub is changed. It is the record that
  a creation is this computer's and how to complete it.

**Root commit**
: The first commit of a company's new repository, holding the company
  skeleton. It creates the repository's default branch.

## Ownership boundary

- **Owns** — repository adoption and its claims report; the copy of the
  published `sdlc` `schema.yaml` into a new project; instrukt's section in
  each hook file, its placement,
  idempotence and removal; the hook status report, including each section's
  last run outcome and refused syncs; which hooks instrukt uses
  and what each hook run starts; composition of attributed commits: author,
  trailers, refusal of caller-written trailers, canonical-branch commit refusal
  and the paths committed.
- **Does not own** — what a sync publishes and the ownership it records
  (publication, `project/instrukt/spec/domain/publication`); the
  editor schemas and modelines init asks publication to write (publication,
  `project/instrukt/spec/feature/publication/editor-schemas`); graph
  generation for a
  remote-main revision (blueprint, `project/instrukt/spec/domain/blueprint`);
  the project layer's schema, the runtime choice and the agents' git authors
  (configuration, `project/instrukt/spec/domain/configuration`); a work
  item's existence (work, `project/instrukt/spec/domain/work`); the
  project's lint, test and other hooks and actions (the project). Recorded
  assignment scope and grants are supplied by the shared admission interface;
  verified App/bot author facts come through the GitHub boundary's interface
  wired by the CLI, without core importing work or runner.
- **Published contracts** — `instrukt init`;
  `instrukt hooks install|remove|status`; `instrukt hooks run <hook>`;
  `instrukt repo commit`;
  `instrukt init --slug <slug>`; the claims report; the section markers;
  the trailer keys.
- **Allowed dependencies** — configuration (project slug, runtime choice,
  agents' git authors, module switches, as customer); publication (the ownership record the claims report reads, the
  published `sdlc` `schema.yaml` init copies, and the editor-schema
  operation init invokes, as customer); publication and
  blueprint (started by hook runs, as customer); work (todo existence through
  the core-declared interface wired by the CLI only when todos is enabled).
- **Implementation roots** — `crates/instrukt-core/` (repository);
  `crates/instrukt-cli/` (command surface).

Company home adoption extends this boundary:

- **Owns** — the sequence of a company home adoption; the creation intent
  and the staging checkout, their resumption and their removal; composition of the root commit; the clone
  of the company's repository as the home; the hook sections in the home; the
  generation lock under which adoptions take turns on a computer.
- **Does not own** — whether the home is created, checked out or refused,
  the repository's name and the skeleton's project layer and ignore lines
  (configuration, `project/instrukt/spec/domain/configuration`); the rendered
  `AGENTS.md`, the company's skill and catalog, the project layer's modeline
  and the sync (publication, `project/instrukt/spec/domain/publication`);
  every call to GitHub (work's GitHub boundary,
  `project/instrukt/spec/domain/work`); the credentials git pushes and clones
  with, which are the computer's own, as for every git operation instrukt runs.
- **Published contracts** — `instrukt init --company <key> [--name <name>]
  [--admin <login>]`
  (`project/instrukt/spec/feature/repository/project-initialization`).
- **Allowed dependencies** — configuration (the generation decision and the
  settings-document operation, as customer); publication (the renderings, the
  editor-schema operation and sync, as customer); work's GitHub boundary
  (creating the repository, its administrator and its default branch, through
  an interface core declares and the CLI wires, as for the verified bot
  author).

<!-- planned:company-home-integration -->
The `post-commit` hook run also hands each commit to the company context's
proposal recording, through an interface core declares and the CLI wires, in
the session and the repository the commit was made in. The run decides
nothing about it: whether the commit is a knowledge proposal is the company
context's (`project/instrukt/spec/domain/company`). Unlike the sync this run
starts detached, the handing waits for the recording, because the speaker it
records exists only while the session's turn is in progress and a detached
run can outlive the turn. The commit therefore waits for:

- one read of the session's turn in progress, retried within a few seconds'
  bound while the runtime or the state database does not answer;
- the retirement checks of the company's standing entries, one containment
  and one ref check each, computed before any write and bounded by the
  administrator-asked commits on branches the remote still holds;
- one short write transaction of the company's proposal store, and at most a
  concurrent recording's transaction on the same store.

A recording that could not be completed is recorded in the hook run's outcome
(`project/instrukt/spec/feature/repository/hook-sections`), and the run never
fails the commit, as no section does.
<!-- /planned:company-home-integration -->

`instrukt repo reclaim` shares the `repo` namespace and is not this context's:
it reclaims the regenerable output of a todo's worktree and belongs to runner
(`project/instrukt/spec/domain/runner`).

## Canonical fields

### Aggregates

#### Hook section (aggregate)

- **Root:** instrukt's section in one hook file.
- **Contains:** the section markers and the one hook-run call between them.
- **Invariants:**
  - A hook file holds at most one instrukt section.
  - No instrukt operation changes a byte of a hook file outside its section,
    except a damaged marker line. Markers pair top to bottom: an opening marker
    while one is open leaves the earlier one unterminated, a closing marker with
    none open is stray, a closing marker with one open closes a section. Every
    complete section is instrukt's and all of them become one fresh section at the
    first one's place (none, for removal). An unterminated or stray marker line is
    kept as the comment `# instrukt: damaged marker kept (was: <marker>)`; the lines
    after it are not touched, including lines identical to instrukt's own, and a
    run line of instrukt's among them is named as a duplicate. The repair is
    reported per file with the lines it changed and kept, `hooks status` names a
    kept damaged marker for as long as it is there, and one damaged file never
    stops another.
  - A section never `exec`s a command (never replaces the shell) and never
    ends the hook early, so every section after it runs; a redirection-only
    `exec` is allowed, and the pre-push section uses one to give the sections
    after it the full ref list on stdin.
  - A section never fails the git operation. The `pre-commit` section waits
    for its staging; publication triggers remain detached.
- **Identifier:** the hook name within the repository's hooks directory.
- **Lifecycle:** written by `instrukt hooks install` or `instrukt init`;
  removed by `instrukt hooks remove`; lost when a hook manager regenerates the
  file, which `instrukt hooks status` reports.

#### Adoption (aggregate)

- **Root:** the adopted repository.
- **Contains:** the project layer `instrukt.yaml`, `docs/project/` with its
  `index.yaml` and `schema.yaml`, the ignore
  line for `.instrukt/`, `.instrukt/schemas/` and the modelines, the hook
  sections.
- **Invariants:**
  - Adoption never overwrites a file that exists, apart from the modeline
    publication keeps at the top of the files it describes.
  - Adoption publishes and claims nothing; its claims report only reads.
  - Running it again on an adopted repository changes nothing.
  - In a company repository
    (`project/instrukt/spec/domain/configuration`), adoption writes no
    `docs/project/`, no project index and no schema copy: the repository's
    documentation is the company's knowledge.
- **Identifier:** the repository.
- **Lifecycle:** created by `instrukt init`; completed by a later run when a
  part is missing.

#### Company home adoption (aggregate)

- **Root:** the company's home on this computer, and, while the adoption is
  under way, its staging checkout.
- **Contains:** the creation intent; the staging checkout with the root
  commit; the home's checkout; its hook sections.
- **Invariants:**
  - The home folder comes into being only as a clone of the company's
    repository, in one step: the clone is made beside the staging checkout,
    given its settings document and its hook sections there, and moved into
    place whole, after which the home is synced when its repository holds a
    project layer. An interrupted adoption
    leaves no home folder behind, only its staging, which the next adoption
    of that company resumes.
  - Nothing is written into a home folder that exists, and an adoption that
    finds one is refused before anything else happens.
  - Whether the company's repository exists is read with git, with the
    computer's own credentials, as the clone that follows reads it: a
    repository the computer reaches is checked out by contract, and one it
    cannot reach is created when configuration's decision says so.
  - A creation starts with its intent: once its prerequisites are checked,
    and before the root commit is composed or anything on GitHub is changed,
    the adoption writes the creation
    intent, `~/.instrukt/state/company-generation/<key>/intent.yaml`, whole and
    atomically. It names the owning organisation, the repository, the company's
    name and its GitHub administrator, and it is what every later step of that
    creation reads them from, a resumed one included.
  - The root commit is composed in the staging checkout, holding the skeleton
    configuration and publication supply, before the repository is created.
  - An adoption that finds a creation intent for the company resumes it
    before it reads anything else, with the organisation, repository and
    administrator the intent names. It asks GitHub to create the repository and
    takes a refusal because the repository exists as no answer yet; it then
    pushes the root commit without force. A push that the repository accepts,
    or that finds the commit already there, completes the creation: the default
    branch is set, the administrator is made and the home is cloned. A push the
    repository refuses because it holds other history makes it a repository by
    contract: the staging checkout is removed and the home is cloned, with
    nothing more changed on GitHub.
  - Without a creation intent, a creation GitHub refuses because the
    repository exists makes it a repository by contract before the root
    commit is pushed: the staging checkout is removed and the home is cloned,
    with nothing changed on GitHub.
  - The root commit is attributed as every attributed commit is: its author
    and `Session:` from the acting agent where they resolve, and
    `Human-Involved:` as the caller of `instrukt init --company` declares it
    with `--human-involved`, `false` when it declares nothing
    (`general/policy/human-involvement-recording`). Nothing about the caller's
    identity sets that value, and a resumed creation keeps the root commit it
    finds, trailer included. It is the one commit instrukt composes on a
    default branch, the commit that creates that branch; from then on the
    branch has one writer, the integrator.
  - Adoptions take turns: one company home adoption runs on a computer at a
    time, holding `~/.instrukt/state/company-generation.lock` for its run.
  - The home is ready when the adoption ends: its settings document names the
    company and its hook sections are installed. A home whose repository
    holds a project layer has been synced, and a sync that refuses is
    reported while the home stays. A home whose repository holds no project
    layer, as one made before generation may, is not synced: the adoption
    reports that neither publication nor automatic review applies there, and
    the home stays.
- **Identifier:** the company's key.
- **Lifecycle:** started by `instrukt init --company <key>`; the staging
  checkout, its creation intent among it, is removed once the home is in
  place; complete when the home is ready. The home is configuration's
  company home from then on.

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

Independent review binds the complete worktree head before integration. Work
records preparation and build review evidence; repository owns no per-commit
notes ref or direct-main review exemption. Only the integrator advances main
through the verified candidate gate.

### Domain events emitted

None. Hook runs start other contexts' commands directly.

### Domain events consumed

None. Git invokes hook files; a developer or agent invokes the commands.

## Event subjects and origins

This context emits no events.

## Use cases

- **hook-sections** — `project/instrukt/spec/feature/repository/hook-sections` — instrukt's hook sections live beside the project's own hooks and keep publication current without blocking git.
- **attributed-commit** — `project/instrukt/spec/feature/repository/attributed-commit` — an agent's commit names the agent, its session and the human's involvement; outside a session it is the developer's own.
- **project-initialization** — `project/instrukt/spec/feature/repository/project-initialization` — a developer adopts instrukt in a repository in one idempotent step and sees what it will claim.

## Governing designs and decisions

- **`project/instrukt/design/architecture/overview`** — git hooks
  belong to everyone; instrukt touches only its marked section and never
  blocks git after the fact.

## Context map position

- **configuration** — Customer / Supplier. Configuration supplies the project
  slug, runtime choice, role/App references, non-bot agent git authors and the
  blueprint switch. Bot author resolution follows
  `project/instrukt/spec/feature/work/github-apps`, consumed through CLI wiring.
- **publication** — Customer / Supplier. Post-hook runs start
  `instrukt sync`; publication decides what it does. Publication supplies the
  ownership record and published `sdlc` schema that `init` reads, and the
  editor-schema operation `init` invokes.
- **blueprint** — Customer / Supplier. With blueprint enabled, hook runs hand
  a push to main or a pull advancing main to blueprint
  (`project/instrukt/spec/feature/blueprint/graph-generation`).
- **work** — Customer / Supplier. Recorded and planned associations are named
  in `Todo:` and `Planned-Todo:` trailers; work owns whether the item exists.
<!-- planned:company-home-integration -->
- **company** — Customer / Supplier. The `post-commit` hook run invokes the
  company context's proposal recording, declared by core and wired by the CLI.
<!-- /planned:company-home-integration -->

- **workflow** — Customer / Supplier. Repository supplies candidate integration
  branches and exact-commit main advancement.
- Core declares the optional todo-existence and graph-generation interfaces;
  the CLI wires work and blueprint implementations when their modules are on.
- **work's GitHub boundary** — Customer / Supplier. Company home adoption has
  the company's repository created, its administrator made and its default
  branch set through an interface core declares and the CLI wires
  (`project/instrukt/spec/feature/work/github-apps`, Company repositories).

## See Also

- [project/instrukt/spec/domain/context-map](context-map.md)
