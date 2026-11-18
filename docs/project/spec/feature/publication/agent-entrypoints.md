---
description: Rendered agent instructions — a repository's `AGENTS.md` files are committed, rendered at each commit from the repository's committed content alone (a project's root file is its purpose and the rendering of `docs/project/index.yaml`, doctrine another repository publishes named by the command that reads it), a company home's file is its master's own content, a check in CI catches a commit made without the hook; and the owner of `docs/general/` renders a global file for each agent runtime the selected runtime reports as configured, in that runtime's own home, with per-runtime `agents` snippets, never over a file instrukt did not write, merging exactly the entrypoint-loading settings keys.
---
# Agent Entrypoints — Spec

## What it is

Agents receive instrukt's instructions through files their programs already
load. A repository's `AGENTS.md` files are part of the repository: instrukt
renders them at each commit from what the commit holds and stages them into
it, so every checkout carries them — on a computer without instrukt, in a
linked worktree, and in a cloud agent's clone. A project's root `AGENTS.md`
is the project's purpose and the selection in `docs/project/index.yaml`; a
company home's `AGENTS.md` is the company's own knowledge. The project that
owns `docs/general/` also renders a user-scope global file for each agent
runtime the user has configured, found by asking the selected runtime: the
always-on `general` content, one line per on-demand domain with the command
that loads it, and that runtime's own `agents` snippets. It sets the one
settings key that makes a runtime load `AGENTS.md` files, and never writes
over an instruction file instrukt did not write. There is no global master
and there are no addenda.

## Canonical fields

The root `AGENTS.md` also lists the project's own procedures that declare
subscriptions, under “Configured Responsibilities”, whether or not events is
enabled on the computer that renders it. No branding line is emitted. For a
supported runtime that reads a separate project override instruction file,
that file is retained alongside the ordinary entrypoint.

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
A rendering never inlines a pending snippet and drops planned material
(`project/instrukt/spec/feature/publication/project-publication`).
Only references outside code are resolved; a reference inside a code span or
fenced code block is left exactly as written and is never reported as
unresolved.

### Committed entrypoints

Every `AGENTS.md` beside an `AGENTS.master.md` in a repository — the root
file, a nested master's file and a company home's file — is committed. It
is rendered from the repository's committed content alone: the masters and
the documentation positions the repository holds. Nothing from the computer
enters it — not the documentation other repositories publish, not the
configuration layers, not the environment — so two computers rendering the
same content with the same instrukt release write the same bytes.

- **The generated line.** Each committed `AGENTS.md` opens with one line
  saying it is generated by instrukt from the master beside it and the
  documentation that master's rendering selects, and that changes go there.
  A file is a rendering only while it opens with that line.
- **What the repository holds is inlined.** A snippet in the closure of a
  rendered selection or of a nested master's required reads is rendered in
  full when it belongs to a position the repository holds: its own
  `docs/project/`, and `docs/general/` or a `docs/domain/<x>/` in its own
  tree.
- **What another repository publishes is named, not copied.** A required
  read of a snippet whose position the repository does not hold is rendered
  as one reference line, its id followed by `instrukt docs get <id>
  --closure`; a bare-domain read the repository does not hold, as the domain's
  name followed by `instrukt docs get --domain <x>`. The line carries nothing
  read from outside the repository: no description, no closure.
- **No configured value.** A `@config` directive in a master, or in a
  document a committed rendering holds in full, is a finding naming the
  document; the door is to list that snippet instead, so it is read on demand
  where `instrukt docs get` resolves the value. `@exec` and `@include` are
  findings as everywhere.
- **Never over a file instrukt did not write.** A rendering is written where
  `AGENTS.md` is absent or is a rendering. A file without the generated line
  is left as it is and named, with the door: move its content into the master
  or into documentation, then delete the file. A rendering whose master no
  longer exists is removed.

### Project entrypoints

- **The root master.** `AGENTS.master.md` at the repository root is the
  project's authored entry. It holds only `## Purpose`. A root master carrying
  `## Required reads` is a finding, naming the master and
  `docs/project/index.yaml` as the place that selects what is included.
- **The root `AGENTS.md`**: the generated line, the master's `## Purpose`,
  then the rendering of `docs/project/index.yaml` (`expanded`, then
  `listed`, then the project's triggered procedures), as above, then the
  configured responsibilities.
- **Without `docs/project/index.yaml`**, the root `AGENTS.md` holds the
  purpose, the project's triggered procedures and its configured
  responsibilities, and the rendering reports a notice naming the missing
  file.
- **Nested masters.** Every other `AGENTS.master.md` renders into the
  `AGENTS.md` beside it, from its own content with its required reads as
  above; it receives no project selection and no procedure lines.

### When a committed entrypoint is rendered

- **At commit.** instrukt's `pre-commit` hook section renders every
  entrypoint of the repository from the content being committed and stages
  each rendering that differs, and each removal, into the commit being made
  (`project/instrukt/spec/feature/repository/hook-sections`). A finding
  leaves that file as it is in the commit and is reported in the hook-run
  outcome; the commit proceeds.
- **`instrukt docs render`** renders every entrypoint of the repository from
  its working tree and writes them; it stages nothing.
  `instrukt docs render --check` writes nothing and exits non-zero naming each
  entrypoint whose file differs from its rendering, each missing one, and
  each finding. Neither needs a home, a publication or a configuration: a
  fresh clone and the binary are enough.
- **Never by sync or checkout.** A sync writes no entrypoint, and no hook
  renders one after a checkout
  (`project/instrukt/spec/feature/repository/hook-sections`).

### A commit made without the hook

A commit made in GitHub's web editor, by a cloud agent or anywhere the hook
section is not installed leaves its entrypoints as they were. The
repository's own CI runs `instrukt docs render --check`, which fails such a
commit naming each stale entrypoint; the repair is `instrukt docs render` and
a commit, or a commit through the hook. In a company's repository, the
workflow that renders the company's skills on main also runs `instrukt docs
render` and commits to main only when the rendering changed; that workflow
belongs to the shared workflows repository.

### Company home entrypoint

A company home (`project/instrukt/spec/feature/configuration/company-homes`)
is the checkout of the company's repository, on a computer and in a container
alike. It holds the company's own knowledge: `AGENTS.master.md` at its root and the
company's documents in its `docs/` folder. Its committed `AGENTS.md` is that
master rendered: the generated line, the company passage, the master's own
content, and each of its required reads in full. A supported runtime that the global agent files'
preparation sets to load project instruction files (Global agent files,
below) loads it from the home it starts in, so a session of such a runtime in
the home starts with it: conversation sessions and event-started work alike,
and an agent that checks the company's repository out elsewhere. The
company-assistance procedure reaches the agent through its role
(`project/instrukt/spec/feature/runner/conversations`); it is instrukt's
doctrine, not company knowledge. instrukt writes no other instruction file in
the home and leaves any other there as it is.

- **What makes a repository a company home.** A repository renders as a
  company home when its root holds `AGENTS.master.md` and its committed
  content holds no project layer: no tracked `instrukt.yaml` at its root. A
  project always commits its layer, and a company home keeps its settings
  document out of git, so the same committed content decides it in the home,
  in a clone and in CI.
- **The company passage.** Between the generated line and the master's
  content, every company rendering carries one fixed passage: an agent
  working in this home may change the company's documents here and commit
  them, and the change reaches the company's agents through the
  repository. The passage names the home and no repository host or
  organization. It is the same for every company and is owned by this spec;
  the company's skill rendering carries the same passage.
- **The company's documents.** A company document is a Markdown file in the
  home's `docs/` folder. The master and the company's documents name a
  company document as a required read by its path relative to the file that
  reads it, and the rendering holds each in full, with its own required
  reads, each before the document that requires it and each once. A company
  document has no snippet id, is not a documentation position and is never
  published.
- **The company's knowledge stands alone.** The master and the company's
  documents reference only the company's own documents. A required read, or
  a reference in their text, to a snippet or a domain — `general`, `sdlc` or
  any other — or to a file outside the home's `docs/` folder is a finding
  naming the document and the reference; so is a required read whose file
  does not exist. So are `@config`, `@exec` and `@include`.
  `instrukt config validate --company <name>` and `instrukt docs render` in
  the home report them, and a home with a finding is not rendered.
- **Who renders it.** The `pre-commit` section where it is installed in the
  home, `instrukt docs render` run in the home, and the company repository's
  workflow on main. The runner starts sessions in the home as it stands and
  renders nothing there.
- **Without a master**, the home has no rendering, and a rendering left
  there is removed.

### Global agent files

Rendered from what is published on the computer, and written only by the sync
of the project that owns `docs/general/`, at the end of a sync that
publishes. They are the computer's own, never committed anywhere.

- **Composition**, per agent runtime, in order:
  1. `general`'s section, rendered from `docs/general/index.yaml`. Its
     `expanded` and `listed` may name ids of any published domain.
  2. One group per other published domain that has an `index.yaml`, ordered
     by domain name: under `# <Name>`, the domain's `description` with the
     command `instrukt docs get --domain <x>`, then one line per procedure of
     that domain carrying an `activation_trigger`, as in step 4 above. A
     domain's `expanded` and `listed` are not rendered here; they are what
     `instrukt docs get --domain <x>` renders.
  3. That runtime's `agents` snippets: the ids `docs/general/index.yaml`
     maps to the runtime's name, in full with their closure, appended to
     that runtime's file only.
- **Rendering.** Required reads resolve from the documentation published on
  the computer and are inlined; nothing is left for an agent program to
  resolve. `@config` directives resolve against the publishing project's
  effective configuration.
- **A domain without `index.yaml`** is absent from every global file; the
  sync reports a notice naming it, and it is found through
  `instrukt docs index --domain <x>`.
- **Targets come from the selected runtime.** The sync asks the runtime that
  `runner.runtime` selects
  (`project/instrukt/spec/feature/configuration/layered-configuration`),
  through its adapter, which enabled instances of which agent runtimes are
  configured and which home each reports, and writes a file for each of
  those runtimes instrukt deploys to; for T3 the adapter reports its enabled
  provider instances (`project/instrukt/spec/feature/runner/session-continuity`).
- **Each instance's own home.** A destination is taken only from what the
  runtime reports. A runtime's file is written in the home an enabled
  instance reports. An enabled instance that reports no home receives its
  file in the runtime vendor's default home, and the sync's output names
  that instance and says its home was not reported. Instances that share a
  home share one file; an instance with a home of its own receives its file
  there.
- **Removing a file written earlier.** A global file instrukt wrote earlier
  for a runtime is removed only when instrukt establishes that no enabled
  instance of that runtime uses that home: the runtime has no enabled
  instance, or every enabled instance of it reports its home and none is
  that one. While any enabled instance of the runtime reports no home, every
  file written earlier for that runtime stays, and `instrukt status` shows
  each as kept, with that reason. A removed file is retracted as owned output
  is: only while its content is what instrukt wrote.
- **Destinations.** Publication writes each runtime's user-scope instruction
  file at the location that runtime loads in that home — a file, or one file
  of instrukt's own in a directory of instruction files. For a runtime
  requiring an entrypoint-loading setting, publication merges the setting
  that enables project instruction files alongside its own instruction
  files; a runtime that loads them already needs no settings change.
  Destinations and loading keys are constants of the publication
  implementation, one per runtime instrukt deploys to, each taken from that
  runtime vendor's own documentation (See Also); they are not configuration
  keys.
- **A configured runtime without a user-scope file** — one that keeps user
  rules in its own settings, one whose vendor documents no location for
  them, or one instrukt has no destination for —
  receives no global file. Its agents read the repository's committed
  `AGENTS.md`.
- **No runtime, no files.** Without a selected runtime, no agent runtime is
  configured: no global file is written, and the files written earlier are
  removed as above. When the selected runtime cannot be asked, nothing is
  established about any instance: the sync leaves every global file and its
  record as they are, reports a notice naming the runtime and the cause, and
  publishes the rest.
- **`agents` names** are validated against the runtimes instrukt deploys to,
  whatever is configured on the computer.
- **Never over a file instrukt did not write.** Before writing anything, the
  sync checks every destination: a file that exists and whose content is not
  the content the written record holds for it — a file instrukt never wrote,
  or one changed since instrukt wrote it — stops the sync before anything is
  published. The refusal names each such file and the door: move its content
  into documentation (a snippet selected by `docs/general/index.yaml`, or an
  `agents` snippet for that runtime) or move the file out of the way, then
  sync again. A missing destination is written.
- **Settings merge.** Only the entrypoint-loading key is set; every other key
  in the settings file, and its other values, stay as they were. A missing
  settings file is created holding only that key. A settings file that cannot
  be parsed is refused and left untouched, and the sync fails naming it.
- **No other agent configuration.** instrukt writes no skill, MCP server, hook
  or other agent setting. Agents reach declared MCP servers through
  `instrukt mcp` (`project/instrukt/spec/feature/runner/mcp-access`), which
  writes none either.

## Use cases

### UC-AE1: An agent that checks out a developer's project starts with what the project selected

```gherkin
Given a developer's project whose master states its purpose
And the project's index selects one snippet in full that requires doctrine another repository publishes, lists another snippet, and the project has a triggered procedure
When the developer commits through instrukt's hook
Then the commit carries one instruction file starting with the line that names its sources
And it holds the project's purpose and the selected snippet in full
And it names the other repository's doctrine with the command that reads it
And it lists the other snippet and the triggered procedure, each with the command that loads it
When an agent checks the repository out on a computer without instrukt
Then it starts with that instruction file
```

### UC-AE2: Every agent runtime the developer configured starts with the always-on doctrine and a map of the rest

```gherkin
Given a developer's runtime has two agent runtimes configured, one of them with a home of its own, and a third agent program is installed but not configured
And the computer publishes always-on doctrine and an on-demand domain with a description and a triggered procedure
And the agent runtimes' settings hold the developer's own preferences
When publication runs for the project that owns the always-on doctrine
Then each configured agent runtime starts with the always-on doctrine in full, read from the home it runs with
And always-on doctrine taken from another domain sits under that domain's heading, in the order the doctrine requires it, even where the domains interleave
And it reads one line naming the on-demand domain, its description and the command that loads it
And it reads the domain's triggered procedure with the command that loads it with its closure
And each configured agent runtime that needs it is set to load project instruction files
And every other setting the developer had is unchanged
And the agent program that is not configured receives nothing
```

### UC-AE3: A developer gives one agent runtime rules of its own

```gherkin
Given a developer's runtime has two agent runtimes configured
And the always-on doctrine maps a policy to one of them
When publication runs for the project that owns the always-on doctrine
Then that runtime's global instructions end with the policy
And the other runtime's global instructions do not contain it
When the developer maps a policy to an agent runtime instrukt does not deploy to
Then publication is refused and names that runtime and the ones instrukt deploys to
```

### UC-AE4: Publication never overwrites global agent instructions the developer wrote

```gherkin
Given a developer's computer where a configured agent runtime's global instruction file holds the developer's own text
When publication runs for the project that owns the always-on doctrine
Then publication stops and names that file
And it says to move the file's content into documentation or the file out of the way
And the file and the previously published documentation are unchanged
When the developer moves the file out of the way and publication runs again
Then that agent runtime starts with the rendered global instructions
```

### UC-AE5: A developer learns that a domain without an index is not on the agents' map

```gherkin
Given a developer's project with a domain that has no index
When publication runs for the project
Then the domain is published and the developer is told it has no index
And the agents' global instructions do not name it
And the domain's snippets are still found by listing that domain
```

### UC-AE6: An agent in a linked worktree receives the published baseline (retired)

### UC-AE7: A fresh primary checkout bootstraps publication once (retired)

### UC-AE8: An operator gives every agent working for a company the company's own knowledge

```gherkin
Given an operator keeps a company's knowledge in the master of its home, which is the company's repository
When the operator changes the master in the repository's web editor
Then the company's workflow commits the rendered instruction file to main
And an agent started in the home once the home holds that commit, and an agent that checks the repository out elsewhere, each start with the master's content
But when the operator's change names a required read of doctrine outside the company's own documents
Then the rendering reports the master and the read, and the committed instruction file is not changed
```

### UC-AE9: A developer's CI catches instructions a commit left stale

```gherkin
Given a developer's repository whose CI checks its rendered instruction files
When a commit made without instrukt's hook changes the project's index
Then the check fails and names the stale instruction file
When the developer renders the instruction files and commits them
Then the check passes
```

### UC-AE10: Two computers commit the same instructions for the same content

```gherkin
Given two developers' computers that publish different doctrine and hold different configuration
When each commits the same project content through instrukt's hook with the same instrukt release
Then both commits carry byte-identical instruction files
But when the project selects a snippet that reads a configured value
Then the rendering reports the snippet and says to list it instead, and the instruction file is not changed
```

### UC-AE11: An agent in a linked worktree reads the instructions of its branch (retired)

### UC-AE12: An operator keeps a company's knowledge separate from other doctrine (retired)

### UC-AE13: An agent runtime the developer stops using no longer receives instructions

```gherkin
Given publication wrote global instructions for two configured agent runtimes
When the developer removes one of them from their runtime and publication runs
Then that runtime's global instructions are removed and the other's are current
But when an enabled instance of the removed runtime remains whose home the runtime does not report
Then that runtime's global instructions are kept, and status shows them as kept because a home was not reported
But when the runtime cannot be asked during publication
Then every global instruction file is left as it was and the developer is told why
```

## Known caveats

- When the project stops publishing `docs/general/`, its next sync retracts
  the global files it wrote, like any other owned output; a file whose
  content is no longer what instrukt wrote is left in place and named.
- A global file the developer edits after instrukt wrote it stops the next
  rendering sync until the edit is moved into documentation or the file is
  moved away.
- A committed rendering names doctrine another repository publishes by its id
  and command. An agent where that doctrine is not published reads the
  project's own content in full and those lines only.
- The rendering depends on the instrukt release that renders it. A CI check
  running another release than the developer's hook may report a difference
  the developer's commit did not cause; CI pins the release it installs.
- Between a commit made without the hook and the commit that repairs it,
  every checkout carries the stale rendering, and a company home started from
  it reads the previous knowledge.
- A home a T3 instance takes only from T3's own process environment is not
  reported to instrukt. Such an instance receives its file in the vendor's
  default home, which the sync names, and the files written earlier for its
  runtime are kept while it reports no home; where it actually runs from
  another home, that home's file is current only up to the last sync that
  could write it.
- After a sync during which the selected runtime could not be asked, the
  global files stay as they were until the next sync that publishes them: a
  new revision of main in the project that owns `docs/general/`, or
  `instrukt sync --force` there. A sync of an unchanged main does nothing.
- How a runtime combines the rendering with other instruction files in a
  home, and whether it reads it again during a session or when it resumes
  one, is that runtime's.
- A company home's knowledge is its master and the company documents its
  required reads reach. Nested masters and documentation positions inside the
  home are not rendered or published.
- A start in a company home renders nothing and takes no docs lock, so it
  never waits on a sync that is publishing; it reads the `AGENTS.md` the
  home holds.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/feature/publication/project-publication](project-publication.md) — the sync the global files are part of.
- [project/instrukt/spec/feature/repository/hook-sections](../repository/hook-sections.md) — the `pre-commit` section that stages the renderings.
- [project/instrukt/spec/domain/docs](../../domain/docs.md) — the `index.yaml` model these renderings follow.
- Each runtime's user-scope location, from its vendor: Claude Code https://code.claude.com/docs/en/memory; Codex https://learn.chatgpt.com/docs/agent-configuration/agents-md; pi https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/configuration.md; OpenCode https://github.com/anomalyco/opencode/blob/dev/services/site/src/docs/content/instructions.mdx; Kiro https://kiro.dev/docs/steering/; Antigravity https://antigravity.google/docs/rules; fx https://fx.sh/docs/configure-fx/project-instructions; Grok Build https://docs.x.ai/build/features/project-rules. Cursor keeps user rules in its settings (https://cursor.com/docs/context/rules); Muse Code documents user rules without a location (https://dev.meta.ai/docs/muse-code/configuration).
- T3's configured provider instances and their settings: `packages/contracts/src/settings.ts` and `packages/contracts/src/providerInstance.ts` in `pingdotgg/t3code`.
