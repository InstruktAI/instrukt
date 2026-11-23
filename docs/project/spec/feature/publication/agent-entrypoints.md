---
description: Rendered agent instructions — a repository's `AGENTS.md` files are committed, rendered at each commit from the repository's committed content alone (a project's root file is its purpose and the rendering of `docs/project/index.yaml`, doctrine another repository publishes named by the command that reads it), a company home's file is its master's own content, a check in CI catches a commit made without the hook; and the owner of `docs/general/` renders a global file for each agent runtime the selected runtime reports as configured, in that runtime's own home, with per-runtime `agents` snippets, written whole on every sync with no record of what was written, merging exactly the entrypoint-loading settings keys.
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
always-on `general` content where a native loader is established, one line
per on-demand domain with the command
that loads it, and that runtime's own `agents` snippets. It sets the one
settings key that makes a runtime load `AGENTS.md` files. Every file instrukt
renders is its own: it is written whole each time, whatever it held, and no
record is kept of what was written. There is no global master and there are
no addenda.

## Canonical fields

The root `AGENTS.md` also lists the project's own procedures that declare
subscriptions, under “Configured Responsibilities”, whether or not events is
enabled on the computer that renders it. No branding line is emitted.
instrukt writes no project override instruction file, such as Codex's
`AGENTS.override.md`: the `AGENTS.md` beside a master is the only file
generated there (`general/policy/agent-artifact-governance`). An
`AGENTS.override.md` beside a root master is a finding of the committed
rendering, naming the file: Codex reads it in place of `AGENTS.md`, so it
hides every committed update. instrukt never deletes it, since it may be a
person's own (`general/policy/agent-artifact-governance`).

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
  where `instrukt docs get` resolves the value. `@exec`, `@include` and a
  reference left for an agent program to resolve are findings as everywhere
  (`general/policy/agent-artifact-governance`: a reference a rendering would
  leave is a finding, and a committed `AGENTS.md` with one keeps its previous
  content).
- **Written whole.** A rendering is written over whatever `AGENTS.md` beside
  its master holds, a file a person wrote included; `instrukt init` keeps a
  copy of a hand-written root file first
  (`project/instrukt/spec/feature/repository/project-initialization`). An
  `AGENTS.md` whose master no longer exists is removed only while it opens
  with the generated line
  (`general/policy/agent-artifact-governance`: the `AGENTS.md` beside a
  master is instrukt's; what a person writes into it by hand is replaced at
  the next commit, and a person's own instructions belong in documentation).

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
workflow that publishes the company's skill on main also writes the
repository's `AGENTS.md`, with the shared workflows repository's own renderer,
and commits to main only when what it rendered differs from what main holds;
that workflow and its renderer belong to the shared workflows repository
(`project/instrukt/policy/shared-workflows`).

### Company home entrypoint

A company home (`project/instrukt/spec/feature/configuration/company-homes`)
is the checkout of the company's repository, on a computer and in a container
alike. It holds the company's own knowledge: `AGENTS.master.md` at its root and the
company's documents in its `docs/` folder. Its committed `AGENTS.md` is that
master rendered. As instrukt renders it, it holds the generated line, the
company passage, the master's own content, and each of its required reads in
full. A supported runtime that the global agent files'
preparation sets to load project instruction files (Global agent files,
below) loads it from the home it starts in, so a session of such a runtime in
the home starts with it: conversation sessions and event-started work alike,
and an agent that checks the company's repository out elsewhere. The
company-assistance procedure reaches the agent through its role
(`project/instrukt/spec/feature/company/conversations`); it is instrukt's
doctrine, not company knowledge. instrukt writes no other instruction file in
the home and leaves any other there as it is.

- **What makes a repository a company home.** The rendering mode comes from
  what the working tree's root holds, tracked or not:
  - a root `instrukt.yaml` that declares `company` is a company's settings
    document: company rendering — the company's home, where that file is
    present and ignored;
  - any other root `instrukt.yaml` is the project layer: project rendering,
    from the moment `instrukt init` writes it
    (`project/instrukt/spec/feature/repository/project-initialization`);
  - no root `instrukt.yaml` and a root `AGENTS.master.md`: company rendering —
    a clone of the company's repository, or its CI.

  A project's root master never stands without its layer: initialization
  writes the two together, and the layer is committed with the project. A
  company's home and a clone of its repository therefore render the same
  bytes. A root `instrukt.yaml` that cannot be read is a finding naming it,
  handled as every finding is: the mode is never guessed from a file that
  cannot be read (`sdlc/principle/design-fundamentals`). A company's settings
  document tracked by mistake changes no rendering; configuration validation
  reports the tracked file
  (`project/instrukt/spec/feature/configuration/company-homes`).
  <!-- planned-change:update-company-passage -->
- **The company passage.** Between the generated line and the master's
  content, every company rendering carries one fixed passage: an agent
  working in this home may change the company's documents here and commit
  them, and the change reaches the company's agents through the
  repository. The passage names the home and no repository host or
  organization. It is the same for every company and is owned by this spec.
  <!-- change:update-company-passage -->
- **The company passage.** Between the generated line and the master's
  content, every company rendering instrukt writes carries one fixed passage.
  It says where the knowledge comes from: the company's repository, its
  master and the Markdown documents under `docs/`; and that the checkout of
  that repository at `~/.instrukt/companies/<slug>/`, `<slug>` being the
  company's key, is the company's home, which serves the company's agents and
  stays on the repository's default branch. It directs an agent with access to the
  repository to improve the knowledge by editing the Markdown documents
  (`*.md`) under `docs/`, subfolders included, on a branch, in a separate
  worktree of the home or in a clone of the repository, and to open a pull
  request that someone other than its author reviews before it merges to the
  default branch, through which the change reaches the company's agents:
  company home knowledge is authored apart from the serving checkout and
  independently reviewed before it reaches the default branch
  (`general/policy/documentation-discipline`). It gives `company-<slug>` as
  the repository's conventional name, which a company's repository need not
  carry. It says that the change is made in those documents alone,
  `AGENTS.md` and any skill published from them being generated output that
  is left as rendered, and what each document carries: a non-empty
  `description` in its frontmatter and what is true of the company now, in
  the present tense, what was true before staying in the repository's
  history (`general/procedure/doc-snippet-authoring`). The path and the
  repository name carry `<slug>` as literal text: the passage takes no
  company's key, names the home and no repository host or organization, and
  is the same for every company, in a home and in a clone of the repository,
  wherever instrukt renders it. It is owned by this spec; a file the shared
  workflows repository's publisher writes carries that publisher's own
  passage (Who renders it, below).
  <!-- /planned-change:update-company-passage -->
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
  does not exist, or that reaches its file through a symbolic link, the
  file's own or a folder's above it, `docs/` included; so are required reads
  that form a cycle, named by the documents in it. A reference in the text
  is an `@` reference in a reference's shape — holding a `/` or ending in
  `.md` — a Markdown link to a file, or a See Also entry; one naming a
  company document stays in the text, and a bare `@word` mentions someone
  and is prose. `@config`, `@exec` and `@include` are findings too, wherever
  they stand, the required reads included.
  `instrukt config validate --company <name>` and `instrukt docs render` in
  the home report them, and a home with a finding is not rendered.
- **Who renders it.** instrukt renders it in two places: the `pre-commit`
  section where it is installed in the home, and `instrukt docs render` run
  in the home or in a clone of the repository. The block order above and the
  same bytes in a home and in a clone hold for instrukt's rendering, and
  `instrukt docs render --check` compares the working file with that
  rendering. The company repository's workflow on main commits an `AGENTS.md`
  as well, rendered by the shared workflows repository's publisher with its
  own renderer; what that file holds is that repository's contract
  (`project/instrukt/policy/shared-workflows`). The runner starts sessions in
  the home as it stands and renders nothing there.
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
     maps to the driver's exact, case-sensitive name, in full with their
     closure, appended to that runtime's file only. Claude appends `claude`
     then `claudeAgent`, sharing the shown-id set so closures appear once.
     An ACP registry agent id is diagnostic identity, never a selection alias.
- **Rendering.** Required reads resolve from the documentation published on
  the computer and are inlined; nothing is left for an agent program to
  resolve. `@config` directives resolve against the publishing project's
  effective configuration. A global file that keeps a reference left for an
  agent program to resolve is written as rendered, and the sync reports a
  finding naming the file and the reference and goes on: `general`'s
  `expanded` can bring in a snippet another project published, which this
  sync does not validate again, and one such snippet stops no other file
  (`general/policy/agent-artifact-governance`: a global file with such a
  reference is written as rendered and named, and the sync goes on;
  `general/principle/trust-and-guardrails`: the next sync rebuilds the file
  once the snippet is corrected, so a refusal would guard nothing).
- **A domain without `index.yaml`** is absent from every global file; the
  sync reports a notice naming it, and it is found through
  `instrukt docs index --domain <x>`.
- **Targets come from the selected runtime.** The sync asks the runtime that
  `runner.runtime` selects
  (`project/instrukt/spec/feature/configuration/layered-configuration`),
  through its adapter, which enabled instances of which agent runtimes are
  configured and which home each reports, and writes a file for each of
  those runtimes with an established native loader; every other enabled
  instance receives an explicit non-delivery outcome. For T3 the adapter
  reports its enabled
  provider instances (`project/instrukt/spec/feature/runner/session-continuity`).
- **Each instance's own home.** A destination is taken only from what the
  runtime reports. A runtime's file is written in the home an enabled
  instance reports. An enabled instance that reports no home receives its
  file in the runtime vendor's default home, and the sync's output names
  that instance and says its home was not reported. Instances with compatible
  selections that share a target share one file; different selections reaching
  one physical file are reported as a target conflict and that target receives
  nothing. An instance with a home of its own receives its file there. An instance whose home the runtime names relative to its own working
  directory, which it does not report, has no known home: the sync names it
  and writes no file and no link for it, never resolving such a path against
  its own working directory.
- **What the sync reports.** Every sync that writes global files names in
  its output each destination it wrote or found current, with the runtime
  and the instances it serves, and each link it ensured, beside the notices
  this section names. That output is where the destinations show:
  `instrukt status` keeps no record of them and reads current configured
  instances and target availability
  (`project/instrukt/spec/feature/publication/instrukt-status`), and
  `instrukt init` reports the ones a first sync will write
  (`project/instrukt/spec/feature/repository/project-initialization`).
- **Links where an instance reads through another folder.** Besides the home
  that receives the file, the runtime reports, per instance, each folder in
  which a link to that file must exist for the instance to read it
  (`project/instrukt/spec/feature/runner/session-continuity`). Publication
  ensures there the link the runtime itself makes: the file's name, pointing
  at the file in the home. A file there under that name, or a link pointing
  elsewhere, is replaced by that link; no link is ever removed, and nothing
  is recorded (`general/policy/agent-artifact-governance`: a runtime's
  folders are deployment targets, never a source). The adapter
  reports the folders; publication knows nothing else of the runtime.
- **Nothing is removed.** A global file stays where it is when its runtime is
  no longer configured, when an instance moves to another home, and when the
  project stops publishing `docs/general/`. Nothing records which files
  instrukt wrote, and nothing is removed on the strength of a memory
  (`general/policy/agent-artifact-governance`: the sync keeps no record of
  what it wrote and never removes a user-scope instruction file).
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
- **Native loading at T3 v0.1.8.** The adapter resolves `H` from the selected
  runtime's home facts. The native destinations are:

  | Driver | Destination | Home and format |
  | --- | --- | --- |
  | `codex` | `H/AGENTS.md` | Reported shared home or `.codex`; plain Markdown; retain shadow links |
  | `claudeAgent` (`claude`) | `H/CLAUDE.md` | `homePath`, `CLAUDE_CONFIG_DIR`, otherwise `.claude`; plain Markdown |
  | `cursor` | `H/rules/instrukt.mdc` | T3 host user's `.cursor`; fixed frontmatter `description: Instrukt doctrine`, `globs: ""`, `alwaysApply: true`, followed by the full rendering |
  | `grok` | `H/AGENTS.md` | Default `.grok`; no selected relocation variable; plain Markdown |
  | `fx` | `H/AGENTS.md` | Instance `FX_HOME`, otherwise `.fx`; plain Markdown |
  | `opencode` | `H/AGENTS.md` | `OPENCODE_CONFIG_DIR`, otherwise `.config/opencode`; plain Markdown |
  | `antigravity` | `H/AGENTS.md` | Default `.gemini`; no selected relocation variable; plain Markdown |
  | `kiro` | `H/steering/instrukt.md` | Instance `KIRO_HOME`, otherwise `.kiro`; plain Markdown |
  | `pi` | `H/AGENTS.md` | Instance `PI_CODING_AGENT_DIR`, otherwise `.pi/agent`; plain Markdown |

  Defaults use the inherited machine user home, never Instrukt's private output
  directory. The only selected relocation variables are `CODEX_HOME`,
  `CLAUDE_CONFIG_DIR`, `OPENCODE_CONFIG_DIR`, `FX_HOME`, `KIRO_HOME` and
  `PI_CODING_AGENT_DIR`. Effective T3 translations into Codex/Claude native
  variables are retained. Nonsecret instance values precede inherited values
  where T3 passes them. Relative or redacted selected overrides are
  `home_unknown`; no default is guessed. Instance `HOME`/`USERPROFILE`, OpenCode
  XDG, Grok variables and private Antigravity profiles do not select targets.
  Relocated Claude/fx output carries `instruction_relocation_unverified`: the
  chosen directory is relocated, but instruction loading is not proven. Cursor's SDK does not take an instance home override. Antigravity
  forces `GEMINI_HOME` to its private profile, but Instrukt does not follow it: its
  default output carries `runtime_home_differs` and does not establish receipt.
  Claude retains the narrowly merged `settings.json` key
  `pluginConfigs.agents-md@builtin.options.instructionFiles` with value
  `claude-md-and-agents-md`. No other row requires a loading setting.
- **Explicit non-delivery.** Muse, generic `acpRegistry` (including a registry
  agent named `gemini`) and unfamiliar drivers have no established generic
  global loader: report `not_delivered`, `mechanism_unestablished`, runtime,
  instance and available registry identity. Unknown homes report `home_unknown`.
  A nonempty native `AGENTS.override.md` shadowing Codex or Pi doctrine reports
  `native_override`; overrides are never changed or deleted. No file, link or
  settings are written for a skipped instance. Previously written files stay.
  Sync and init name each skipped instance and say nothing was delivered;
  a configured instance with no target is distinct from no configured runtime.
  No start-time injection, guessed file, settings database edit or user-message
  prepend substitutes for a missing native mechanism.
- **No runtime, no files.** An unset `runner.runtime` selects the runtime
  compiled into this instrukt
  (`project/instrukt/spec/feature/configuration/layered-configuration`), and
  the sync asks it. When `runner.runtime` names no compiled adapter, no agent
  runtime is configured and no global file is written. When the selected
  runtime cannot be asked, no global file is written in that sync: it
  reports a notice naming the runtime and the cause, publishes the rest, and
  the next sync of that project writes them, whether or not main has moved.
- **`agents` names** use the open syntax in the position-index contract,
  independently of configuration or native-loader knowledge.
- **Written whole, every time.** Each destination is written whole by every
  sync that publishes, whatever it holds: a file another tool or a person
  wrote there is overwritten. Nothing is refused because of what a file holds
  (`general/policy/agent-artifact-governance`: the global agent files are
  instrukt's; what a person writes into one by hand is replaced at the next
  sync, and a person's own instructions belong in documentation — a snippet
  `docs/general/index.yaml` selects, or an `agents` snippet for that runtime
  — from which the file is rebuilt every time).
- **Settings merge.** Only the entrypoint-loading key is set; every other key
  in the settings file, and its other values, stay as they were. A missing
  settings file is created holding only that key. A settings file that cannot
  be parsed is left untouched and named, and the sync goes on: that file is
  the user's (`general/policy/agent-artifact-governance`: an agent's user
  settings belong to its user).
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

### UC-AE2: A developer sees native instruction delivery and explicit non-delivery for configured agent runtimes

```gherkin
Given a developer's runtime has several agent runtimes configured, some with homes of their own, and another agent program is installed but not configured
And the computer publishes always-on doctrine and an on-demand domain with a description and a triggered procedure
And the agent runtimes' settings hold the developer's own preferences
And one configured agent runtime's global instruction file holds text another tool wrote
When publication runs for the project that owns the always-on doctrine
Then each configured agent runtime with an established native loader receives the always-on doctrine in full in its native format and home
And every configured instance without an established loader or home is named with its non-delivery reason
And the text the other tool wrote is gone
And always-on doctrine taken from another domain sits under that domain's heading, in the order the doctrine requires it, even where the domains interleave
And it reads one line naming the on-demand domain, its description and the command that loads it
And it reads the domain's triggered procedure with the command that loads it with its closure
And each configured agent runtime that needs it is set to load project instruction files
And every other setting the developer had is unchanged
And the agent program that is not configured receives nothing
But when the always-on doctrine changes and publication runs while the runtime cannot be asked
Then the rest of the documentation is published
And no global instruction file is written, and the developer is told which runtime could not be asked and why
When publication runs again while the runtime can be asked
Then every configured agent runtime with an established native loader receives the changed doctrine
```

### UC-AE3: A developer gives one agent runtime rules of its own

```gherkin
Given a developer's runtime has two agent runtimes configured
And the always-on doctrine maps a policy to one of them
When publication runs for the project that owns the always-on doctrine
Then that runtime's global instructions end with the policy
And the other runtime's global instructions do not contain it
When the developer maps a policy to another syntactically valid provider name
Then the index remains valid independently of configured instances
But when the provider name is malformed
Then publication is refused and explains the name syntax
```

### UC-AE4: Publication never overwrites global agent instructions the developer wrote (retired)

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

### UC-AE13: An agent runtime the developer stops using no longer receives instructions (retired)

### UC-AE14: A developer whose agent runtimes could not be asked is told, and the next publication catches up (retired)

## Known caveats

- Global files are never removed. The file of a runtime that is no longer
  configured, of a home an instance no longer uses, or of a project that
  stopped publishing `docs/general/` stays where it is, with the doctrine of
  the last sync that wrote it, until the developer deletes it.
- A global file the developer edits is overwritten by the next sync that
  publishes; an edit that should last belongs in documentation.
- A committed rendering names doctrine another repository publishes by its id
  and command. An agent where that doctrine is not published reads the
  project's own content in full and those lines only.
- The rendering depends on the instrukt release that renders it. A CI check
  running another release than the developer's hook may report a difference
  the developer's commit did not cause; CI pins the release it installs.
- instrukt and the shared workflows repository's publisher each write a
  company repository's `AGENTS.md` whole, from the same master and documents,
  and no contract makes the two renderings the same bytes. The publisher
  commits its rendering to main; the hook section stages instrukt's into the
  commit being made; `instrukt docs render` writes instrukt's to the working
  file, and `instrukt docs render --check` compares the working file with
  it. In a checkout whose working file is the one the publisher committed,
  the check names `AGENTS.md` as differing wherever the two renderings
  differ, although no document changed. After `instrukt docs render` writes
  the file there, the check reports no difference for `AGENTS.md` while the
  commit still holds the publisher's file; it exits zero only where it also
  names no other entrypoint and no finding.
- Between a commit made without the hook and the commit that repairs it,
  every checkout carries the stale rendering, and a company home started from
  it reads the previous knowledge.
- A home a T3 instance takes only from T3's own process environment is not
  reported to instrukt. Such an instance receives its file in the vendor's
  default home, which the sync names; where it actually runs from another
  home, that home's file is current only up to the last sync that could
  write it.
- On a T3 release newer than the pinned one, a Codex instance with an
  overlay home and a `CODEX_HOME` in its instance environment but no home
  setting is read as the pinned release reads it: its file is written in the
  default home and the overlay is linked to it. Such a release takes that
  `CODEX_HOME` as the shared home instead; when that folder holds an
  `AGENTS.md` of its own, T3 points the overlay's link at it when it next
  creates the provider, and the instance reads that file until the next sync
  links it again.
- After a sync during which the selected runtime could not be asked, the
  global files stay as they were until the next sync of the project that owns
  `docs/general/`: a hook trigger there, or `instrukt sync` run in it
  (`project/instrukt/spec/feature/publication/project-publication`).
- Native disable switches, size limits and overrides retain their own behavior.
  A file being present does not establish that a session loaded its current bytes.
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
- Each runtime's user-scope location, from its vendor: Claude Code https://code.claude.com/docs/en/memory; Codex https://learn.chatgpt.com/docs/agent-configuration/agents-md; pi https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/configuration.md; OpenCode https://github.com/anomalyco/opencode/blob/dev/services/site/src/docs/content/instructions.mdx; Kiro https://kiro.dev/docs/steering/; Antigravity https://antigravity.google/docs/rules; fx https://fx.sh/docs/configure-fx/project-instructions; Grok Build https://docs.x.ai/build/features/project-rules. Cursor local user rules https://prod.cursor.com/help/customization/rules#where-are-rules-stored and native rule format https://cursor.com/docs/rules#rule-file-structure; Muse Code documents user rules without a location (https://dev.meta.ai/docs/muse-code/configuration).
- T3's configured provider instances and their settings: `packages/contracts/src/settings.ts` and `packages/contracts/src/providerInstance.ts` in `pingdotgg/t3code`.
